package main

import (
	"bufio"
	"context"
	"crypto/tls"
	"encoding/binary"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/oschwald/geoip2-golang"
	utls "github.com/refraction-networking/utls"
	"github.com/schollz/progressbar/v3"
	"github.com/syndtr/goleveldb/leveldb"
	"github.com/syndtr/goleveldb/leveldb/opt"
	"golang.org/x/net/proxy"
)

// Config
const (
	WorkerCount        = 1000             // High concurrency
	JobBuffer          = 2000             // Job channel buffer size
	StreamingThreshold = 10 * 1024 * 1024 // 10MB threshold for streaming mode
	GeoDBFile          = "GeoLite2-Country.mmdb"
	GeoDBURL           = "https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/GeoLite2-Country.mmdb"
)

type ValidationResult struct {
	Domain  string
	Success bool
	IP      string
	Country string
	Info    string
}

// isBlockedCountry returns true if the country code should be skipped (CN/HK/MO/IR/RU/KP).
func isBlockedCountry(code string) bool {
	return code == "CN" || code == "HK" || code == "MO" || code == "IR" || code == "RU" || code == "KP"
}

func main() {
	// 强制使用 Go 内置解析器，避免并发 CGO 解析限制
	os.Setenv("GODEBUG", "netdns=go")

	inputFile := flag.String("f", "", "Input TXT/CSV file containing SNIs")
	debugMode := flag.Bool("debug", false, "Enable debug logging")
	proxyString := flag.String("p", "", "Proxy for Geo download (http://127.0.0.1:10808 or socks5://127.0.0.1:10808)")
	dnsAddr := flag.String("dns", "", "DNS server address (required, e.g. 119.29.29.29 or [2402:4e00::]:53)")
	ttlDays := flag.Int("ttl", 7, "Days to remember failures (default 7)")
	maxLines := flag.Int("max", 0, "Max lines to read from input (0 = unlimited)")

	// XHTTP & Reality 专项校验参数
	xhttpMode := flag.Bool("xhttp", false, "Enable XHTTP validation (H2 minimum)")
	realityMode := flag.Bool("reality", false, "Enable Reality validation (TLS 1.3, X25519, H2)")

	flag.Parse()

	if *inputFile == "" || *dnsAddr == "" {
		fmt.Println("Usage: sni_tester -f <input_file> -dns <dns_server> [-debug] [-p <proxy>] [-xhttp] [-reality] [-ttl <days>] [-max <lines>]")
		fmt.Println("  Example: sni_tester -f domains.txt -dns 119.29.29.29")
		fmt.Println("  Example: sni_tester -f domains.txt -dns [2402:4e00::]:53")
		os.Exit(1)
	}

	// 解析 DNS 地址并自动补全端口
	if strings.Contains(*dnsAddr, ":") && !strings.Contains(*dnsAddr, "[") {
		// 可能是 IPv6 地址没加括号，也可能是 IPv4+端口
		if net.ParseIP(*dnsAddr) != nil {
			// 纯 IPv6 地址，加括号和端口
			*dnsAddr = "[" + *dnsAddr + "]:53"
		}
		// 否则已是 host:port 格式
	} else if !strings.Contains(*dnsAddr, ":") {
		// 纯 IPv4 地址，加端口
		*dnsAddr = *dnsAddr + ":53"
	}

	// 根据 DNS 地址类型自动决定 UDP 传输协议
	dnsHost, _, _ := net.SplitHostPort(*dnsAddr)
	dnsUDP := "udp4"
	if net.ParseIP(dnsHost) != nil && net.ParseIP(dnsHost).To4() == nil {
		dnsUDP = "udp6"
	}
	fmt.Printf("DNS: %s (transport: %s)\n", *dnsAddr, dnsUDP)

	// 0. 确定存储子目录
	subDir := ""
	if *realityMode {
		subDir = "reality"
	} else if *xhttpMode {
		subDir = "xhttp"
	}

	baseTargetDir := findTargetDir()
	if baseTargetDir == "" {
		fmt.Println("Error: Could not find rust/tgbot/src/resources/sni directory.")
		os.Exit(1)
	}

	targetDir := baseTargetDir
	if subDir != "" {
		targetDir = filepath.Join(baseTargetDir, subDir)
	}
	fmt.Printf("Detected target directory: %s\n", targetDir)

	// 0. Check network connectivity
	if err := checkNetworkConnectivity(*dnsAddr, dnsUDP); err != nil {
		fmt.Printf("Network check: %v (Accessible: false)\n", err)
	} else {
		fmt.Println("Network connectivity check passed - https://google.com is accessible.")
		if !*debugMode {
			fmt.Println("This program should only run when google.com is NOT accessible (unless -debug).")
			os.Exit(1)
		}
	}

	// 0.1 GeoIP DB Handling
	prepareGeoDB(*proxyString)

	db, err := geoip2.Open(GeoDBFile)
	if err != nil {
		fmt.Printf("Error opening GeoIP DB: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	// 1. 自适应模式识别
	fileInfo, err := os.Stat(*inputFile)
	if err != nil {
		fmt.Printf("Error accessing input file: %v\n", err)
		os.Exit(1)
	}

	var totalLines int
	isLargeFile := fileInfo.Size() >= StreamingThreshold

	limit := 0
	if *maxLines > 0 {
		limit = *maxLines
	}

	if isLargeFile {
		fmt.Printf("Large file detected (%.2f MB). Using streaming mode...\n", float64(fileInfo.Size())/(1024*1024))
		totalLines, _ = countLines(*inputFile, limit)
	} else {
		fmt.Printf("Small file detected (%.2f KB). Using fast-load mode...\n", float64(fileInfo.Size())/1024)
		totalLines, _ = countLines(*inputFile, limit)
	}

	if *maxLines > 0 && totalLines > *maxLines {
		totalLines = *maxLines
	}
	fmt.Printf("Total lines to process: %d\n", totalLines)

	// 2. Setup Memory Indices (Success Map) and LevelDB Failure History
	successMap := make(map[string]struct{})
	loadExistingIntoMap(targetDir, successMap)

	// LevelDB path per protocol mode (reality/xhttp/default)
	historyDBDir := "failed_history.db"
	if subDir != "" {
		historyDBDir = fmt.Sprintf("failed_history_%s.db", subDir)
	}

	failDB, err := leveldb.OpenFile(historyDBDir, &opt.Options{
		WriteBuffer:            16 * 1024 * 1024, // 16MB write buffer
		CompactionTableSize:    8 * 1024 * 1024,  // 8MB per table
		BlockCacheCapacity:     8 * 1024 * 1024,  // 8MB block cache
		OpenFilesCacheCapacity: 64,
	})
	if err != nil {
		fmt.Printf("Error opening LevelDB: %v\n", err)
		os.Exit(1)
	}
	defer failDB.Close()

	now := time.Now().Unix()
	ttlSec := int64(*ttlDays * 24 * 3600)

	failCount, purged := cleanAndCountFailureHistory(failDB, now, ttlSec)
	if purged > 0 {
		fmt.Printf("Purged %d expired entries from failure history.\n", purged)
	}

	fmt.Printf("Memory loaded: %d succeeded, %d failed in history.\n", len(successMap), failCount)

	// 2.1 Shared DNS Resolver (使用用户指定的 DNS)
	resolver := &net.Resolver{
		PreferGo: true,
		Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
			d := net.Dialer{Timeout: 5 * time.Second}
			return d.DialContext(ctx, dnsUDP, *dnsAddr)
		},
	}

	// 3. Setup Workers
	jobs := make(chan string, JobBuffer)
	results := make(chan ValidationResult, WorkerCount)
	var wg sync.WaitGroup

	for w := 1; w <= WorkerCount; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for domain := range jobs {
				// 1. DNS Resolution (唯一的 DNS 解析点，使用用户指定的 DNS)
				var ip string
				ips, err := resolver.LookupHost(context.Background(), domain)
				if err != nil || len(ips) == 0 {
					// DNS 解析失败，直接标记为失败，不再尝试其他路径
					errMsg := "DNS resolution failed"
					if err != nil {
						errMsg = err.Error()
					}
					results <- ValidationResult{
						Domain:  domain,
						Success: false,
						IP:      "",
						Country: "UNKNOWN",
						Info:    errMsg,
					}
					continue
				}
				ip = ips[0]

				country := "UNKNOWN"
				// LOCAL GEOIP LOOKUP ONLY - NO EXTERNAL API CALLS
				record, geoErr := db.Country(net.ParseIP(ip))
				if geoErr == nil {
					if record.Country.IsoCode != "" {
						country = record.Country.IsoCode
					} else if record.RegisteredCountry.IsoCode != "" {
						country = record.RegisteredCountry.IsoCode
					}
				}

				// 2. Early Skip if blocked country (CN/HK/MO)
				if isBlockedCountry(country) {
					results <- ValidationResult{
						Domain:  domain,
						Success: false,
						IP:      ip,
						Country: country,
						Info:    fmt.Sprintf("Skipped (Country: %s)", country),
					}
					continue
				}

				// 3. Perform TLS Handshake (仅使用已解析的 IP，不再做 DNS)
				success, finalIP, info := checkSNI(domain, ip, *debugMode, *xhttpMode, *realityMode, resolver)
				if finalIP != "" {
					ip = finalIP // Update if checkSNI got a different IP (unlikely but possible)
				}

				// Re-verify country if it was UNKNOWN and we now have a final IP
				if country == "UNKNOWN" && finalIP != "" {
					record, geoErr := db.Country(net.ParseIP(finalIP))
					if geoErr == nil {
						if record.Country.IsoCode != "" {
							country = record.Country.IsoCode
						} else if record.RegisteredCountry.IsoCode != "" {
							country = record.RegisteredCountry.IsoCode
						}
					}
				}
				if country == "" {
					country = "UNKNOWN"
				}

				results <- ValidationResult{Domain: domain, Success: success, IP: ip, Country: country, Info: info}
			}
		}()
	}

	var bar *progressbar.ProgressBar
	if !*debugMode {
		bar = progressbar.Default(int64(totalLines), "Testing")
	}

	validDomainsMap := make(map[string][]string)
	failureList := make([]string, 0, 100)

	doneChan := make(chan bool)
	go func() {
		newSuccessCount := 0
		newFailureCount := 0

		for res := range results {
			if !*debugMode && bar != nil {
				bar.Add(1)
			}

			if res.Success {
				msg := fmt.Sprintf("\033[32m[PASS] %s (IP: %s, Country: %s, Info: %s)\033[0m", res.Domain, res.IP, res.Country, res.Info)
				// 明确逻辑：CN/HK/MO 或 UNKNOWN 域名绝对不写入任何输出文件，改为记入失败库废弃
				// CRITICAL: Domains from CN/HK/MO or UNKNOWN MUST NOT be written to any output files.
				if res.Country != "" && !isBlockedCountry(res.Country) && res.Country != "UNKNOWN" {
					validDomainsMap[res.Country] = append(validDomainsMap[res.Country], res.Domain)
					newSuccessCount++
					if newSuccessCount >= 100 {
						batchSave(targetDir, validDomainsMap)
						for k := range validDomainsMap {
							delete(validDomainsMap, k)
						}
						newSuccessCount = 0
					}
				} else {
					// 虽然验证成功，但因为区域问题（CN/HK/MO/UNKNOWN）被废弃，记入 LevelDB
					failureList = append(failureList, res.Domain)
					newFailureCount++
					if newFailureCount >= 500 {
						appendFailureHistoryDB(failDB, failureList)
						failureList = failureList[:0]
						newFailureCount = 0
					}
				}
				if *debugMode {
					fmt.Println(msg)
				} else {
					fmt.Printf("\r\033[K%s\n", msg)
				}
			} else {
				// 明确逻辑：所有失败（包含 CN/HK/MO 跳过）都必须记入 LevelDB 失败库
				// CRITICAL: All failures AND skipped CN/HK/MO domains MUST be recorded in LevelDB failure history.
				// Record Failure
				msg := ""
				if isBlockedCountry(res.Country) {
					msg = fmt.Sprintf("\033[31m[SKIP] %s is in %s\033[0m", res.Domain, res.Country)
				} else {
					msg = fmt.Sprintf("[FAIL] %s: %s", res.Domain, res.Info)
				}

				failureList = append(failureList, res.Domain)
				newFailureCount++
				if newFailureCount >= 500 {
					appendFailureHistoryDB(failDB, failureList)
					failureList = failureList[:0]
					newFailureCount = 0
				}
				if *debugMode {
					fmt.Println(msg)
				} else if isBlockedCountry(res.Country) || res.Country == "UNKNOWN" {
					fmt.Printf("\r\033[K%s\n", msg)
				}
			}
		}

		// Final Batch Save
		if len(validDomainsMap) > 0 {
			batchSave(targetDir, validDomainsMap)
		}
		if len(failureList) > 0 {
			appendFailureHistoryDB(failDB, failureList)
		}

		if !*debugMode && bar != nil {
			bar.Finish()
			fmt.Println()
		}
		doneChan <- true
	}()

	// 5. Streaming Feed with Smart Filter
	file, err := os.Open(*inputFile)
	if err != nil {
		fmt.Printf("Error opening input: %v\n", err)
		os.Exit(1)
	}

	skippedCount := 0
	lineNum := 0
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		lineNum++
		if *maxLines > 0 && lineNum > *maxLines {
			break
		}
		raw := scanner.Text()
		domain := cleanDomain(raw)
		if domain == "" {
			if !*debugMode {
				skippedCount++
			}
			continue
		}

		// 1. Skip if already succeeded
		if _, exists := successMap[domain]; exists {
			if !*debugMode {
				skippedCount++
			}
			continue
		}

		// 2. Skip if failed recently (LevelDB lookup)
		if isFailedRecently(failDB, domain, now, ttlSec) {
			if !*debugMode {
				skippedCount++
			}
			continue
		}

		// Mark as seen in this session to avoid duplicates in input file
		successMap[domain] = struct{}{}

		// To keep progress bar in sync when we skip in feed
		if skippedCount > 0 && !*debugMode && bar != nil {
			bar.Add(skippedCount)
			skippedCount = 0
		}

		jobs <- domain
	}
	if err := scanner.Err(); err != nil {
		fmt.Printf("\nError scanning input: %v\n", err)
	}
	file.Close()
	close(jobs)
	wg.Wait()
	close(results)
	<-doneChan

	fmt.Println("Task completed successfully.")
}

// Byte-level high-performance parser
func cleanDomain(raw string) string {
	raw = strings.TrimSpace(raw)
	if len(raw) == 0 || raw[0] == '#' || (len(raw) >= 2 && raw[0:2] == "//") {
		return ""
	}

	var parts []string
	if idx := strings.IndexByte(raw, ','); idx != -1 {
		parts = strings.SplitN(raw, ",", 3)
	} else if strings.IndexByte(raw, '\t') != -1 {
		parts = strings.Split(raw, "\t")
	} else {
		parts = strings.Fields(raw)
	}

	for _, part := range parts {
		part = strings.TrimSpace(part)
		if len(part) == 0 {
			continue
		}
		if isNumeric(part) {
			continue
		}
		if strings.IndexByte(part, '.') != -1 {
			part = strings.Trim(part, `"',`)
			if idx := strings.IndexByte(part, ':'); idx != -1 {
				part = part[:idx]
			}
			if len(part) <= 2 && (part == "A" || part == "B" || part == "ID") {
				continue
			}
			return part
		}
	}
	return ""
}

func isNumeric(s string) bool {
	if len(s) == 0 {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

func countLines(path string, limit int) (int, error) {
	f, err := os.Open(path)
	if err != nil {
		return 0, err
	}
	defer f.Close()
	count := 0
	buf := make([]byte, 64*1024)
	for {
		c, err := f.Read(buf)
		for i := 0; i < c; i++ {
			if buf[i] == '\n' {
				count++
				if limit > 0 && count >= limit {
					return count, nil
				}
			}
		}
		if err == io.EOF {
			break
		}
		if err != nil {
			return count, err
		}
	}
	return count, nil
}

// --- Persistence & History (LevelDB) ---

// cleanAndCountFailureHistory iterates all LevelDB entries once,
// deletes expired ones, and returns (activeCount, purgedCount).
func cleanAndCountFailureHistory(db *leveldb.DB, now int64, ttlSec int64) (int, int) {
	active := 0
	purged := 0
	batch := new(leveldb.Batch)

	iter := db.NewIterator(nil, nil)
	for iter.Next() {
		val := iter.Value()
		if len(val) == 8 {
			ts := int64(binary.LittleEndian.Uint64(val))
			if (now - ts) >= ttlSec {
				batch.Delete(iter.Key())
				purged++
				continue
			}
		} else {
			// Malformed entry, remove it
			batch.Delete(iter.Key())
			purged++
			continue
		}
		active++
	}
	iter.Release()

	if purged > 0 {
		db.Write(batch, nil)
	}

	return active, purged
}

// isFailedRecently checks LevelDB for a domain and returns true if it failed within the TTL.
func isFailedRecently(db *leveldb.DB, domain string, now int64, ttlSec int64) bool {
	val, err := db.Get([]byte(domain), nil)
	if err != nil {
		return false // not found or error
	}
	if len(val) != 8 {
		return false
	}
	lastFail := int64(binary.LittleEndian.Uint64(val))
	return (now - lastFail) < ttlSec
}

// appendFailureHistoryDB writes a batch of failed domains into LevelDB.
func appendFailureHistoryDB(db *leveldb.DB, domains []string) {
	now := time.Now().Unix()
	buf := make([]byte, 8)
	binary.LittleEndian.PutUint64(buf, uint64(now))

	batch := new(leveldb.Batch)
	for _, d := range domains {
		batch.Put([]byte(d), buf)
	}
	db.Write(batch, nil)
}

func loadExistingIntoMap(dir string, m map[string]struct{}) {
	files, _ := filepath.Glob(filepath.Join(dir, "*.txt"))
	for _, f := range files {
		baseName := strings.ToUpper(filepath.Base(f))
		if baseName == "CN.TXT" || baseName == "HK.TXT" || baseName == "MO.TXT" {
			continue
		}
		file, err := os.Open(f)
		if err != nil {
			continue
		}
		sc := bufio.NewScanner(file)
		for sc.Scan() {
			d := cleanDomain(sc.Text())
			if d != "" {
				m[d] = struct{}{}
			}
		}
		file.Close()
	}
}

func batchSave(targetDir string, m map[string][]string) {
	for country, list := range m {
		writeTargetFile(targetDir, country, list)
	}
}

func writeTargetFile(targetDir string, countryCode string, domains []string) error {
	filename := fmt.Sprintf("%s.txt", strings.ToUpper(countryCode))
	targetPath := filepath.Join(targetDir, filename)
	os.MkdirAll(targetDir, 0o755)
	f, err := os.OpenFile(targetPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return err
	}
	defer f.Close()
	for _, d := range domains {
		f.WriteString(d + "\n")
	}
	return nil
}

// --- Utils ---

func findTargetDir() string {
	cwd, _ := os.Getwd()
	dir := cwd
	for {
		target := filepath.Join(dir, "rust", "tgbot", "src", "resources", "sni")
		if info, err := os.Stat(target); err == nil && info.IsDir() {
			return target
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return ""
}

func checkSNI(domain string, targetIP string, debug bool, xhttp bool, reality bool, resolver *net.Resolver) (bool, string, string) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}

	// 始终使用已解析的 IP 地址进行连接，不再通过域名拨号
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(context.Background(), "tcp", addr)
	if err != nil {
		return false, "", err.Error()
	}
	config := &utls.Config{
		ServerName: domain,
		MinVersion: utls.VersionTLS12,
		MaxVersion: utls.VersionTLS13,
		NextProtos: []string{"h2", "http/1.1"},
	}
	if reality || xhttp {
		config.MinVersion = utls.VersionTLS13
	}
	uConn := utls.UClient(rawConn, config, utls.HelloChrome_Auto)
	defer uConn.Close()
	uConn.SetDeadline(time.Now().Add(10 * time.Second))
	if err := uConn.Handshake(); err != nil {
		return false, "", err.Error()
	}
	state := uConn.ConnectionState()

	if state.Version != utls.VersionTLS13 && (reality || xhttp) {
		return false, "", fmt.Sprintf("Requirement: TLS 1.3 (got %04x)", state.Version)
	}

	remoteAddr := uConn.RemoteAddr().String()
	ip, _, _ := net.SplitHostPort(remoteAddr)

	if reality {
		// Reality usually has H2, but we prioritize the X25519 requirement per user
		// X25519 key exchange check (accept X25519, X25519MLKEM768, X25519Kyber768Draft00)
		hs := uConn.HandshakeState
		if hs.ServerHello != nil {
			group := hs.ServerHello.ServerShare.Group
			if group != utls.X25519 && group != utls.X25519MLKEM768 && group != utls.X25519Kyber768Draft00 {
				return false, "", fmt.Sprintf("Reality: key exchange not X25519-based (got %d)", group)
			}
		}
	}

	h3Supported := false
	if xhttp {
		// H2/H3 requirement. Check H3 via Alt-Svc if TCP ALPN is not H2 or just as extra verification.
		h3Supported = checkH3Support(domain, ip, resolver)
		if state.NegotiatedProtocol != "h2" && !h3Supported {
			return false, "", "XHTTP: Neither H2 nor H3 support detected"
		}
	}

	// For XHTTP info display
	info := "Validated"
	if xhttp {
		if state.NegotiatedProtocol == "h2" && h3Supported {
			info = "Validated (H2+H3)"
		} else if h3Supported {
			info = "Validated (H3 only)"
		} else {
			info = "Validated (H2 only)"
		}
	}

	return true, ip, info
}

// checkH3Support makes a HEAD request and checks Alt-Svc header for H3 support.
func checkH3Support(domain string, targetIP string, resolver *net.Resolver) bool {
	transport := &http.Transport{
		TLSClientConfig: &tls.Config{ServerName: domain},
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			connectAddr := addr
			if targetIP != "" {
				_, port, _ := net.SplitHostPort(addr)
				connectAddr = net.JoinHostPort(targetIP, port)
			}
			return (&net.Dialer{Timeout: 5 * time.Second, Resolver: resolver}).DialContext(ctx, "tcp", connectAddr)
		},
		ForceAttemptHTTP2: true,
	}
	client := &http.Client{Transport: transport, Timeout: 8 * time.Second}

	req, err := http.NewRequest("HEAD", "https://"+domain, nil)
	if err != nil {
		return false
	}
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()

	altSvc := resp.Header.Get("Alt-Svc")
	return strings.Contains(altSvc, "h3")
}

func checkNetworkConnectivity(dnsAddr string, dnsUDP string) error {
	resolver := &net.Resolver{PreferGo: true, Dial: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return net.DialTimeout(dnsUDP, dnsAddr, 5*time.Second)
	}}
	transport := &http.Transport{DialContext: (&net.Dialer{Timeout: 10 * time.Second, Resolver: resolver}).DialContext}
	client := &http.Client{Transport: transport, Timeout: 10 * time.Second}
	resp, err := client.Get("https://google.com")
	if err != nil {
		return err
	}
	resp.Body.Close()
	return nil
}

func downloadFile(filepath string, urlStr string, proxyString string) error {
	transport := &http.Transport{}
	if proxyString != "" {
		pu, _ := url.Parse(proxyString)
		if pu.Scheme == "http" || pu.Scheme == "https" {
			transport.Proxy = http.ProxyURL(pu)
		}
		if pu.Scheme == "socks5" {
			dialer, _ := proxy.FromURL(pu, proxy.Direct)
			transport.DialContext = (dialer.(proxy.ContextDialer)).DialContext
		}
	}
	client := &http.Client{Transport: transport, Timeout: 10 * time.Minute}
	resp, err := client.Get(urlStr)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	out, _ := os.Create(filepath)
	defer out.Close()
	io.Copy(out, resp.Body)
	return nil
}

func prepareGeoDB(proxyString string) {
	if _, err := os.Stat(GeoDBFile); os.IsNotExist(err) {
		fmt.Println("GeoLite2-Country.mmdb not found. Trying download...")
		if err := downloadFile(GeoDBFile, GeoDBURL, proxyString); err != nil {
			fmt.Printf("GeoIP download failed: %v\n", err)
			os.Exit(1)
		}
		fmt.Println("Download complete.")
	}
}
