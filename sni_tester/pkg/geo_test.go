package pkg

import (
	"os"
	"testing"

	maxminddb "github.com/oschwald/maxminddb-golang/v2"
)

func TestExtractCountryFromUnifiedRecord(t *testing.T) {
	// 1. IPinfo 格式
	r1 := UnifiedGeoRecord{
		CountryCode: "JP",
	}
	if got := r1.ExtractCountry(); got != "JP" {
		t.Errorf("ExtractCountry() = %s; want JP", got)
	}

	r2 := UnifiedGeoRecord{
		Country: "SG",
	}
	if got := r2.ExtractCountry(); got != "SG" {
		t.Errorf("ExtractCountry() = %s; want SG", got)
	}

	// 2. MaxMind 格式
	r3 := UnifiedGeoRecord{}
	r3.MaxMindCountry.ISOCode = "US"
	if got := r3.ExtractCountry(); got != "US" {
		t.Errorf("ExtractCountry() = %s; want US", got)
	}

	// 3. 空值
	r4 := UnifiedGeoRecord{}
	if got := r4.ExtractCountry(); got != "UNKNOWN" {
		t.Errorf("ExtractCountry() = %s; want UNKNOWN", got)
	}
}

func TestExtractASNFromUnifiedRecord(t *testing.T) {
	// 1. IPinfo 格式 ("AS13335", "Cloudflare")
	r1 := UnifiedGeoRecord{
		ASN:    "AS13335",
		ASName: "Cloudflare, Inc.",
	}
	asn1, org1 := r1.ExtractASN()
	if asn1 != 13335 || org1 != "Cloudflare, Inc." {
		t.Errorf("ExtractASN() = (%d, %s); want (13335, Cloudflare, Inc.)", asn1, org1)
	}

	// 2. MaxMind 格式
	r2 := UnifiedGeoRecord{
		MaxMindASN: 15169,
		MaxMindOrg: "Google LLC",
	}
	asn2, org2 := r2.ExtractASN()
	if asn2 != 15169 || org2 != "Google LLC" {
		t.Errorf("ExtractASN() = (%d, %s); want (15169, Google LLC)", asn2, org2)
	}
}

func TestGetCachedCountryInvalidIPFallsBackToUnknown(t *testing.T) {
	got := GetCachedCountry("not-an-ip", nil)
	if got != "UNKNOWN" {
		t.Fatalf("GetCachedCountry(%q) = %q, want %q", "not-an-ip", got, "UNKNOWN")
	}
}

func TestGetCachedASNInvalidIPFallsBackToZero(t *testing.T) {
	res, err := GetCachedASN("not-an-ip", nil)
	if err != nil {
		t.Fatalf("GetCachedASN(%q) unexpected error: %v", "not-an-ip", err)
	}
	if res.ASN != 0 || res.Org != "" {
		t.Fatalf("GetCachedASN(%q) = %+v, want zero ASNResult", "not-an-ip", res)
	}
}

// 真实 IPinfo Lite 库验证：一条 IP 必须同时给出 Country 与 ASN，
// 这正是「一个 mmdb 取代 GeoLite2-Country + GeoLite2-ASN」的核心收益。
func TestRealIPinfoLiteProvidesBothCountryAndASN(t *testing.T) {
	dbPath := "../ipinfo_lite.mmdb"
	if _, err := os.Stat(dbPath); err != nil {
		t.Skipf("ipinfo_lite.mmdb not present: %v", err)
	}

	db, err := maxminddb.Open(dbPath)
	if err != nil {
		t.Fatalf("open %s: %v", dbPath, err)
	}
	defer db.Close()

	tests := []struct {
		ip      string
		country string
		asn     uint32
	}{
		{"8.8.8.8", "US", 15169},
		{"1.1.1.1", "AU", 13335},
		{"144.76.0.1", "DE", 24940},
	}

	for _, tt := range tests {
		t.Run(tt.ip, func(t *testing.T) {
			countryCache.Clear()
			asnResultCache.Clear()

			if got := GetCachedCountry(tt.ip, db); got != tt.country {
				t.Errorf("Country(%s) = %q, want %q", tt.ip, got, tt.country)
			}
			got, err := GetCachedASN(tt.ip, db)
			if err != nil {
				t.Fatalf("GetCachedASN(%s): %v", tt.ip, err)
			}
			if got.ASN != tt.asn {
				t.Errorf("ASN(%s) = %d, want %d", tt.ip, got.ASN, tt.asn)
			}
			if got.Org == "" {
				t.Errorf("Org(%s) is empty, want organization name", tt.ip)
			}
		})
	}
}
