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

	utls "github.com/refraction-networking/utls"
)

// SurvivalOptions defines parameters for Phase 1: survival probe mode.
type SurvivalOptions struct {
	InputFile  string
	OutputFile string
	Workers    int
	Timeout    time.Duration
	MaxLines   int
}

// SurvivalStats records results of the Phase 1 probe.
type SurvivalStats struct {
	Total    int
	Survived int
	Dead     int
	Duration time.Duration
}

// ProbeDomainSurvival executes the lightweight Phase 1 survival test:
// 1. TCP 443 handshake using the outbound dialer (or SOCKS5 proxy)
// 2. Light ClientHello without parsing certificates or reading full bodies
// 3. Verifies that the connection is NOT immediately reset by GFW within timeout
func ProbeDomainSurvival(ctx context.Context, domain string, targetIP string, timeout time.Duration) bool {
	if timeout <= 0 {
		timeout = 3 * time.Second
	}

	dialer := NewDialer(timeout)
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(ctx, "tcp", addr)
	if err != nil {
		return false
	}
	defer rawConn.Close()

	// Lightweight TLS ClientHello probe
	config := &utls.Config{
		ServerName:         domain,
		InsecureSkipVerify: true,
		MinVersion:         utls.VersionTLS12,
		MaxVersion:         utls.VersionTLS13,
		NextProtos:         []string{"h2", "http/1.1"},
	}

	helloID := utls.HelloChrome_Auto
	uConn := utls.UClient(rawConn, config, helloID)
	defer uConn.Close()
	uConn.SetDeadline(time.Now().Add(timeout))

	// If GFW drops or resets the connection, Handshake() will fail
	if err := uConn.Handshake(); err != nil {
		return false
	}

	// 握手成功后微延时 150ms 进行非阻塞探读，捕捉 GFW 迟发的 TCP RST 报文，消除假阳性
	uConn.SetReadDeadline(time.Now().Add(150 * time.Millisecond))
	var probeBuf [1]byte
	_, rerr := uConn.Read(probeBuf[:])
	if rerr != nil {
		errStr := rerr.Error()
		if strings.Contains(errStr, "reset") || strings.Contains(errStr, "broken pipe") {
			return false
		}
	}

	return true
}

// RunProbeSurvival executes Phase 1 batch survival probing over a list of domains.
func RunProbeSurvival(ctx context.Context, opts SurvivalOptions) (*SurvivalStats, error) {
	inFile, err := os.Open(opts.InputFile)
	if err != nil {
		return nil, fmt.Errorf("open input file %s: %w", opts.InputFile, err)
	}
	defer inFile.Close()

	type survivalTarget struct {
		domain string
		ip     string
	}

	var targets []survivalTarget
	scanner := bufio.NewScanner(inFile)
	for scanner.Scan() {
		d, ip := ParseDomainIPLine(scanner.Text())
		if d != "" {
			targets = append(targets, survivalTarget{domain: d, ip: ip})
		}
		if opts.MaxLines > 0 && len(targets) >= opts.MaxLines {
			break
		}
	}

	total := len(targets)
	if total == 0 {
		return &SurvivalStats{}, nil
	}

	workers := opts.Workers
	if workers <= 0 {
		workers = 30
	}
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 3 * time.Second
	}

	outFile, err := os.Create(opts.OutputFile)
	if err != nil {
		return nil, fmt.Errorf("create output file %s: %w", opts.OutputFile, err)
	}
	defer outFile.Close()
	writer := bufio.NewWriter(outFile)
	defer writer.Flush()

	var outMu sync.Mutex
	var survivedCount int64
	var deadCount int64

	jobs := make(chan survivalTarget, workers*2)
	var wg sync.WaitGroup

	start := time.Now()

	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for t := range jobs {
				select {
				case <-ctx.Done():
					return
				default:
				}

				targetIP := t.ip
				if targetIP == "" {
					// 仅在未预先解析时才执行本地 DNS 兜底
					ips, err := ResolveFast(ctx, t.domain)
					if err != nil || len(ips) == 0 {
						atomic.AddInt64(&deadCount, 1)
						continue
					}
					targetIP = ips[0]
				}

				// 排除 Cloudflare (REALITY 不可用)
				if IsCloudflareIP(targetIP) {
					atomic.AddInt64(&deadCount, 1)
					continue
				}

				// 轻量连通与防 GFW 重置探测
				if ProbeDomainSurvival(ctx, t.domain, targetIP, timeout) {
					atomic.AddInt64(&survivedCount, 1)
					outMu.Lock()
					// 输出 domain,ip 供 Phase 3 build-pools 直接使用
					writer.WriteString(fmt.Sprintf("%s,%s\n", t.domain, targetIP))
					outMu.Unlock()
				} else {
					atomic.AddInt64(&deadCount, 1)
				}
			}
		}()
	}

	go func() {
		for _, t := range targets {
			select {
			case <-ctx.Done():
				break
			case jobs <- t:
			}
		}
		close(jobs)
	}()

	wg.Wait()
	writer.Flush()

	return &SurvivalStats{
		Total:    total,
		Survived: int(survivedCount),
		Dead:     int(deadCount),
		Duration: time.Since(start),
	}, nil
}
