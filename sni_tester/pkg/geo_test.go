package pkg

import (
	"testing"
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
