package pkg

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/oschwald/geoip2-golang"
)

var countryCache = NewLRU[string, string](100000)
var asnResultCache = NewLRU[string, ASNResult](100000)

func GetCachedCountry(ip string, geoDB *geoip2.Reader) string {
	if cached, ok := countryCache.Get(ip); ok {
		return cached
	}

	countryCode := "UNKNOWN"
	record, err := geoDB.Country(net.ParseIP(ip))
	if err == nil {
		if record.Country.IsoCode != "" {
			countryCode = record.Country.IsoCode
		} else if record.RegisteredCountry.IsoCode != "" {
			countryCode = record.RegisteredCountry.IsoCode
		}
	}

	countryCache.Set(ip, countryCode)
	return countryCode
}

func GetCachedASN(ip string, asnDB *geoip2.Reader) (ASNResult, error) {
	if cached, ok := asnResultCache.Get(ip); ok {
		return cached, nil
	}

	asn, org := GetASN(net.ParseIP(ip), asnDB)
	result := ASNResult{ASN: asn, Org: org}
	asnResultCache.Set(ip, result)
	return result, nil
}

func GetASN(ip net.IP, db *geoip2.Reader) (uint32, string) {
	record, err := db.ASN(ip)
	if err != nil {
		return 0, ""
	}
	return uint32(record.AutonomousSystemNumber), record.AutonomousSystemOrganization
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

	if needCountry {
		if err := downloadWithMirrors(geoFile, GeoDBURL, proxyURL); err != nil {
			return fmt.Errorf("failed to download GeoLite2-Country: %w", err)
		}
	}
	if needASN {
		if err := downloadWithMirrors(asnFile, GeoASNURL, proxyURL); err != nil {
			return fmt.Errorf("failed to download GeoLite2-ASN: %w", err)
		}
	}

	return nil
}

func downloadWithMirrors(filePath, primaryURL, proxyString string) error {
	filename := filepath.Base(primaryURL)
	// Try mirrors first
	for _, mirror := range GeoDBMirrors {
		base := mirror
		if !strings.HasPrefix(base, "http") {
			base = "https://" + base
		}
		base = strings.TrimRight(base, "/")
		mirrorURL := base + GeoDBGitHubPath + filename
		if err := tryDownload(filePath, mirrorURL, proxyString); err == nil {
			return nil
		}
	}
	// Fallback to original URL
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
