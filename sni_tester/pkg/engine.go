package pkg

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/oschwald/geoip2-golang"
)

type jobResult struct {
	domain  string
	success bool
	ip      string
	country string
	asn     uint32
	org     string
	info    string
}

type Engine struct {
	cfg         Config
	geoDB       *geoip2.Reader
	asnDB       *geoip2.Reader
	storage     *StorageManager
	dnsLimiter  *DNSRateLimiter
	timeoutCtrl *TimeoutController
	cancel      func()
}

func NewEngine(cfg Config) (*Engine, error) {
	geoDB, err := geoip2.Open(cfg.GeoDBFile)
	if err != nil {
		return nil, fmt.Errorf("failed to open GeoIP database: %w", err)
	}

	asnDB, err := geoip2.Open(cfg.GeoASNFile)
	if err != nil {
		geoDB.Close()
		return nil, fmt.Errorf("failed to open GeoASN database: %w", err)
	}

	storage, err := NewStorageManager(cfg.BadgerDBDir, cfg.TTLDays)
	if err != nil {
		geoDB.Close()
		asnDB.Close()
		return nil, fmt.Errorf("failed to create storage manager: %w", err)
	}

	return &Engine{
		cfg:         cfg,
		geoDB:       geoDB,
		asnDB:       asnDB,
		storage:     storage,
		dnsLimiter:  NewDNSRateLimiter(),
		timeoutCtrl: NewTimeoutController(),
	}, nil
}

func (e *Engine) Close() {
	if e.geoDB != nil {
		e.geoDB.Close()
	}
	if e.asnDB != nil {
		e.asnDB.Close()
	}
	if e.storage != nil {
		e.storage.Close()
	}
}

func (e *Engine) Stop() {
	if e.cancel != nil {
		e.cancel()
	}
}

func (e *Engine) Run(ctx context.Context, domains []string, cb ProgressCallback) (*Result, error) {
	ctx, cancel := context.WithCancel(ctx)
	e.cancel = cancel
	defer cancel()

	if e.cfg.ResetAll {
		if err := e.storage.ClearAll(); err != nil {
			return nil, fmt.Errorf("failed to clear history: %w", err)
		}
	}

	skipMap := make(map[string]struct{})
	if !e.cfg.ForceRetry {
		for d := range e.storage.LoadSuccessHistory() {
			skipMap[d] = struct{}{}
		}
		for d := range e.storage.LoadBlockedHistory() {
			skipMap[d] = struct{}{}
		}
	}

	now := time.Now().Unix()
	var toTest []string
	var stats Stats
	for _, d := range domains {
		if _, skipped := skipMap[d]; skipped {
			stats.Skipped++
			if cb != nil {
				cb(ProgressEvent{Type: "skipped", Domain: d, Stats: stats})
			}
			continue
		}
		if !e.cfg.ForceRetry && e.storage.IsFailedRecently(d, now) {
			stats.Skipped++
			if cb != nil {
				cb(ProgressEvent{Type: "skipped", Domain: d, Stats: stats})
			}
			continue
		}
		toTest = append(toTest, d)
	}

	jobs := make(chan string, 5000)
	results := make(chan jobResult, 2000)

	maxWorkers := 2000
	if e.cfg.FixedWorkers > 0 {
		maxWorkers = e.cfg.FixedWorkers
	}

	var wg sync.WaitGroup
	workerSem := make(chan struct{}, maxWorkers)
	for i := 0; i < maxWorkers; i++ {
		workerSem <- struct{}{}
	}

	initialWorkers := InitialWorkers
	if e.cfg.FixedWorkers > 0 {
		initialWorkers = e.cfg.FixedWorkers
	}

	spawnWorker := func() {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				select {
				case <-workerSem:
				case <-ctx.Done():
					return
				default:
					return
				}

				var domain string
				select {
				case domain = <-jobs:
				case <-ctx.Done():
					workerSem <- struct{}{}
					return
				}

				result := e.processDomain(ctx, domain)

				select {
				case results <- result:
				case <-ctx.Done():
					workerSem <- struct{}{}
					return
				}
				workerSem <- struct{}{}
			}
		}()
	}

	for i := 0; i < initialWorkers; i++ {
		spawnWorker()
	}

	go func() {
		for _, d := range toTest {
			select {
			case jobs <- d:
			case <-ctx.Done():
				return
			}
		}
		close(jobs)
	}()

	total := len(toTest)
	stats.Total = total
	countryDomains := make(map[string][]string)
	var failedDomains []string
	var resultList []DomainResult

	done := make(chan struct{})
	go func() {
		defer close(done)
		for r := range results {
			if r.success {
				countryDomains[r.country] = append(countryDomains[r.country], r.domain)
				stats.Success++
			} else {
				failedDomains = append(failedDomains, r.domain)
				stats.Failed++
			}
			resultList = append(resultList, DomainResult{
				Domain:  r.domain,
				Success: r.success,
				IP:      r.ip,
				Country: r.country,
				ASN:     r.asn,
				Org:     r.org,
				Info:    r.info,
			})

			if cb != nil {
				cb(ProgressEvent{
					Type:     "result",
					Domain:   r.domain,
					Success:  r.success,
					Country:  r.country,
					IP:       r.ip,
					Info:     r.info,
					Progress: float64(stats.Success+stats.Failed) / float64(total),
					Stats:    stats,
				})
			}
		}
	}()

	wg.Wait()
	close(results)
	<-done

	if !e.cfg.Debug {
		SaveBatch(e.cfg.OutputDir, countryDomains, e.storage.DB())
		if len(failedDomains) > 0 {
			e.storage.AppendFailureHistory(failedDomains)
		}
	}

	return &Result{
		DomainResults: resultList,
		Stats:         stats,
	}, nil
}

func (e *Engine) processDomain(ctx context.Context, domain string) jobResult {
	dnsTimeout := e.timeoutCtrl.GetTimeout("dns")
	dnsCtx, cancel := context.WithTimeout(ctx, dnsTimeout)
	defer cancel()

	start := time.Now()
	ips, err := ResolveWithFailover(dnsCtx, domain)
	e.timeoutCtrl.Record(time.Since(start), "dns")
	if err != nil || len(ips) == 0 {
		errMsg := "DNS resolution failed"
		if err != nil {
			errMsg = fmt.Sprintf("DNS failed: %v", err)
		}
		return jobResult{domain: domain, success: false, info: errMsg}
	}
	ip := ips[0]

	country := GetCachedCountry(ip, e.geoDB)
	if country == "" {
		country = "UNKNOWN"
	}

	if IsBlockedCountry(country) {
		asnRes, _ := GetCachedASN(ip, e.asnDB)
		if asnRes.ASN > 0 {
			e.storage.AddASNToBlocklist(asnRes.ASN, asnRes.Org, country)
		}
		e.storage.AddBlockedDomain(domain, "COUNTRY", country)
		return jobResult{domain: domain, success: false, ip: ip, country: country, info: "Country blocked"}
	}

	asnRes, _ := GetCachedASN(ip, e.asnDB)

	tlsTimeout := e.timeoutCtrl.GetTimeout("tls")
	start = time.Now()
	tlsResult := GetCachedTLS(domain, ip, tlsTimeout, true)
	e.timeoutCtrl.Record(time.Since(start), "tls")

	if tlsResult == nil || !tlsResult.HandshakeOK {
		errMsg := "TLS handshake failed"
		if tlsResult != nil && tlsResult.Error != "" {
			errMsg = tlsResult.Error
		}
		return jobResult{domain: domain, success: false, ip: ip, country: country, asn: asnRes.ASN, org: asnRes.Org, info: errMsg}
	}

	success, info := ValidateDomain(tlsResult)
	finalIP := tlsResult.IP
	if finalIP == "" {
		finalIP = ip
	}

	if success && country == "UNKNOWN" && finalIP != "" {
		country = GetCachedCountry(finalIP, e.geoDB)
	}

	if success {
		e.storage.SaveSuccess(domain, country)
	}

	return jobResult{
		domain:  domain,
		success: success,
		ip:      finalIP,
		country: country,
		asn:     asnRes.ASN,
		org:     asnRes.Org,
		info:    info,
	}
}
