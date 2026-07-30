package pkg

import (
	"time"

	utls "github.com/refraction-networking/utls"
)

// Config holds configuration for the SNI tester
type Config struct {
	FixedWorkers  int
	ForceRetry    bool
	ResetAll      bool
	TTLDays       int
	MaxLines      int
	Debug         bool
	UseBuiltinDNS bool
	DNSAddr       string
	GeoDBFile     string
	GeoASNFile    string
	BadgerDBDir   string
	OutputDir     string
	Shutdown      bool
	GeoProxy      string
}

// DefaultConfig returns default configuration
func DefaultConfig() Config {
	return Config{
		FixedWorkers:  0,
		TTLDays:       7,
		GeoDBFile:     "GeoLite2-Country.mmdb",
		GeoASNFile:    "GeoLite2-ASN.mmdb",
		BadgerDBDir:   "badger_db",
		UseBuiltinDNS: true,
	}
}

// DNS Failover Configuration
const (
	dnsServerTimeout = 800 * time.Millisecond
	dnsMaxServers    = 5
	dnsRetryRounds   = 2
	dnsRetryDelay    = 100 * time.Millisecond
)

// DNS Rate Limiter Configuration
const (
	dnsGlobalLimit        = 300
	dnsAliyunDoHLimit     = 15
	dnsAliyunUDPLimit     = 80
	dnsTencentLimit       = 50
	dnsDomesticLimit      = 50
	dnsInternationalLimit = 500
	dnsMaxConcurrent      = 100
	dnsBurstSize          = 20
)

// DNSConfig separates DNS servers by protocol
type DNSConfig struct {
	DoH []string
	DoT []string
	UDP []string
}

// DNS Health Tracking Configuration
const (
	dnsHealthEpsilon      = 10.0
	dnsMaxConsecutiveFail = 3
	dnsWeightDecay        = 0.5
	dnsMinWeight          = 0.05
	dnsRecoveryBoost      = 1.5
)

// DNS Provider IP Mapping (UDP/TCP)
var DNSProviderMapUDP = map[string]DNSProvider{
	"223.5.5.5":       ProviderAliyunUDP,
	"223.6.6.6":       ProviderAliyunUDP,
	"119.29.29.29":    ProviderTencent,
	"182.254.116.116": ProviderTencent,
	"120.53.53.53":    ProviderTencent,
	"1.12.12.12":      ProviderTencent,
	"114.114.114.114": ProviderDomestic,
	"114.114.115.115": ProviderDomestic,
	"180.76.76.76":    ProviderDomestic,
	"1.2.4.8":         ProviderDomestic,
	"210.2.4.8":       ProviderDomestic,
	"117.50.22.22":    ProviderDomestic,
	"117.50.11.11":    ProviderDomestic,
	"180.184.1.1":     ProviderDomestic,
	"180.184.2.2":     ProviderDomestic,
	"1.1.1.1":         ProviderGlobal,
	"1.0.0.1":         ProviderGlobal,
	"8.8.8.8":         ProviderGlobal,
	"8.8.4.4":         ProviderGlobal,
	"9.9.9.9":         ProviderGlobal,
	"149.112.112.112": ProviderGlobal,
	"208.67.222.222":  ProviderGlobal,
	"208.67.220.220":  ProviderGlobal,
}

// DNS Provider IP Mapping (DoH/DoT)
var DNSProviderMapDoH = map[string]DNSProvider{
	"223.5.5.5":       ProviderAliyunDoH,
	"223.6.6.6":       ProviderAliyunDoH,
	"119.29.29.29":    ProviderTencent,
	"182.254.116.116": ProviderTencent,
	"180.184.1.1":     ProviderDomestic,
	"1.2.4.8":         ProviderDomestic,
	"1.1.1.1":         ProviderGlobal,
	"8.8.8.8":         ProviderGlobal,
	"9.9.9.9":         ProviderGlobal,
}

// DefaultDNSServers is the default DNS server pool by protocol (IPv4 only)
var DefaultDNSServers = DNSConfig{
	DoH: []string{
		"https://doh.pub/dns-query",
		"https://dns.alidns.com/dns-query",
		"https://dns.360.cn/dns-query",
		"https://1.1.1.1/dns-query",
		"https://dns.google/dns-query",
		"https://dns.quad9.net/dns-query",
		"https://doh.opendns.com/dns-query",
		"https://dns.adguard.com/dns-query",
	},
	DoT: []string{
		"dot.pub:853",
		"dns.alidns.com:853",
		"dns.360.cn:853",
		"1.1.1.1:853",
		"dns.google:853",
		"dns.quad9.net:853",
		"dns.adguard.com:853",
	},
	UDP: []string{
		"1.1.1.1", "1.0.0.1",
		"8.8.8.8", "8.8.4.4",
		"9.9.9.9", "149.112.112.112",
		"208.67.222.222", "208.67.220.220",
		"8.26.56.26", "8.20.247.20",
		"94.140.14.14", "94.140.15.15",
		"64.6.64.6", "64.6.65.6",
		"4.2.2.1", "4.2.2.2", "4.2.2.3",
		"77.88.8.1", "77.88.8.2", "77.88.8.7", "77.88.8.8",
		"80.80.80.80", "80.80.81.81",
		"45.11.45.11", "185.222.222.222",
		"119.29.29.29", "119.28.28.28",
		"223.5.5.5", "223.6.6.6",
		"114.114.114.114", "114.114.115.115",
		"114.114.114.110", "114.114.115.110",
		"114.114.114.119", "114.114.115.119",
		"180.76.76.76",
		"180.184.1.1", "180.184.2.2",
		"101.226.4.6", "218.30.118.6", "123.125.81.6",
		"1.2.4.8", "210.2.4.8",
		"117.50.22.22", "117.50.11.11",
		"52.80.66.66",
		"120.53.53.53", "1.12.12.12",
	},
}

// ClientHelloProfiles are the candidate TLS fingerprint profiles
var ClientHelloProfiles = []utls.ClientHelloID{
	utls.HelloChrome_Auto,
	utls.HelloFirefox_Auto,
	utls.HelloIOS_Auto,
}

// ALPNProfiles are the candidate ALPN profile sets
var ALPNProfiles = [][]string{
	{"h2", "http/1.1"},
	{"http/1.1", "h2"},
	{"h2"},
	{"http/1.1"},
}

// UserAgentPool is the common browser User-Agent pool
var UserAgentPool = []string{
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
	"Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
	"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
	"Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1",
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
}

// Config constants
const (
	InitialWorkers     = 100
	MaxWorkers         = 2000
	MinWorkers         = 10
	JobBuffer          = 5000
	StreamingThreshold = 10 * 1024 * 1024
	GeoDBFile          = "GeoLite2-Country.mmdb"
	GeoDBURL           = "https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/GeoLite2-Country.mmdb"
	GeoASNFile         = "GeoLite2-ASN.mmdb"
	GeoASNURL          = "https://github.com/P3TERX/GeoLite.mmdb/releases/latest/download/GeoLite2-ASN.mmdb"
	GeoDBGitHubPath    = "/P3TERX/GeoLite.mmdb/releases/latest/download/"
	BadgerDBDir        = "badger_db"
	BatchSaveSize      = 10000
)

var GeoDBMirrors = []string{
	"https://gh.h233.eu.org/https://github.com",
	"https://rapidgit.jjda.de5.net/https://github.com",
	"https://gh.ddlc.top/https://github.com",
	"https://gh-proxy.org/https://github.com",
	"https://cdn.gh-proxy.org/https://github.com",
	"https://edgeone.gh-proxy.org/https://github.com",
	"https://cors.isteed.cc/github.com",
	"https://ghproxy.it/https://github.com",
	"https://github.boki.moe/https://github.com",
	"https://gh.jasonzeng.dev/https://github.com",
	"https://gh.monlor.com/https://github.com",
	"https://github.tbedu.top/https://github.com",
	"https://github.geekery.cn/https://github.com",
	"https://github.ednovas.xyz/https://github.com",
	"https://ghfile.geekertao.top/https://github.com",
	"https://ghp.keleyaa.com/https://github.com",
	"https://gh.chjina.com/https://github.com",
	"https://ghpxy.hwinzniej.top/https://github.com",
	"https://cdn.crashmc.com/https://github.com",
	"https://git.yylx.win/https://github.com",
	"https://gitproxy.mrhjx.cn/https://github.com",
	"https://ghproxy.cxkpro.top/https://github.com",
	"https://gh.xxooo.cf/https://github.com",
	"https://gh.idayer.com/https://github.com",
	"https://raw.ihtw.moe/github.com",
	"https://gh.zwy.one/https://github.com",
	"https://ghproxy.monkeyray.net/https://github.com",
	"https://ghproxy.net/https://github.com",
	"https://ghfast.top/https://github.com",
	"https://wget.la/https://github.com",
	"https://hk.gh-proxy.org/https://github.com",
	"gitclone.com",
	"githubfast.com",
}

// BadgerDB GC config
const (
	GCInterval = 15 * time.Minute
	GCRatio    = 0.3
)

// IDM-style download config
const (
	DownloadChunkSize = 1024 * 1024
	DownloadWorkers   = 8
)

// SeedBlockedASNs is the initial set of blocked ASNs
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
