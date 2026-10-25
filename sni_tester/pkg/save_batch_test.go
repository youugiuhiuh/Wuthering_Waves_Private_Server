package pkg

import (
	"context"
	"testing"
)

// 回归：批量落库这条路径同样不能丢 ASN/Org。
// processDomain 已经把 ASN 查出来了，SaveBatch 再写一次空值等于白查。
func TestSaveBatchPersistsASNAndOrg(t *testing.T) {
	s := newTestStorage(t)

	results := map[string][]jobResult{
		"US": {
			{domain: "a.com", success: true, country: "US", asn: 15169, org: "Google LLC"},
			{domain: "b.com", success: true, country: "US", asn: 13335, org: "Cloudflare, Inc."},
		},
	}

	if err := SaveBatch(context.Background(), t.TempDir(), results, s.DB()); err != nil {
		t.Fatalf("SaveBatch: %v", err)
	}

	a := readSuccessInfo(t, s, "a.com")
	if a.ASN != 15169 || a.Org != "Google LLC" {
		t.Errorf("a.com ASN/Org = %d/%q, want 15169/\"Google LLC\"", a.ASN, a.Org)
	}
	b := readSuccessInfo(t, s, "b.com")
	if b.ASN != 13335 || b.Org != "Cloudflare, Inc." {
		t.Errorf("b.com ASN/Org = %d/%q, want 13335/\"Cloudflare, Inc.\"", b.ASN, b.Org)
	}
}

// SaveBatch 里 db 为 nil 时只写 .pb，不应崩。
func TestSaveBatchNilDBNilResultsDoesNotPanic(t *testing.T) {
	if err := SaveBatch(context.Background(), t.TempDir(), nil, nil); err != nil {
		t.Fatalf("SaveBatch(nil, nil) = %v, want nil", err)
	}
}
