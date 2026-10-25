package pkg

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

// PreResolveOptions holds configuration for Phase 1: overseas pre-resolution.
type PreResolveOptions struct {
	InputFile  string
	OutputFile string
	Workers    int
	Timeout    time.Duration
	MaxLines   int
}

// PreResolveStats records statistics of the pre-resolution phase.
type PreResolveStats struct {
	Total    int
	Resolved int
	Failed   int
	Duration time.Duration
}

// ParseDomainIPLine parses a raw line which could be:
// 1. "domain" (e.g. "example.com")
// 2. "rank,domain" (e.g. "1,example.com")
// 3. "domain,ip" (e.g. "example.com,93.184.216.34")
// 4. "rank,domain,ip" (e.g. "1,example.com,93.184.216.34")
func ParseDomainIPLine(raw string) (domain string, ip string) {
	raw = strings.TrimSpace(raw)
	if len(raw) == 0 || raw[0] == '#' || (len(raw) >= 2 && raw[0:2] == "//") {
		return "", ""
	}

	parts := strings.Split(raw, ",")
	cleanParts := make([]string, 0, len(parts))
	for _, p := range parts {
		trimmed := strings.TrimSpace(p)
		if trimmed != "" {
			cleanParts = append(cleanParts, trimmed)
		}
	}

	if len(cleanParts) == 0 {
		return "", ""
	}

	// 过滤开头的序号 (如果第一列是纯数字)
	if isNumeric(cleanParts[0]) {
		cleanParts = cleanParts[1:]
	}

	if len(cleanParts) == 0 {
		return "", ""
	}

	domain = cleanParts[0]

	// 如果包含第二列且该列为有效 IP 地址
	if len(cleanParts) >= 2 {
		candidateIP := cleanParts[1]
		if parsed := net.ParseIP(candidateIP); parsed != nil {
			ip = candidateIP
		}
	}

	return domain, ip
}

// RunResolvePreMap executes Phase 1 overseas DNS pre-resolution:
// Resolves input domains concurrently using pure overseas Anycast DNS servers (FastDNSServers)
// and outputs a mapping of "domain,ip" for Phase 2 survival probing.
func RunResolvePreMap(ctx context.Context, opts PreResolveOptions) (*PreResolveStats, error) {
	inFile, err := os.Open(opts.InputFile)
	if err != nil {
		return nil, fmt.Errorf("open input file %s: %w", opts.InputFile, err)
	}
	defer inFile.Close()

	var domains []string
	scanner := bufio.NewScanner(inFile)
	for scanner.Scan() {
		d, _ := ParseDomainIPLine(scanner.Text())
		if d != "" {
			domains = append(domains, d)
		}
		if opts.MaxLines > 0 && len(domains) >= opts.MaxLines {
			break
		}
	}

	total := len(domains)
	if total == 0 {
		return &PreResolveStats{}, nil
	}

	workers := opts.Workers
	if workers <= 0 {
		workers = 50
	}

	outFile, err := os.Create(opts.OutputFile)
	if err != nil {
		return nil, fmt.Errorf("create output file %s: %w", opts.OutputFile, err)
	}
	defer outFile.Close()
	writer := bufio.NewWriter(outFile)
	defer writer.Flush()

	var outMu sync.Mutex
	var resolvedCount int64
	var failedCount int64

	jobs := make(chan string, workers*2)
	var wg sync.WaitGroup

	start := time.Now()

	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for d := range jobs {
				select {
				case <-ctx.Done():
					return
				default:
				}

				ips, err := ResolveFast(ctx, d)
				if err != nil || len(ips) == 0 {
					atomic.AddInt64(&failedCount, 1)
					continue
				}

				// 排除 Cloudflare 避免后续无谓测试
				if IsCloudflareIP(ips[0]) {
					atomic.AddInt64(&failedCount, 1)
					continue
				}

				atomic.AddInt64(&resolvedCount, 1)
				outMu.Lock()
				writer.WriteString(fmt.Sprintf("%s,%s\n", d, ips[0]))
				outMu.Unlock()
			}
		}()
	}

	go func() {
		for _, d := range domains {
			select {
			case <-ctx.Done():
				break
			case jobs <- d:
			}
		}
		close(jobs)
	}()

	wg.Wait()
	writer.Flush()

	return &PreResolveStats{
		Total:    total,
		Resolved: int(resolvedCount),
		Failed:   int(failedCount),
		Duration: time.Since(start),
	}, nil
}
