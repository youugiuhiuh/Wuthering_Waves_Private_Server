package main

import (
	"testing"
	"time"
)

func TestKeyExpiredEmptyIsExpired(t *testing.T) {
	e := minisignKeyEntry{PublicKey: "X", ExpiresAt: ""}
	if !e.expired() {
		t.Fatal("空 ExpiresAt 应视为过期")
	}
}

func TestKeyExpiredMalformedIsExpired(t *testing.T) {
	e := minisignKeyEntry{PublicKey: "X", ExpiresAt: "not-a-date"}
	if !e.expired() {
		t.Fatal("无法解析的 ExpiresAt 应视为过期")
	}
}

func TestKeyExpiredFutureIsNotExpired(t *testing.T) {
	e := minisignKeyEntry{PublicKey: "X", ExpiresAt: "2999-12-31"}
	if e.expired() {
		t.Fatal("远期日期不应过期")
	}
}

func TestKeyExpiredPastIsExpired(t *testing.T) {
	e := minisignKeyEntry{PublicKey: "X", ExpiresAt: "2000-01-01"}
	if !e.expired() {
		t.Fatal("过去日期应过期")
	}
}

func TestIsActiveRequiresEmptyRetiredAt(t *testing.T) {
	active := minisignKeyEntry{PublicKey: "X", ExpiresAt: "2999-12-31", RetiredAt: ""}
	retired := minisignKeyEntry{PublicKey: "X", ExpiresAt: "2999-12-31", RetiredAt: "2026-01-01"}
	if !active.isActive() {
		t.Fatal("retired_at 为空应为活跃")
	}
	if retired.isActive() {
		t.Fatal("retired_at 非空应为历史")
	}
}

func TestKeyListsDisjoint(t *testing.T) {
	for _, e := range minisignActiveKeys {
		if e.RetiredAt != "" {
			t.Fatalf("活跃列表含 retired_at 非空项: %s", e.PublicKey)
		}
	}
	for _, e := range minisignHistoricalKeys {
		if e.RetiredAt == "" {
			t.Fatalf("历史列表含 retired_at 为空项: %s", e.PublicKey)
		}
	}
}

func TestTrustedCommentParsing(t *testing.T) {
	v, a, err := parseTrustedComment("v1.5.3:aegis")
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if v != "v1.5.3" || a != "aegis" {
		t.Fatalf("解析结果错误: %q %q", v, a)
	}
}

// 文档性测试：记录 trusted comment 的解析语义与格式约定。
//
// ⚠️ 本测试**不覆盖** main.go 里的版本校验逻辑：真正的比较在
// installReleaseBinary（需网络与真实 release），当前无夹具可达。
// 因此把调用方改回 strings.HasPrefix 时，**本测试不会失败**
// （已实测：把 main.go:884 的比较注入为恒放行，本测试仍通过）。
// 该模式的静态兜底见计划 Task 12 的 grep 检查。
func TestTrustedCommentParsingIsNotAVersionGuard(t *testing.T) {
	// 仅锁定「解析结果本身正确」与「子串变体不是同一个字符串」这两个事实，
	// 不声称守护 main.go 的比较逻辑。
	v, _, err := parseTrustedComment("v1.5.3:aegis")
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if v == "v1.5.3-evil" || v == "xv1.5.3" {
		t.Fatal("解析结果不可能是子串/前缀变体（若至此说明解析被改坏）")
	}
}

func TestExpiredLogicUsesWallClock(t *testing.T) {
	// 确认比较确实基于当前时间（而非硬编码）
	future := time.Now().AddDate(1, 0, 0).Format("2006-01-02")
	if (&minisignKeyEntry{ExpiresAt: future}).expired() {
		t.Fatal("一年后到期不应视为过期")
	}
}
