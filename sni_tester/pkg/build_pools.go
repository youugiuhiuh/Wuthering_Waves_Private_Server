package pkg

import (
	"bufio"
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	maxminddb "github.com/oschwald/maxminddb-golang/v2"
)

// BuildPoolsOptions holds settings for Phase 2: VPS deep pool builder mode.
type BuildPoolsOptions struct {
	CandidatesFile string
	OutputDir      string
	Workers        int
	Timeout        time.Duration
	GeoDBFile      string
	MaxLines       int
}

// BuildPoolsStats captures metrics from Phase 2.
type BuildPoolsStats struct {
	Total        int
	PQCount      int
	ClassicCount int
	Rejected     int
	Duration     time.Duration
}

// RunBuildPools executes Phase 2 on the user's overseas VPS:
// 1. Reads survived candidate domains from Phase 1
// 2. Performs deep TLS 1.3 / PQ / HTTP / Cert Limits probing via PerformCombinedProbe
// 3. Classifies targets into PQ and Classic pools
// 4. Determines country using IPinfo Lite GeoDB
// 5. Outputs pure dual-pool .pb files organized by country
func RunBuildPools(ctx context.Context, opts BuildPoolsOptions) (*BuildPoolsStats, error) {
	inFile, err := os.Open(opts.CandidatesFile)
	if err != nil {
		return nil, fmt.Errorf("open candidates file %s: %w", opts.CandidatesFile, err)
	}
	defer inFile.Close()

	type buildPoolTarget struct {
		domain string
		ip     string
	}

	var targets []buildPoolTarget
	scanner := bufio.NewScanner(inFile)
	for scanner.Scan() {
		d, ip := ParseDomainIPLine(scanner.Text())
		if d != "" {
			targets = append(targets, buildPoolTarget{domain: d, ip: ip})
		}
		if opts.MaxLines > 0 && len(targets) >= opts.MaxLines {
			break
		}
	}

	total := len(targets)
	if total == 0 {
		return &BuildPoolsStats{}, nil
	}

	workers := opts.Workers
	if workers <= 0 {
		workers = 30
	}
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	outDir := opts.OutputDir
	if outDir == "" {
		outDir = "rust/aegis/src/resources/sni"
	}
	if err := os.MkdirAll(outDir, 0755); err != nil {
		return nil, fmt.Errorf("mkdir output %s: %w", outDir, err)
	}

	// Open GeoDB if available
	geoFile := opts.GeoDBFile
	if geoFile == "" {
		geoFile = GeoDBFile
	}
	var geoDB *maxminddb.Reader
	if _, err := os.Stat(geoFile); err == nil {
		geoDB, _ = maxminddb.Open(geoFile)
		if geoDB != nil {
			defer geoDB.Close()
		}
	}

	// Two-stage pipeline: DNS Stage -> TLS Stage
	taskChan := make(chan buildPoolTarget, workers*2)
	dnsReadyChan := make(chan dnsResolvedTarget, workers*2)

	type probeResult struct {
		domain  string
		country string
		isPQ    bool
		isValid bool
	}
	resultsChan := make(chan probeResult, workers*2)

	// Stream domains
	go func() {
		defer close(taskChan)
		for _, t := range targets {
			select {
			case <-ctx.Done():
				return
			case taskChan <- t:
			}
		}
	}()

	// DNS Workers
	var dnsWg sync.WaitGroup
	dnsWorkers := workers
	if dnsWorkers < 20 {
		dnsWorkers = 20
	}
	for i := 0; i < dnsWorkers; i++ {
		dnsWg.Add(1)
		go func() {
			defer dnsWg.Done()
			for t := range taskChan {
				select {
				case <-ctx.Done():
					return
				default:
				}

				targetIP := t.ip
				if targetIP == "" {
					ips, err := ResolveFast(ctx, t.domain)
					if err != nil || len(ips) == 0 {
						resultsChan <- probeResult{domain: t.domain, isValid: false}
						continue
					}
					targetIP = ips[0]
				}

				if IsCloudflareIP(targetIP) {
					resultsChan <- probeResult{domain: t.domain, isValid: false}
					continue
				}

				select {
				case <-ctx.Done():
					return
				case dnsReadyChan <- dnsResolvedTarget{domain: t.domain, ip: targetIP}:
				}
			}
		}()
	}

	go func() {
		dnsWg.Wait()
		close(dnsReadyChan)
	}()

	// TLS Deep Probe Workers
	var tlsWg sync.WaitGroup
	for i := 0; i < workers; i++ {
		tlsWg.Add(1)
		go func() {
			defer tlsWg.Done()
			for target := range dnsReadyChan {
				select {
				case <-ctx.Done():
					return
				default:
				}

				tlsRes, err := PerformCombinedProbe(ctx, target.domain, target.ip, timeout)
				if err != nil || tlsRes == nil || !tlsRes.HandshakeOK {
					resultsChan <- probeResult{domain: target.domain, isValid: false}
					continue
				}

				valid, _ := ValidateDomain(tlsRes)
				if !valid {
					resultsChan <- probeResult{domain: target.domain, isValid: false}
					continue
				}

				country := "UNKNOWN"
				targetIP := tlsRes.IP
				if targetIP == "" {
					targetIP = target.ip
				}
				if geoDB != nil && targetIP != "" {
					country = GetCachedCountry(targetIP, geoDB)
				}

				// 严禁将审查管辖国（CN, HK, MO, IR, RU, KP）作为 REALITY 伪装目标，直接丢弃
				if IsBlockedCountry(country) {
					resultsChan <- probeResult{domain: target.domain, isValid: false}
					continue
				}

				resultsChan <- probeResult{
					domain:  target.domain,
					country: country,
					isPQ:    tlsRes.IsPQ,
					isValid: true,
				}
			}
		}()
	}

	go func() {
		tlsWg.Wait()
		close(resultsChan)
	}()

	// Collect by country
	countryPQ := make(map[string][]string)
	countryClassic := make(map[string][]string)
	var pqCount, classicCount, rejectedCount int64

	start := time.Now()

	for r := range resultsChan {
		if !r.isValid {
			atomic.AddInt64(&rejectedCount, 1)
			continue
		}

		country := r.country
		if country == "" {
			country = "UNKNOWN"
		}

		if r.isPQ {
			atomic.AddInt64(&pqCount, 1)
			countryPQ[country] = append(countryPQ[country], r.domain)
		} else {
			atomic.AddInt64(&classicCount, 1)
			countryClassic[country] = append(countryClassic[country], r.domain)
		}
	}

	// Write out dual-pool .pb files
	allCountries := make(map[string]bool)
	for c := range countryPQ {
		allCountries[c] = true
	}
	for c := range countryClassic {
		allCountries[c] = true
	}

	for country := range allCountries {
		addPQ := countryPQ[country]
		addClassic := countryClassic[country]

		pbPath := filepath.Join(outDir, fmt.Sprintf("%s.pb", strings.ToUpper(country)))

		// 合并磁盘上已有的 .pb，保留原有的可用域名池，实现增量沉淀
		var existPQ []string
		var existClassic []string
		if data, err := os.ReadFile(pbPath); err == nil {
			if dl, err := ParseProtobufDomainList(data); err == nil {
				existPQ = dl.PqDomains
				existClassic = dl.ClassicDomains
			}
		}

		mergedPQ := append(existPQ, addPQ...)
		mergedClassic := append(existClassic, addClassic...)

		if len(mergedPQ) > 0 || len(mergedClassic) > 0 {
			if err := WriteProtobufDualDomainFile(mergedPQ, mergedClassic, pbPath); err != nil {
				fmt.Printf("Warning: failed to write %s: %v\n", pbPath, err)
			}
		}
	}

	return &BuildPoolsStats{
		Total:        total,
		PQCount:      int(pqCount),
		ClassicCount: int(classicCount),
		Rejected:     int(rejectedCount),
		Duration:     time.Since(start),
	}, nil
}
