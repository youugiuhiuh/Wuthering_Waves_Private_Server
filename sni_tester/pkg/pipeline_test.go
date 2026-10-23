package pkg

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestTwoStageStreamingPipeline(t *testing.T) {
	tmpDir := t.TempDir()
	inPath := filepath.Join(tmpDir, "sample.pb")
	outPath := filepath.Join(tmpDir, "sample_out.pb")

	// 构造测试数据
	testDomains := []string{
		"1.1.1.1", // IP 形式或合法域名
		"google.com",
		"cloudflare.com",
		"invalid-non-existent-domain-xyz-12345.com",
	}
	err := WriteProtobufDomainFile(testDomains, inPath)
	if err != nil {
		t.Fatalf("WriteProtobufDomainFile failed: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	stats, err := UpgradePbFile(ctx, inPath, outPath, 10, 2*time.Second, false)
	if err != nil {
		t.Fatalf("UpgradePbFile failed: %v", err)
	}

	if stats.Total != len(testDomains) {
		t.Errorf("stats.Total = %d; want %d", stats.Total, len(testDomains))
	}

	// 确认输出文件生成并可解析
	data, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("Failed to read output pb: %v", err)
	}

	dl, err := ParseProtobufDomainList(data)
	if err != nil {
		t.Fatalf("Failed to parse output pb: %v", err)
	}

	if len(dl.Domains) == 0 && stats.PQCount+stats.ClassicCount > 0 {
		t.Errorf("Expected non-empty domains in output")
	}
}
