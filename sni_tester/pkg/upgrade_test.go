package pkg

import (
	"os"
	"path/filepath"
	"testing"

	utls "github.com/refraction-networking/utls"
)

func TestIsPQGroup(t *testing.T) {
	tests := []struct {
		group utls.CurveID
		want  bool
	}{
		{utls.X25519MLKEM768, true},
		{utls.X25519Kyber768Draft00, true},
		{CurveSecP256r1MLKEM768, true},
		{utls.X25519, false},
		{utls.CurveP256, false},
		{utls.CurveP384, false},
	}

	for _, tt := range tests {
		got := IsPQGroup(tt.group)
		if got != tt.want {
			t.Errorf("IsPQGroup(%v) = %v; want %v", tt.group, got, tt.want)
		}
	}
}

func TestWriteProtobufDualDomainFile(t *testing.T) {
	tmpDir := t.TempDir()
	filePath := filepath.Join(tmpDir, "test.pb")

	pq := []string{"cloudflare.com", "google.com"}
	classic := []string{"example.com", "google.com"} // 重复的 google.com 属于全集去重

	err := WriteProtobufDualDomainFile(pq, classic, filePath)
	if err != nil {
		t.Fatalf("WriteProtobufDualDomainFile failed: %v", err)
	}

	data, err := os.ReadFile(filePath)
	if err != nil {
		t.Fatalf("ReadFile failed: %v", err)
	}

	pb, err := ParseProtobufDomainList(data)
	if err != nil {
		t.Fatalf("ParseProtobufDomainList failed: %v", err)
	}

	if len(pb.PqDomains) != 2 {
		t.Errorf("PqDomains len = %d; want 2", len(pb.PqDomains))
	}
	if len(pb.ClassicDomains) != 2 {
		t.Errorf("ClassicDomains len = %d; want 2", len(pb.ClassicDomains))
	}
	// 全集去重后应为 3: cloudflare.com, example.com, google.com
	if len(pb.Domains) != 3 {
		t.Errorf("Domains len = %d; want 3", len(pb.Domains))
	}
}

func TestUpgradePbFileEmpty(t *testing.T) {
	tmpDir := t.TempDir()
	emptyPb := filepath.Join(tmpDir, "empty.pb")
	os.WriteFile(emptyPb, []byte{}, 0644)

	stats, err := UpgradePbFile(t.Context(), emptyPb, "", 1, 0, false)
	if err != nil {
		t.Fatalf("UpgradePbFile failed on empty: %v", err)
	}
	if stats.Total != 0 {
		t.Errorf("stats.Total = %d; want 0", stats.Total)
	}
}
