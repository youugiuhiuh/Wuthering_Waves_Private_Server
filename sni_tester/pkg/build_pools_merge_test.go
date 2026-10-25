package pkg

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// RED 测试: 验证 build-pools 会合并已有 .pb 文件，而不会覆盖丢掉原有的域名
func TestRunBuildPools_MergesWithExistingPB(t *testing.T) {
	tmpDir := t.TempDir()
	candFile := filepath.Join(tmpDir, "candidates.txt")
	outDir := filepath.Join(tmpDir, "pb_out")
	os.MkdirAll(outDir, 0755)

	// 1. 先在 outDir 下写入一个已有的 US.pb，包含 old.example.com
	existingPB := filepath.Join(outDir, "US.pb")
	if err := WriteProtobufDualDomainFile([]string{"existing-pq.com"}, []string{"existing-classic.com"}, existingPB); err != nil {
		t.Fatalf("write existing pb: %v", err)
	}

	// 2. 传入空候选文件或一个被拒绝的域名，运行 RunBuildPools
	if err := os.WriteFile(candFile, []byte("invalid-domain-123.test\n"), 0644); err != nil {
		t.Fatalf("write cand file: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	opts := BuildPoolsOptions{
		CandidatesFile: candFile,
		OutputDir:      outDir,
		Workers:        1,
		Timeout:        1 * time.Second,
	}

	_, _ = RunBuildPools(ctx, opts)

	// 3. 验证已有的 US.pb 依然存在，且里面的 existing-pq.com 没有丢失
	data, err := os.ReadFile(existingPB)
	if err != nil {
		t.Fatalf("read existing pb after build: %v", err)
	}
	dl, err := ParseProtobufDomainList(data)
	if err != nil {
		t.Fatalf("parse pb: %v", err)
	}

	foundPQ := false
	for _, d := range dl.PqDomains {
		if d == "existing-pq.com" {
			foundPQ = true
			break
		}
	}
	if !foundPQ {
		t.Errorf("expected existing-pq.com to be preserved in US.pb, got: %v", dl.PqDomains)
	}
}
