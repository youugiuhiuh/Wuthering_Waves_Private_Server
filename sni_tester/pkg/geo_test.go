package pkg

import "testing"

// Regression coverage for the geoip2-golang v2 migration. v2 lookups take a
// netip.Addr, so the IP string must be parsed (and validated) before hitting the
// database. Invalid input must fall back to the documented defaults instead of
// dereferencing a nil reader, which is what the pre-migration code did.
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
