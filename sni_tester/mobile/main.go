package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	utls "github.com/refraction-networking/utls"
)

const (
	InitialWorkers = 100
	MaxWorkers     = 2000
	MinWorkers     = 10
	JobBuffer      = 5000
)

var DnsPool = []string{
	"1.1.1.1", "1.0.0.1",
	"8.8.8.8", "8.8.4.4",
	"9.9.9.9", "149.112.112.112",
	"208.67.222.222", "208.67.220.220",
}

var dnsIndex uint32
var dnsCache sync.Map

type ValidationResult struct {
	Domain  string `json:"domain"`
	Success bool   `json:"success"`
	IP      string `json:"ip"`
	Country string `json:"country"`
	Info    string `json:"info"`
}

type FailureHistory struct {
	Version  int             `json:"version"`
	Failures []FailureRecord `json:"failures"`
}

type FailureRecord struct {
	Domain    string `json:"domain"`
	Timestamp int64  `json:"timestamp"`
}

func pickClientHelloID() utls.ClientHelloID {
	return utls.HelloChrome_Auto
}

func pickALPNProfile() []string {
	return []string{"h2", "http/1.1"}
}

func checkSNI(domain string, targetIP string, xhttp, reality bool) (bool, string, string) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(context.Background(), "tcp", addr)
	if err != nil {
		return false, "", err.Error()
	}
	defer rawConn.Close()

	alpn := pickALPNProfile()
	config := &utls.Config{
		ServerName: domain,
		MinVersion: utls.VersionTLS12,
		MaxVersion: utls.VersionTLS13,
		NextProtos: alpn,
	}
	if reality || xhttp {
		config.MinVersion = utls.VersionTLS13
	}

	helloID := pickClientHelloID()
	uConn := utls.UClient(rawConn, config, helloID)
	defer uConn.Close()

	uConn.SetDeadline(time.Now().Add(10 * time.Second))
	if err := uConn.Handshake(); err != nil {
		return false, "", err.Error()
	}

	state := uConn.ConnectionState()
	if state.Version != utls.VersionTLS13 && (reality || xhttp) {
		return false, "", fmt.Sprintf("TLS 1.3 required (got %04x)", state.Version)
	}

	info := "Validated"
	return true, targetIP, info
}

func resolveDNS(domain string) (string, error) {
	if cached, ok := dnsCache.Load(domain); ok {
		return cached.(string), nil
	}

	idx := atomic.AddUint32(&dnsIndex, 1) % uint32(len(DnsPool))
	resolver := &net.Resolver{
		PreferGo: true,
		Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
			d := net.Dialer{Timeout: 3 * time.Second}
			return d.DialContext(ctx, "udp4", DnsPool[idx]+":53")
		},
	}

	ips, err := resolver.LookupHost(context.Background(), domain)
	if err != nil || len(ips) == 0 {
		return "", err
	}

	ip := ips[0]
	dnsCache.Store(domain, ip)
	return ip, nil
}

func isBlockedCountry(code string) bool {
	return code == "CN" || code == "HK" || code == "MO" || code == "IR" || code == "RU" || code == "KP"
}

type SNITester struct {
	inputFile    string
	mode         string
	isRunning    bool
	successCount int
	failCount    int
	results      []ValidationResult
	mu           sync.Mutex
}

func NewSNITester() *SNITester {
	return &SNITester{
		results: make([]ValidationResult, 0),
	}
}

func (s *SNITester) Start(xhttp, reality bool) error {
	s.isRunning = true
	s.results = s.results[:0]

	file, err := os.Open(s.inputFile)
	if err != nil {
		return err
	}
	defer file.Close()

	var lines []string
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		lines = append(lines, scanner.Text())
	}

	total := len(lines)
	done := 0

	jobs := make(chan string, JobBuffer)
	results := make(chan ValidationResult, MaxWorkers)
	var wg sync.WaitGroup

	for i := 0; i < InitialWorkers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for domain := range jobs {
				domain = strings.TrimSpace(domain)
				if domain == "" {
					continue
				}

				ip, err := resolveDNS(domain)
				if err != nil {
					results <- ValidationResult{Domain: domain, Success: false, IP: "", Country: "UNKNOWN", Info: "DNS failed"}
					continue
				}

				success, finalIP, info := checkSNI(domain, ip, xhttp, reality)
				results <- ValidationResult{Domain: domain, Success: success, IP: finalIP, Country: "US", Info: info}
			}
		}()
	}

	go func() {
		for _, domain := range lines {
			jobs <- domain
		}
		close(jobs)
	}()

	go func() {
		for res := range results {
			s.mu.Lock()
			s.results = append(s.results, res)
			if res.Success {
				s.successCount++
			} else {
				s.failCount++
			}
			s.mu.Unlock()
			done++

			jsonLine, _ := json.Marshal(res)
			fmt.Println(string(jsonLine))

			progress := float64(done) / float64(total) * 100
			if done%100 == 0 {
				fmt.Fprintf(os.Stderr, "\rProgress: %.1f%% (%d/%d) Success: %d Fail: %d",
					progress, done, total, s.successCount, s.failCount)
			}
		}
		s.isRunning = false
		fmt.Fprintf(os.Stderr, "\nCompleted: %d success, %d failed\n", s.successCount, s.failCount)
	}()

	wg.Wait()
	return nil
}

func cleanDomain(raw string) string {
	domain := strings.TrimSpace(raw)
	domain = strings.Split(domain, "#")[0]
	domain = strings.Split(domain, "//")[0]
	domain = strings.Trim(domain, " \t\r\n")
	if strings.HasPrefix(domain, "*.") {
		domain = domain[2:]
	}
	return domain
}

func main() {
	os.Setenv("GODEBUG", "netdns=go")

	inputFile := flag.String("f", "", "Input TXT file containing SNIs")
	xhttp := flag.Bool("xhttp", false, "Enable XHTTP validation")
	reality := flag.Bool("reality", false, "Enable Reality validation")

	flag.Parse()

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester_adb -f <input_file> [-xhttp] [-reality]")
		fmt.Println("  Example: sni_tester_adb -f domains.txt -reality")
		os.Exit(1)
	}

	absPath, err := filepath.Abs(*inputFile)
	if err == nil {
		*inputFile = absPath
	}

	tester := NewSNITester()
	tester.inputFile = *inputFile

	fmt.Fprintf(os.Stderr, "Starting SNI test: %s (Reality: %v, XHTTP: %v)\n", *inputFile, *reality, *xhttp)

	if err := tester.Start(*xhttp, *reality); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
