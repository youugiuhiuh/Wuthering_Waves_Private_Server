package main

import (
	"bufio"
	"context"
	crand "crypto/rand"
	"crypto/tls"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/oschwald/geoip2-golang"
	utls "github.com/refraction-networking/utls"
	"github.com/schollz/progressbar/v3"
	"github.com/syndtr/goleveldb/leveldb"
	"github.com/syndtr/goleveldb/leveldb/opt"
	"golang.org/x/net/proxy"
)

// Internal DNS Pool (International Public DNS)
var DnsPool = []string{
	// --- 1级超大容量/超高并发 (优先) ---
	"1.1.1.1", "1.0.0.1", "1.1.1.2", "1.0.0.2", "1.1.1.3", "1.0.0.3", // Cloudflare
	"8.8.8.8", "8.8.4.4", // Google
	"9.9.9.9", "149.112.112.112", "149.112.112.9", // Quad9
	"208.67.222.222", "208.67.220.220", "208.67.222.123", "208.67.220.123", "208.67.222.220", "208.67.220.222", // OpenDNS (Cisco)

	// --- 2级优秀大型公共 DNS ---
	"8.26.56.26", "8.20.247.20", // Comodo Secure DNS
	"156.154.70.2", "156.154.71.2", "156.154.70.3", "156.154.71.3", // Neustar UltraDNS
	"94.140.14.14", "94.140.15.15", "94.140.14.140", "94.140.14.141", "94.140.14.15", "94.140.15.16", // AdGuard DNS (高配)
	"64.6.64.6", "64.6.65.6", // Verisign

	// --- 3级区域性/备用 DNS ---
	"4.2.2.1", "4.2.2.2", "4.2.2.3", "4.2.2.4", "4.2.2.5", "4.2.2.6", // Level3
	"77.88.8.1", "77.88.8.2", "77.88.8.3", "77.88.8.7", "77.88.8.8", "77.88.8.88", // Yandex (俄罗斯大厂)
	"80.80.80.80", "80.80.81.81", // Freenom
	"45.11.45.11",     // DNS.SB
	"185.222.222.222", // DNS.SB (IPv4 secondary)

	// --- 4级国内骨干 DNS (防漏补强) ---
	"1.12.12.12", "120.53.53.53", // Tencent Edge
	"119.29.29.29", "119.28.28.28", // DNSPod
	"223.5.5.5", "223.6.6.6", // AliDNS
	"114.114.114.114", "114.114.115.115", // 114 DNS
	"114.114.114.110", "114.114.115.110",
	"114.114.114.119", "114.114.115.119",
	"180.76.76.76",               // Baidu
	"180.184.1.1", "180.184.2.2", // ByteDance
	"101.226.4.6", "218.30.118.6", "123.125.81.6", // 360 DNS
	"1.2.4.8", "210.2.4.8", // CNNIC
	"117.50.22.22", "52.80.66.66", // OneDNS
}

// Candidate TLS fingerprints (uTLS ClientHelloIDs) for 随机指纹
var clientHelloProfiles = []utls.ClientHelloID{
	utls.HelloChrome_Auto,
	utls.HelloFirefox_Auto,
	utls.HelloIOS_Auto,
}

// Candidate ALPN profile sets for 随机协商顺序
var alpnProfiles = [][]string{
	{"h2", "http/1.1"},
	{"http/1.1", "h2"},
	{"h2"},
	{"http/1.1"},
}

// Common browser User-Agent pool for HTTP 请求
var userAgentPool = []string{
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
	"Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
	"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
	"Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1",
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
}

// Global DNS Cache
var dnsCache sync.Map

// Round-robin counter for DNS pool
var dnsIndex uint32

// randIndex returns a 随机下标 in [0, n).
func randIndex(n int) int {
	if n <= 1 {
		return 0
	}
	max := big.NewInt(int64(n))
	v, err := crand.Int(crand.Reader, max)
	if err != nil {
		return 0
	}
	return int(v.Int64())
}

func pickClientHelloID() utls.ClientHelloID {
	return clientHelloProfiles[randIndex(len(clientHelloProfiles))]
}

func pickALPNProfile() []string {
	return alpnProfiles[randIndex(len(alpnProfiles))]
}

func pickUserAgent() string {
	return userAgentPool[randIndex(len(userAgentPool))]
}

// Config
const (
	InitialWorkers     = 100              // Starting concurrency
	MaxWorkers         = 2000             // Absolute maximum concurrency
	MinWorkers         = 10               // Absolute minimum concurrency
	JobBuffer          = 5000             // Job channel buffer size
	StreamingThreshold = 10 * 1024 * 1024 // 10MB threshold for streaming mode
	GeoDBFile          = "GeoLite2-Country.mmdb"
	GeoDBURL           = "https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/GeoLite2-Country.mmdb"
	GeoASNFile         = "GeoLite2-ASN.mmdb"
	GeoASNURL          = "https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/GeoLite2-ASN.mmdb"
	ASNHistoryDB       = "asn_blacklist.db"
)

// ASNInfo represents an ASN entry in the blacklist
type ASNInfo struct {
	Org     string
	Country string
	AddedAt int64
}

// Seed blocked ASN list (CN/HK/MO/IR/RU/KP)
var SeedBlockedASNs = map[uint32]string{
	45102:  "Alibaba US Technology Co., Ltd.",
	55967:  "Beijing Baidu Netcom Science and Technology Co., Ltd.",
	132203: "Tencent Building, Kejizhongyi Avenue",
	4808:   "China Unicom Beijing Province Network",
	4134:   "Chinanet",
	4811:   "China Telecom Group",
	38283:  "CHINANET SiChuan Telecom Internet Data Center",
	24151:  "China Internet Network Infomation Center",
	21859:  "Zenlayer Inc",
	17623:  "China Telecom (Beijing) IDC",
	4837:   "China Unicom IP network",
	58466:  "Shanghai Bell Networks",
	45275:  "Tencent Cloud Computing (Beijing)",
	58772:  "Alibaba (Beijing) Technology",
	24542:  "Chinanet IDC",
	23650:  "Chinanet Jiangsu Province Network",
	23764:  "Chinanet Guangdong Province Network",
	4538:   "China Telecom Group",
	38176:  "Tencent Cloud",
	17429:  "Tencent Cloud Computing",
	149979: "Tencent Cloud Computing",
	45081:  "Tencent Cloud Computing",
	141024: "Tencent Cloud Computing",
	136190: "Tencent Cloud",
	212238: "Tencent Cloud",
}

type ValidationResult struct {
	Domain  string
	Success bool
	IP      string
	Country string
	ASN     uint32
	Org     string
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
	dnsAddr := flag.String("dns", "", "DNS server address (optional, e.g. 119.29.29.29). If empty, uses built-in high-concurrency mainland DNS pool.")
	dohURL := flag.String("doh", "", "DNS-over-HTTPS endpoint (e.g. https://cloudflare-dns.com/dns-query). If set, use DoH instead of UDP DNS.")
	ttlDays := flag.Int("ttl", 7, "Days to remember failures (default 7)")
	maxLines := flag.Int("max", 0, "Max lines to read from input (0 = unlimited)")
	fixedWorkers := flag.Int("w", 0, "Fixed worker count (disables AIMD automatic scaling)")

	// Optional: auto-shutdown after task completion
	autoShutdown := flag.Bool("shutdown", false, "Shutdown system immediately after task completion")

	// XHTTP & Reality 专项校验参数
	xhttpMode := flag.Bool("xhttp", false, "Enable XHTTP validation (H2 minimum)")
	realityMode := flag.Bool("reality", false, "Enable Reality validation (TLS 1.3, X25519, H2)")

	// Import mode
	importFile := flag.String("import", "", "Import result JSON file from mobile app")

	// ADB mode
	adbMode := flag.Bool("adb", false, "Use ADB to run test on connected Android device")
	adbWiFi := flag.Bool("adb-wifi", false, "Enable WiFi ADB and show QR code for wireless connection")

	flag.Parse()

	// Handle import mode
	if *importFile != "" {
		handleImport(*importFile)
		return
	}

	// Handle ADB mode
	if *adbMode || *adbWiFi {
		if *inputFile == "" {
			fmt.Println("Usage: sni_tester -adb -f <input_file> [-xhttp] [-reality]")
			fmt.Println("  Example: sni_tester -adb -f domains.txt -reality")
			os.Exit(1)
		}
		runADBMode(*adbWiFi, *inputFile, *realityMode, *xhttpMode)
		return
	}

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester -f <input_file> [-dns <dns_server>] [-w <workers>] [-debug] [-p <proxy>] [-xhttp] [-reality] [-ttl <days>] [-max <lines>]")
		fmt.Println("  Example: sni_tester -f domains.txt (uses built-in top international DNS pool)")
		fmt.Println("  Example: sni_tester -f domains.txt -w 2000 (disables AIMD, forces 2000 workers)")
		fmt.Println("  Example: sni_tester -f domains.txt -dns 1.1.1.1")
		os.Exit(1)
	}

	useBuiltInDNS := *dnsAddr == ""
	var dnsUDP string
	if !useBuiltInDNS {
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
		dnsUDP = "udp4"
		if net.ParseIP(dnsHost) != nil && net.ParseIP(dnsHost).To4() == nil {
			dnsUDP = "udp6"
		}
		fmt.Printf("DNS: %s (transport: %s)\n", *dnsAddr, dnsUDP)
	} else {
		fmt.Printf("DNS: Built-in High-Concurrency International DNS Pool (%d Servers) + Memory Cache\n", len(DnsPool))
		dnsUDP = "udp4" // Built-in pool are all IPv4
	}

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

	// 0. Check network connectivity (only if using explicit single DNS)
	if !useBuiltInDNS {
		if err := checkNetworkConnectivity(*dnsAddr, dnsUDP); err != nil {
			fmt.Printf("Network check: %v (Accessible: false)\n", err)
		} else {
			fmt.Println("Network connectivity check passed - https://google.com is accessible.")
			if !*debugMode {
				fmt.Println("This program should only run when google.com is NOT accessible (unless -debug).")
				os.Exit(1)
			}
		}
	} else {
		// For built in pool, just do a quick check against the first server to verify general connectivity
		if err := checkNetworkConnectivity(DnsPool[0]+":53", "udp4"); err != nil {
			fmt.Printf("Network check: %v (Accessible: false)\n", err)
		} else {
			fmt.Println("Network connectivity check passed - https://google.com is accessible.")
			if !*debugMode {
				fmt.Println("This program should only run when google.com is NOT accessible (unless -debug).")
				os.Exit(1)
			}
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
	defer os.Remove(GeoDBFile)

	// 0.2 ASN DB Handling
	prepareGeoASNDB(*proxyString)

	asnDB, err := geoip2.Open(GeoASNFile)
	if err != nil {
		fmt.Printf("Error opening ASN DB: %v\n", err)
		os.Exit(1)
	}
	defer asnDB.Close()
	defer os.Remove(GeoASNFile)

	// 0.3 ASN Blacklist DB
	asnBlocklistDB, err := leveldb.OpenFile(ASNHistoryDB, &opt.Options{
		WriteBuffer:            8 * 1024 * 1024,
		CompactionTableSize:    4 * 1024 * 1024,
		BlockCacheCapacity:     4 * 1024 * 1024,
		OpenFilesCacheCapacity: 32,
	})
	if err != nil {
		fmt.Printf("Error opening ASN blacklist DB: %v\n", err)
		os.Exit(1)
	}
	defer asnBlocklistDB.Close()

	now := time.Now().Unix()
	ttlSec := int64(*ttlDays * 24 * 3600)

	// 加载种子 ASN 黑名单到内存
	var asnBlocklist sync.Map
	for asn, org := range SeedBlockedASNs {
		asnBlocklist.Store(asn, ASNInfo{Org: org, Country: "SEED", AddedAt: now})
	}

	// 清理并加载持久化 ASN 黑名单
	asnPurged := cleanASNBlacklist(asnBlocklistDB, now, ttlSec)
	if asnPurged > 0 {
		fmt.Printf("Purged %d expired entries from ASN blacklist.\n", asnPurged)
	}
	loadASNBlacklist(asnBlocklistDB, &asnBlocklist)

	fmt.Printf("ASN blacklist loaded: %d entries\n", getASNBlocklistCount(&asnBlocklist))

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

	failCount, purged := cleanAndCountFailureHistory(failDB, now, ttlSec)
	if purged > 0 {
		fmt.Printf("Purged %d expired entries from failure history.\n", purged)
	}

	fmt.Printf("Memory loaded: %d succeeded, %d failed in history.\n", len(successMap), failCount)

	// 2.1 Shared DNS Resolver Initialization
	var resolver *net.Resolver
	if useBuiltInDNS {
		resolver = &net.Resolver{
			PreferGo: true,
			Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
				d := net.Dialer{Timeout: 3 * time.Second}
				// Round-robin index selection
				idx := atomic.AddUint32(&dnsIndex, 1) % uint32(len(DnsPool))
				targetDns := DnsPool[idx] + ":53"
				return d.DialContext(ctx, "udp4", targetDns)
			},
		}
	} else {
		resolver = &net.Resolver{
			PreferGo: true,
			Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
				d := net.Dialer{Timeout: 5 * time.Second}
				return d.DialContext(ctx, dnsUDP, *dnsAddr)
			},
		}
	}

	// Optional: DNS-over-HTTPS (DoH) client
	useDoH := *dohURL != ""
	var dohClient *http.Client
	if useDoH {
		dohClient = &http.Client{
			Timeout: 5 * time.Second,
		}
		fmt.Printf("DoH enabled: %s\n", *dohURL)
	}

	// 3. Setup Dynamic Workers (AIMD Concurrency Controller)
	jobs := make(chan string, JobBuffer)

	maxConcurrent := MaxWorkers
	if *fixedWorkers > 0 {
		maxConcurrent = *fixedWorkers
	}
	results := make(chan ValidationResult, maxConcurrent)
	var wg sync.WaitGroup

	// Dynamic Semaphore for controlling active workers
	workerSemaphore := make(chan struct{}, maxConcurrent)
	// Initialize with starting workers
	currentWorkers := InitialWorkers
	if *fixedWorkers > 0 {
		currentWorkers = *fixedWorkers
	}
	for i := 0; i < currentWorkers; i++ {
		workerSemaphore <- struct{}{}
	}
	// This function spawns a single worker that pulls from the jobs channel.
	// It stops pulling and exits when the semaphore is drained (downscaling).
	spawnWorker := func() {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				// Try to acquire/hold a token. If we can't, it means we've been downscaled.
				select {
				case <-workerSemaphore:
					// We hold a token, proceed to get a job
				default:
					return // Downscaled: Semaphore is empty, exit this worker
				}

				domain, ok := <-jobs
				if !ok {
					// Jobs channel closed, we are done
					workerSemaphore <- struct{}{} // Return the token before exiting
					return
				}

				// 1. DNS Resolution (Check cache first)
				var ip string
				if cachedIP, hit := dnsCache.Load(domain); hit {
					ip = cachedIP.(string)
				} else {
					var ips []string
					var err error
					if useDoH {
						ips, err = lookupHostDoH(dohClient, *dohURL, domain)
					} else {
						ips, err = resolver.LookupHost(context.Background(), domain)
					}
					if err != nil || len(ips) == 0 {
						errMsg := "DNS resolution failed"
						if err != nil {
							errMsg = err.Error()
						}
						results <- ValidationResult{Domain: domain, Success: false, IP: "", Country: "UNKNOWN", Info: errMsg}
						workerSemaphore <- struct{}{} // Return the token
						continue
					}
					ip = ips[0]
					dnsCache.Store(domain, ip) // Store in cache for future hits
				}

				countryCode := "UNKNOWN"
				record, geoErr := db.Country(net.ParseIP(ip))
				if geoErr == nil {
					if record.Country.IsoCode != "" {
						countryCode = record.Country.IsoCode
					} else if record.RegisteredCountry.IsoCode != "" {
						countryCode = record.RegisteredCountry.IsoCode
					}
				}

				// 先检查国家黑名单 (使用 ISO 代码)
				if isBlockedCountry(countryCode) {
					// 动态学习 ASN: 查询该 IP 的 ASN 并加入黑名单
					asn, org := getASN(net.ParseIP(ip), asnDB)
					if asn > 0 {
						addASNToBlacklist(asnBlocklistDB, asn, org, countryCode)
					}
					results <- ValidationResult{Domain: domain, Success: false, IP: ip, Country: countryCode, Info: fmt.Sprintf("Skipped (Country: %s)", countryCode)}
					workerSemaphore <- struct{}{} // Return the token
					continue
				}

				// 2.1 ASN Check - 跳过黑名单中的 ASN
				asn, org := getASN(net.ParseIP(ip), asnDB)
				if asn > 0 && isASNBlocked(asn, &asnBlocklist) {
					results <- ValidationResult{Domain: domain, Success: false, IP: ip, Country: countryCode, ASN: asn, Org: org, Info: fmt.Sprintf("Skipped (ASN: %d %s)", asn, org)}
					workerSemaphore <- struct{}{}
					continue
				}

				// 3. Perform TLS Handshake
				success, finalIP, info := checkSNI(domain, ip, *debugMode, *xhttpMode, *realityMode, resolver)
				if finalIP != "" {
					ip = finalIP
				}

				if countryCode == "UNKNOWN" && finalIP != "" {
					record, geoErr := db.Country(net.ParseIP(finalIP))
					if geoErr == nil {
						if record.Country.IsoCode != "" {
							countryCode = record.Country.IsoCode
						} else if record.RegisteredCountry.IsoCode != "" {
							countryCode = record.RegisteredCountry.IsoCode
						}
					}
				}
				if countryCode == "" {
					countryCode = "UNKNOWN"
				}

				results <- ValidationResult{Domain: domain, Success: success, IP: ip, Country: countryCode, ASN: asn, Org: org, Info: info}
				workerSemaphore <- struct{}{} // Return the token
			}
		}()
	}

	// Spawn initial workers
	for i := 0; i < currentWorkers; i++ {
		spawnWorker()
	}

	var bar *progressbar.ProgressBar
	if !*debugMode {
		bar = progressbar.Default(int64(totalLines), "Testing")
	}

	validDomainsMap := make(map[string][]string)
	failureList := make([]string, 0, 100)

	doneChan := make(chan bool)

	// 统计变量
	var stats struct {
		mu             sync.Mutex
		total          int
		success        int
		failed         int
		skippedCountry int
		skippedASN     int
		countryStats   map[string]int
	}
	stats.countryStats = make(map[string]int)

	go func() {
		newSuccessCount := 0
		newFailureCount := 0

		// AIMD State Variables
		consecutiveSuccesses := 0
		lastScaleDown := time.Now()

		for res := range results {
			stats.mu.Lock()
			stats.total++
			stats.mu.Unlock()

			if !*debugMode && bar != nil {
				bar.Add(1)
			}

			if res.Success {
				msg := fmt.Sprintf("\033[32m[PASS] %s (IP: %s, Country: %s, Info: %s)\033[0m", res.Domain, res.IP, res.Country, res.Info)
				stats.mu.Lock()
				stats.success++
				stats.countryStats[res.Country]++
				stats.mu.Unlock()
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
				stats.mu.Lock()
				if isBlockedCountry(res.Country) {
					msg = fmt.Sprintf("\033[31m[SKIP] %s is in %s\033[0m", res.Domain, res.Country)
					stats.skippedCountry++
				} else if strings.Contains(res.Info, "ASN:") {
					msg = fmt.Sprintf("\033[31m[SKIP] %s (ASN blocked)\033[0m", res.Domain)
					stats.skippedASN++
				} else {
					msg = fmt.Sprintf("[FAIL] %s: %s", res.Domain, res.Info)
					stats.failed++
				}
				stats.mu.Unlock()

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

			// --- AIMD Concurrency Control Logic ---
			if *fixedWorkers == 0 {
				// Is this a network/load error? (DNS lookup failed, i/o timeout, TLS handshake timeout)
				isNetworkError := !res.Success && (strings.Contains(res.Info, "lookup") || strings.Contains(res.Info, "timeout") || strings.Contains(res.Info, "i/o"))

				if res.Success || (!res.Success && !isNetworkError) {
					// Successful network interaction (even if the domain is just an invalid SNI, the network is responding fine)
					consecutiveSuccesses++
					// Additive Increase: Every 50 smooth operations, add 20 workers
					if consecutiveSuccesses >= 50 && currentWorkers < MaxWorkers {
						consecutiveSuccesses = 0
						increment := 20
						if currentWorkers+increment > MaxWorkers {
							increment = MaxWorkers - currentWorkers
						}
						currentWorkers += increment
						for i := 0; i < increment; i++ {
							workerSemaphore <- struct{}{}
							spawnWorker()
						}
					}
				} else if isNetworkError {
					consecutiveSuccesses = 0 // Reset smooth streak

					// Multiplicative Decrease: Halve workers, with cooldown to avoid over-shrinking
					if time.Since(lastScaleDown) > 2*time.Second {
						newWorkerCount := currentWorkers / 2
						if newWorkerCount < MinWorkers {
							newWorkerCount = MinWorkers
						}
						reduction := currentWorkers - newWorkerCount

						// Drain semaphore to trigger workers to exit
						for i := 0; i < reduction; i++ {
							select {
							case <-workerSemaphore:
								// Successfully drained a token
							default:
								// Semaphore already empty, break outer loop
								goto drainDone
							}
						}
					drainDone:

						currentWorkers = newWorkerCount
						lastScaleDown = time.Now()
						if !*debugMode && bar != nil {
							bar.Describe(fmt.Sprintf("Testing (W:%d)", currentWorkers))
						} else {
							fmt.Printf("\r\033[K⚠️ Network congested. Scale down workers to %d\n", currentWorkers)
						}
					}
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

	// Deduplication Phase
	fmt.Println("\nRunning post-scan deduplication on output files...")
	deduplicateTargetDir(targetDir)

	// 输出统计信息
	fmt.Println("\n========================================")
	fmt.Println("              测试统计                  ")
	fmt.Println("========================================")
	fmt.Printf("  总计:     %d\n", stats.total)
	fmt.Printf("  成功:     \033[32m%d\033[0m\n", stats.success)
	fmt.Printf("  失败:     \033[31m%d\033[0m\n", stats.failed)
	fmt.Printf("  跳过(CN): %d\n", stats.skippedCountry)
	fmt.Printf("  跳过(ASN): %d\n", stats.skippedASN)
	fmt.Println("----------------------------------------")
	fmt.Println("  按国家分布:")
	for country, count := range stats.countryStats {
		fmt.Printf("    %s: %d\n", country, count)
	}
	fmt.Println("========================================")

	// 构建通知消息
	notifyMsg := fmt.Sprintf("成功: %d | 失败: %d | 跳过(CN): %d | 跳过(ASN): %d",
		stats.success, stats.failed, stats.skippedCountry, stats.skippedASN)

	// Emit desktop notification based on OS
	if runtime.GOOS == "windows" {
		psScript := fmt.Sprintf(`
Add-Type -AssemblyName System.Windows.Forms
$notify = New-Object System.Windows.Forms.NotifyIcon
$notify.Icon = [System.Drawing.SystemIcons]::Information
$notify.BalloonTipTitle = 'SNI Tester'
$notify.BalloonTipText = '%s'
$notify.Visible = $True
$notify.ShowBalloonTip(5000)
Start-Sleep -Seconds 5
$notify.Dispose()
`, notifyMsg)
		exec.Command("powershell", "-NoProfile", "-WindowStyle", "Hidden", "-Command", psScript).Start()
	} else if runtime.GOOS == "darwin" {
		exec.Command("osascript", "-e", fmt.Sprintf(`display notification "%s" with title "SNI Tester"`, notifyMsg)).Start()
	} else {
		// Try to find notify-send and emit desktop notification for Linux
		if path, err := exec.LookPath("notify-send"); err == nil {
			exec.Command(path, "-u", "normal", "-t", "5000", "SNI Tester", notifyMsg).Start()
		}
	}

	// Optional: auto-shutdown (may require administrative privileges)
	if *autoShutdown {
		fmt.Println("Auto-shutdown requested. Trying to shutdown system...")
		switch runtime.GOOS {
		case "windows":
			// Shutdown after 5 seconds
			_ = exec.Command("shutdown", "/s", "/t", "5").Start()
		case "darwin", "linux":
			// Requires appropriate permissions (e.g. run as root or sudo)
			_ = exec.Command("shutdown", "-h", "now").Start()
		default:
			fmt.Println("Auto-shutdown is not supported on this OS.")
		}
	}
}

// deduplicateTargetDir reads all .txt files in the given directory and removes duplicate lines.
func deduplicateTargetDir(dir string) {
	files, err := os.ReadDir(dir)
	if err != nil {
		fmt.Printf("Error reading target dir for deduplication: %v\n", err)
		return
	}

	for _, f := range files {
		if !f.IsDir() && strings.HasSuffix(f.Name(), ".txt") {
			filePath := filepath.Join(dir, f.Name())
			deduplicateFile(filePath)
		}
	}
}

func deduplicateFile(filePath string) {
	file, err := os.Open(filePath)
	if err != nil {
		return
	}

	uniqueLines := make(map[string]struct{})
	var lines []string

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		if _, exists := uniqueLines[line]; !exists {
			uniqueLines[line] = struct{}{}
			lines = append(lines, line)
		}
	}
	file.Close()

	// Write back
	err = os.WriteFile(filePath, []byte(strings.Join(lines, "\n")+"\n"), 0o644)
	if err == nil {
		fmt.Printf("Deduplicated: %s (%d unique lines)\n", filepath.Base(filePath), len(lines))
	} else {
		fmt.Printf("Failed to write deduplicated file %s: %v\n", filePath, err)
	}
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
		writeBinaryDomainFile(targetDir, country, list)
	}
}

func writeBinaryDomainFile(targetDir string, countryCode string, domains []string) error {
	filename := fmt.Sprintf("%s.bin", strings.ToUpper(countryCode))
	targetPath := filepath.Join(targetDir, filename)
	os.MkdirAll(targetDir, 0o755)

	if len(domains) == 0 {
		return nil
	}

	sort.Strings(domains)
	domains = dedupeStrings(domains)

	f, err := os.Create(targetPath)
	if err != nil {
		return err
	}
	defer f.Close()

	for _, d := range domains {
		binary.Write(f, binary.BigEndian, uint16(len(d)))
		f.WriteString(d)
	}
	return nil
}

func dedupeStrings(sorted []string) []string {
	if len(sorted) == 0 {
		return sorted
	}
	result := []string{sorted[0]}
	for i := 1; i < len(sorted); i++ {
		if sorted[i] != result[len(result)-1] {
			result = append(result, sorted[i])
		}
	}
	return result
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
		TLSClientConfig: &tls.Config{
			ServerName: domain,
			NextProtos: pickALPNProfile(),
		},
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
	req.Header.Set("User-Agent", pickUserAgent())
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

// DoH JSON response structure (RFC 8484 JSON API used by Cloudflare, Google, etc.)
type dohJSONResponse struct {
	Answer []struct {
		Data string `json:"data"`
		Type int    `json:"type"`
	} `json:"Answer"`
}

// lookupHostDoH resolves A records for the given name using a DNS-over-HTTPS JSON API.
// It expects endpoints like https://cloudflare-dns.com/dns-query or https://dns.google/resolve.
func lookupHostDoH(client *http.Client, endpoint string, name string) ([]string, error) {
	if client == nil {
		client = &http.Client{Timeout: 5 * time.Second}
	}

	u, err := url.Parse(endpoint)
	if err != nil {
		return nil, fmt.Errorf("invalid DoH endpoint: %w", err)
	}
	q := u.Query()
	q.Set("name", name)
	q.Set("type", "A")
	u.RawQuery = q.Encode()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "GET", u.String(), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/dns-json")
	req.Header.Set("User-Agent", pickUserAgent())

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("DoH HTTP status %d", resp.StatusCode)
	}

	var parsed dohJSONResponse
	if err := json.NewDecoder(resp.Body).Decode(&parsed); err != nil {
		return nil, err
	}

	var ips []string
	for _, ans := range parsed.Answer {
		// Type 1 = A record
		if ans.Type == 1 && ans.Data != "" {
			ips = append(ips, ans.Data)
		}
	}
	if len(ips) == 0 {
		return nil, fmt.Errorf("no A records in DoH response")
	}
	return ips, nil
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

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("unexpected HTTP status: %d", resp.StatusCode)
	}

	out, err := os.Create(filepath)
	if err != nil {
		return err
	}
	defer out.Close()

	if _, err := io.Copy(out, resp.Body); err != nil {
		return err
	}

	return out.Close()
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

func prepareGeoASNDB(proxyString string) {
	if _, err := os.Stat(GeoASNFile); os.IsNotExist(err) {
		fmt.Println("GeoLite2-ASN.mmdb not found. Trying download...")
		if err := downloadFile(GeoASNFile, GeoASNURL, proxyString); err != nil {
			fmt.Printf("GeoASN download failed: %v\n", err)
			os.Exit(1)
		}
		fmt.Println("Download complete.")
	}
}

func getASN(ip net.IP, db *geoip2.Reader) (uint32, string) {
	record, err := db.ASN(ip)
	if err != nil {
		return 0, ""
	}
	return uint32(record.AutonomousSystemNumber), record.AutonomousSystemOrganization
}

func loadASNBlacklist(db *leveldb.DB, asnMap *sync.Map) {
	iter := db.NewIterator(nil, nil)
	for iter.Next() {
		key := string(iter.Key())
		var info ASNInfo
		if err := json.Unmarshal(iter.Value(), &info); err == nil {
			var asn uint64
			fmt.Sscanf(key, "%d", &asn)
			asnMap.Store(uint32(asn), info)
		}
	}
	iter.Release()
}

func isASNBlocked(asn uint32, asnMap *sync.Map) bool {
	_, ok := asnMap.Load(asn)
	return ok
}

func addASNToBlacklist(db *leveldb.DB, asn uint32, org, country string) {
	info := ASNInfo{
		Org:     org,
		Country: country,
		AddedAt: time.Now().Unix(),
	}
	data, _ := json.Marshal(info)
	key := fmt.Sprintf("%d", asn)
	db.Put([]byte(key), data, nil)
}

func cleanASNBlacklist(db *leveldb.DB, now int64, ttlSec int64) int {
	purged := 0
	batch := new(leveldb.Batch)

	iter := db.NewIterator(nil, nil)
	for iter.Next() {
		var info ASNInfo
		if err := json.Unmarshal(iter.Value(), &info); err == nil {
			if (now - info.AddedAt) >= ttlSec {
				batch.Delete(iter.Key())
				purged++
			}
		} else {
			batch.Delete(iter.Key())
			purged++
		}
	}
	iter.Release()

	if purged > 0 {
		db.Write(batch, nil)
	}
	return purged
}

func getASNBlocklistCount(asnMap *sync.Map) int {
	count := 0
	asnMap.Range(func(key, value interface{}) bool {
		count++
		return true
	})
	return count
}

type MobileResult struct {
	Version   string `json:"version"`
	Mode      string `json:"mode"`
	Timestamp string `json:"timestamp"`
	Results   []struct {
		Domain  string `json:"domain"`
		Success bool   `json:"success"`
		IP      string `json:"ip"`
		Country string `json:"country"`
		Info    string `json:"info"`
	} `json:"results"`
}

func handleImport(jsonFile string) {
	data, err := os.ReadFile(jsonFile)
	if err != nil {
		fmt.Printf("Error reading file: %v\n", err)
		os.Exit(1)
	}

	var result MobileResult
	if err := json.Unmarshal(data, &result); err != nil {
		fmt.Printf("Error parsing JSON: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("Importing %d results (mode: %s)\n", len(result.Results), result.Mode)

	baseTargetDir := findTargetDir()
	if baseTargetDir == "" {
		fmt.Println("Error: Could not find rust/tgbot/src/resources/sni directory.")
		os.Exit(1)
	}

	subDir := ""
	modeLower := strings.ToLower(result.Mode)
	if modeLower == "reality" {
		subDir = "reality"
	} else if modeLower == "xhttp" {
		subDir = "xhttp"
	}

	targetDir := baseTargetDir
	if subDir != "" {
		targetDir = filepath.Join(baseTargetDir, subDir)
	}

	os.MkdirAll(targetDir, 0755)

	countryMap := make(map[string][]string)
	for _, r := range result.Results {
		if !r.Success {
			continue
		}
		if r.Country == "" || r.Country == "UNKNOWN" {
			continue
		}
		if isBlockedCountry(r.Country) {
			continue
		}
		countryMap[r.Country] = append(countryMap[r.Country], r.Domain)
	}

	for country, domains := range countryMap {
		filename := fmt.Sprintf("%s.txt", strings.ToUpper(country))
		targetPath := filepath.Join(targetDir, filename)

		existing := make(map[string]bool)
		if f, err := os.Open(targetPath); err == nil {
			scanner := bufio.NewScanner(f)
			for scanner.Scan() {
				existing[scanner.Text()] = true
			}
			f.Close()
		}

		file, err := os.OpenFile(targetPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
		if err != nil {
			fmt.Printf("Error writing %s: %v\n", filename, err)
			continue
		}
		count := 0
		for _, domain := range domains {
			if !existing[domain] {
				file.WriteString(domain + "\n")
				count++
			}
		}
		file.Close()
		fmt.Printf("  %s: %d new domains added\n", country, count)
	}

	fmt.Printf("Import completed. Files saved to: %s\n", targetDir)
}
