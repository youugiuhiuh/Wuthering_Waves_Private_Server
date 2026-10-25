package pkg

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/dgraph-io/badger/v4"
)

func newTestStorage(t *testing.T) *StorageManager {
	t.Helper()
	s, err := NewStorageManager(t.TempDir(), 7)
	if err != nil {
		t.Fatalf("NewStorageManager: %v", err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func readSuccessInfo(t *testing.T, s *StorageManager, domain string) SuccessInfo {
	t.Helper()
	var info SuccessInfo
	err := s.DB().View(func(txn *badger.Txn) error {
		item, err := txn.Get(append(keyPrefixSuccess(), domain...))
		if err != nil {
			return err
		}
		return item.Value(func(v []byte) error {
			return json.Unmarshal(v, &info)
		})
	})
	if err != nil {
		t.Fatalf("read success info for %s: %v", domain, err)
	}
	return info
}

// 回归：SaveSuccess 必须把 ASN/Org 一并落库。
// 否则 processDomain 已经查过的 ASN 白查，存量记录里 ASN 永远是 0、Org 永远为空。
func TestSaveSuccessPersistsASNAndOrg(t *testing.T) {
	s := newTestStorage(t)

	asn := ASNResult{ASN: 15169, Org: "Google LLC"}
	if err := s.SaveSuccess("example.com", "US", asn); err != nil {
		t.Fatalf("SaveSuccess: %v", err)
	}

	info := readSuccessInfo(t, s, "example.com")
	if info.Country != "US" {
		t.Errorf("Country = %q, want US", info.Country)
	}
	if info.ASN != 15169 {
		t.Errorf("ASN = %d, want 15169", info.ASN)
	}
	if info.Org != "Google LLC" {
		t.Errorf("Org = %q, want %q", info.Org, "Google LLC")
	}
	if info.TestedAt <= time.Now().Add(-time.Minute).Unix() {
		t.Errorf("TestedAt = %d, want a recent timestamp", info.TestedAt)
	}
}

// LoadSuccessHistory 仍要按 Country 分组，不能被 ASN 改动影响。
func TestLoadSuccessHistoryStillGroupsByCountry(t *testing.T) {
	s := newTestStorage(t)

	if err := s.SaveSuccess("a.com", "US", ASNResult{ASN: 1, Org: "OrgA"}); err != nil {
		t.Fatalf("SaveSuccess a.com: %v", err)
	}
	if err := s.SaveSuccess("b.com", "DE", ASNResult{}); err != nil {
		t.Fatalf("SaveSuccess b.com: %v", err)
	}

	got := s.LoadSuccessHistory()
	if got["a.com"] != "US" || got["b.com"] != "DE" {
		t.Errorf("LoadSuccessHistory = %v, want a.com=US b.com=DE", got)
	}
}
