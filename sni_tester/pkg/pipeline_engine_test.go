package pkg

import (
	"context"
	"testing"
	"time"
)

func TestPerformCombinedProbe_Endlessm(t *testing.T) {
	// 测试单连接复合探测：一次握手同时完成 TLS1.3 + PQ判断 + HTTP审查
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	res, err := PerformCombinedProbe(ctx, "endlessm.com", "", 4*time.Second)
	if err != nil {
		t.Skipf("Network unavailable for live probe test: %v", err)
	}

	if !res.HandshakeOK {
		t.Fatalf("HandshakeOK = false, err=%s", res.Error)
	}

	if res.CertChainLen <= 0 || res.CertChainLen > 3 {
		t.Errorf("Unexpected CertChainLen: %d", res.CertChainLen)
	}

	valid, reason := ValidateDomain(res)
	if !valid {
		t.Errorf("Expected endlessm.com to be valid, got false: %s", reason)
	}
}

func TestEngineTwoStagePipelineStream(t *testing.T) {
	cfg := DefaultConfig()
	cfg.OutputDir = t.TempDir()
	cfg.Debug = false // 验证非 debug 模式下流式统计不膨胀 resultList
	cfg.GeoDBFile = "../ipinfo_lite.mmdb"
	cfg.GeoASNFile = "../ipinfo_lite.mmdb"

	engine, err := NewEngine(cfg)
	if err != nil {
		t.Skipf("Skipping live engine test if GeoDB is missing: %v", err)
	}
	defer engine.Close()

	domains := []string{
		"microsoft.com",
		"invalid-non-existent-domain-xyz-12345.com",
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	result, err := engine.Run(ctx, domains, nil)
	if err != nil {
		t.Fatalf("engine.Run failed: %v", err)
	}

	if result.Stats.Total != len(domains) {
		t.Errorf("Stats.Total = %d, want %d", result.Stats.Total, len(domains))
	}

	// 验证内存优化：非 debug 模式下 resultList 不累积全量切片，避免百万级内存膨胀
	if len(result.DomainResults) != 0 {
		t.Errorf("Expected result.DomainResults to be empty in non-debug mode, got len=%d", len(result.DomainResults))
	}
}
