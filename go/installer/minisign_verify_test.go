package main

import (
	"os"
	"path/filepath"
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

// ---- 前接 Task 5 审查的 F5-1：对齐 Go 与 Rust 的到期日边界 ----

// TestKeyExpiryBoundaryMatchesRust 锁定 Go 与 Rust 在到期日【当天】的语义一致。
//
// Rust `key_expired` 用严格字符串比较 `now_str > expires_at`，
// 所以 expires_at == 今天 → 未过期（仍可用）。
// 修复前 Go 用 `time.Now().After(当天 00:00Z)`，当天即视为过期，两侧相差约 24h。
func TestKeyExpiryBoundaryMatchesRust(t *testing.T) {
	today := time.Now().UTC().Format("2006-01-02")
	if (&minisignKeyEntry{ExpiresAt: today}).expired() {
		t.Fatal("到期日当天不应视为过期（必须与 Rust 的严格字符串比较一致）")
	}

	yesterday := time.Now().UTC().AddDate(0, 0, -1).Format("2006-01-02")
	if !(&minisignKeyEntry{ExpiresAt: yesterday}).expired() {
		t.Fatal("到期日已过应视为过期")
	}

	if (&minisignKeyEntry{ExpiresAt: "2999-12-31"}).expired() {
		t.Fatal("远期不应过期")
	}
	if !(&minisignKeyEntry{ExpiresAt: "2000-01-01"}).expired() {
		t.Fatal("久远过去应过期")
	}
}

// ---- verifyMinisign 双表接口（brief Step 1）----

func TestVerifyMinisignRejectsMissingFiles(t *testing.T) {
	if _, err := verifyMinisign("/nonexistent/bin", "/nonexistent/sig",
		minisignActiveKeys, minisignHistoricalKeys); err == nil {
		t.Fatal("文件缺失应报错")
	}
}

func TestVerifyMinisignRejectsGarbageSignature(t *testing.T) {
	dir := t.TempDir()
	bin := dir + "/bin"
	sig := dir + "/bin.minisig"
	if err := os.WriteFile(bin, []byte("data"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(sig, []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := verifyMinisign(bin, sig, minisignActiveKeys, minisignHistoricalKeys); err == nil {
		t.Fatal("垃圾签名应报错")
	}
}

func TestVerifyMinisignSkipsExpiredActiveKey(t *testing.T) {
	dir := t.TempDir()
	bin := dir + "/bin"
	sig := dir + "/bin.minisig"
	if err := os.WriteFile(bin, []byte("data"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(sig, []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	expired := []minisignKeyEntry{{
		PublicKey: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
		ExpiresAt: "2000-01-01",
	}}
	if _, err := verifyMinisign(bin, sig, expired, nil); err == nil {
		t.Fatal("过期活跃钥应被跳过致失败")
	}
}

func TestVerifyMinisignTriesHistoricalKeysDespiteExpiry(t *testing.T) {
	dir := t.TempDir()
	bin := dir + "/bin"
	sig := dir + "/bin.minisig"
	if err := os.WriteFile(bin, []byte("data"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(sig, []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	historical := []minisignKeyEntry{{
		PublicKey: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
		ExpiresAt: "2000-01-01",
		RetiredAt: "2026-01-01",
	}}
	// 仍应报错（签名无效），但关键是不能 panic 且确实尝试了历史钥
	if _, err := verifyMinisign(bin, sig, nil, historical); err == nil {
		t.Fatal("无效签名应报错")
	}
}

// ---- 真实签名夹具：真正能区分回归的用例 ----
//
// 上面的 "garbage" 签名用例只能验证「失败」，无法区分「历史钥是否忽略了过期」——
// 因为垃圾签名在任何情况下都会失败。下面的夹具用例才是核心语义的守卫。
//
// 夹具取自 minisign-verify crate 自带向量，与 Rust 侧 Task 2 使用同一份，
// 保证 Go/Rust 两侧可用同一输入对照。这是【测试专用】密钥，与生产密钥无关，
// 因此生产密钥轮换后本夹具依然稳定有效。
const (
	testPubKey = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3"
	testSig    = "untrusted comment: signature from minisign secret key\n" +
		"RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n" +
		"trusted comment: timestamp:1556193335\tfile:test\n" +
		"y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg=="
	testData           = "test"
	testTrustedComment = "timestamp:1556193335\tfile:test"
)

// writeVerifyFixture 把夹具二进制（内容为 testData）与签名写入临时目录。
func writeVerifyFixture(t *testing.T) (binPath, sigPath string) {
	t.Helper()
	dir := t.TempDir()
	binPath = filepath.Join(dir, "bin")
	sigPath = filepath.Join(dir, "bin.minisig")
	if err := os.WriteFile(binPath, []byte(testData), 0o600); err != nil {
		t.Fatalf("写入夹具二进制失败: %v", err)
	}
	if err := os.WriteFile(sigPath, []byte(testSig), 0o600); err != nil {
		t.Fatalf("写入夹具签名失败: %v", err)
	}
	return binPath, sigPath
}

func TestVerifyMinisignAcceptsValidSignatureWithActiveKey(t *testing.T) {
	bin, sig := writeVerifyFixture(t)
	active := []minisignKeyEntry{{PublicKey: testPubKey, ExpiresAt: "2999-12-31"}}
	info, err := verifyMinisign(bin, sig, active, nil)
	if err != nil {
		t.Fatalf("有效签名在活跃钥下应通过: %v", err)
	}
	if info.TrustedComment != testTrustedComment {
		t.Fatalf("trusted comment 不符: %q", info.TrustedComment)
	}
}

func TestVerifyMinisignFixtureExpiredActiveKeyIsSkipped(t *testing.T) {
	bin, sig := writeVerifyFixture(t)
	// 同一把钥、仍活跃（RetiredAt 为空），但已过期 → 必须被跳过
	active := []minisignKeyEntry{{PublicKey: testPubKey, ExpiresAt: "2000-01-01"}}
	if _, err := verifyMinisign(bin, sig, active, nil); err == nil {
		t.Fatal("过期活跃钥必须被跳过（此处应因无可用钥而失败）")
	}
}

// TestVerifyMinisignHistoricalKeyIgnoresExpiry 是本任务的核心语义守卫。
//
// 实测（2026-09-14）：给历史钥循环注入过期检查后，**仅本用例失败** ——
// brief 里基于垃圾签名的同义用例在任何实现下都通过，无法捕获该回归。
func TestVerifyMinisignHistoricalKeyIgnoresExpiry(t *testing.T) {
	bin, sig := writeVerifyFixture(t)
	historical := []minisignKeyEntry{{
		PublicKey: testPubKey,
		ExpiresAt: "2000-01-01", // 刻意已过期
		RetiredAt: "2026-01-01",
	}}
	info, err := verifyMinisign(bin, sig, nil, historical)
	if err != nil {
		t.Fatalf("历史钥必须忽略过期并完成验证: %v", err)
	}
	if info.TrustedComment != testTrustedComment {
		t.Fatalf("trusted comment 不符: %q", info.TrustedComment)
	}
}

func TestVerifyMinisignRejectsTamperedData(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin")
	sig := filepath.Join(dir, "bin.minisig")
	if err := os.WriteFile(bin, []byte("tampered"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(sig, []byte(testSig), 0o600); err != nil {
		t.Fatal(err)
	}
	active := []minisignKeyEntry{{PublicKey: testPubKey, ExpiresAt: "2999-12-31"}}
	if _, err := verifyMinisign(bin, sig, active, nil); err == nil {
		t.Fatal("被篡改的数据必须被拒绝")
	}
}

func TestVerifyMinisignRejectsWhenNoKeysSupplied(t *testing.T) {
	bin, sig := writeVerifyFixture(t)
	if _, err := verifyMinisign(bin, sig, nil, nil); err == nil {
		t.Fatal("无任何密钥时应失败")
	}
}

// TestVerifyMinisignSkipsRetiredKeyInActiveList 锁定活跃循环里的 isActive() 检查。
//
// Rust 侧活跃循环的条件是 `!is_active(entry) || key_expired(entry.expires_at)`，
// Go 必须镜像一致。实测（2026-09-14）：若从活跃循环移除 `!entry.isActive()`，
// 本用例是唯一会失败的测试 —— 其余用例都发现不了。
func TestVerifyMinisignSkipsRetiredKeyInActiveList(t *testing.T) {
	bin, sig := writeVerifyFixture(t)
	// 该钥能验证夹具，但已标记退役（RetiredAt 非空）。
	// 即便被误放进 active 列表，也必须被 isActive() 检查拦下。
	active := []minisignKeyEntry{{
		PublicKey: testPubKey,
		ExpiresAt: "2999-12-31",
		RetiredAt: "2026-01-01",
	}}
	if _, err := verifyMinisign(bin, sig, active, nil); err == nil {
		t.Fatal("已退役的密钥即便出现在 active 列表中也必须被跳过")
	}
}
