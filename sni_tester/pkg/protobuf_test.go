package pkg

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"google.golang.org/protobuf/proto"
	snipb "sni_tester/proto"
)

func TestWriteProtobufDomainFile_Deduplication(t *testing.T) {
	tmpDir := t.TempDir()
	tmpFile := filepath.Join(tmpDir, "test.pb")

	domains := []string{
		"example.com",
		"test.com",
		"example.com",
		"another.com",
		"test.com",
		"zzz.com",
	}

	err := WriteProtobufDomainFile(domains, tmpFile)
	if err != nil {
		t.Fatalf("WriteProtobufDomainFile failed: %v", err)
	}

	data, err := os.ReadFile(tmpFile)
	if err != nil {
		t.Fatalf("failed to read file: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("failed to parse: %v", err)
	}

	expected := []string{"another.com", "example.com", "test.com", "zzz.com"}
	if len(parsed) != len(expected) {
		t.Errorf("expected %d domains, got %d", len(expected), len(parsed))
	}

	for i, d := range parsed {
		if i < len(expected) && d != expected[i] {
			t.Errorf("domain[%d]: expected %s, got %s", i, expected[i], d)
		}
	}
}

func TestWriteProtobufDomainFile_EmptyInput(t *testing.T) {
	tmpDir := t.TempDir()
	tmpFile := filepath.Join(tmpDir, "empty.pb")

	err := WriteProtobufDomainFile([]string{}, tmpFile)
	if err != nil {
		t.Errorf("expected nil for empty input, got error: %v", err)
	}

	if _, err := os.Stat(tmpFile); !os.IsNotExist(err) {
		t.Error("expected no file to be created for empty input")
	}
}

func TestWriteProtobufDomainFile_Sorting(t *testing.T) {
	tmpDir := t.TempDir()
	tmpFile := filepath.Join(tmpDir, "sorted.pb")

	domains := []string{
		"zzz.com",
		"aaa.com",
		"mmm.com",
		"bbb.com",
	}

	err := WriteProtobufDomainFile(domains, tmpFile)
	if err != nil {
		t.Fatalf("WriteProtobufDomainFile failed: %v", err)
	}

	data, err := os.ReadFile(tmpFile)
	if err != nil {
		t.Fatalf("failed to read file: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("failed to parse: %v", err)
	}

	if !sort.StringsAreSorted(parsed) {
		t.Errorf("domains are not sorted: %v", parsed)
	}
}

func TestParseProtobufDomains_ValidData(t *testing.T) {
	domains := []string{"example.com", "test.com", "demo.org"}
	pb := &snipb.DomainList{ClassicDomains: domains}

	data, err := proto.Marshal(pb)
	if err != nil {
		t.Fatalf("failed to marshal: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("ParseProtobufDomains failed: %v", err)
	}

	expected := []string{"demo.org", "example.com", "test.com"}
	if len(parsed) != len(expected) {
		t.Errorf("expected %d domains, got %d", len(expected), len(parsed))
	}

	for i, d := range parsed {
		if i < len(expected) && d != expected[i] {
			t.Errorf("domain[%d]: expected %s, got %s", i, expected[i], d)
		}
	}
}

func TestParseProtobufDomains_InvalidDomain(t *testing.T) {
	domains := []string{"example.com", "invalid", "test.com", ""}
	pb := &snipb.DomainList{ClassicDomains: domains}

	data, err := proto.Marshal(pb)
	if err != nil {
		t.Fatalf("failed to marshal: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("ParseProtobufDomains failed: %v", err)
	}

	for _, d := range parsed {
		if d == "" {
			t.Error("empty domain should be filtered")
		}
	}
}

func TestProtobufRoundTrip(t *testing.T) {
	tmpDir := t.TempDir()
	tmpFile := filepath.Join(tmpDir, "roundtrip.pb")

	original := []string{
		"alpha.com",
		"beta.com",
		"gamma.com",
		"delta.com",
		"alpha.com",
	}

	err := WriteProtobufDomainFile(original, tmpFile)
	if err != nil {
		t.Fatalf("WriteProtobufDomainFile failed: %v", err)
	}

	data, err := os.ReadFile(tmpFile)
	if err != nil {
		t.Fatalf("failed to read file: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("ParseProtobufDomains failed: %v", err)
	}

	expected := []string{"alpha.com", "beta.com", "delta.com", "gamma.com"}
	if len(parsed) != len(expected) {
		t.Errorf("expected %d domains after dedup, got %d", len(expected), len(parsed))
	}
}

func TestSaveBatchDual(t *testing.T) {
	tmpDir := t.TempDir()
	pqMap := map[string][]string{
		"US": {"pq.us.com"},
		"JP": {"pq.jp.com"},
	}
	classicMap := map[string][]string{
		"US": {"classic.us.com"},
		"SG": {"classic.sg.com"},
	}

	err := SaveBatchDual(t.Context(), tmpDir, pqMap, classicMap, nil)
	if err != nil {
		t.Fatalf("SaveBatchDual failed: %v", err)
	}

	usData, err := os.ReadFile(filepath.Join(tmpDir, "US.pb"))
	if err != nil {
		t.Fatalf("Failed to read US.pb: %v", err)
	}
	dl, err := ParseProtobufDomainList(usData)
	if err != nil {
		t.Fatalf("Failed to parse US.pb: %v", err)
	}

	if len(dl.PqDomains) != 1 || dl.PqDomains[0] != "pq.us.com" {
		t.Errorf("PqDomains = %v; want [pq.us.com]", dl.PqDomains)
	}
	if len(dl.ClassicDomains) != 1 || dl.ClassicDomains[0] != "classic.us.com" {
		t.Errorf("ClassicDomains = %v; want [classic.us.com]", dl.ClassicDomains)
	}
	all, err := ParseProtobufDomains(usData)
	if err != nil {
		t.Fatalf("Failed to parse all domains: %v", err)
	}
	if len(all) != 2 {
		t.Errorf("all domains len = %d; want 2", len(all))
	}
}
