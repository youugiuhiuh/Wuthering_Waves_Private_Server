package pkg

import (
	"context"
	"fmt"
	"sync"
	"time"

	maxminddb "github.com/oschwald/maxminddb-golang/v2"
)

type jobResult struct {
	domain  string
	success bool
	isPQ    bool
	ip      string
	country string
	asn     uint32
	org     string
	info    string
}

type Engine struct {
	cfg         Config
	geoDB       *maxminddb.Reader
	asnDB       *maxminddb.Reader
	storage     *StorageManager
	dnsLimiter  *DNSRateLimiter
	timeoutCtrl *TimeoutController
	cancel      func()
}

func NewEngine(cfg Config) (*Engine, error) {
	geoDB, err := maxminddb.Open(cfg.GeoDBFile)
	if err != nil {
		return nil, fmt.Errorf("failed to open GeoIP database: %w", err)
	}

	var asnDB *maxminddb.Reader
	if cfg.GeoASNFile == cfg.GeoDBFile {
		asnDB = geoDB
	} else {
		asnDB, err = maxminddb.Open(cfg.GeoASNFile)
		if err != nil {
			geoDB.Close()
			return nil, fmt.Errorf("failed to open GeoASN database: %w", err)
		}
	}

	storage, err := NewStorageManager(cfg.BadgerDBDir, cfg.TTLDays)
	if err != nil {
		geoDB.Close()
		if asnDB != geoDB {
			asnDB.Close()
		}
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
	if e.asnDB != nil && e.asnDB != e.geoDB {
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

	now := time.Now().Unix()
	var toTest []string
	var stats Stats
	countryPQDomains := make(map[string][]string)
	countryClassicDomains := make(map[string][]string)
	if !e.cfg.ForceRetry {
		fmt.Println("Loading success/blocked history...")
		successCountry := e.storage.LoadSuccessHistory()
		blocked := e.storage.LoadBlockedHistory()
		for _, d := range domains {
			if country, ok := successCountry[d]; ok {
				countryClassicDomains[country] = append(countryClassicDomains[country], d)
				stats.Skipped++
				if cb != nil {
					cb(ProgressEvent{Type: "skipped", Domain: d, Stats: stats})
				}
				continue
			}
			if _, ok := blocked[d]; ok {
				stats.Skipped++
				if cb != nil {
					cb(ProgressEvent{Type: "skipped", Domain: d, Stats: stats})
				}
				continue
			}
			if e.storage.IsFailedRecently(d, now) {
				stats.Skipped++
				if cb != nil {
					cb(ProgressEvent{Type: "skipped", Domain: d, Stats: stats})
				}
				continue
			}
			toTest = append(toTest, d)
		}
	} else {
		toTest = append(toTest, domains...)
	}
	fmt.Printf("Filtering: %d total, %d to test, %d skipped\n", len(domains), len(toTest), stats.Skipped)

	workers := InitialWorkers
	if e.cfg.FixedWorkers > 0 {
		workers = e.cfg.FixedWorkers
	}

	type dnsResolvedItem struct {
		domain  string
		ip      string
		country string
		asn     uint32
		org     string
	}

	taskChan := make(chan string, workers*2)
	dnsReadyChan := make(chan dnsResolvedItem, workers*2)
	results := make(chan jobResult, workers*2)

	// 1. 流式投喂待测域名
	go func() {
		defer close(taskChan)
		for _, d := range toTest {
			select {
			case taskChan <- d:
			case <-ctx.Done():
				return
			}
		}
	}()

	// 2. 阶段 1: 专用 DNS 并发解析池 (UDP 预解析 + GeoIP/Cloudflare 前置淘汰，零 TLS 阻塞)
	var dnsWg sync.WaitGroup
	for i := 0; i < workers; i++ {
		dnsWg.Add(1)
		go func() {
			defer dnsWg.Done()
			dnsTimeout := e.timeoutCtrl.GetTimeout("dns")
			for domain := range taskChan {
				select {
				case <-ctx.Done():
					return
				default:
				}

				dnsCtx, cancel := context.WithTimeout(ctx, dnsTimeout)
				start := time.Now()
				ips, err := ResolveFast(dnsCtx, domain)
				e.timeoutCtrl.Record(time.Since(start), "dns")
				cancel()

				if err != nil || len(ips) == 0 {
					errMsg := "DNS resolution failed"
					if err != nil {
						errMsg = fmt.Sprintf("DNS failed: %v", err)
					}
					select {
					case results <- jobResult{domain: domain, success: false, info: errMsg}:
					case <-ctx.Done():
						return
					}
					continue
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
					select {
					case results <- jobResult{domain: domain, success: false, ip: ip, country: country, info: "Country blocked"}:
					case <-ctx.Done():
						return
					}
					continue
				}

				asnRes, _ := GetCachedASN(ip, e.asnDB)
				if IsCloudflare(ip, asnRes.ASN, asnRes.Org) {
					e.storage.AddBlockedDomain(domain, "CLOUDFLARE_INCOMPATIBLE", country)
					select {
					case results <- jobResult{domain: domain, success: false, ip: ip, country: country, asn: asnRes.ASN, org: asnRes.Org, info: "Cloudflare incompatible with REALITY"}:
					case <-ctx.Done():
						return
					}
					continue
				}

				select {
				case dnsReadyChan <- dnsResolvedItem{
					domain:  domain,
					ip:      ip,
					country: country,
					asn:     asnRes.ASN,
					org:     asnRes.Org,
				}:
				case <-ctx.Done():
					return
				}
			}
		}()
	}

	go func() {
		dnsWg.Wait()
		close(dnsReadyChan)
	}()

	// 3. 阶段 2: 专用 TLS/HTTP 验证池 (直接消费已解析 IP 目标，单连接复合探测，零 DNS 等待)
	var tlsWg sync.WaitGroup
	for i := 0; i < workers; i++ {
		tlsWg.Add(1)
		go func() {
			defer tlsWg.Done()
			tlsTimeout := e.timeoutCtrl.GetTimeout("tls")
			for target := range dnsReadyChan {
				select {
				case <-ctx.Done():
					return
				default:
				}

				start := time.Now()
				probeCtx, cancel := context.WithTimeout(ctx, tlsTimeout)
				tlsResult, err := PerformCombinedProbe(probeCtx, target.domain, target.ip, tlsTimeout)
				e.timeoutCtrl.Record(time.Since(start), "tls")
				cancel()

				if err != nil || tlsResult == nil || !tlsResult.HandshakeOK {
					errMsg := "TLS handshake failed"
					if tlsResult != nil && tlsResult.Error != "" {
						errMsg = tlsResult.Error
					} else if err != nil {
						errMsg = err.Error()
					}
					select {
					case results <- jobResult{
						domain:  target.domain,
						success: false,
						ip:      target.ip,
						country: target.country,
						asn:     target.asn,
						org:     target.org,
						info:    errMsg,
					}:
					case <-ctx.Done():
						return
					}
					continue
				}

				valid, info := ValidateDomain(tlsResult)
				finalIP := tlsResult.IP
				if finalIP == "" {
					finalIP = target.ip
				}

				if valid && target.country == "UNKNOWN" && finalIP != "" {
					target.country = GetCachedCountry(finalIP, e.geoDB)
				}

				if valid {
					e.storage.SaveSuccess(target.domain, target.country)
				}

				select {
				case results <- jobResult{
					domain:  target.domain,
					success: valid,
					isPQ:    tlsResult.SupportsRealityPQ(),
					ip:      finalIP,
					country: target.country,
					asn:     target.asn,
					org:     target.org,
					info:    info,
				}:
				case <-ctx.Done():
					return
				}
			}
		}()
	}

	go func() {
		tlsWg.Wait()
		close(results)
	}()

	total := len(toTest)
	stats.Total = total
	var failedDomains []string
	var resultList []DomainResult

	done := make(chan struct{})
	go func() {
		defer close(done)
		for r := range results {
			if r.success {
				if r.isPQ {
					countryPQDomains[r.country] = append(countryPQDomains[r.country], r.domain)
				} else {
					countryClassicDomains[r.country] = append(countryClassicDomains[r.country], r.domain)
				}
				stats.Success++
			} else {
				failedDomains = append(failedDomains, r.domain)
				stats.Failed++
			}

			// 内存优化：仅在 debug 模式下在 RAM 中维护百万级全量结构体切片，避免频繁 GC 停顿
			if e.cfg.Debug {
				resultList = append(resultList, DomainResult{
					Domain:  r.domain,
					Success: r.success,
					IP:      r.ip,
					Country: r.country,
					ASN:     r.asn,
					Org:     r.org,
					Info:    r.info,
				})
			}

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

	<-done

	if !e.cfg.Debug {
		fmt.Println("Saving results...")
		SaveBatchDual(ctx, e.cfg.OutputDir, countryPQDomains, countryClassicDomains, e.storage.DB())
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
	ips, err := ResolveFast(dnsCtx, domain)
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
	if IsCloudflare(ip, asnRes.ASN, asnRes.Org) {
		e.storage.AddBlockedDomain(domain, "CLOUDFLARE_INCOMPATIBLE", country)
		return jobResult{domain: domain, success: false, ip: ip, country: country, asn: asnRes.ASN, org: asnRes.Org, info: "Cloudflare incompatible with REALITY"}
	}

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
		isPQ:    tlsResult.SupportsRealityPQ(),
		ip:      finalIP,
		country: country,
		asn:     asnRes.ASN,
		org:     asnRes.Org,
		info:    info,
	}
}
