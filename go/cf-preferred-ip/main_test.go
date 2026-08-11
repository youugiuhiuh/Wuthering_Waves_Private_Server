package main

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseCandidatesRanksLossThenSpeedStably(t *testing.T) {
	csv := "IP 地址,已发送,已接收,丢包率,平均延迟,下载速度,数据中心\n" +
		"162.159.2.2,4,4,0.00%,100 ms,1.00 MB/s,HKG\n" +
		"invalid,4,4,0.00%,100 ms,9.00 MB/s,HKG\n" +
		"162.159.1.1,4,4,0.00%,100 ms,2.00 MB/s,HKG\n" +
		"162.159.3.3,4,4,1.00%,100 ms,9.00 MB/s,HKG\n" +
		"162.159.4.4,4,4,0.00%,100 ms,2.00 MB/s,HKG\n"
	got, err := parseCandidates(strings.NewReader(csv))
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"162.159.1.1", "162.159.4.4", "162.159.2.2", "162.159.3.3"}
	if len(got) != len(want) {
		t.Fatalf("got %d candidates, want %d", len(got), len(want))
	}
	for i, ip := range want {
		if got[i].IP != ip {
			t.Fatalf("candidate %d = %q, want %q", i, got[i].IP, ip)
		}
	}
}

func TestAssignCandidatesCyclesOnlyWhenNeeded(t *testing.T) {
	candidates := []Candidate{{IP: "1.1.1.1"}, {IP: "2.2.2.2"}}
	got := assignCandidates(candidates, []string{"one.example", "two.example", "three.example"})
	want := []string{"1.1.1.1", "2.2.2.2", "1.1.1.1"}
	for i, ip := range want {
		if got[i].IP != ip {
			t.Fatalf("mapping %d = %q, want %q", i, got[i].IP, ip)
		}
	}
}

func TestReplaceAndRestoreOwnedBlockPreservesOtherHosts(t *testing.T) {
	input := "127.0.0.1 localhost\n10.0.0.7 private.example\n# BEGIN aegis-cf-preferred-ip\n9.9.9.9 old.example\n# END aegis-cf-preferred-ip\n"
	replaced, err := replaceOwnedBlock(input, []HostMapping{{IP: "1.1.1.1", Domain: "one.example"}})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(replaced, "127.0.0.1 localhost\n10.0.0.7 private.example") || !strings.Contains(replaced, "1.1.1.1 one.example") || strings.Contains(replaced, "old.example") {
		t.Fatalf("unexpected replacement: %q", replaced)
	}
	restored, changed, err := removeOwnedBlock(replaced)
	if err != nil || !changed {
		t.Fatalf("restore changed=%v err=%v", changed, err)
	}
	if !strings.Contains(restored, "127.0.0.1 localhost\n10.0.0.7 private.example") || strings.Contains(restored, "one.example") {
		t.Fatalf("unexpected restore: %q", restored)
	}
}

func TestParseCandidatesRejectsNoValidIPv4Rows(t *testing.T) {
	_, err := parseCandidates(strings.NewReader("IP 地址,丢包率,下载速度\nnot-an-ip,0.00%,2.00 MB/s\n"))
	if err == nil {
		t.Fatal("expected no-candidate error")
	}
}

func TestParseCandidatesRejectsIPv4MappedIPv6(t *testing.T) {
	csv := "IP,已发送,已接收,丢包率,平均延迟,下载速度\n::ffff:162.159.1.1,4,4,0.00%,100 ms,2.00 MB/s\n"
	_, err := parseCandidates(strings.NewReader(csv))
	if err == nil {
		t.Fatal("expected no-candidate error")
	}
}

func TestParseCandidatesRejectsNonFiniteScores(t *testing.T) {
	csv := "IP,Sent,Recv,Loss,Latency,Speed\n" +
		"162.159.1.1,4,4,NaN%,100 ms,1 MB/s\n" +
		"162.159.1.2,4,4,+Inf%,100 ms,1 MB/s\n" +
		"162.159.1.3,4,4,0%,100 ms,-Inf MB/s\n"
	_, err := parseCandidates(strings.NewReader(csv))
	if err == nil {
		t.Fatal("expected non-finite scores to be rejected")
	}
}

func TestReplaceOwnedBlockRejectsUnclosedExistingBlock(t *testing.T) {
	_, err := replaceOwnedBlock("127.0.0.1 localhost\n# BEGIN aegis-cf-preferred-ip\n", nil)
	if err == nil {
		t.Fatal("expected unclosed-block error")
	}
}

func TestRemoveOwnedBlockRejectsMalformedOwnedMarkers(t *testing.T) {
	for name, input := range map[string]string{
		"orphan end":       "127.0.0.1 localhost\n# END aegis-cf-preferred-ip\n",
		"nested begin":     "# BEGIN aegis-cf-preferred-ip\n# BEGIN aegis-cf-preferred-ip\n# END aegis-cf-preferred-ip\n",
		"duplicate blocks": "# BEGIN aegis-cf-preferred-ip\n1.1.1.1 one.example\n# END aegis-cf-preferred-ip\n# BEGIN aegis-cf-preferred-ip\n2.2.2.2 two.example\n# END aegis-cf-preferred-ip\n",
	} {
		t.Run(name, func(t *testing.T) {
			_, _, err := removeOwnedBlock(input)
			if err == nil {
				t.Fatal("expected malformed marker error")
			}
		})
	}
}

func TestRemoveOwnedBlockPreservesMarkerLookingText(t *testing.T) {
	input := "prefix # BEGIN aegis-cf-preferred-ip\n# END aegis-cf-preferred-ip suffix\n"
	got, changed, err := removeOwnedBlock(input)
	if err != nil || changed || got != input {
		t.Fatalf("got %q, changed=%v, err=%v", got, changed, err)
	}
}

func TestValidDomainRejectsInvalidHostnames(t *testing.T) {
	for _, domain := range []string{"", "192.0.2.1", "-bad.example", "bad-.example", "bad_example"} {
		if validDomain(domain) {
			t.Errorf("validDomain(%q) = true, want false", domain)
		}
	}
}

func TestRunDoesNotReplaceHostsWhenCFSTHasNoCandidates(t *testing.T) {
	original := "127.0.0.1 localhost\n# BEGIN aegis-cf-preferred-ip\n9.9.9.9 old.example\n# END aegis-cf-preferred-ip\n"
	dir := t.TempDir()
	hosts := filepath.Join(dir, "hosts")
	if err := os.WriteFile(hosts, []byte(original), 0644); err != nil {
		t.Fatal(err)
	}
	err := run([]string{"one.example"}, commandDeps{hostsPath: hosts, executablePath: filepath.Join(dir, "tool"), runCFST: func(_, output string) error {
		return os.WriteFile(output, []byte("IP,Sent,Recv,Loss,Latency,Speed\ninvalid,4,4,0%,1 ms,1 MB/s\n"), 0600)
	}})
	if err == nil {
		t.Fatal("expected error")
	}
	got, readErr := os.ReadFile(hosts)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != original {
		t.Fatalf("hosts changed on failure: %q", got)
	}
}

func TestRunWritesMappingsAndRemovesTemporaryCFSTData(t *testing.T) {
	dir := t.TempDir()
	hosts := filepath.Join(dir, "hosts")
	if err := os.WriteFile(hosts, []byte("127.0.0.1 localhost\n"), 0644); err != nil {
		t.Fatal(err)
	}
	var tempDir string
	err := run([]string{"one.example", "two.example"}, commandDeps{hostsPath: hosts, executablePath: filepath.Join(dir, "tool"), runCFST: func(_, output string) error {
		tempDir = filepath.Dir(output)
		return os.WriteFile(output, []byte("IP,Sent,Recv,Loss,Latency,Speed\n1.1.1.1,4,4,0%,1 ms,1 MB/s\n"), 0600)
	}})
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(hosts)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(got), "1.1.1.1 one.example\n1.1.1.1 two.example") {
		t.Fatalf("missing mappings: %q", got)
	}
	if _, err := os.Stat(tempDir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("temporary CFST data remains at %q: %v", tempDir, err)
	}
}

func TestRunCFSTUsesBinaryDirectoryForDefaultIPFile(t *testing.T) {
	dir := t.TempDir()
	binary := filepath.Join(dir, "CloudflareST")
	if err := os.WriteFile(binary, []byte("#!/bin/sh\nprintf '%s' \"$PWD\" > \"$2\"\n"), 0755); err != nil {
		t.Fatal(err)
	}
	output := filepath.Join(t.TempDir(), "result.csv")
	if err := runCFST(binary, output); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(output)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != dir {
		t.Fatalf("CFST working directory = %q, want %q", got, dir)
	}
}

func TestRunRestoreSkipsCFSTAndRemovesOwnedBlock(t *testing.T) {
	dir := t.TempDir()
	hosts := filepath.Join(dir, "hosts")
	if err := os.WriteFile(hosts, []byte("127.0.0.1 localhost\n# BEGIN aegis-cf-preferred-ip\n1.1.1.1 one.example\n# END aegis-cf-preferred-ip\n"), 0644); err != nil {
		t.Fatal(err)
	}
	err := run([]string{"--restore"}, commandDeps{hostsPath: hosts, runCFST: func(_, _ string) error {
		return errors.New("CFST must not run during restore")
	}})
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(hosts)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "127.0.0.1 localhost\n" {
		t.Fatalf("unexpected restored hosts: %q", got)
	}
}

func TestCLIExitsNonzeroWhenRunFails(t *testing.T) {
	command := exec.Command(os.Args[0], "-test.run=TestCLIHelperProcess", "--")
	command.Env = append(os.Environ(), "GO_WANT_CLI_HELPER_PROCESS=1")
	if err := command.Run(); err == nil {
		t.Fatal("CLI exited successfully after run failure")
	}
}

func TestElevatedChildExitStatusIsPreserved(t *testing.T) {
	command := exec.Command("sh", "-c", "exit 42")
	if err := command.Run(); err == nil {
		t.Fatal("expected child failure")
	} else if got := elevatedExitStatus(err); got != 42 {
		t.Fatalf("elevated child status = %d, want 42", got)
	}
	if got := elevatedExitStatus(errors.New("UAC cancelled")); got != 1 {
		t.Fatalf("startup failure status = %d, want 1", got)
	}
}

func TestCLIHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_CLI_HELPER_PROCESS") != "1" {
		return
	}
	os.Args = []string{"aegis-cf-preferred-ip"}
	main()
}
