package pkg

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"sync/atomic"
	"time"
)

type UpgradeStats struct {
	FilePath     string
	Total        int
	PQCount      int
	ClassicCount int
	DeadCount    int
}

type dnsResolvedTarget struct {
	domain string
	ip     string
}

type domainTaskResult struct {
	domain  string
	isPQ    bool
	isValid bool
}

// UpgradePbFile retests all domains in an existing .pb file for PQ TLS support
// using a two-stage streaming pipeline (DNS Stage -> TLS Stage) with live progress.
func UpgradePbFile(ctx context.Context, pbPath string, outPath string, workers int, timeout time.Duration, keepDead bool) (*UpgradeStats, error) {
	data, err := os.ReadFile(pbPath)
	if err != nil {
		return nil, fmt.Errorf("failed to read pb file %s: %w", pbPath, err)
	}

	domains, err := ParseProtobufDomains(data)
	if err != nil {
		return nil, fmt.Errorf("failed to parse domains from %s: %w", pbPath, err)
	}

	total := len(domains)
	if total == 0 {
		return &UpgradeStats{FilePath: pbPath, Total: 0}, nil
	}

	if workers <= 0 {
		workers = 30
	}
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	if outPath == "" {
		outPath = pbPath
	}

	bufferSize := workers * 2
	if bufferSize > 1000 {
		bufferSize = 1000
	}

	taskChan := make(chan string, bufferSize)
	dnsReadyChan := make(chan dnsResolvedTarget, bufferSize)
	resultsChan := make(chan domainTaskResult, bufferSize)

	// 1. 流式投喂任务（零大内存缓冲区）
	go func() {
		defer close(taskChan)
		for _, d := range domains {
			select {
			case <-ctx.Done():
				return
			case taskChan <- d:
			}
		}
	}()

	// 2. 階段 1: DNS 併發解析池 (極速無狀態 UDP 競速)
	dnsWorkers := workers
	if dnsWorkers < 20 {
		dnsWorkers = 20
	}
	var dnsWg sync.WaitGroup
	for i := 0; i < dnsWorkers; i++ {
		dnsWg.Add(1)
		go func() {
			defer dnsWg.Done()
			for d := range taskChan {
				select {
				case <-ctx.Done():
					return
				default:
				}

				ips, err := ResolveFast(ctx, d)
				if err != nil || len(ips) == 0 {
					select {
					case <-ctx.Done():
						return
					case resultsChan <- domainTaskResult{domain: d, isValid: false}:
					}
					continue
				}

				if IsCloudflareIP(ips[0]) {
					select {
					case <-ctx.Done():
						return
					case resultsChan <- domainTaskResult{domain: d, isValid: false}:
					}
					continue
				}

				select {
				case <-ctx.Done():
					return
				case dnsReadyChan <- dnsResolvedTarget{domain: d, ip: ips[0]}:
				}
			}
		}()
	}

	go func() {
		dnsWg.Wait()
		close(dnsReadyChan)
	}()

	// 3. 階段 2: TLS 握手與驗證池 (已預先解析好 IP，零 DNS 等待)
	tlsWorkers := workers
	var tlsWg sync.WaitGroup
	for i := 0; i < tlsWorkers; i++ {
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
					select {
					case <-ctx.Done():
						return
					case resultsChan <- domainTaskResult{domain: target.domain, isValid: false}:
					}
					continue
				}

				valid, _ := ValidateDomain(tlsRes)
				if !valid {
					select {
					case <-ctx.Done():
						return
					case resultsChan <- domainTaskResult{domain: target.domain, isValid: false}:
					}
					continue
				}

				select {
				case <-ctx.Done():
					return
				case resultsChan <- domainTaskResult{
					domain:  target.domain,
					isPQ:    tlsRes.SupportsRealityPQ(),
					isValid: true,
				}:
				}
			}
		}()
	}

	go func() {
		tlsWg.Wait()
		close(resultsChan)
	}()

	// 4. 結果流式聚合與實時進度展示
	var pqDomains []string
	var classicDomains []string
	var deadDomains []string

	startTime := time.Now()
	var processed int64
	baseName := filepath.Base(pbPath)
	ticker := time.NewTicker(200 * time.Millisecond)
	defer ticker.Stop()

	// 終端進度刷新協程
	doneProgress := make(chan struct{})
	go func() {
		defer close(doneProgress)
		for {
			select {
			case <-ticker.C:
				p := atomic.LoadInt64(&processed)
				elapsed := time.Since(startTime).Seconds()
				rate := 0.0
				if elapsed > 0 {
					rate = float64(p) / elapsed
				}
				pct := float64(p) / float64(total) * 100
				eta := "--"
				if rate > 0 && p < int64(total) {
					remainSec := float64(int64(total)-p) / rate
					eta = fmt.Sprintf("%.0fs", remainSec)
				}
				fmt.Printf("\r  [%s] %5.1f%% (%d/%d) | %.0f/s | ETA: %s",
					baseName, pct, p, total, rate, eta)
			case <-ctx.Done():
				return
			}
		}
	}()

	for r := range resultsChan {
		atomic.AddInt64(&processed, 1)
		if r.isValid {
			if r.isPQ {
				pqDomains = append(pqDomains, r.domain)
			} else {
				classicDomains = append(classicDomains, r.domain)
			}
		} else {
			deadDomains = append(deadDomains, r.domain)
			if keepDead {
				classicDomains = append(classicDomains, r.domain)
			}
		}
	}

	ticker.Stop()
	// 清空進度行並輸出最終換行
	fmt.Printf("\r                                                                               \r")

	// 寫入雙池 protobuf
	if err := WriteProtobufDualDomainFile(pqDomains, classicDomains, outPath); err != nil {
		return nil, fmt.Errorf("failed to write upgraded pb to %s: %w", outPath, err)
	}

	stats := &UpgradeStats{
		FilePath:     pbPath,
		Total:        total,
		PQCount:      len(pqDomains),
		ClassicCount: len(classicDomains),
		DeadCount:    len(deadDomains),
	}

	return stats, nil
}

// UpgradePbDir scans dir for all *.pb files and upgrades each one in order of file size.
func UpgradePbDir(ctx context.Context, dir string, outDir string, workers int, timeout time.Duration, keepDead bool) ([]UpgradeStats, error) {
	files, err := filepath.Glob(filepath.Join(dir, "*.pb"))
	if err != nil {
		return nil, err
	}

	if len(files) == 0 {
		return nil, fmt.Errorf("no .pb files found in %s", dir)
	}

	if outDir != "" {
		if err := os.MkdirAll(outDir, 0755); err != nil {
			return nil, err
		}
	}

	// 按文件大小升序排序：數以百計的小國家文件秒級完成，大國家隨後集中並發突破
	sort.Slice(files, func(i, j int) bool {
		fi, err1 := os.Stat(files[i])
		fj, err2 := os.Stat(files[j])
		if err1 == nil && err2 == nil {
			return fi.Size() < fj.Size()
		}
		return files[i] < files[j]
	})

	var allStats []UpgradeStats
	totalFiles := len(files)

	for idx, f := range files {
		select {
		case <-ctx.Done():
			return allStats, ctx.Err()
		default:
		}

		targetOut := f
		if outDir != "" {
			targetOut = filepath.Join(outDir, filepath.Base(f))
		}

		baseName := filepath.Base(f)
		fmt.Printf("[%d/%d] Upgrading %s...\n", idx+1, totalFiles, baseName)
		stats, err := UpgradePbFile(ctx, f, targetOut, workers, timeout, keepDead)
		if err != nil {
			fmt.Printf("  ❌ Error upgrading %s: %v\n", f, err)
			continue
		}

		allStats = append(allStats, *stats)
		pqPct := float64(stats.PQCount) / float64(max(stats.Total, 1)) * 100
		fmt.Printf("  ✅ Done %s: Total=%d | PQ=%d (%.1f%%) | Classic=%d | Dead=%d\n",
			baseName,
			stats.Total,
			stats.PQCount,
			pqPct,
			stats.ClassicCount,
			stats.DeadCount,
		)
	}

	return allStats, nil
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
