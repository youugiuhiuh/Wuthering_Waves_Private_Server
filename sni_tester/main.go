package main

import (
	"bufio"
	"bytes"
	"context"
	crand "crypto/rand"
	"crypto/tls"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"math"
	"math/big"
	"math/rand"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/dgraph-io/badger/v4"
	"github.com/dgraph-io/badger/v4/options"
	"github.com/miekg/dns"
	"github.com/oschwald/geoip2-golang"
	utls "github.com/refraction-networking/utls"
	"github.com/schollz/progressbar/v3"
	"golang.org/x/net/proxy"
)

// DNS Failover Configuration
const (
	dnsServerTimeout = 800 * time.Millisecond // Single DNS server timeout
	dnsMaxServers    = 5                      // Max DNS servers to try per domain
	dnsRetryRounds   = 2                      // Retry rounds on failure
	dnsRetryDelay    = 100 * time.Millisecond // Delay between retry rounds
)

// DNS Priority: DoH → DoT → UDP

// DNSConfig separates DNS servers by protocol
type DNSConfig struct {
	DoH []string // DoH servers (RFC 8484 wire format)
	DoT []string // DoT servers (TLS 853)
	UDP []string // UDP DNS servers (IPv4 only)
}

// DNS Health Tracking Configuration
const (
	dnsHealthEpsilon      = 10.0 // Smoothing factor for new servers
	dnsMaxConsecutiveFail = 3    // Consecutive failures before decay
	dnsWeightDecay        = 0.5  // Weight decay factor per consecutive fail
	dnsMinWeight          = 0.05 // Minimum weight (5% floor)
	dnsRecoveryBoost      = 1.5  // Weight recovery multiplier on success
)

// DnsHealth tracks DNS server health and weight
type DnsHealth struct {
	SuccessCount    uint32
	FailCount       uint32
	ConsecutiveFail uint32
	Weight          float64
}

// DnsHealthMap stores health data for all DNS servers
var DnsHealthMap sync.Map

// DNS server pools by protocol (IPv4 only)
var DNS = DNSConfig{
	DoH: []string{
		// 国内 DoH
		"https://doh.pub/dns-query",        // 腾讯 DNSPod
		"https://dns.alidns.com/dns-query", // 阿里
		"https://dns.360.cn/dns-query",     // 360
		// 国外 DoH
		"https://1.1.1.1/dns-query",         // Cloudflare
		"https://dns.google/dns-query",      // Google
		"https://dns.quad9.net/dns-query",   // Quad9
		"https://doh.opendns.com/dns-query", // OpenDNS
		"https://dns.adguard.com/dns-query", // AdGuard
	},
	DoT: []string{
		// 国内 DoT
		"dot.pub:853",        // 腾讯 DNSPod
		"dns.alidns.com:853", // 阿里
		"dns.360.cn:853",     // 360
		// 国外 DoT
		"1.1.1.1:853",         // Cloudflare
		"dns.google:853",      // Google
		"dns.quad9.net:853",   // Quad9
		"dns.adguard.com:853", // AdGuard
	},
	UDP: []string{
		// 1级超大容量/超高并发 (优先)
		"1.1.1.1", "1.0.0.1", // Cloudflare
		"8.8.8.8", "8.8.4.4", // Google
		"9.9.9.9", "149.112.112.112", // Quad9
		"208.67.222.222", "208.67.220.220", // OpenDNS

		// 2级优秀大型公共 DNS
		"8.26.56.26", "8.20.247.20", // Comodo
		"94.140.14.14", "94.140.15.15", // AdGuard
		"64.6.64.6", "64.6.65.6", // Verisign

		// 3级区域性/备用 DNS
		"4.2.2.1", "4.2.2.2", "4.2.2.3", // Level3
		"77.88.8.1", "77.88.8.2", "77.88.8.7", "77.88.8.8", // Yandex
		"80.80.80.80", "80.80.81.81", // Freenom
		"45.11.45.11", "185.222.222.222", // DNS.SB

		// 4级国内骨干 DNS (IPv4 only)
		"119.29.29.29", "119.28.28.28", // DNSPod 腾讯
		"223.5.5.5", "223.6.6.6", // AliDNS 阿里
		"114.114.114.114", "114.114.115.115", // 114DNS 纯净版
		"114.114.114.110", "114.114.115.110", // 114DNS 家庭版
		"114.114.114.119", "114.114.115.119", // 114DNS 安全版
		"180.76.76.76",               // Baidu 百度
		"180.184.1.1", "180.184.2.2", // ByteDance
		"101.226.4.6", "218.30.118.6", "123.125.81.6", // 360 DNS
		"1.2.4.8", "210.2.4.8", // CNNIC
		"117.50.22.22", "117.50.11.11", // OneDNS
		"52.80.66.66",                // OneDNS 备用
		"120.53.53.53", "1.12.12.12", // 腾讯 Edge
	},
}

// Legacy flat pool for backwards compatibility
var DnsPool = DNS.UDP

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

// Debug mode flag
var isDebugMode bool

// DNS Prefetch
var dnsPrefetchQueue = make(chan string, 500)
var dnsPrefetchCache sync.Map

// Adaptive Timeout Controller
type TimeoutController struct {
	mu         sync.Mutex
	samples    []float64
	dnsSamples []float64
	tlsSamples []float64
	index      int
	baseDNS    time.Duration
	baseTLS    time.Duration
}

var timeoutCtrl = TimeoutController{
	samples:    make([]float64, 100),
	dnsSamples: make([]float64, 100),
	tlsSamples: make([]float64, 100),
	baseDNS:    2 * time.Second,
	baseTLS:    10 * time.Second,
}

func (t *TimeoutController) Record(duration time.Duration, kind string) {
	t.mu.Lock()
	defer t.mu.Unlock()

	var samples []float64
	switch kind {
	case "dns":
		samples = t.dnsSamples
	case "tls":
		samples = t.tlsSamples
	default:
		samples = t.samples
	}

	samples[t.index] = duration.Seconds()
	t.index = (t.index + 1) % len(samples)
}

func (t *TimeoutController) GetTimeout(kind string) time.Duration {
	t.mu.Lock()
	defer t.mu.Unlock()

	var samples []float64
	var base time.Duration
	switch kind {
	case "dns":
		samples = t.dnsSamples
		base = t.baseDNS
	case "tls":
		samples = t.tlsSamples
		base = t.baseTLS
	default:
		samples = t.samples
		base = t.baseDNS
	}

	sum := 0.0
	count := 0
	for _, s := range samples {
		if s > 0 {
			sum += s
			count++
		}
	}
	if count == 0 {
		return base
	}

	avg := sum / float64(count)

	variance := 0.0
	for _, s := range samples {
		if s > 0 {
			diff := s - avg
			variance += diff * diff
		}
	}
	std := 0.0
	if count > 1 {
		std = variance / float64(count-1)
	}
	std = math.Sqrt(std)

	timeout := avg + 2*std

	minTimeout := base
	maxTimeout := base * 5
	if timeout < minTimeout.Seconds() {
		timeout = minTimeout.Seconds()
	}
	if timeout > maxTimeout.Seconds() {
		timeout = maxTimeout.Seconds()
	}

	return time.Duration(timeout * 1e9)
}

// Graceful shutdown
var isShuttingDown atomic.Bool
var dbRef *badger.DB

func triggerGracefulShutdown() {
	if isShuttingDown.Load() {
		return
	}
	isShuttingDown.Store(true)
	fmt.Println("\n\n收到退出信号，正在优雅关闭...")
	fmt.Println("正在保存数据...")
	gracefulShutdown(dbRef)
	os.Exit(0)
}

func gracefulShutdown(db *badger.DB) {
	close(dnsPrefetchQueue)
	if db != nil {
		fmt.Println("正在关闭数据库...")
		db.RunValueLogGC(0.5)
		db.Close()
	}
	fmt.Println("再见!")
}

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
	BadgerDBDir        = "badger_db" // 统一的 BadgerDB 目录
)

// BadgerDB GC config
const (
	GCInterval = 15 * time.Minute // 定期GC间隔
	GCRatio    = 0.3              // GC清理比例
)

// IDM-style download config
const (
	DownloadChunkSize = 1024 * 1024 // 1MB per chunk
	DownloadWorkers   = 8           // Parallel download threads
)

// ASNInfo represents an ASN entry in the blacklist
type ASNInfo struct {
	Org     string
	Country string
	AddedAt int64
}

// SuccessInfo represents a successful domain entry
type SuccessInfo struct {
	Domain   string
	Country  string
	ASN      uint32
	Org      string
	TestedAt int64
}

// BlockedInfo represents a blocked domain entry
type BlockedInfo struct {
	Domain   string
	Reason   string // "COUNTRY" 或 "ASN"
	Code     string // 国家代码或ASN
	TestedAt int64
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

	// 优雅退出信号通道
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	// 启动信号监听 goroutine
	go func() {
		<-sigChan
		triggerGracefulShutdown()
	}()

	inputFile := flag.String("f", "", "Input TXT/CSV file containing SNIs")
	debugMode := flag.Bool("debug", false, "Enable debug logging")
	proxyString := flag.String("p", "", "Proxy for Geo download (http://127.0.0.1:10808 or socks5://127.0.0.1:10808)")
	dnsAddr := flag.String("dns", "", "DNS server address (optional, e.g. 119.29.29.29). If empty, uses built-in DNS pool (DoH → DoT → UDP).")
	ttlDays := flag.Int("ttl", 7, "Days to remember failures (default 7)")
	maxLines := flag.Int("max", 0, "Max lines to read from input (0 = unlimited)")
	fixedWorkers := flag.Int("w", 0, "Fixed worker count (disables AIMD automatic scaling)")

	// Optional: auto-shutdown after task completion
	autoShutdown := flag.Bool("shutdown", false, "Shutdown system immediately after task completion")
	forceRetry := flag.Bool("force", false, "Force re-test domains that were previously skipped (ignores failure history)")
	resetAll := flag.Bool("reset", false, "Clear all history (success + failures) and test from scratch. Also clears existing .bin output files.")

	// XHTTP & Reality 专项校验参数
	xhttpMode := flag.Bool("xhttp", false, "Enable XHTTP validation (H2 minimum)")
	realityMode := flag.Bool("reality", false, "Enable Reality validation (TLS 1.3, X25519, H2)")
	runBoth := flag.Bool("both", false, "Run both Reality and XHTTP modes automatically (reality → xhttp)")

	flag.Parse()

	// 参数冲突检测
	if *runBoth && (*xhttpMode || *realityMode) {
		fmt.Println("Error: -both cannot be used with -xhttp or -reality")
		fmt.Println("Use -both alone to run both modes sequentially.")
		fmt.Println("Use -xhttp or -reality for single mode.")
		os.Exit(1)
	}

	isDebugMode = *debugMode
	if isDebugMode {
		fmt.Println("[DEBUG] Debug mode enabled - skipping network isolation checks")
	}

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester -f <input_file> [options]")
		fmt.Println("Options:")
		fmt.Println("  -f <file>       Input TXT/CSV file containing SNIs (required)")
		fmt.Println("  -dns <addr>     DNS server address (default: built-in DNS pool)")
		fmt.Println("  -w <workers>    Fixed worker count (disables AIMD)")
		fmt.Println("  -debug          Enable debug logging")
		fmt.Println("  -force          Re-test previously skipped/failed domains")
		fmt.Println("  -reset          Clear all history and test from scratch")
		fmt.Println("  -p <proxy>      Proxy for GeoDB download")
		fmt.Println("  -ttl <days>     Days to remember failures (default: 7)")
		fmt.Println("  -max <lines>    Max lines to read from input")
		fmt.Println("  -shutdown       Shutdown system after completion")
		fmt.Println("")
		fmt.Println("Mode selection:")
		fmt.Println("  -reality        Enable Reality validation (TLS 1.3, X25519, H2)")
		fmt.Println("  -xhttp          Enable XHTTP validation (H2 minimum)")
		fmt.Println("  -both           Run both Reality and XHTTP modes automatically")
		fmt.Println("")
		fmt.Println("Examples:")
		fmt.Println("  ./sni_tester -f domains.txt                    # Default mode")
		fmt.Println("  ./sni_tester -f domains.txt -reality           # Reality mode only")
		fmt.Println("  ./sni_tester -f domains.txt -xhttp             # XHTTP mode only")
		fmt.Println("  ./sni_tester -f domains.txt -both               # Both modes (reality→xhttp)")
		fmt.Println("  ./sni_tester -f domains.txt -both -force -reset")
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

	// 确定存储子目录和模式前缀
	var modes []string
	if *runBoth {
		modes = []string{"reality", "xhttp"}
	} else if *realityMode {
		modes = []string{"reality"}
	} else if *xhttpMode {
		modes = []string{"xhttp"}
	} else {
		modes = []string{""} // 默认模式
	}

	// 显示执行模式
	if len(modes) > 0 && modes[0] != "" {
		fmt.Printf("\nRunning %d mode(s): %s\n", len(modes), strings.Join(modes, " → "))
	}

	// 设置第一模式的子目录设置 (用于初始检查)
	if len(modes) > 0 && modes[0] != "" {
		setModePrefix(modes[0])
	}

	baseTargetDir := findTargetDir()
	if baseTargetDir == "" {
		fmt.Println("Error: Could not find rust/tgbot/src/resources/sni directory.")
		os.Exit(1)
	}

	// 模式统计汇总
	type ModeResult struct {
		Mode           string
		Total          int
		Success        int
		Failed         int
		SkippedCountry int
		SkippedASN     int
		CountryStats   map[string]int
	}
	var allModeResults []ModeResult

	// 设置第一模式的 targetDir (用于网络检查和文件解析)
	targetDir := baseTargetDir
	if len(modes) > 0 && modes[0] != "" {
		targetDir = filepath.Join(baseTargetDir, modes[0])
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
			fmt.Println("[DEBUG] Skipping network isolation check")
		}
	}

	// 0.1 GeoIP/ASN DB Handling (download with network switch if needed)
	prepareGeoDBs(*proxyString)

	geoDB, err := geoip2.Open(GeoDBFile)
	if err != nil {
		fmt.Printf("Error opening GeoIP DB: %v\n", err)
		os.Exit(1)
	}
	defer geoDB.Close()
	defer os.Remove(GeoDBFile)

	// 0.2 ASN DB Handling
	asnDB, err := geoip2.Open(GeoASNFile)
	if err != nil {
		fmt.Printf("Error opening ASN DB: %v\n", err)
		os.Exit(1)
	}
	defer asnDB.Close()
	defer os.Remove(GeoASNFile)

	// 0.3 Unified BadgerDB for all data storage
	db, err := badger.Open(badger.DefaultOptions(BadgerDBDir).
		WithSyncWrites(true).            // 数据安全
		WithMemTableSize(64 << 20).      // 64MB
		WithValueLogFileSize(256 << 20). // 256MB
		WithCompression(options.ZSTD).
		WithNumVersionsToKeep(1).
		WithCompactL0OnClose(true)) // 关闭时压缩L0层
	if err != nil {
		fmt.Printf("Error opening BadgerDB: %v\n", err)
		os.Exit(1)
	}
	dbRef = db

	// 2. 启动时抽样验证数据完整性
	var corruptCount int
	db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		count := 0
		for iter.Seek([]byte("failed:")); count < 100; iter.Next() {
			if !iter.Valid() {
				break
			}
			_, err := iter.Item().ValueCopy(nil)
			if err != nil {
				corruptCount++
			}
			count++
		}
		return nil
	})
	if corruptCount > 0 {
		fmt.Printf("[Startup] Warning: %d corrupted entries found\n", corruptCount)
	}

	// 1. 启动定期 GC goroutine
	go func() {
		ticker := time.NewTicker(GCInterval)
		defer ticker.Stop()
		for {
			select {
			case <-ticker.C:
				if isShuttingDown.Load() {
					return
				}
				fmt.Println("[GC] Running value log garbage collection...")
				if err := db.RunValueLogGC(GCRatio); err != nil {
					fmt.Printf("[GC] Error: %v\n", err)
				}
			}
		}
	}()

	// 程序结束时运行 GC 清理过期数据
	defer func() {
		if dbRef != nil {
			dbRef.RunValueLogGC(0.5)
		}
	}()

	now := time.Now().Unix()
	ttlDaysValue = *ttlDays
	ttlSec := int64(ttlDaysValue * 24 * 3600)

	// 加载种子 ASN 黑名单到内存
	var asnBlocklist sync.Map
	for asn, org := range SeedBlockedASNs {
		asnBlocklist.Store(asn, ASNInfo{Org: org, Country: "SEED", AddedAt: now})
	}

	// 加载持久化 ASN 黑名单 (BadgerDB 使用 TTL，无需手动清理)
	loadASNBlacklist(db, &asnBlocklist)

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

	// 2. Setup Memory Indices (Success Map) and BadgerDB Failure History
	// 在多模式下，successMap 在所有模式间共享
	successMap := make(map[string]struct{})

	// 全局重置 (清除所有模式的历史)
	if *resetAll {
		fmt.Println("[RESET] Clearing all history and existing output files...")
		// Clear BadgerDB
		if err := clearAllHistory(db); err != nil {
			fmt.Printf("[RESET] Warning: failed to clear BadgerDB: %v\n", err)
		} else {
			fmt.Println("[RESET] BadgerDB history cleared.")
		}
		// Delete existing .bin files in all mode directories
		for _, mode := range modes {
			if mode == "" {
				continue
			}
			modeDir := filepath.Join(baseTargetDir, mode)
			binFiles, _ := filepath.Glob(filepath.Join(modeDir, "*.bin"))
			for _, f := range binFiles {
				if err := os.Remove(f); err != nil {
					fmt.Printf("[RESET] Warning: failed to remove %s: %v\n", f, err)
				}
			}
			if len(binFiles) > 0 {
				fmt.Printf("[RESET] Removed %d existing .bin files from %s mode.\n", len(binFiles), mode)
			}
			txtFiles, _ := filepath.Glob(filepath.Join(modeDir, "*.txt"))
			for _, f := range txtFiles {
				baseName := strings.ToUpper(filepath.Base(f))
				if baseName == "CN.TXT" || baseName == "HK.TXT" || baseName == "MO.TXT" {
					continue
				}
				if err := os.Remove(f); err != nil {
					fmt.Printf("[RESET] Warning: failed to remove %s: %v\n", f, err)
				}
			}
		}
		fmt.Println("[RESET] Starting fresh test...")
	}

	// Load existing successes from ALL mode directories (unless reset)
	// successMap is shared across modes to avoid re-testing same domains
	if !*resetAll {
		// Load from base directory (no mode subdirectory)
		loadExistingIntoMap(baseTargetDir, successMap)
		// Load from each mode directory
		for _, mode := range modes {
			if mode != "" {
				modeDir := filepath.Join(baseTargetDir, mode)
				loadExistingIntoMap(modeDir, successMap)
				loadExistingBinFiles(modeDir, successMap)
			}
		}
		loadSuccessHistory(db, successMap)
		loadBlockedHistory(db, successMap)
	}

	// Force mode: still load existing .bin files to skip re-testing
	if *forceRetry && !*resetAll {
		for _, mode := range modes {
			if mode != "" {
				modeDir := filepath.Join(baseTargetDir, mode)
				loadExistingBinFiles(modeDir, successMap)
			}
		}
	}

	failCount, purged := cleanAndCountFailureHistory(db, now, ttlSec)
	if purged > 0 {
		fmt.Printf("Purged %d expired entries from failure history.\n", purged)
	}

	fmt.Printf("Memory loaded: %d succeeded, %d failed in history.\n", len(successMap), failCount)

	// 2.1 Shared DNS Resolver (used for TLS connection dialing, not DNS lookups)
	resolver := net.DefaultResolver

	// 3. Setup DNS Prefetch Workers
	fmt.Println("Starting DNS prefetch workers...")
	for i := 0; i < 3; i++ {
		go func(workerID int) {
			for domain := range dnsPrefetchQueue {
				if isShuttingDown.Load() {
					return
				}
				if _, exists := dnsPrefetchCache.Load(domain); exists {
					continue
				}
				if _, exists := dnsCache.Load(domain); exists {
					continue
				}
				ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
				ips, err := resolveWithFailover(ctx, domain)
				cancel()
				if err == nil && len(ips) > 0 {
					dnsPrefetchCache.Store(domain, ips[0])
				}
			}
		}(i)
	}

	// Mode loop: run test for each mode
	for modeIdx, mode := range modes {
		// Set mode-specific directory and prefix
		if mode != "" {
			targetDir = filepath.Join(baseTargetDir, mode)
			setModePrefix(mode)
			fmt.Printf("\n%s\n", strings.Repeat("=", 60))
			fmt.Printf("Mode %d/%d: %s\n", modeIdx+1, len(modes), strings.ToUpper(mode))
			fmt.Printf("Output directory: %s\n", targetDir)
			fmt.Printf("%s\n\n", strings.Repeat("=", 60))
			// Create target directory if needed
			if err := os.MkdirAll(targetDir, 0o755); err != nil {
				fmt.Printf("Error creating target directory %s: %v\n", targetDir, err)
				continue
			}
		}

		// 4. Setup Dynamic Workers (AIMD Concurrency Controller)
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

					// 1. DNS Resolution (Check cache first, then prefetch)
					var ip string
					if cachedIP, hit := dnsCache.Load(domain); hit {
						ip = cachedIP.(string)
					} else if prefetchedIP, exists := dnsPrefetchCache.LoadAndDelete(domain); exists {
						ip = prefetchedIP.(string)
						dnsCache.Store(domain, ip)
					} else {
						var ips []string
						var err error
						dnsTimeout := timeoutCtrl.GetTimeout("dns")
						ctx, cancel := context.WithTimeout(context.Background(), dnsTimeout)
						start := time.Now()
						ips, err = resolveWithFailover(ctx, domain)
						timeoutCtrl.Record(time.Since(start), "dns")
						cancel()
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
					record, geoErr := geoDB.Country(net.ParseIP(ip))
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
							addASNToBlacklist(db, asn, org, countryCode)
						}
						// 记录到被阻止数据库
						addBlockedDomain(db, domain, "COUNTRY", countryCode)
						results <- ValidationResult{Domain: domain, Success: false, IP: ip, Country: countryCode, Info: fmt.Sprintf("Skipped (Country: %s)", countryCode)}
						workerSemaphore <- struct{}{} // Return the token
						continue
					}

					// 2.1 ASN Check - 跳过黑名单中的 ASN
					asn, org := getASN(net.ParseIP(ip), asnDB)
					if asn > 0 && isASNBlocked(asn, &asnBlocklist) {
						// 记录到被阻止数据库
						addBlockedDomain(db, domain, "ASN", fmt.Sprintf("%d", asn))
						results <- ValidationResult{Domain: domain, Success: false, IP: ip, Country: countryCode, ASN: asn, Org: org, Info: fmt.Sprintf("Skipped (ASN: %d %s)", asn, org)}
						workerSemaphore <- struct{}{}
						continue
					}

					// 3. Perform TLS Handshake
					tlsTimeout := timeoutCtrl.GetTimeout("tls")
					success, finalIP, info := checkSNI(domain, ip, *debugMode, *xhttpMode, *realityMode, resolver, tlsTimeout)
					if finalIP != "" {
						ip = finalIP
					}

					if countryCode == "UNKNOWN" && finalIP != "" {
						record, geoErr := geoDB.Country(net.ParseIP(finalIP))
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
							batchSave(targetDir, validDomainsMap, db)
							for k := range validDomainsMap {
								delete(validDomainsMap, k)
							}
							newSuccessCount = 0
						}
					} else {
						// 虽然验证成功，但因为区域问题（CN/HK/MO/UNKNOWN）被废弃，记入 BadgerDB
						failureList = append(failureList, res.Domain)
						newFailureCount++
						if newFailureCount >= 500 {
							appendFailureHistoryDB(db, failureList)
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
					// 明确逻辑：所有失败（包含 CN/HK/MO 跳过）都必须记入 BadgerDB 失败库
					// CRITICAL: All failures AND skipped CN/HK/MO domains MUST be recorded in BadgerDB failure history.
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
						appendFailureHistoryDB(db, failureList)
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
				batchSave(targetDir, validDomainsMap, db)
			}
			if len(failureList) > 0 {
				appendFailureHistoryDB(db, failureList)
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

			// 1. Skip if already succeeded (unless -force is set)
			if !*forceRetry {
				if _, exists := successMap[domain]; exists {
					if !*debugMode {
						skippedCount++
					}
					continue
				}
			}

			// 2. Skip if failed recently (BadgerDB lookup)
			if !*forceRetry && isFailedRecently(db, domain, now, ttlSec) {
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

			// Trigger DNS prefetch for this domain
			select {
			case dnsPrefetchQueue <- domain:
			default:
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

		// Collect mode results for summary
		allModeResults = append(allModeResults, ModeResult{
			Mode:           mode,
			Total:          stats.total,
			Success:        stats.success,
			Failed:         stats.failed,
			SkippedCountry: stats.skippedCountry,
			SkippedASN:     stats.skippedASN,
			CountryStats:   stats.countryStats,
		})
	} // End of mode loop

	// Print summary for multiple modes
	if len(allModeResults) > 1 {
		fmt.Println("\n========================================")
		fmt.Println("              总体统计                  ")
		fmt.Println("========================================")
		var totalSuccess, totalFailed, totalSkippedCN, totalSkippedASN int
		for _, r := range allModeResults {
			modeName := r.Mode
			if modeName == "" {
				modeName = "default"
			}
			fmt.Printf("  [%s] Success: %d, Failed: %d, Skipped: CN=%d, ASN=%d\n",
				strings.ToUpper(modeName), r.Success, r.Failed, r.SkippedCountry, r.SkippedASN)
			totalSuccess += r.Success
			totalFailed += r.Failed
			totalSkippedCN += r.SkippedCountry
			totalSkippedASN += r.SkippedASN
		}
		fmt.Println("----------------------------------------")
		fmt.Printf("  [TOTAL] Success: %d, Failed: %d, Skipped: CN=%d, ASN=%d\n",
			totalSuccess, totalFailed, totalSkippedCN, totalSkippedASN)
		fmt.Println("========================================")
		// Update notifyMsg for summary
		notifyMsg := fmt.Sprintf("Total: Success %d | Failed %d | Skipped CN %d ASN %d",
			totalSuccess, totalFailed, totalSkippedCN, totalSkippedASN)
		fmt.Println(notifyMsg)
	}

	// Use single mode stats for notification if not multiple modes
	notifyMsg := "SNI Tester completed"
	if len(allModeResults) == 1 {
		r := allModeResults[0]
		notifyMsg = fmt.Sprintf("Success %d | Failed %d | Skipped CN %d ASN %d",
			r.Success, r.Failed, r.SkippedCountry, r.SkippedASN)
	}

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

	// Optional: auto-shutdown (requires admin privileges on Windows)
	if *autoShutdown {
		fmt.Println("Auto-shutdown requested. Executing shutdown...")
		_ = exec.Command("shutdown.exe", "/s", "/t", "0").Start()
	}

	// 正常结束，优雅关闭
	gracefulShutdown(dbRef)
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

// --- Persistence & History (BadgerDB) ---

// Key prefixes for unified BadgerDB
// Mode-specific prefixes (default/xhttp/reality)
var modePrefix = "default"

// setModePrefix sets the global modePrefix for BadgerDB key prefixes
func setModePrefix(mode string) {
	switch mode {
	case "reality", "xhttp":
		modePrefix = mode
	default:
		modePrefix = "default"
	}
}

// Helper functions for mode-specific key prefixes
func keyPrefixFailed() []byte {
	return []byte("failed:" + modePrefix + ":")
}
func keyPrefixSuccess() []byte {
	return []byte("success:" + modePrefix + ":")
}
func keyPrefixBlockedCountry() []byte {
	return []byte("blocked:country:")
}
func keyPrefixBlockedASN() []byte {
	return []byte("blocked:asn:")
}
func keyPrefixASN() []byte {
	return []byte("asn:")
}

// String versions for TrimPrefix (include trailing colon)
func strKeyPrefixSuccess() string {
	return "success:" + modePrefix + ":"
}
func strKeyPrefixASN() string {
	return "asn:"
}
func strKeyPrefixBlockedCountry() string {
	return "blocked:country:"
}
func strKeyPrefixBlockedASN() string {
	return "blocked:asn:"
}

// Package-level variable for TTL days (set from flag)
var ttlDaysValue int

// cleanAndCountFailureHistory - BadgerDB 使用 TTL，无需手动清理过期数据
// 只统计数量即可
func cleanAndCountFailureHistory(db *badger.DB, now int64, ttlSec int64) (int, int) {
	active := 0
	_ = db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		prefix := keyPrefixFailed()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			active++
		}
		return nil
	})
	return active, 0 // BadgerDB 自动清理过期数据
}

// isFailedRecently checks BadgerDB for a domain and returns true if it failed within the TTL.
func isFailedRecently(db *badger.DB, domain string, now int64, ttlSec int64) bool {
	key := append(keyPrefixFailed(), domain...)
	var lastFail int64
	err := db.View(func(txn *badger.Txn) error {
		item, err := txn.Get(key)
		if err != nil {
			return err
		}
		val, err := item.ValueCopy(nil)
		if err != nil || len(val) != 8 {
			return err
		}
		lastFail = int64(binary.LittleEndian.Uint64(val))
		return nil
	})
	if err != nil {
		return false
	}
	return (now - lastFail) < ttlSec
}

// appendFailureHistoryDB writes a batch of failed domains into BadgerDB with TTL.
func appendFailureHistoryDB(db *badger.DB, domains []string) {
	if len(domains) == 0 {
		return
	}
	now := time.Now().Unix()
	ttl := time.Duration(ttlDaysValue) * 24 * time.Hour

	wb := db.NewWriteBatch()
	defer wb.Cancel()

	for _, d := range domains {
		key := append(keyPrefixFailed(), d...)
		buf := make([]byte, 8)
		binary.LittleEndian.PutUint64(buf, uint64(now))
		wb.SetEntry(&badger.Entry{
			Key:       key,
			Value:     buf,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	}
	wb.Flush()

	// 3. 写入后验证 - 抽样检查最后一条
	lastKey := append(keyPrefixFailed(), domains[len(domains)-1]...)
	err := db.View(func(txn *badger.Txn) error {
		_, err := txn.Get(lastKey)
		return err
	})
	if err != nil {
		fmt.Printf("[WriteVerify] Warning: failed to verify write for %s: %v\n", domains[len(domains)-1], err)
	}
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

func clearAllHistory(db *badger.DB) error {
	if db == nil {
		return nil
	}
	// Drop all data in BadgerDB
	err := db.DropAll()
	if err != nil {
		return err
	}
	// Run GC to clean up
	db.RunValueLogGC(0.5)
	return nil
}

func loadExistingBinFiles(dir string, m map[string]struct{}) {
	files, _ := filepath.Glob(filepath.Join(dir, "*.bin"))
	for _, f := range files {
		baseName := strings.ToUpper(filepath.Base(f))
		if baseName == "CN.BIN" || baseName == "HK.BIN" || baseName == "MO.BIN" {
			continue
		}
		data, err := os.ReadFile(f)
		if err != nil {
			continue
		}
		// Parse binary format: [2 bytes length BE][domain bytes]...
		offset := 0
		for offset+2 <= len(data) {
			length := int(binary.BigEndian.Uint16(data[offset : offset+2]))
			offset += 2
			if length == 0 || length > 512 || offset+length > len(data) {
				break
			}
			domain := string(data[offset : offset+length])
			if domain != "" && strings.Contains(domain, ".") {
				m[domain] = struct{}{}
			}
			offset += length
		}
	}
}

func loadSuccessHistory(db *badger.DB, m map[string]struct{}) {
	if db == nil {
		return
	}
	_ = db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		prefix := keyPrefixSuccess()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixSuccess())
			m[domain] = struct{}{}
		}
		return nil
	})
	fmt.Printf("Loaded %d successful domains from history\n", len(m))
}

func loadBlockedHistory(db *badger.DB, m map[string]struct{}) {
	if db == nil {
		return
	}
	_ = db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()

		// Load country-blocked domains (shared across modes)
		prefixCountry := keyPrefixBlockedCountry()
		for iter.Seek(prefixCountry); iter.ValidForPrefix(prefixCountry); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixBlockedCountry())
			m[domain] = struct{}{}
		}

		// Load ASN-blocked domains (shared across modes)
		prefixASN := keyPrefixBlockedASN()
		iter.Seek(prefixASN)
		for iter.Seek(prefixASN); iter.ValidForPrefix(prefixASN); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixBlockedASN())
			m[domain] = struct{}{}
		}
		return nil
	})
	fmt.Printf("Loaded %d blocked domains from history\n", len(m))
}

func batchSave(targetDir string, m map[string][]string, db *badger.DB) {
	if isDebugMode {
		total := 0
		for country, list := range m {
			total += len(list)
			fmt.Printf("[DEBUG] Would save %d domains for country %s\n", len(list), country)
		}
		fmt.Printf("[DEBUG] Skipping BadgerDB persistence (%d total domains)\n", total)
		fmt.Printf("[DEBUG] Skipping sni/ output\n")
		return
	}

	for country, list := range m {
		writeBinaryDomainFile(targetDir, country, list)
	}
	// 保存到成功历史数据库
	if db != nil && len(m) > 0 {
		now := time.Now().Unix()
		ttl := time.Duration(ttlDaysValue) * 24 * time.Hour

		wb := db.NewWriteBatch()
		defer wb.Cancel()

		for country, list := range m {
			for _, domain := range list {
				info := SuccessInfo{
					Domain:   domain,
					Country:  country,
					TestedAt: now,
				}
				data, _ := json.Marshal(info)
				key := append(keyPrefixSuccess(), domain...)
				wb.SetEntry(&badger.Entry{
					Key:       key,
					Value:     data,
					ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
				})
			}
		}
		wb.Flush()
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

func checkSNI(domain string, targetIP string, debug bool, xhttp bool, reality bool, resolver *net.Resolver, tlsTimeout time.Duration) (bool, string, string) {
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
	uConn.SetDeadline(time.Now().Add(tlsTimeout))
	start := time.Now()
	if err := uConn.Handshake(); err != nil {
		return false, "", err.Error()
	}
	timeoutCtrl.Record(time.Since(start), "tls")
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

func shuffleStrings(s []string) {
	for i := len(s) - 1; i > 0; i-- {
		j := randIndex(i + 1)
		s[i], s[j] = s[j], s[i]
	}
}

func calcDnsWeight(h *DnsHealth) float64 {
	if h == nil {
		return 1.0 / dnsHealthEpsilon
	}
	total := float64(h.SuccessCount + h.FailCount)
	if total == 0 {
		return 1.0 / dnsHealthEpsilon
	}
	base := float64(h.SuccessCount) / (total + dnsHealthEpsilon)
	if h.ConsecutiveFail >= dnsMaxConsecutiveFail {
		decay := math.Pow(dnsWeightDecay, float64(h.ConsecutiveFail-dnsMaxConsecutiveFail+1))
		base *= decay
	}
	if base < dnsMinWeight {
		base = dnsMinWeight
	}
	return base
}

func updateDnsHealth(server string, success bool) {
	h, _ := DnsHealthMap.LoadOrStore(server, &DnsHealth{})
	hh := h.(*DnsHealth)
	if success {
		hh.ConsecutiveFail = 0
		hh.SuccessCount++
		newWeight := hh.Weight * dnsRecoveryBoost
		if newWeight > 1.0 {
			newWeight = 1.0
		}
		hh.Weight = newWeight
	} else {
		hh.ConsecutiveFail++
		hh.FailCount++
		hh.Weight = calcDnsWeight(hh)
	}
	DnsHealthMap.Store(server, hh)
}

var dnsRng = rand.New(rand.NewSource(time.Now().UnixNano()))

func selectWeightedServers(servers []string, count int) []string {
	type weightedServer struct {
		server string
		weight float64
	}
	ws := make([]weightedServer, 0, len(servers))
	var totalWeight float64
	for _, s := range servers {
		h, ok := DnsHealthMap.Load(s)
		var w float64
		if ok {
			w = calcDnsWeight(h.(*DnsHealth))
		} else {
			w = 1.0 / dnsHealthEpsilon
		}
		ws = append(ws, weightedServer{s, w})
		totalWeight += w
	}
	if totalWeight <= 0 {
		return servers[:count]
	}
	selected := make([]string, 0, count)
	used := make(map[string]bool)
	for len(selected) < count && len(selected) < len(servers) {
		r := dnsRng.Float64() * totalWeight
		cumulative := 0.0
		for _, w := range ws {
			if used[w.server] {
				continue
			}
			cumulative += w.weight
			if r <= cumulative {
				selected = append(selected, w.server)
				used[w.server] = true
				totalWeight -= w.weight
				break
			}
		}
	}
	for _, s := range servers {
		if len(selected) >= count {
			break
		}
		if !used[s] {
			selected = append(selected, s)
			used[s] = true
		}
	}
	return selected
}

func resolveWithFailover(ctx context.Context, domain string) ([]string, error) {
	return resolveWithDNS(ctx, domain)
}

func resolveWithUDP(ctx context.Context, domain string) ([]string, error) {
	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(domain), dns.TypeA)

	servers := make([]string, len(DnsPool))
	copy(servers, DnsPool)
	shuffleStrings(servers)

	baseTimeout := timeoutCtrl.GetTimeout("dns")
	var lastErr error

	for round := 0; round < dnsRetryRounds; round++ {
		roundServers := selectWeightedServers(servers, dnsMaxServers)
		for _, server := range roundServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}

			c := &dns.Client{
				Timeout: baseTimeout,
				Net:     "udp4",
			}

			start := time.Now()
			in, _, err := c.ExchangeContext(ctx, msg, server+":53")
			elapsed := time.Since(start)
			timeoutCtrl.Record(elapsed, "dns")

			if err != nil {
				errStr := err.Error()
				if strings.Contains(errStr, "NXDOMAIN") ||
					strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
				lastErr = err
				updateDnsHealth(server, false)
				continue
			}

			var ips []string
			for _, rr := range in.Answer {
				if a, ok := rr.(*dns.A); ok {
					ips = append(ips, a.A.String())
				}
			}
			if len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
		}

		if round < dnsRetryRounds-1 {
			time.Sleep(dnsRetryDelay * time.Duration(round+1))
		}
	}

	return nil, lastErr
}

func downloadFile(filePath string, urlStr string, proxyString string) error {
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

	contentLength, err := getContentLength(client, urlStr)
	if err != nil {
		return fmt.Errorf("failed to get file size: %w", err)
	}

	if contentLength <= 0 || !supportsRange(client, urlStr) {
		fmt.Println("服务器不支持断点续传，使用单线程下载...")
		return downloadSingle(client, filePath, urlStr)
	}

	fmt.Printf("文件大小: %.2f MB, 启用 %d 线程并行下载...\n", float64(contentLength)/1024/1024, DownloadWorkers)

	// Quick Range test before starting parallel download
	testReq, _ := http.NewRequest("GET", urlStr, nil)
	testReq.Header.Set("Range", "bytes=0-1023")
	testResp, err := client.Do(testReq)
	if err != nil || (testResp.StatusCode != http.StatusPartialContent && testResp.StatusCode != http.StatusOK) {
		if testResp != nil {
			testResp.Body.Close()
		}
		fmt.Println("Range请求测试失败，切换单线程下载...")
		return downloadSingle(client, filePath, urlStr)
	}
	testResp.Body.Close()

	tmpDir := filepath.Dir(filePath) + "/.download_tmp_" + filepath.Base(filePath)
	os.MkdirAll(tmpDir, 0755)
	defer os.RemoveAll(tmpDir)

	chunkSize := DownloadChunkSize
	numChunks := int((contentLength + int64(chunkSize) - 1) / int64(chunkSize))
	if numChunks < DownloadWorkers {
		numChunks = DownloadWorkers
		chunkSize = int((contentLength + int64(numChunks) - 1) / int64(numChunks))
	}

	var wg sync.WaitGroup
	errChan := make(chan error, numChunks)
	chunkPaths := make([]string, numChunks)

	for i := 0; i < numChunks; i++ {
		wg.Add(1)
		go func(chunkIdx int) {
			defer wg.Done()

			start := int64(chunkIdx) * int64(chunkSize)
			end := start + int64(chunkSize) - 1
			if end >= contentLength {
				end = contentLength - 1
			}
			if start >= contentLength {
				chunkPaths[chunkIdx] = ""
				return
			}

			req, _ := http.NewRequest("GET", urlStr, nil)
			req.Header.Set("Range", fmt.Sprintf("bytes=%d-%d", start, end))

			resp, err := client.Do(req)
			if err != nil {
				errChan <- fmt.Errorf("chunk %d download failed: %w", chunkIdx, err)
				return
			}
			defer resp.Body.Close()

			if resp.StatusCode != http.StatusPartialContent && resp.StatusCode != http.StatusOK {
				errChan <- fmt.Errorf("chunk %d: unexpected status %d", chunkIdx, resp.StatusCode)
				return
			}

			chunkPath := tmpDir + fmt.Sprintf("/chunk_%d", chunkIdx)
			out, err := os.Create(chunkPath)
			if err != nil {
				errChan <- fmt.Errorf("chunk %d: failed to create file: %w", chunkIdx, err)
				return
			}

			_, err = io.Copy(out, resp.Body)
			out.Close()
			if err != nil {
				errChan <- fmt.Errorf("chunk %d: failed to write: %w", chunkIdx, err)
				return
			}

			chunkPaths[chunkIdx] = chunkPath
		}(i)
	}

	wg.Wait()
	close(errChan)

	for err := range errChan {
		if err != nil {
			return err
		}
	}

	out, err := os.Create(filePath)
	if err != nil {
		return fmt.Errorf("failed to create final file: %w", err)
	}
	defer out.Close()

	for i := 0; i < numChunks; i++ {
		if chunkPaths[i] == "" {
			continue
		}
		data, err := os.ReadFile(chunkPaths[i])
		if err != nil {
			return fmt.Errorf("failed to read chunk %d: %w", i, err)
		}
		out.Write(data)
	}

	return out.Close()
}

func notifyUser(title, message string) {
	fmt.Printf("\n========================================\n")
	fmt.Printf("通知: %s\n", title)
	fmt.Printf("========================================\n")
	fmt.Printf("%s\n", message)
	fmt.Printf("========================================\n")

	if runtime.GOOS == "windows" {
		psScript := fmt.Sprintf(`
Add-Type -AssemblyName System.Windows.Forms
$notify = New-Object System.Windows.Forms.NotifyIcon
$notify.Icon = [System.Drawing.SystemIcons]::Warning
$notify.BalloonTipTitle = '%s'
$notify.BalloonTipText = '%s'
$notify.Visible = $True
$notify.ShowBalloonTip(10000)
Start-Sleep -Seconds 10
$notify.Dispose()
`, title, message)
		exec.Command("powershell", "-NoProfile", "-WindowStyle", "Hidden", "-Command", psScript).Start()
	} else if runtime.GOOS == "darwin" {
		exec.Command("osascript", "-e", fmt.Sprintf(`display notification "%s" with title "%s"`, message, title)).Run()
	} else {
		fmt.Printf("\n[提示] 请查看上方通知消息\n")
	}
}

func waitForNetworkChange() bool {
	fmt.Println("\n等待网络切换检测...")
	fmt.Println("请切换到下载网络 (如开启代理/vpn)...")

	initialIPs := getCurrentPublicIPs()
	timeout := time.After(60 * time.Second)
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-timeout:
			fmt.Println("等待超时 (60秒)，继续下载...")
			return false
		case <-ticker.C:
			currentIPs := getCurrentPublicIPs()
			if len(currentIPs) > 0 && !sameIPSets(initialIPs, currentIPs) {
				fmt.Printf("检测到网络切换: %v\n", currentIPs)
				return true
			}
		}
	}
}

func getCurrentPublicIPs() []string {
	var ips []string
	ifaces, err := net.Interfaces()
	if err != nil {
		return ips
	}
	for _, iface := range ifaces {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		for _, addr := range addrs {
			if ipnet, ok := addr.(*net.IPNet); ok {
				ip := ipnet.IP
				if ip != nil && ip.To4() != nil && !ip.IsLoopback() {
					ips = append(ips, ip.String())
				}
			}
		}
	}
	return ips
}

func sameIPSets(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	set := make(map[string]bool)
	for _, ip := range a {
		set[ip] = true
	}
	for _, ip := range b {
		if !set[ip] {
			return false
		}
	}
	return true
}

func checkConnectivity(url string) bool {
	client := &http.Client{Timeout: 5 * time.Second}
	resp, err := client.Get(url)
	if err != nil {
		return false
	}
	resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func validateTestNetwork() bool {
	fmt.Println("正在验证测试网络...")

	domesticSites := []struct {
		url  string
		name string
	}{
		{"https://www.baidu.com", "百度"},
		{"https://www.qq.com", "腾讯"},
		{"https://www.aliyun.com", "阿里云"},
		{"https://www.taobao.com", "淘宝"},
		{"https://www.jd.com", "京东"},
		{"https://www.so.com", "360搜索"},
		{"https://www.sina.com.cn", "新浪"},
		{"https://www.163.com", "网易"},
		{"https://www.bilibili.com", "哔哩哔哩"},
		{"https://www.douyin.com", "抖音"},
	}

	foreignSites := []struct {
		url  string
		name string
	}{
		{"https://www.google.com", "Google"},
		{"https://www.youtube.com", "YouTube"},
		{"https://twitter.com", "Twitter"},
		{"https://www.facebook.com", "Facebook"},
		{"https://www.instagram.com", "Instagram"},
		{"https://www.reddit.com", "Reddit"},
	}

	domesticOK := 0
	domesticChecked := 0
	for _, site := range domesticSites {
		if checkConnectivity(site.url) {
			domesticOK++
			fmt.Printf("  [✓] %s 可访问\n", site.name)
		}
		domesticChecked++
		if domesticOK >= 3 {
			break
		}
	}
	if domesticOK < 3 && domesticChecked < len(domesticSites) {
		for _, site := range domesticSites[domesticChecked:] {
			if checkConnectivity(site.url) {
				domesticOK++
				fmt.Printf("  [✓] %s 可访问\n", site.name)
			}
			domesticChecked++
			if domesticOK >= 3 {
				break
			}
		}
	}

	foreignBlocked := 0
	for _, site := range foreignSites {
		if !checkConnectivity(site.url) {
			foreignBlocked++
			fmt.Printf("  [✗] %s 已封锁\n", site.name)
		} else {
			fmt.Printf("  [⚠] %s 可访问 (异常)\n", site.name)
		}
	}

	fmt.Printf("\n  国内网站: %d/3 最低要求\n", domesticOK)
	fmt.Printf("  国外网站: %d/%d 已封锁\n", foreignBlocked, len(foreignSites))

	if domesticOK >= 3 && foreignBlocked >= 2 {
		fmt.Println("  结论: 测试网络验证通过!")
		return true
	}

	if domesticOK < 3 {
		fmt.Println("  结论: 国内网站访问不足3个，请检查网络")
	} else {
		fmt.Println("  结论: 国外网站封锁不足，测试网络隔离失败")
	}
	return false
}

func getContentLength(client *http.Client, urlStr string) (int64, error) {
	resp, err := client.Head(urlStr)
	if err != nil {
		return 0, err
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return 0, fmt.Errorf("HEAD status: %d", resp.StatusCode)
	}
	return resp.ContentLength, nil
}

func supportsRange(client *http.Client, urlStr string) bool {
	req, _ := http.NewRequest("GET", urlStr, nil)
	req.Header.Set("Range", "bytes=0-0")
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	resp.Body.Close()
	return resp.StatusCode == http.StatusPartialContent
}

func downloadSingle(client *http.Client, filePath string, urlStr string) error {
	return downloadWithSpeedMonitor(client, filePath, urlStr, 0)
}

func downloadWithSpeedMonitor(client *http.Client, filePath string, urlStr string, minSpeedBps int64) error {
	const slowSpeedThreshold = 10 * 1024 // 10 KB/s
	const checkInterval = 3 * time.Second
	const stallTimeout = 30 * time.Second

	resp, err := client.Get(urlStr)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("unexpected status: %d", resp.StatusCode)
	}

	out, err := os.Create(filePath)
	if err != nil {
		return err
	}
	defer out.Close()

	var totalWritten int64
	var lastWritten int64
	var lastCheckTime = time.Now()

	progressTicker := time.NewTicker(checkInterval)
	defer progressTicker.Stop()
	stallTimer := time.NewTimer(stallTimeout)
	defer stallTimer.Stop()

	buf := make([]byte, 32*1024)
	for {
		select {
		case <-progressTicker.C:
			now := time.Now()
			elapsed := now.Sub(lastCheckTime).Seconds()
			if elapsed > 0 {
				speed := float64(totalWritten-lastWritten) / elapsed
				speedKB := speed / 1024
				progress := float64(totalWritten) / float64(resp.ContentLength) * 100
				fmt.Printf("\r下载进度: %.1f%% (%.1f KB/s)", progress, speedKB)

				if speed < float64(slowSpeedThreshold) && totalWritten > 1024*1024 {
					stallTimer.Stop()
					fmt.Println()
					return fmt.Errorf("download stall: speed too slow (%.1f KB/s < %d KB/s)", speedKB, slowSpeedThreshold/1024)
				}

				if speed < float64(minSpeedBps) && minSpeedBps > 0 {
					stallTimer.Stop()
					fmt.Println()
					return fmt.Errorf("download below minimum speed requirement")
				}
			}
			lastWritten = totalWritten
			lastCheckTime = now
			stallTimer.Reset(stallTimeout)

		case <-stallTimer.C:
			fmt.Println()
			return fmt.Errorf("download stall: no data received for %v", stallTimeout)

		default:
			n, err := resp.Body.Read(buf)
			if n > 0 {
				written, wErr := out.Write(buf[:n])
				if wErr != nil {
					return wErr
				}
				totalWritten += int64(written)
			}
			if err != nil {
				if err == io.EOF {
					fmt.Printf("\r下载进度: 100.0%%\n")
					return nil
				}
				return err
			}
		}
	}
}

func prepareGeoDBs(proxyString string) {
	needCountry := false
	needASN := false

	if _, err := os.Stat(GeoDBFile); os.IsNotExist(err) {
		needCountry = true
	}
	if _, err := os.Stat(GeoASNFile); os.IsNotExist(err) {
		needASN = true
	}

	if !needCountry && !needASN {
		fmt.Println("Geo数据库已存在，跳过下载。")
		return
	}

	fmt.Println("\n========================================")
	fmt.Println("          Geo数据库准备阶段              ")
	fmt.Println("========================================")
	fmt.Println("步骤1: 请切换到下载专用网络 (高带宽)")
	fmt.Println("       (如开启代理下载数据库文件)")
	fmt.Println("========================================")
	fmt.Println("\n切换好后按 Enter 继续下载...")
	fmt.Scanln()

	if needCountry {
		downloadGeoFileWithNetworkSwitch(GeoDBFile, GeoDBURL, proxyString, "GeoLite2-Country.mmdb")
	}
	if needASN {
		downloadGeoFileWithNetworkSwitch(GeoASNFile, GeoASNURL, proxyString, "GeoLite2-ASN.mmdb")
	}

	fmt.Println("\n========================================")
	fmt.Println("          Geo数据库准备阶段              ")
	fmt.Println("========================================")
	if isDebugMode {
		fmt.Println("[DEBUG] Skipping network isolation check")
	} else {
		fmt.Println("步骤2: 请切换到测试专用网络")
		fmt.Println("       (关闭代理，使用ISP直连)")
		fmt.Println("========================================")
		waitForTestNetwork()
	}
}

func waitForTestNetwork() {
	notifyUser("下载完成", "Geo数据库下载完成！\n请切换到测试专用网络，完成后程序将自动继续...")

	initialIPs := getCurrentPublicIPs()
	lastValidIPs := initialIPs
	hasSwitched := false
	timeout := time.After(120 * time.Second)
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	recheckTimer := time.NewTimer(0)
	defer recheckTimer.Stop()

	fmt.Println("等待切换到测试网络...")
	for {
		select {
		case <-timeout:
			fmt.Println("\n等待超时 (120秒)")
			fmt.Println("按 Enter 继续运行，或 Ctrl+C 退出...")
			fmt.Scanln()
			return

		case <-recheckTimer.C:
			if validateTestNetwork() {
				return
			}
			fmt.Println("\n验证失败，请确保已切换到正确的测试网络")
			fmt.Println("切换好后按 Enter 继续验证，或 Ctrl+C 退出...")
			fmt.Scanln()
			hasSwitched = true
			recheckTimer.Reset(1 * time.Second)

		case <-ticker.C:
			currentIPs := getCurrentPublicIPs()
			if !sameIPSets(initialIPs, currentIPs) {
				fmt.Printf("\n检测到网络已切换: %v\n", currentIPs)
				fmt.Println("正在等待网络稳定 (5秒)...")
				recheckTimer.Stop()
				recheckTimer.Reset(5 * time.Second)
				initialIPs = currentIPs
				hasSwitched = true
			} else if hasSwitched && !sameIPSets(lastValidIPs, currentIPs) {
				fmt.Printf("\n检测到网络再次切换: %v\n", currentIPs)
				fmt.Println("正在等待网络稳定 (5秒)...")
				recheckTimer.Stop()
				recheckTimer.Reset(5 * time.Second)
				lastValidIPs = currentIPs
			}
		}
	}
}

func prepareGeoDB(proxyString string) {}

func prepareGeoASNDB(proxyString string) {}

func downloadGeoFileWithNetworkSwitch(filePath, urlStr, proxyString, displayName string) {
	maxRetries := 3
	for attempt := 1; attempt <= maxRetries; attempt++ {
		fmt.Printf("\n[%d/%d] 正在下载 %s...\n", attempt, maxRetries, displayName)

		err := downloadFile(filePath, urlStr, proxyString)
		if err == nil {
			fmt.Printf("%s 下载成功!\n", displayName)
			return
		}

		errMsg := err.Error()
		isSlowDown := strings.Contains(errMsg, "stall") || strings.Contains(errMsg, "slow")

		if attempt < maxRetries {
			if isSlowDown {
				notifyUser("下载速度太慢", fmt.Sprintf("%s 下载速度太慢，请切换到更快的网络后继续。\n系统将在检测到网络切换后自动继续...", displayName))
			} else {
				notifyUser("下载失败", fmt.Sprintf("%s 下载失败: %v\n请检查网络后继续，系统将在检测到网络切换后自动重试...", displayName, err))
			}

			networkChanged := waitForNetworkChange()
			if networkChanged {
				fmt.Println("检测到网络切换，正在重试...")
				continue
			}
		} else {
			notifyUser("下载多次失败", fmt.Sprintf("%s 下载多次失败: %v\n请手动下载并放置到程序目录。\nURL: %s", displayName, err, urlStr))
			fmt.Printf("\n下载失败，请手动下载: %s\n保存为: %s\n", urlStr, filePath)
			fmt.Println("\n按 Ctrl+C 退出，或手动放置文件后按 Enter 继续...")
			fmt.Scanln()
			if _, statErr := os.Stat(filePath); statErr == nil {
				fmt.Printf("检测到文件，继续运行...\n")
				return
			}
			os.Exit(1)
		}
	}
}

func getASN(ip net.IP, db *geoip2.Reader) (uint32, string) {
	record, err := db.ASN(ip)
	if err != nil {
		return 0, ""
	}
	return uint32(record.AutonomousSystemNumber), record.AutonomousSystemOrganization
}

func loadASNBlacklist(db *badger.DB, asnMap *sync.Map) {
	_ = db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		prefix := keyPrefixASN()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			item := iter.Item()
			key := string(item.Key())
			asnStr := strings.TrimPrefix(key, strKeyPrefixASN())
			var asn uint32
			fmt.Sscanf(asnStr, "%d", &asn)
			val, _ := item.ValueCopy(nil)
			var info ASNInfo
			if err := json.Unmarshal(val, &info); err == nil {
				asnMap.Store(asn, info)
			}
		}
		return nil
	})
}

func isASNBlocked(asn uint32, asnMap *sync.Map) bool {
	_, ok := asnMap.Load(asn)
	return ok
}

func addASNToBlacklist(db *badger.DB, asn uint32, org, country string) {
	info := ASNInfo{
		Org:     org,
		Country: country,
		AddedAt: time.Now().Unix(),
	}
	data, _ := json.Marshal(info)
	key := append(keyPrefixASN(), fmt.Sprintf("%d", asn)...)

	ttl := time.Duration(ttlDaysValue) * 24 * time.Hour
	now := time.Now().Unix()

	_ = db.Update(func(txn *badger.Txn) error {
		return txn.SetEntry(&badger.Entry{
			Key:       key,
			Value:     data,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	})
}

func addBlockedDomain(db *badger.DB, domain, reason, code string) {
	info := BlockedInfo{
		Domain:   domain,
		Reason:   reason,
		Code:     code,
		TestedAt: time.Now().Unix(),
	}
	data, _ := json.Marshal(info)

	var key []byte
	if reason == "COUNTRY" {
		key = append(keyPrefixBlockedCountry(), domain...)
	} else if reason == "ASN" {
		key = append(keyPrefixBlockedASN(), domain...)
	} else {
		key = append(keyPrefixBlockedCountry(), domain...)
	}

	ttl := time.Duration(ttlDaysValue) * 24 * time.Hour
	now := time.Now().Unix()

	_ = db.Update(func(txn *badger.Txn) error {
		return txn.SetEntry(&badger.Entry{
			Key:       key,
			Value:     data,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	})
}

func getASNBlocklistCount(asnMap *sync.Map) int {
	count := 0
	asnMap.Range(func(key, value interface{}) bool {
		count++
		return true
	})
	return count
}

// --- DoH RFC 8484 Wire Format ---

func lookupHostDoHWire(client *http.Client, endpoint string, name string) ([]string, error) {
	if client == nil {
		client = &http.Client{Timeout: 5 * time.Second}
	}

	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(name), dns.TypeA)

	packed, err := msg.Pack()
	if err != nil {
		return nil, fmt.Errorf("failed to pack DNS message: %w", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "POST", endpoint, bytes.NewReader(packed))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/dns-message")
	req.Header.Set("Accept", "application/dns-message")
	req.Header.Set("User-Agent", pickUserAgent())

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("DoH HTTP status %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	respMsg := new(dns.Msg)
	if err := respMsg.Unpack(body); err != nil {
		return nil, fmt.Errorf("failed to unpack DNS response: %w", err)
	}

	var ips []string
	for _, rr := range respMsg.Answer {
		if a, ok := rr.(*dns.A); ok {
			ips = append(ips, a.A.String())
		}
	}

	if len(ips) == 0 {
		return nil, fmt.Errorf("no A records in DoH response")
	}
	return ips, nil
}

// --- DoT Support ---

func lookupHostDoT(server string, name string) ([]string, error) {
	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(name), dns.TypeA)

	timeout := 5 * time.Second

	conn, err := dns.DialWithTLS("tcp", server, &tls.Config{
		ServerName: strings.Split(server, ":")[0],
		MinVersion: tls.VersionTLS12,
	})
	if err != nil {
		return nil, fmt.Errorf("DoT connection failed: %w", err)
	}
	defer conn.Close()

	conn.SetDeadline(time.Now().Add(timeout))

	if err := conn.WriteMsg(msg); err != nil {
		return nil, fmt.Errorf("DoT write failed: %w", err)
	}

	resp, err := conn.ReadMsg()
	if err != nil {
		return nil, fmt.Errorf("DoT read failed: %w", err)
	}

	var ips []string
	for _, rr := range resp.Answer {
		if a, ok := rr.(*dns.A); ok {
			ips = append(ips, a.A.String())
		}
	}

	if len(ips) == 0 {
		return nil, fmt.Errorf("no A records in DoT response")
	}
	return ips, nil
}

// --- Unified DNS Resolution with Priority DoH → DoT → UDP ---

func resolveWithDNS(ctx context.Context, domain string) ([]string, error) {
	// Try DoH first (RFC 8484 wire format)
	if len(DNS.DoH) > 0 {
		dohServers := selectWeightedServers(DNS.DoH, 3)
		for _, server := range dohServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}
			start := time.Now()
			ips, err := lookupHostDoHWire(nil, server, domain)
			latency := time.Since(start)
			timeoutCtrl.Record(latency, "dns")

			if err == nil && len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
			errStr := ""
			if err != nil {
				errStr = err.Error()
				if strings.Contains(errStr, "NXDOMAIN") || strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
			}
			updateDnsHealth(server, false)
		}
	}

	// Try DoT second
	if len(DNS.DoT) > 0 {
		dotServers := selectWeightedServers(DNS.DoT, 3)
		for _, server := range dotServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}
			start := time.Now()
			ips, err := lookupHostDoT(server, domain)
			latency := time.Since(start)
			timeoutCtrl.Record(latency, "dns")

			if err == nil && len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
			errStr := ""
			if err != nil {
				errStr = err.Error()
				if strings.Contains(errStr, "NXDOMAIN") || strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
			}
			updateDnsHealth(server, false)
		}
	}

	// Fall back to UDP
	return resolveWithUDP(ctx, domain)
}
