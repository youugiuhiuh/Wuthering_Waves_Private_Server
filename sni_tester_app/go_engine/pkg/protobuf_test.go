package pkg

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"google.golang.org/protobuf/proto"
	snipb "go_engine/proto"
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

	var pb snipb.DomainList
	if err := proto.Unmarshal(data, &pb); err != nil {
		t.Fatalf("failed to unmarshal: %v", err)
	}

	expected := []string{"another.com", "example.com", "test.com", "zzz.com"}
	if len(pb.Domains) != len(expected) {
		t.Errorf("expected %d domains, got %d", len(expected), len(pb.Domains))
	}

	for i, d := range pb.Domains {
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

	var pb snipb.DomainList
	if err := proto.Unmarshal(data, &pb); err != nil {
		t.Fatalf("failed to unmarshal: %v", err)
	}

	if !sort.StringsAreSorted(pb.Domains) {
		t.Errorf("domains are not sorted: %v", pb.Domains)
	}
}

func TestParseProtobufDomains_ValidData(t *testing.T) {
	domains := []string{"example.com", "test.com", "demo.org"}
	pb := &snipb.DomainList{Domains: domains}

	data, err := proto.Marshal(pb)
	if err != nil {
		t.Fatalf("failed to marshal: %v", err)
	}

	parsed, err := ParseProtobufDomains(data)
	if err != nil {
		t.Fatalf("ParseProtobufDomains failed: %v", err)
	}

	if len(parsed) != len(domains) {
		t.Errorf("expected %d domains, got %d", len(domains), len(parsed))
	}

	for i, d := range parsed {
		if i < len(domains) && d != domains[i] {
			t.Errorf("domain[%d]: expected %s, got %s", i, domains[i], d)
		}
	}
}

func TestParseProtobufDomains_InvalidDomain(t *testing.T) {
	domains := []string{"example.com", "invalid", "test.com", ""}
	pb := &snipb.DomainList{Domains: domains}

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
