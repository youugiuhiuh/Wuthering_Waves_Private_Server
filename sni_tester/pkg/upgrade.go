package pkg

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"
)

type UpgradeStats struct {
	FilePath     string
	Total        int
	PQCount      int
	ClassicCount int
	DeadCount    int
}

// UpgradePbFile retests all domains in an existing .pb file for PQ TLS support
// and rewrites the file with pq_domains and classic_domains.
func UpgradePbFile(ctx context.Context, pbPath string, outPath string, workers int, timeout time.Duration, keepDead bool) (*UpgradeStats, error) {
	data, err := os.ReadFile(pbPath)
	if err != nil {
		return nil, fmt.Errorf("failed to read pb file %s: %w", pbPath, err)
	}

	domains, err := ParseProtobufDomains(data)
	if err != nil {
		return nil, fmt.Errorf("failed to parse domains from %s: %w", pbPath, err)
	}

	if len(domains) == 0 {
		return &UpgradeStats{FilePath: pbPath, Total: 0}, nil
	}

	if workers <= 0 {
		workers = 20
	}
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	if outPath == "" {
		outPath = pbPath
	}

	type domainTaskResult struct {
		domain  string
		isPQ    bool
		isValid bool
	}

	tasks := make(chan string, len(domains))
	for _, d := range domains {
		tasks <- d
	}
	close(tasks)

	results := make(chan domainTaskResult, len(domains))
	var wg sync.WaitGroup

	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for d := range tasks {
				select {
				case <-ctx.Done():
					return
				default:
				}

				// 极速并发 UDP 解析并做 TLS 握手
				ips, err := ResolveFast(ctx, d)
				if err != nil || len(ips) == 0 {
					results <- domainTaskResult{domain: d, isValid: false}
					continue
				}

				tlsRes, err := PerformTLSHandshake(d, ips[0], timeout, true)
				if err != nil || tlsRes == nil || !tlsRes.HandshakeOK {
					results <- domainTaskResult{domain: d, isValid: false}
					continue
				}

				valid, _ := ValidateDomain(tlsRes)
				if !valid {
					results <- domainTaskResult{domain: d, isValid: false}
					continue
				}

				results <- domainTaskResult{
					domain:  d,
					isPQ:    tlsRes.IsPQ,
					isValid: true,
				}
			}
		}()
	}

	wg.Wait()
	close(results)

	var pqDomains []string
	var classicDomains []string
	var deadDomains []string

	for r := range results {
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

	// 寫入雙池 protobuf
	if err := WriteProtobufDualDomainFile(pqDomains, classicDomains, outPath); err != nil {
		return nil, fmt.Errorf("failed to write upgraded pb to %s: %w", outPath, err)
	}

	stats := &UpgradeStats{
		FilePath:     pbPath,
		Total:        len(domains),
		PQCount:      len(pqDomains),
		ClassicCount: len(classicDomains),
		DeadCount:    len(deadDomains),
	}

	return stats, nil
}

// UpgradePbDir scans dir for all *.pb files and upgrades each one.
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

	// 按文件大小升序排序：数以百计的小国家文件秒级完成，大国家随后集中并发处理
	sort.Slice(files, func(i, j int) bool {
		fi, err1 := os.Stat(files[i])
		fj, err2 := os.Stat(files[j])
		if err1 == nil && err2 == nil {
			return fi.Size() < fj.Size()
		}
		return files[i] < files[j]
	})

	var allStats []UpgradeStats
	for _, f := range files {
		select {
		case <-ctx.Done():
			return allStats, ctx.Err()
		default:
		}

		targetOut := f
		if outDir != "" {
			targetOut = filepath.Join(outDir, filepath.Base(f))
		}

		fmt.Printf("Upgrading %s (%s)...\n", filepath.Base(f), f)
		stats, err := UpgradePbFile(ctx, f, targetOut, workers, timeout, keepDead)
		if err != nil {
			fmt.Printf("  Error upgrading %s: %v\n", f, err)
			continue
		}

		allStats = append(allStats, *stats)
		fmt.Printf("  Done %s: Total=%d, PQ=%d (%.1f%%), Classic=%d, Dead=%d\n",
			filepath.Base(f),
			stats.Total,
			stats.PQCount,
			float64(stats.PQCount)/float64(max(stats.Total, 1))*100,
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
