package main

import (
	"slices"
	"testing"
)

// findAsset 依赖 latestRelease.Assets，这里只构造最小字段。
func releaseWithAssets(names ...string) *latestRelease {
	rel := &latestRelease{}
	for _, name := range names {
		rel.Assets = append(rel.Assets, releaseAsset{Name: name})
	}
	return rel
}

// ---- aegisAssetCandidates：架构 → 资产名优先级 ----

func TestAegisAssetCandidates(t *testing.T) {
	cases := map[string][]string{
		// amd64 保留对旧版无后缀 aegis 的兼容回退。
		"amd64": {"aegis-linux-amd64", "aegis"},
		// arm64 绝不允许回退到 x86_64 二进制：装了也跑不起来。
		"arm64": {"aegis-linux-arm64"},
	}
	for arch, want := range cases {
		if got := aegisAssetCandidates(arch); !slices.Equal(got, want) {
			t.Fatalf("arch %s: got %v want %v", arch, got, want)
		}
	}
}

// 未知架构返回空：调用方必须当作硬错误，不能静默选一个跑不起来的二进制。
func TestAegisAssetCandidatesRejectsUnknownArch(t *testing.T) {
	for _, arch := range []string{"riscv64", "386", ""} {
		if got := aegisAssetCandidates(arch); len(got) != 0 {
			t.Fatalf("unknown arch %q must return empty, got %v", arch, got)
		}
	}
}

// ---- selectAegisAsset：从 release 资产清单中按优先级选取 ----

func TestSelectAegisAssetPrefersArchSpecific(t *testing.T) {
	rel := releaseWithAssets("installer-linux-amd64", "aegis-linux-amd64", "aegis-linux-arm64", "aegis")

	name, asset := selectAegisAsset(rel, "amd64")
	if name != "aegis-linux-amd64" || asset == nil || asset.Name != "aegis-linux-amd64" {
		t.Fatalf("amd64: got %q asset=%v", name, asset)
	}

	name, asset = selectAegisAsset(rel, "arm64")
	if name != "aegis-linux-arm64" || asset == nil || asset.Name != "aegis-linux-arm64" {
		t.Fatalf("arm64: got %q asset=%v", name, asset)
	}
}

// 旧版 release 只有裸 aegis，amd64 必须还能装上（否则存量部署全断）。
func TestSelectAegisAssetFallsBackToLegacyForAmd64(t *testing.T) {
	rel := releaseWithAssets("aegis", "installer")

	name, asset := selectAegisAsset(rel, "amd64")
	if name != "aegis" || asset == nil {
		t.Fatalf("amd64 legacy fallback: got %q asset=%v", name, asset)
	}
}

// 安全关键：arm64 遇到只有裸 aegis 的旧版 release 必须拒绝，而不是装 x86 二进制。
func TestSelectAegisAssetRefusesLegacyForArm64(t *testing.T) {
	rel := releaseWithAssets("aegis", "installer")

	name, asset := selectAegisAsset(rel, "arm64")
	if name != "" || asset != nil {
		t.Fatalf("arm64 must not fall back to x86_64 aegis: got %q asset=%v", name, asset)
	}
}

func TestSelectAegisAssetRefusesUnknownArch(t *testing.T) {
	rel := releaseWithAssets("aegis", "aegis-linux-amd64", "aegis-linux-arm64")
	if name, asset := selectAegisAsset(rel, "riscv64"); name != "" || asset != nil {
		t.Fatalf("unknown arch must select nothing, got %q asset=%v", name, asset)
	}
}

func TestSelectAegisAssetEmptyRelease(t *testing.T) {
	if name, asset := selectAegisAsset(releaseWithAssets(), "amd64"); name != "" || asset != nil {
		t.Fatalf("empty release must select nothing, got %q asset=%v", name, asset)
	}
}
