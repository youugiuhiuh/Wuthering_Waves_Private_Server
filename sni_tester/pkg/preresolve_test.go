package pkg

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// RED 测试 1: RunResolvePreMap 读取域名输入并并发输出 domain,ip 映射
func TestRunResolvePreMap_E2E(t *testing.T) {
	tmpDir := t.TempDir()
	inFile := filepath.Join(tmpDir, "domains.txt")
	outFile := filepath.Join(tmpDir, "resolved.csv")

	// 写入一个公共知名域名（海外 Anycast DNS 均能稳定解析）
	content := "1,dns.google\n"
	if err := os.WriteFile(inFile, []byte(content), 0644); err != nil {
		t.Fatalf("write input: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	opts := PreResolveOptions{
		InputFile:  inFile,
		OutputFile: outFile,
		Workers:    2,
		Timeout:    2 * time.Second,
	}

	stats, err := RunResolvePreMap(ctx, opts)
	if err != nil {
		t.Fatalf("RunResolvePreMap failed: %v", err)
	}

	if stats.Total != 1 {
		t.Errorf("stats.Total = %d, want 1", stats.Total)
	}

	// 验证输出文件格式为 domain,ip
	data, err := os.ReadFile(outFile)
	if err != nil {
		t.Fatalf("read output: %v", err)
	}

	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) == 0 || lines[0] == "" {
		t.Fatalf("expected non-empty output lines, got 0")
	}

	parts := strings.Split(lines[0], ",")
	if len(parts) != 2 || parts[0] != "dns.google" || parts[1] == "" {
		t.Errorf("unexpected output line format: %q", lines[0])
	}
}

// RED 测试 2: 验证 ParseDomainIPLine 支持解析纯域名和 domain,ip 两种格式
func TestParseDomainIPLine(t *testing.T) {
	cases := []struct {
		input      string
		wantDomain string
		wantIP     string
	}{
		{"example.com", "example.com", ""},
		{"example.com,93.184.216.34", "example.com", "93.184.216.34"},
		{"1,example.com,93.184.216.34", "example.com", "93.184.216.34"},
		{"1,example.com", "example.com", ""},
		{"   spaced.com , 1.2.3.4  ", "spaced.com", "1.2.3.4"},
	}

	for _, c := range cases {
		d, ip := ParseDomainIPLine(c.input)
		if d != c.wantDomain || ip != c.wantIP {
			t.Errorf("ParseDomainIPLine(%q) = (%q, %q), want (%q, %q)", c.input, d, ip, c.wantDomain, c.wantIP)
		}
	}
}
