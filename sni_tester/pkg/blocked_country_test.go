package pkg

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// RED 测试: 验证 build-pools 会自动丢弃 CN, HK, MO, IR, RU, KP 等审查管辖地区的域名
func TestRunBuildPools_FiltersBlockedCountries(t *testing.T) {
	tmpDir := t.TempDir()
	candFile := filepath.Join(tmpDir, "candidates.txt")
	outDir := filepath.Join(tmpDir, "pb_out")

	content := "test.com\n"
	if err := os.WriteFile(candFile, []byte(content), 0644); err != nil {
		t.Fatalf("write file: %v", err)
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

	// 确保绝对不会生成 CN.pb, IR.pb, RU.pb, KP.pb
	blocked := []string{"CN.pb", "HK.pb", "MO.pb", "IR.pb", "RU.pb", "KP.pb"}
	for _, b := range blocked {
		p := filepath.Join(outDir, b)
		if _, err := os.Stat(p); err == nil {
			t.Errorf("blocked country pb %s was generated, should be discarded", b)
		}
	}
}

// 验证 IsBlockedCountry 判定
func TestIsBlockedCountry(t *testing.T) {
	for _, c := range []string{"CN", "HK", "MO", "IR", "RU", "KP"} {
		if !IsBlockedCountry(c) {
			t.Errorf("expected %s to be blocked, got false", c)
		}
	}
	for _, c := range []string{"US", "DE", "JP", "SG", "GB"} {
		if IsBlockedCountry(c) {
			t.Errorf("expected %s NOT to be blocked, got true", c)
		}
	}
}
