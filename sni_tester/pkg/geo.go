package pkg

import (
	"fmt"
	"io"
	"net/http"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	maxminddb "github.com/oschwald/maxminddb-golang/v2"
)

var countryCache sync.Map
var asnResultCache sync.Map

// UnifiedGeoRecord seamlessly supports both IPinfo Lite MMDB and MaxMind GeoLite2 schemas.
type UnifiedGeoRecord struct {
	// IPinfo schema top-level fields
	Country     string `maxminddb:"country"`
	CountryCode string `maxminddb:"country_code"`
	CountryName string `maxminddb:"country_name"`
	ASN         string `maxminddb:"asn"`
	ASName      string `maxminddb:"as_name"`
	ASDomain    string `maxminddb:"as_domain"`

	// MaxMind schema nested fields
	MaxMindCountry struct {
		ISOCode string `maxminddb:"iso_code"`
	} `maxminddb:"country"`
	MaxMindRegisteredCountry struct {
		ISOCode string `maxminddb:"iso_code"`
	} `maxminddb:"registered_country"`
	MaxMindASN uint32 `maxminddb:"autonomous_system_number"`
	MaxMindOrg string `maxminddb:"autonomous_system_organization"`
}

func (r *UnifiedGeoRecord) ExtractCountry() string {
	if r.CountryCode != "" {
		return strings.ToUpper(r.CountryCode)
	}
	if len(r.Country) == 2 {
		return strings.ToUpper(r.Country)
	}
	if r.MaxMindCountry.ISOCode != "" {
		return strings.ToUpper(r.MaxMindCountry.ISOCode)
	}
	if r.MaxMindRegisteredCountry.ISOCode != "" {
		return strings.ToUpper(r.MaxMindRegisteredCountry.ISOCode)
	}
	return "UNKNOWN"
}

func (r *UnifiedGeoRecord) ExtractASN() (uint32, string) {
	if r.ASN != "" {
		clean := strings.TrimPrefix(strings.ToUpper(r.ASN), "AS")
		if n, err := strconv.ParseUint(clean, 10, 32); err == nil {
			org := r.ASName
			if org == "" {
				org = r.ASDomain
			}
			return uint32(n), org
		}
	}
	if r.MaxMindASN > 0 {
		return r.MaxMindASN, r.MaxMindOrg
	}
	return 0, ""
}

func GetCachedCountry(ip string, geoDB *maxminddb.Reader) string {
	if cached, ok := countryCache.Load(ip); ok {
		return cached.(string)
	}

	countryCode := "UNKNOWN"
	if geoDB != nil {
		if addr, err := netip.ParseAddr(ip); err == nil {
			res := geoDB.Lookup(addr)
			if res.Found() {
				var rec UnifiedGeoRecord
				if err := res.Decode(&rec); err == nil {
					countryCode = rec.ExtractCountry()
				}
			}
		}
	}

	countryCache.Store(ip, countryCode)
	return countryCode
}

func GetCachedASN(ip string, asnDB *maxminddb.Reader) (ASNResult, error) {
	if cached, ok := asnResultCache.Load(ip); ok {
		return cached.(ASNResult), nil
	}

	result := ASNResult{}
	if asnDB != nil {
		if addr, err := netip.ParseAddr(ip); err == nil {
			asn, org := GetASN(addr, asnDB)
			result = ASNResult{ASN: asn, Org: org}
		}
	}
	asnResultCache.Store(ip, result)
	return result, nil
}

func GetASN(ip netip.Addr, db *maxminddb.Reader) (uint32, string) {
	if db == nil {
		return 0, ""
	}
	res := db.Lookup(ip)
	if !res.Found() {
		return 0, ""
	}
	var rec UnifiedGeoRecord
	if err := res.Decode(&rec); err != nil {
		return 0, ""
	}
	return rec.ExtractASN()
}

func IsBlockedCountry(code string) bool {
	return code == "CN" || code == "HK" || code == "MO" || code == "IR" || code == "RU" || code == "KP"
}

func PrepareGeoDBs(geoFile, asnFile, proxyURL string) error {
	needCountry := false
	needASN := false

	if _, err := os.Stat(geoFile); os.IsNotExist(err) {
		needCountry = true
	}
	if _, err := os.Stat(asnFile); os.IsNotExist(err) {
		needASN = true
	}

	if !needCountry && !needASN {
		return nil
	}

	// 若两个目标文件相同（如统一使用 ipinfo_lite.mmdb），只下载一次
	if geoFile == asnFile && needCountry {
		fmt.Println("[GeoDB] Downloading unified IPinfo Lite database (Country + ASN)...")
		if err := downloadWithMirrors(geoFile, GeoDBURL, proxyURL); err != nil {
			return fmt.Errorf("failed to download unified IPinfo Lite database: %w", err)
		}
		return nil
	}

	if needCountry {
		if err := downloadWithMirrors(geoFile, GeoDBURL, proxyURL); err != nil {
			return fmt.Errorf("failed to download GeoIP database: %w", err)
		}
	}
	if needASN {
		if err := downloadWithMirrors(asnFile, GeoASNURL, proxyURL); err != nil {
			return fmt.Errorf("failed to download GeoASN database: %w", err)
		}
	}

	return nil
}

func downloadWithMirrors(filePath, primaryURL, proxyString string) error {
	filename := filepath.Base(primaryURL)
	for i, mirror := range GeoDBMirrors {
		base := mirror
		if !strings.HasPrefix(base, "http") {
			base = "https://" + base
		}
		base = strings.TrimRight(base, "/")
		mirrorURL := base + GeoDBGitHubPath + filename
		fmt.Printf("[GeoDB] Trying mirror %d/%d: %s\n", i+1, len(GeoDBMirrors), mirrorURL)
		if err := tryDownload(filePath, mirrorURL, proxyString); err == nil {
			return nil
		}
	}
	return tryDownload(filePath, primaryURL, proxyString)
}

func tryDownload(filePath, urlStr, proxyString string) error {
	transport := &http.Transport{}
	if proxyString != "" {
		pu, err := url.Parse(proxyString)
		if err == nil {
			if pu.Scheme == "http" || pu.Scheme == "https" {
				transport.Proxy = http.ProxyURL(pu)
			}
		}
	}
	client := &http.Client{Transport: transport, Timeout: 10 * time.Minute}

	resp, err := client.Get(urlStr)
	if err != nil {
		return fmt.Errorf("download request failed: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("unexpected status code: %d", resp.StatusCode)
	}

	dir := filepath.Dir(filePath)
	if dir != "" {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return fmt.Errorf("failed to create directory: %w", err)
		}
	}

	out, err := os.Create(filePath)
	if err != nil {
		return fmt.Errorf("failed to create file: %w", err)
	}
	defer out.Close()

	_, err = io.Copy(out, resp.Body)
	if err != nil {
		return fmt.Errorf("failed to write file: %w", err)
	}

	return nil
}
