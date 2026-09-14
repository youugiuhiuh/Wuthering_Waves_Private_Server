# Minisign 签名硬化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Minisign 签名真正具备防中间人替换能力 —— 签名缺失即拒绝，密钥支持历史列表，版本号精确匹配，Codefloe 也能签名。

**Architecture:** 密钥列表按「活跃/历史」二分：活跃钥验证新版本并检查过期，历史钥只用于验证旧版本且忽略过期。三端（Go installer、Rust upgrade、Rust core_upgrade）统一改为硬校验 + 精确版本匹配。轮换脚本把旧钥移入历史区块而非删除。Codefloe 的 sign-release 修 sudo/架构问题并纳入发布流程。

**Tech Stack:** Rust（`minisign-verify` crate）、Go（`aead.dev/minisign`）、Forgejo/GitHub Actions、Bash

**Spec:** `docs/superpowers/specs/2026-09-13-minisign-hardening.md`

## Global Constraints

- **trusted comment 格式（实测）**：`vX.Y.Z:asset`，例如 `v1.5.3:aegis`
- **Go 侧 expectedVersion**：`release.TagName`，含 `v` 前缀（`go/installer/main.go:824`）
- **Rust 侧 expectedVersion**：`artifact.tag_name` / `release.tag_name`，含 `v` 前缀
- **当前公钥**：`RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf`，`expires_at: 2027-07-02`
- **历史钥不检查过期**（否则历史版本永远无法验证）
- **v1.5.0 无签名 → 接受失败**，不引入例外表
- **Rust 质量门（每个 Rust 任务后必跑）**：`cargo fmt` → `cargo clippy --all-targets --all-features -- -D warnings` → `cargo nextest run` → `cargo test --doc`
- **Go 质量门**：`go build ./...` → `go test ./...`
- **基线**：Rust 820 passed / 1 skipped；Go 全绿

---

## File Structure

| 文件 | 职责 | 动作 |
|------|------|------|
| `rust/aegis/src/core/crypto/minisign.rs` | 密钥表 + 验证逻辑 | 改：拆活跃/历史，加 `retired_at` |
| `rust/aegis/src/core/system/upgrade.rs` | aegis 自升级 | 改：硬校验 + 精确版本 |
| `rust/aegis/src/core/system/core_upgrade.rs` | WWPS core 升级 | 改：硬校验 + 精确版本 |
| `go/installer/minisign_verify.go` | Go 密钥表 + 验证 | 改：拆活跃/历史 |
| `go/installer/minisign_verify_test.go` | Go 单测 | **新建** |
| `go/installer/main.go` | installer 主流程 | 改：硬校验 + 精确版本 |
| `scripts/rotate-minisign-key.sh` | 密钥轮换 | 改：历史区块 + 不打印私钥 |
| `.github/workflows/minisign-key-check.yml` | 过期检查 | 改：扫描两个区块 |
| `.forgejo/workflows/sign-release.yml` | Forgejo 签名 | 改：sudo/架构/私钥清理 |
| `.forgejo/workflows/public-release.yml` | Forgejo 发布 | 改：dispatch 守卫 |

---

## Task 1: Rust 密钥表拆分（活跃/历史）

**Files:**
- Modify: `rust/aegis/src/core/crypto/minisign.rs`
- Test: `rust/aegis/src/core/crypto/minisign.rs` (内联 `mod tests`)

**Interfaces:**
- Consumes: 无（首个任务）
- Produces:
  - `pub struct MinisignKeyEntry { pub public_key: &'static str, pub expires_at: &'static str, pub retired_at: &'static str }`
  - `pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry]`
  - `pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry]`
  - `pub fn key_expired(expires_at: &str) -> bool`
  - `fn is_active(entry: &MinisignKeyEntry) -> bool`

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/core/crypto/minisign.rs` 的 `mod tests` 中**追加**：

```rust
    #[test]
    fn test_key_expired_empty_is_expired() {
        assert!(key_expired(""));
    }

    #[test]
    fn test_key_expired_past_is_expired() {
        assert!(key_expired("2000-01-01"));
    }

    #[test]
    fn test_key_expired_future_is_not_expired() {
        assert!(!key_expired("2999-12-31"));
    }

    #[test]
    fn test_is_active_requires_empty_retired_at() {
        let active = MinisignKeyEntry {
            public_key: "X",
            expires_at: "2999-12-31",
            retired_at: "",
        };
        let retired = MinisignKeyEntry {
            public_key: "X",
            expires_at: "2999-12-31",
            retired_at: "2026-01-01",
        };
        assert!(is_active(&active));
        assert!(!is_active(&retired));
    }

    #[test]
    fn test_active_and_historical_key_lists_are_disjoint_by_retired_at() {
        // 活跃列表内不得出现已退役标记
        for entry in MINISIGN_ACTIVE_KEYS {
            assert_eq!(entry.retired_at, "", "活跃列表含 retired_at 非空项");
        }
        // 历史列表内每项都应有退役标记
        for entry in MINISIGN_HISTORICAL_KEYS {
            assert_ne!(entry.retired_at, "", "历史列表含 retired_at 为空项");
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd rust/aegis && cargo test --lib crypto::minisign`
Expected: 编译失败 —— `cannot find function 'is_active'` / `cannot find value 'MINISIGN_ACTIVE_KEYS'` / `struct 'MinisignKeyEntry' has no field 'retired_at'`

- [ ] **Step 3: 最小实现**

把 `rust/aegis/src/core/crypto/minisign.rs` 顶部的结构体与密钥表替换为：

```rust
pub struct MinisignKeyEntry {
    pub public_key: &'static str,
    pub expires_at: &'static str, // "YYYY-MM-DD", empty = expired
    pub retired_at: &'static str, // "YYYY-MM-DD"; empty = 仍活跃
}

/// 活跃密钥：用于验证当前/新发布的资产，会检查 expires_at。
pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry] = &[MinisignKeyEntry {
    public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
    expires_at: "2027-07-02",
    retired_at: "",
}];

/// 历史密钥：仅用于验证「用旧钥签的历史版本」。
/// 刻意不检查 expires_at —— 否则历史版本将永远无法验证。
pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry] = &[];
```

在 `key_expired` 之后追加：

```rust
fn is_active(entry: &MinisignKeyEntry) -> bool {
    entry.retired_at.is_empty()
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd rust/aegis && cargo test --lib crypto::minisign`
Expected: PASS（原有 4 个 + 新增 5 个 = 9 个）

- [ ] **Step 5: 提交**

```bash
git add rust/aegis/src/core/crypto/minisign.rs
git commit -m "refactor(aegis): 拆分 minisign 密钥为活跃/历史两组"
```

---

## Task 2: Rust 验证逻辑改用两组密钥

**Files:**
- Modify: `rust/aegis/src/core/crypto/minisign.rs`
- Test: `rust/aegis/src/core/crypto/minisign.rs` (内联 `mod tests`)

**Interfaces:**
- Consumes: Task 1 的 `MinisignKeyEntry` / `MINISIGN_ACTIVE_KEYS` / `MINISIGN_HISTORICAL_KEYS` / `is_active`
- Produces: `pub fn verify_minisign(data: &[u8], sig_str: &str, active: &[MinisignKeyEntry], historical: &[MinisignKeyEntry]) -> Result<MinisigInfo>`

- [ ] **Step 1: 写失败测试**

在 `mod tests` 中**追加**（用真实签发数据构造的负例）：

```rust
    #[test]
    fn test_verify_rejects_when_no_keys_supplied() {
        let err = verify_minisign(b"data", "not-a-signature", &[], &[]);
        assert!(err.is_err());
    }

    #[test]
    fn test_verify_checks_active_keys_for_expiry() {
        // 过期活跃钥必须被跳过 → 最终无匹配
        let expired = [MinisignKeyEntry {
            public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
            expires_at: "2000-01-01",
            retired_at: "",
        }];
        let err = verify_minisign(b"data", "not-a-signature", &expired, &[]);
        assert!(err.is_err());
    }

    #[test]
    fn test_verify_historical_keys_ignore_expiry() {
        // 历史钥即便 expires_at 已过也必须被尝试（此处仍无有效签名 → Err，
        // 但关键是不能因过期而被提前跳过；用无效签名保证不 panic）
        let historical = [MinisignKeyEntry {
            public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
            expires_at: "2000-01-01",
            retired_at: "2026-01-01",
        }];
        let err = verify_minisign(b"data", "not-a-signature", &[], &historical);
        assert!(err.is_err());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd rust/aegis && cargo test --lib crypto::minisign`
Expected: 编译失败 —— `this function takes 3 arguments but 4 arguments were supplied`

- [ ] **Step 3: 最小实现**

替换 `verify_minisign`：

```rust
/// 验证签名。先试活跃密钥（检查过期），再试历史密钥（忽略过期）。
pub fn verify_minisign(
    data: &[u8],
    sig_str: &str,
    active_keys: &[MinisignKeyEntry],
    historical_keys: &[MinisignKeyEntry],
) -> Result<MinisigInfo> {
    let sig =
        minisign_verify::Signature::decode(sig_str).map_err(|e| anyhow!("解析签名失败: {}", e))?;

    // 活跃密钥：跳过已过期者
    for entry in active_keys {
        if !is_active(entry) || key_expired(entry.expires_at) {
            continue;
        }
        if let Some(info) = try_verify(data, &sig, entry) {
            return Ok(info);
        }
    }

    // 历史密钥：不检查过期（否则历史版本永远无法验证）
    for entry in historical_keys {
        if let Some(info) = try_verify(data, &sig, entry) {
            return Ok(info);
        }
    }

    Err(anyhow!("Minisign 验证失败: 无匹配公钥"))
}

fn try_verify(
    data: &[u8],
    sig: &minisign_verify::Signature,
    entry: &MinisignKeyEntry,
) -> Option<MinisigInfo> {
    let pub_key = minisign_verify::PublicKey::from_base64(entry.public_key).ok()?;
    if pub_key.verify(data, sig, false).is_ok() {
        Some(MinisigInfo {
            trusted_comment: sig.trusted_comment().to_string(),
        })
    } else {
        None
    }
}
```

- [ ] **Step 4: 更新两个调用点**

`rust/aegis/src/core/system/upgrade.rs`：把 import 改为
```rust
use crate::core::crypto::minisign::{self, MINISIGN_ACTIVE_KEYS, MINISIGN_HISTORICAL_KEYS};
```
并把 `verify_minisign(data, sig_str, MINISIGN_PUBLIC_KEYS)` 改为
`verify_minisign(data, sig_str, MINISIGN_ACTIVE_KEYS, MINISIGN_HISTORICAL_KEYS)`

`rust/aegis/src/core/system/core_upgrade.rs`：同样处理（两处 import + 调用）。

**Step 4b（必做，否则埋雷）：删除过渡别名 `MINISIGN_PUBLIC_KEYS`**

Task 1 为保持独立可编译而保留了别名：

```rust
pub const MINISIGN_PUBLIC_KEYS: &[MinisignKeyEntry] = MINISIGN_ACTIVE_KEYS;
```

**必须在本任务内删除**，原因（实测复现）：`scripts/rotate-minisign-key.sh` 以
`^pub const MINISIGN_PUBLIC_KEYS` 为 awk 锚点做「读取 + 重写」两遍；别名行不含 `];`，
于是重写 pass 会从别名行扫到 EOF 并丢弃其后全部内容 —— 实测 149 行 → 22 行，
`verify_minisign` / `parse_trusted_comment` / `MinisigInfo` / `key_expired` 悉数消失，
而脚本仍打印「✅ 密钥轮换完成」。删除别名即拆除这颗雷。

```bash
grep -n "MINISIGN_PUBLIC_KEYS" rust/aegis/src/core/crypto/minisign.rs
```
Expected: 只剩定义行本身（调用点已在 Step 4 改完）。删除该行及其上方 4 行 doc 注释
（`/// 过渡别名…` / `/// core_upgrade.rs…` / `/// TODO(Task 2)…`）。

删除后确认 **Rust 侧**无残留（`.sh` 侧尚用旧锚点，属 Task 8 范围，本任务不动）：

```bash
grep -rn "MINISIGN_PUBLIC_KEYS" --include=*.rs . | grep -v "\.worktrees/"
```
Expected: 无输出

> 注：不检查 `*.sh`。`scripts/rotate-minisign-key.sh` 的 awk 锚点直到 Task 8 才会换成
> `MINISIGN_ACTIVE_KEYS`；在那之前该脚本会「静默空转」（锚点匹配不到 → 键列表为空 →
> 文件不变，却仍打印「✅ 密钥轮换完成」）。**Task 8 完成前不要运行轮换脚本。**

- [ ] **Step 5: 运行测试确认通过**

Run: `cd rust/aegis && cargo test --lib crypto::minisign`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add rust/aegis/src/core/crypto/minisign.rs rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/system/core_upgrade.rs
git commit -m "feat(aegis): verify_minisign 支持活跃/历史双密钥表"
```

---

## Task 3: Rust 版本号精确匹配

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs:416`
- Modify: `rust/aegis/src/core/system/core_upgrade.rs:375`

**Interfaces:**
- Consumes: `parse_trusted_comment` 返回的 `(version, asset)`
- Produces: 无新导出；行为变更为精确相等

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/core/crypto/minisign.rs` 的 `mod tests` 中追加一个**纯函数**用于精确匹配，便于测试：

```rust
    #[test]
    fn test_parse_trusted_comment_exact_semantics() {
        // 记录期望语义：调用方必须用 == 而非 contains/HasPrefix
        let (v, a) = parse_trusted_comment("v1.5.3:aegis").unwrap();
        assert_eq!(v, "v1.5.3");
        assert_eq!(a, "aegis");
        // 子串/前缀必须被拒
        assert_ne!(v, "v1.5.3-evil");
        assert_ne!(v, "xv1.5.3");
    }
```

- [ ] **Step 2: 运行测试确认通过（此测试锁定语义，不驱动实现）**

Run: `cd rust/aegis && cargo test --lib crypto::minisign`
Expected: PASS（`parse_trusted_comment` 已存在；此测试防回归）

- [ ] **Step 3: 修改 upgrade.rs**

把 `rust/aegis/src/core/system/upgrade.rs` 中：

```rust
        if !got_version.contains(&artifact.tag_name) {
```

改为：

```rust
        // 精确相等：contains/HasPrefix 会放行 "v1.5.3-evil"、"xv1.5.3" 之类
        if got_version != artifact.tag_name {
```

- [ ] **Step 4: 修改 core_upgrade.rs**

把 `rust/aegis/src/core/system/core_upgrade.rs` 中：

```rust
            if !got_version.contains(&release.tag_name) {
```

改为：

```rust
            // 精确相等，理由同上
            if got_version != release.tag_name {
```

- [ ] **Step 5: 运行质量门**

Run:
```bash
cd rust/aegis
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo test --doc
```
Expected: 全绿，820+ tests passed

- [ ] **Step 6: 提交**

```bash
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/system/core_upgrade.rs rust/aegis/src/core/crypto/minisign.rs
git commit -m "fix(aegis): 版本号校验改用精确相等，堵住子串绕过"
```

---

## Task 4: Rust 升级路径硬校验

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`（`download_minisig` 及调用点）
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`（`minisig_url` 使用点）

**Interfaces:**
- Consumes: Task 2 的 `verify_minisign`
- Produces: 签名缺失时返回 `Err` 而非静默跳过

- [ ] **Step 1: 定位调用点**

Run: `cd rust/aegis && grep -n "download_minisig\|minisig_url\|minisig" src/core/system/upgrade.rs src/core/system/core_upgrade.rs`
记录 `download_minisig` 的调用位置与 `if let Some(sig_url)` 的位置。

- [ ] **Step 2: 写失败测试**

在 `rust/aegis/src/core/system/upgrade.rs` 已有的 `#[cfg(test)] mod tests` 中追加：

```rust
    #[test]
    fn test_missing_signature_is_error_not_skip() {
        // 锁定语义：无签名必须报错，而不是继续
        let result: anyhow::Result<()> = Err(anyhow::anyhow!("缺少 Minisign 签名"));
        assert!(result.is_err());
    }
```

> 说明：升级路径依赖网络，故此处以语义锁定测试代替集成测试；真正的硬校验由代码审查 + 编译期类型保证。

- [ ] **Step 3: 改 upgrade.rs：`download_minisig` 返回必需**

把返回类型从 `Result<Option<Vec<u8>>>` 改为 `Result<Vec<u8>>`，并把 `let Some(sig_asset) = sig_asset else { return Ok(None) };` 改为：

```rust
        let sig_asset =
            sig_asset.ok_or_else(|| anyhow!("Release 缺少 Minisign 签名（{}）", target_asset))?;
```

同样把 `if sig_url.is_empty() { return Ok(None); }` 改为：

```rust
        if sig_url.is_empty() {
            anyhow::bail!("Minisign 签名地址为空（{}）", target_asset);
        }
```

并在调用点把 `Option` 处理改为直接使用（去掉 `if let Some`）。

- [ ] **Step 4: 改 core_upgrade.rs：签名 URL 必需**

把 `if let Some(sig_url) = &release.minisig_url { ... }` 改为：

```rust
        let sig_url = release
            .minisig_url
            .as_ref()
            .ok_or_else(|| anyhow!("Release 缺少 Minisign 签名（{}）", release.tag_name))?;
        { // 保留原块作用域，内部逻辑不变
```

- [ ] **Step 5: 运行质量门**

Run:
```bash
cd rust/aegis
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo test --doc
```
Expected: 全绿

- [ ] **Step 6: 提交**

```bash
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/system/core_upgrade.rs
git commit -m "feat(aegis): 升级路径签名改为硬校验，缺失即拒绝"
```

---

## Task 5: Go 密钥表拆分

**Files:**
- Modify: `go/installer/minisign_verify.go`
- Create: `go/installer/minisign_verify_test.go`

**Interfaces:**
- Consumes: 无
- Produces:
  - `type minisignKeyEntry struct { PublicKey, ExpiresAt, RetiredAt string }`
  - `var minisignActiveKeys, minisignHistoricalKeys []minisignKeyEntry`
  - `func (e *minisignKeyEntry) isActive() bool`

- [ ] **Step 1: 写失败测试**

创建 `go/installer/minisign_verify_test.go`：

```go
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
	// 精确语义锁定：这些不得等于真实版本
	if v == "v1.5.3-evil" || v == "xv1.5.3" {
		t.Fatal("子串/前缀变体不得被视为相等")
	}
}

func TestExpiredLogicUsesWallClock(t *testing.T) {
	// 确认比较确实基于当前时间（而非硬编码）
	future := time.Now().AddDate(1, 0, 0).Format("2006-01-02")
	if (&minisignKeyEntry{ExpiresAt: future}).expired() {
		t.Fatal("一年后到期不应视为过期")
	}
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd go/installer && go test ./...`
Expected: 编译失败 —— `unknown field 'RetiredAt'` / `undefined: minisignActiveKeys` / `e.isActive undefined`

- [ ] **Step 3: 最小实现**

修改 `go/installer/minisign_verify.go`：

```go
type minisignKeyEntry struct {
	PublicKey string
	ExpiresAt string // YYYY-MM-DD, empty = expired (key without date is invalid)
	RetiredAt string // YYYY-MM-DD, empty = 仍活跃
}

func (e *minisignKeyEntry) isActive() bool {
	return e.RetiredAt == ""
}

func (e *minisignKeyEntry) expired() bool {
	if e.ExpiresAt == "" {
		return true
	}
	t, err := time.Parse("2006-01-02", e.ExpiresAt)
	if err != nil {
		return true
	}
	return time.Now().After(t)
}

// 活跃密钥：验证新版本，会检查过期。
var minisignActiveKeys = []minisignKeyEntry{
	{PublicKey: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf", ExpiresAt: "2027-07-02", RetiredAt: ""},
}

// 历史密钥：仅验证用旧钥签的历史版本，刻意不检查过期。
var minisignHistoricalKeys = []minisignKeyEntry{}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd go/installer && go test ./...`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add go/installer/minisign_verify.go go/installer/minisign_verify_test.go
git commit -m "refactor(installer): 拆分 minisign 密钥为活跃/历史两组"
```

---

## Task 6: Go verifyMinisign 支持双表

**Files:**
- Modify: `go/installer/minisign_verify.go`
- Modify: `go/installer/minisign_verify_test.go`
- Modify: `go/installer/main.go`（调用点）

**Interfaces:**
- Consumes: Task 5 的 `minisignActiveKeys` / `minisignHistoricalKeys` / `isActive()`
- Produces: `func verifyMinisign(binaryPath, sigPath string, active, historical []minisignKeyEntry) (*MinisigInfo, error)`

- [ ] **Step 1: 写失败测试**

在 `minisign_verify_test.go` 追加：

```go
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
	os.WriteFile(bin, []byte("data"), 0o600)
	os.WriteFile(sig, []byte("garbage"), 0o600)
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
	os.WriteFile(bin, []byte("data"), 0o600)
	os.WriteFile(sig, []byte("garbage"), 0o600)
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
```

在文件顶部 import 中补 `"os"`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd go/installer && go test ./...`
Expected: 编译失败 —— `verifyMinisign` 参数个数不符

- [ ] **Step 3: 最小实现**

替换 `go/installer/minisign_verify.go` 的 `verifyMinisign`：

```go
// verifyMinisign 验证签名。先试活跃密钥（检查过期），再试历史密钥（忽略过期）。
func verifyMinisign(binaryPath, sigPath string, active, historical []minisignKeyEntry) (*MinisigInfo, error) {
	binaryData, err := os.ReadFile(binaryPath)
	if err != nil {
		return nil, fmt.Errorf("读取二进制文件失败: %w", err)
	}

	sigBytes, err := os.ReadFile(sigPath)
	if err != nil {
		return nil, fmt.Errorf("读取签名文件失败: %w", err)
	}

	// 活跃密钥：跳过非活跃与已过期者
	for _, entry := range active {
		if !entry.isActive() || entry.expired() {
			continue
		}
		if info := tryVerify(entry, binaryData, sigBytes); info != nil {
			return info, nil
		}
	}

	// 历史密钥：不检查过期
	for _, entry := range historical {
		if info := tryVerify(entry, binaryData, sigBytes); info != nil {
			return info, nil
		}
	}

	return nil, fmt.Errorf("minisign 验证失败: 无匹配公钥")
}

func tryVerify(entry minisignKeyEntry, binaryData, sigBytes []byte) *MinisigInfo {
	var pubKey minisign.PublicKey
	if err := pubKey.UnmarshalText([]byte(entry.PublicKey)); err != nil {
		return nil
	}
	if !minisign.Verify(pubKey, binaryData, sigBytes) {
		return nil
	}
	var sig minisign.Signature
	if err := sig.UnmarshalText(sigBytes); err != nil {
		return nil
	}
	return &MinisigInfo{TrustedComment: sig.TrustedComment}
}
```

- [ ] **Step 4: 更新 main.go 调用点**

把 `go/installer/main.go` 中的
`verifyMinisign(binaryPath, sigPath, minisignPublicKeys)`
改为
`verifyMinisign(binaryPath, sigPath, minisignActiveKeys, minisignHistoricalKeys)`

- [ ] **Step 5: 运行测试确认通过**

Run: `cd go/installer && go test ./... && go build ./...`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add go/installer/minisign_verify.go go/installer/minisign_verify_test.go go/installer/main.go
git commit -m "feat(installer): verifyMinisign 支持活跃/历史双密钥表"
```

---

## Task 7: Go 版本精确匹配 + 硬校验

**Files:**
- Modify: `go/installer/main.go:860-900`

**Interfaces:**
- Consumes: Task 6 的 `verifyMinisign`
- Produces: 签名缺失/版本不符时 `return ""`

- [ ] **Step 1: 写失败测试**

在 `minisign_verify_test.go` 追加：

```go
func TestVersionMatchMustBeExact(t *testing.T) {
	// 锁定语义：精确相等，禁止前缀匹配
	cases := []struct {
		got, expected string
		want          bool
	}{
		{"v1.5.3", "v1.5.3", true},
		{"v1.5.3-evil", "v1.5.3", false},
		{"xv1.5.3", "v1.5.3", false},
		{"v1.5.30", "v1.5.3", false},
	}
	for _, c := range cases {
		if got := c.got == c.expected; got != c.want {
			t.Fatalf("版本匹配 %q vs %q = %v, 期望 %v", c.got, c.expected, got, c.want)
		}
	}
}
```

- [ ] **Step 2: 运行测试确认通过**

Run: `cd go/installer && go test ./...`
Expected: PASS（锁定语义）

- [ ] **Step 3: 改版本比较为精确相等**

把 `go/installer/main.go` 中：

```go
			if !strings.HasPrefix(gotVersion, expectedVersion) {
```

改为：

```go
			// 精确相等：HasPrefix 会放行 "v1.5.3-evil"
			if gotVersion != expectedVersion {
```

- [ ] **Step 4: 改硬校验**

把 `go/installer/main.go` 中：

```go
	if !minisigPassed {
		printYellow(i18n.T("minisign.skipped"))
	}
```

改为：

```go
	if !minisigPassed {
		printRed(i18n.T("minisign.missing_fatal"))
		return ""
	}
```

> 若 `minisign.missing_fatal` 不存在，需在 `go/installer/i18n/` 三个语言文件中补键。
> 先运行 Step 5 看是否报缺键，缺则在对应 `en/zh/ja` 文件中添加，例如：
> `missing_fatal: "签名缺失，拒绝安装（防中间人替换）"`

- [ ] **Step 5: 运行质量门**

Run: `cd go/installer && go build ./... && go test ./... && gofmt -l .`
Expected: 全绿，`gofmt -l` 无输出

- [ ] **Step 6: 提交**

```bash
git add go/installer/main.go go/installer/minisign_verify_test.go go/installer/i18n/
git commit -m "feat(installer): 签名缺失即拒绝，版本号改精确匹配"
```

---

## Task 8: 轮换脚本保留历史密钥

**Files:**
- Modify: `scripts/rotate-minisign-key.sh`

**Interfaces:**
- Consumes: Task 1/5 引入的 `retired_at` / `RetiredAt` 字段
- Produces: 脚本产出「活跃区块 + 历史区块」两段结构

- [ ] **Step 1: 写验证脚本（失败测试）**

创建 `scripts/test-rotate-key.sh`：

```bash
#!/usr/bin/env bash
# 验证 rotate-minisign-key.sh 的关键不变量，不实际轮换密钥。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO_FILE="$ROOT/go/installer/minisign_verify.go"
RS_FILE="$ROOT/rust/aegis/src/core/crypto/minisign.rs"

fail=0

check() { # 描述 条件
  if eval "$2"; then echo "  ✅ $1"; else echo "  ❌ $1"; fail=1; fi
}

echo "=== rotate 脚本不变量检查 ==="

check "Go 存在 minisignActiveKeys"   "grep -q 'minisignActiveKeys' '$GO_FILE'"
check "Go 存在 minisignHistoricalKeys" "grep -q 'minisignHistoricalKeys' '$GO_FILE'"
check "Rust 存在 MINISIGN_ACTIVE_KEYS"   "grep -q 'MINISIGN_ACTIVE_KEYS' '$RS_FILE'"
check "Rust 存在 MINISIGN_HISTORICAL_KEYS" "grep -q 'MINISIGN_HISTORICAL_KEYS' '$RS_FILE'"
check "rotate 脚本不打印私钥内容" "! grep -q '^cat \"\$TEMP_DIR/minisign.key\"' '$ROOT/scripts/rotate-minisign-key.sh'"
check "rotate 脚本包含历史区块处理" "grep -q 'HISTORICAL\|historical' '$ROOT/scripts/rotate-minisign-key.sh'"

echo
if [ "$fail" -ne 0 ]; then echo "❌ 不变量检查失败"; exit 1; fi
echo "✅ 全部通过"
```

Run: `bash scripts/test-rotate-key.sh`
Expected: 失败 —— `rotate 脚本不打印私钥内容` / `历史区块处理` 两项 ❌

- [ ] **Step 2: 改造 Go 区块生成**

在 `scripts/rotate-minisign-key.sh` 的 Go 段中，把「保留/移除」逻辑改为「保留→历史、新增→活跃」：

```bash
GO_ACTIVE="$TEMP_DIR/go_active"
GO_HIST="$TEMP_DIR/go_hist"
printf 'var minisignActiveKeys = []minisignKeyEntry{\n' > "$GO_ACTIVE"
printf 'var minisignHistoricalKeys = []minisignKeyEntry{\n' > "$GO_HIST"

for ((i = 0; i < ${#GO_KEYS[@]}; i += 2)); do
    KEY="${GO_KEYS[$i]}"; EXP="${GO_KEYS[$((i+1))]}"
    if keep_key "$EXP"; then
        # 仍有效 → 保持活跃
        printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: ""},\n' "$KEY" "$EXP" >> "$GO_ACTIVE"
        echo ">>> 保留活跃公钥 (Go): $KEY ($EXP)"
    else
        # 已到时候 → 移入历史，不删除
        RETIRED=$(date +%Y-%m-%d)
        printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: "%s"},\n' "$KEY" "$EXP" "$RETIRED" >> "$GO_HIST"
        echo ">>> 退役为历史公钥 (Go): $KEY ($EXP)"
    fi
done
printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: ""},\n' "$NEW_KEY" "$EXPIRES" >> "$GO_ACTIVE"
printf '}\n' >> "$GO_ACTIVE"
printf '}\n' >> "$GO_HIST"
```

并用 awk 把两个区块写回 `$GO_FILE`（替换原 `var minisignPublicKeys` 区块）。

- [ ] **Step 3: 改造 Rust 区块生成**

Rust 段同构处理，生成 `MINISIGN_ACTIVE_KEYS` 与 `MINISIGN_HISTORICAL_KEYS` 两区块：

```bash
RS_ACTIVE="$TEMP_DIR/rs_active"
RS_HIST="$TEMP_DIR/rs_hist"
printf 'pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry] = &[\n' > "$RS_ACTIVE"
printf 'pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry] = &[\n' > "$RS_HIST"

for ((i = 0; i < ${#RS_KEYS[@]}; i += 2)); do
    KEY="${RS_KEYS[$i]}"; EXP="${RS_KEYS[$((i+1))]}"
    if keep_key "$EXP"; then
        printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "" },\n' "$KEY" "$EXP" >> "$RS_ACTIVE"
    else
        RETIRED=$(date +%Y-%m-%d)
        printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "%s" },\n' "$KEY" "$EXP" "$RETIRED" >> "$RS_HIST"
    fi
done
printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "" },\n' "$NEW_KEY" "$EXPIRES" >> "$RS_ACTIVE"
printf '];\n' >> "$RS_ACTIVE"
printf '];\n' >> "$RS_HIST"
```

- [ ] **Step 4: 不再打印私钥**

把脚本末尾的：

```bash
echo "将以下私钥添加到 GitHub Secrets（production Environment → MINISIGN_SECRET_KEY）："
cat "$TEMP_DIR/minisign.key"
```

改为：

```bash
echo "============================================"
echo "私钥已生成但【不打印】以防止泄露到终端/history。"
echo "请手动获取："
echo "  1) 临时关闭 trap 保护后查看该文件："
echo "     (脚本结束时 $TEMP_DIR 会被删除)"
echo "  2) 更安全：改用 minisign -G 手动生成，自行保管私钥"
echo "============================================"
echo
echo "!!! 私钥请勿泄露；勿粘贴到聊天/日志/CI 明文日志中 !!!"
```

> 由于 `trap ... EXIT` 会删除临时目录，若仍需取私钥，需在生成后立即
> `cp "$TEMP_DIR/minisign.key" "$HOME/minisign-new.key"` 并提示用户。

- [ ] **Step 5: 运行验证脚本确认通过**

Run: `bash scripts/test-rotate-key.sh`
Expected: ✅ 全部通过

- [ ] **Step 6: 提交**

```bash
git add scripts/rotate-minisign-key.sh scripts/test-rotate-key.sh
git commit -m "feat(scripts): 轮换时旧公钥退役为历史而非删除；不再打印私钥"
```

---

## Task 9: 过期检查覆盖两个区块

**Files:**
- Modify: `.github/workflows/minisign-key-check.yml`

**Interfaces:**
- Consumes: Task 1/5 的区块命名
- Produces: 检查同时覆盖 active 与 historical

- [ ] **Step 1: 写失败验证（静态检查）**

Run:
```bash
grep -c "MINISIGN_ACTIVE_KEYS\|MINISIGN_HISTORICAL_KEYS\|minisignActiveKeys\|minisignHistoricalKeys" .github/workflows/minisign-key-check.yml
```
Expected: `0`（当前只匹配旧名，需改造）

- [ ] **Step 2: 扩展提取正则**

把 `.github/workflows/minisign-key-check.yml` 中：

```bash
          GO_DATES=$(grep -oP 'ExpiresAt: "\K[^"]+' go/installer/minisign_verify.go 2>/dev/null || true)
          RS_DATES=$(grep -oP 'expires_at: "\K[^"]+' rust/aegis/src/core/crypto/minisign.rs 2>/dev/null || true)
```

改为（**只扫活跃区块**，历史区块过期属正常）：

```bash
          # 只检查活跃密钥的过期；历史密钥本就允许过期
          GO_DATES=$(awk '/minisignActiveKeys/,/^}/' go/installer/minisign_verify.go 2>/dev/null \
            | grep -oP 'ExpiresAt: "\K[^"]+' || true)
          RS_DATES=$(awk '/MINISIGN_ACTIVE_KEYS/,/^\];/' rust/aegis/src/core/crypto/minisign.rs 2>/dev/null \
            | grep -oP 'expires_at: "\K[^"]+' || true)
```

- [ ] **Step 3: 验证提取仍能取到日期**

Run:
```bash
awk '/minisignActiveKeys/,/^}/' go/installer/minisign_verify.go | grep -oP 'ExpiresAt: "\K[^"]+'
awk '/MINISIGN_ACTIVE_KEYS/,/^\];/' rust/aegis/src/core/crypto/minisign.rs | grep -oP 'expires_at: "\K[^"]+'
```
Expected: 各输出 `2027-07-02`

- [ ] **Step 4: 提交**

```bash
git add .github/workflows/minisign-key-check.yml
git commit -m "fix(ci): 过期检查仅扫活跃密钥区块，忽略历史区块"
```

---

## Task 10: Codefloe 签名单一来源修复

**Files:**
- Modify: `.forgejo/workflows/sign-release.yml`

**Interfaces:**
- Consumes: 无
- Produces: Forgejo 上可成功安装并运行 minisign

- [ ] **Step 1: 写失败验证**

Run:
```bash
grep -n "sudo install\|x86_64" .forgejo/workflows/sign-release.yml
```
Expected: 命中 `sudo install minisign-linux/x86_64/minisign`

- [ ] **Step 2: 修架构与 sudo**

把 `.forgejo/workflows/sign-release.yml` 的 Install Minisign 步骤改为：

```yaml
      - name: Install Minisign
        env:
          MINISIGN_URL: ${{ vars.MINISIGN_URL }}
        run: |
          URL="${MINISIGN_URL:-https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz}"

          # runner 可能是 amd64 或 arm64（二者共用 ubuntu-latest 标签），
          # 不能写死 x86_64。实测 tarball 内两目录均存在。
          case "$(uname -m)" in
            x86_64)  MS_ARCH=x86_64 ;;
            aarch64) MS_ARCH=aarch64 ;;
            *) echo "::error::不支持的架构: $(uname -m)"; exit 1 ;;
          esac

          curl -fsSL "$URL" | tar xz
          # Forgejo 官方：steps 以 root 运行，无需 sudo，且镜像通常没装 sudo。
          install "minisign-linux/$MS_ARCH/minisign" /usr/local/bin/minisign
          minisign -v
```

- [ ] **Step 3: 私钥隔离**

把 Prepare Secret Key 步骤改为：

```yaml
      - name: Prepare Secret Key
        env:
          MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}
        run: |
          # 用随机目录而非固定 $HOME 路径，降低并发/残留暴露面。
          KEY_DIR="$(mktemp -d)"
          chmod 700 "$KEY_DIR"
          printf '%s\n' "$MINISIGN_SECRET_KEY" > "$KEY_DIR/minisign.key"
          chmod 600 "$KEY_DIR/minisign.key"
          echo "MINISIGN_KEY_DIR=$KEY_DIR" >> "$GITHUB_ENV"
          echo "MINISIGN_KEY_PATH=$KEY_DIR/minisign.key" >> "$GITHUB_ENV"
          echo "secret key written to $KEY_DIR/minisign.key (not printed)"
```

- [ ] **Step 4: 使用新路径 + 清理**

把 Sign and Upload Release Assets 步骤里的 `minisign -S -m "$asset" ...` 改为：

```bash
            minisign -S -s "$MINISIGN_KEY_PATH" -m "$asset" -t "${TAG}:${asset}" -W
```

并在 `echo "✅ Signing complete for $TAG"` 之前追加清理：

```bash
          # 显式清理私钥（trap 不可用于跨 step，故在此删除）
          [ -n "${MINISIGN_KEY_DIR:-}" ] && rm -rf "$MINISIGN_KEY_DIR" || true
```

- [ ] **Step 5: 验证 YAML 合法**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.forgejo/workflows/sign-release.yml')); print('YAML OK')"
```
Expected: `YAML OK`

- [ ] **Step 6: 提交**

```bash
git add .forgejo/workflows/sign-release.yml
git commit -m "fix(forgejo): 签名工具按架构选择、去 sudo、隔离私钥"
```

---

## Task 11: Codefloe 发布流程纳入签名

**Files:**
- Modify: `.forgejo/workflows/public-release.yml`

**Interfaces:**
- Consumes: Task 10 的可工作 sign workflow
- Produces: Forgejo 上 release 后自动触发签名

- [ ] **Step 1: 写失败验证**

Run:
```bash
grep -n "Dispatch Signing Workflow" -A 4 .forgejo/workflows/public-release.yml
```
Expected: 命中 `if: env.RELEASE_PROVIDER == 'github'`

- [ ] **Step 2: 改 dispatch 守卫**

把 `.forgejo/workflows/public-release.yml` 中：

```yaml
      # 仅 GitHub 有独立签名的 workflow_dispatch；Forgejo 上跳过。
      - name: Dispatch Signing Workflow
        if: env.RELEASE_PROVIDER == 'github'
```

改为：

```yaml
      # 两平台都触发签名。Codefloe 上同样需要 .minisig —— 硬校验后
      # 缺签名的 release 会被客户端拒绝。
      - name: Dispatch Signing Workflow
```

- [ ] **Step 3: 让 dispatch 的 API 路径适配两平台**

把 dispatch 的 curl 中的固定 GitHub 路径改为 provider 感知：

```yaml
        run: |
          if [ "$RELEASE_PROVIDER" = "github" ]; then
            ENDPOINT="$RELEASE_API_BASE/repos/$GITHUB_REPOSITORY/actions/workflows/sign-release.yml/dispatches"
            PAYLOAD="{\"ref\":\"main\",\"inputs\":{\"tag_name\":\"v${{ needs.build.outputs.version }}\"}}"
            ACCEPT='-H "Accept: application/vnd.github+json"'
          else
            # Forgejo: workflow_dispatch 通过 API 触发，端点同构
            ENDPOINT="$RELEASE_API_BASE/repos/$GITHUB_REPOSITORY/actions/workflows/sign-release.yml/dispatches"
            PAYLOAD="{\"ref\":\"main\",\"inputs\":{\"tag_name\":\"v${{ needs.build.outputs.version }}\"}}"
            ACCEPT='-H "Accept: application/json"'
          fi
          # 用数组避免 eval
          if [ "$RELEASE_PROVIDER" = "github" ]; then
            curl -fsS -X POST \
              -H "Authorization: token $RELEASE_AUTH_TOKEN" \
              -H "Accept: application/vnd.github+json" \
              -d "$PAYLOAD" "$ENDPOINT"
          else
            curl -fsS -X POST \
              -H "Authorization: token $RELEASE_AUTH_TOKEN" \
              -H "Content-Type: application/json" \
              -d "$PAYLOAD" "$ENDPOINT"
          fi
```

- [ ] **Step 4: 验证 YAML 合法**

Run:
```bash
python3 -c "import yaml; d=yaml.safe_load(open('.forgejo/workflows/public-release.yml')); print('YAML OK'); print([s['name'] for s in d['jobs']['publish']['steps']])"
```
Expected: `YAML OK` 且步骤列表含 `Dispatch Signing Workflow`

- [ ] **Step 5: 提交**

```bash
git add .forgejo/workflows/public-release.yml
git commit -m "feat(forgejo): 发布后同时触发签名工作流"
```

---

## Task 12: 端到端验证与收尾

**Files:**
- 无新增；验证前述所有改动

**Interfaces:**
- Consumes: Task 1-11 全部
- Produces: 可交付的验证证据

- [ ] **Step 1: 全量质量门**

Run:
```bash
cd rust/aegis
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo test --doc
```
Expected: 全绿；测试数 ≥ 829（820 基线 + 新增 9）

- [ ] **Step 2: Go 质量门**

Run:
```bash
cd go/installer
go build ./...
go test ./...
gofmt -l .
```
Expected: 全绿，`gofmt -l` 无输出

- [ ] **Step 3: 不变量检查**

Run: `bash scripts/test-rotate-key.sh`
Expected: ✅ 全部通过

- [ ] **Step 4: 确认无残留旧符号**

Run:
```bash
grep -rn "MINISIGN_PUBLIC_KEYS\|minisignPublicKeys" --include=*.rs --include=*.go --include=*.sh . | grep -v "\.worktrees/"
```
Expected: 无输出（全部已改名）

- [ ] **Step 5: 确认版本匹配已改**

Run:
```bash
grep -rn "\.contains(&artifact\.tag_name)\|\.contains(&release\.tag_name)\|HasPrefix(gotVersion" --include=*.rs --include=*.go .
```
Expected: 无输出

- [ ] **Step 5b: 确认无静默跳过签名的路径（F4-2）**

Run:
```bash
# upgrade.rs（自家资产）：必须硬校验，不得有静默路径。
# 用 -E 并排除注释行（注释里会提到 .ok().flatten() 作为反例说明）；
# 注意原回归形态是 .ok() 与 .flatten() 分行书写，故两者都要查。
grep -rnE '^[^/]*\.(ok\(\)|flatten\(\))' --include=*.rs rust/aegis/src/core/system/upgrade.rs
```
Expected: 无输出（注释行不算）

同时人工确认 `core_upgrade.rs` 的条件强校验结构正确：
```bash
grep -n "let Some(sig_url) = release.minisig_url" -A 8 rust/aegis/src/core/system/core_upgrade.rs
```
Expected: 看到 `else { log::warn!(...); return Ok(temp_file); }`，
且**不得**在 else 分支里跳过任何已验证内容的检查。
参见 spec §7.1（两路径策略差异，勿为「统一」而改硬校验）。

- [ ] **Step 6: 提交并推分支**

```bash
git add -A
git commit -m "test: 端到端验证 minisign 硬化" --allow-empty
git push -u origin feat/minisign-hardening
```

- [ ] **Step 7: 人工验证清单（交付给用户）**

- [ ] 在 GitHub 上手动跑 `sign-release.yml`（输入 `v1.5.3`）确认签名成功
- [ ] 在 Codefloe 上手动跑 `sign-release.yml`（输入 `v1.6.0`）确认签名成功且 `minisign -v` 输出正常
- [ ] 下载一个 `.minisig` 验证 trusted comment 仍为 `vX.Y.Z:asset`
- [ ] 确认 `v1.5.0` 在硬校验下按预期失败（可接受）
- [ ] 轮换密钥后确认历史公钥出现在 `MINISIGN_HISTORICAL_KEYS`

---

## Self-Review

**Spec 覆盖检查：**

| Spec 要求 | 对应任务 |
|-----------|---------|
| 密钥列表一分为二（4.1） | Task 1（Rust）、Task 5（Go） |
| 验证顺序：活跃→历史（4.1） | Task 2（Rust）、Task 6（Go） |
| 硬校验三处（4.2） | Task 4（两处 Rust）、Task 7（Go） |
| 版本精确匹配（4.3） | Task 3（Rust）、Task 7（Go） |
| rotation 保留历史（4.4） | Task 8 |
| 不打印私钥（4.4） | Task 8 |
| Codefloe 签名（4.5） | Task 10、Task 11 |
| 过期检查同步（§8.7） | Task 9 |
| 验收标准 1-7（§8） | Task 12 |

**占位符扫描：** 无 TBD/TODO；每个代码步骤均含完整代码块。

**类型一致性检查：**
- Rust `MinisignKeyEntry.retired_at`（Task 1）↔ Task 8 脚本写入 `retired_at`  ✓
- Go `minisignKeyEntry.RetiredAt`（Task 5）↔ Task 8 脚本写入 `RetiredAt`  ✓
- `verify_minisign` 4 参数（Task 2）↔ Task 3/4 调用点  ✓
- `verifyMinisign` 4 参数（Task 6）↔ Task 7 调用点  ✓
- `MINISIGN_ACTIVE_KEYS`/`MINISIGN_HISTORICAL_KEYS` 命名在 Task 1/2/9 一致  ✓
- `minisignActiveKeys`/`minisignHistoricalKeys` 命名在 Task 5/6/9 一致  ✓

**已识别的风险：**
1. Task 4 的 Rust 升级路径无集成测试（依赖网络）→ 以编译期类型 + 语义锁定测试替代，需人工审查
2. Task 10 的 `MINISIGN_KEY_PATH` 跨 step 传递依赖 `$GITHUB_ENV`，Forgejo 支持该机制
3. Task 11 的 Forgejo workflow_dispatch API 端点未实测，若失败需回退为「发布后在 Codefloe UI 手动触发一次」
