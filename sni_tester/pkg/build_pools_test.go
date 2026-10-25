package pkg

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// RED 测试: RunBuildPools 读取候选域名并生成符合双池标准的 .pb 文件
func TestRunBuildPools_E2E(t *testing.T) {
	tmpDir := t.TempDir()
	candFile := filepath.Join(tmpDir, "candidates.txt")
	outDir := filepath.Join(tmpDir, "pb_output")

	// 写入一个已知的非 Cloudflare 合格目标用于测试
	content := "endlessm.com\n"
	if err := os.WriteFile(candFile, []byte(content), 0644); err != nil {
		t.Fatalf("write candidates file: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	opts := BuildPoolsOptions{
		CandidatesFile: candFile,
		OutputDir:      outDir,
		Workers:        2,
		Timeout:        4 * time.Second,
	}

	stats, err := RunBuildPools(ctx, opts)
	if err != nil {
		t.Fatalf("RunBuildPools failed: %v", err)
	}

	if stats.Total != 1 {
		t.Errorf("stats.Total = %d, want 1", stats.Total)
	}

	// 如果网络探针通过，验证生成了 .pb 文件；若网络不可达（如离线运行），stats.Rejected 会为 1
	entries, err := os.ReadDir(outDir)
	if err != nil {
		t.Fatalf("read outDir: %v", err)
	}
	if stats.PQCount+stats.ClassicCount > 0 && len(entries) == 0 {
		t.Errorf("expected .pb files generated for valid domains, got 0")
	}
}
