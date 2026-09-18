# Onboarding Hardening (P1–P5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 SimpleX onboarding 真正闭环 —— aegis 装上日志器使 139 处 `log::*` 生效，并修掉 P2–P5 四个 onboarding/运维缺陷。

**Architecture:** 五个互相独立的小改动，全部沿用既有模式：Rust 侧新增一个零依赖的 bin-only 日志模块，其余三个是定点逻辑修正；Go 安装器侧新增两个纯函数（可单测）+ 两处接线，i18n 文案补齐三语。不改任何既有配置语义。

**Tech Stack:** Rust 2024（`log` 0.4.34，无新增依赖）、Go 1.26（`go/installer`，无新增依赖）、systemd。

**Spec:** `docs/superpowers/specs/2026-09-18-p1-p5-fixes-design.md`（已批准：D1–D5；开放项裁决见该文档 §7 —— logger 默认 Info、重配提示默认 `N`）

## Global Constraints

- **工作树**：`/home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening`，分支 `fix/onboarding-hardening`，基线 main = `b720f5e`
- **Rust 目标目录必须复用**（否则重建 matrix-sdk 全套，10+ 分钟）：每条 cargo 命令前
  `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`
- **Rust 质量门**（每个 Rust 任务提交前全绿）：
  `cargo fmt --check` ｜ `cargo clippy --all-targets --all-features -- -D warnings` ｜
  `cargo nextest run --cargo-profile fast-test` ｜ `cargo test --doc`
- **Go 质量门**：在 `go/installer` 下 `gofmt -l .`（须为空）｜ `go vet ./...` ｜ `go test ./...`
- **回归基线**：Rust **920 passed, 1 skipped**（`cargo nextest run --cargo-profile fast-test`，实测 1m42s）；Go 两包 ok
- **不新增任何依赖**（Rust 与 Go 均无）
- **i18n 硬约束**：新增 key 必须同时写进 `zh.json` / `en.json` / `ja.json`。
  `i18n_test.go` 的 `TestAllKeysExist` 强制 zh ⊆ en 且 zh ⊆ ja；带占位符的文案必须保留 `%s`
- **git pathspec 相对当时 cwd 解析**：在 `rust/aegis` 下用 `git add src/...`
- **`log::*` 不允许出现在 `src/main/logging.rs` 之外的新增代码里**（本计划的 Go 侧同理不得引入日志器）
- **明确不许改动**（spec §3 非目标）：`run_setup` / `--setup` / `--setup-stdin` / `--setup-keyval` 的全量覆盖语义；
  `--tg-simplex` 的硬校验；`--discord` 的既有拒绝路径；`simplex_store` / 数据目录的销毁；P6 各项
- **禁写占位符**：本计划每一步都含可直接粘贴的完整代码

---

### Task 1: P1 — aegis 安装日志器，使 139 处 `log::*` 生效

**Files:**
- Create: `rust/aegis/src/main/logging.rs`
- Modify: `rust/aegis/src/main/mod.rs`
- Modify: `rust/aegis/src/main.rs:25-26`
- Test: `rust/aegis/src/main/logging.rs`（同文件内 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 无（`log` crate 已在 `[dependencies]`）
- Produces: `aegis` bin 内的 `main::logging::init_logger()`（无参、无返回）。
  守卫用公开 API：`log::logger()` 返回 `&'static dyn Log`，未安装时是 `NopLogger`，其 `enabled()` 恒为 `false`
  （已在 `~/.cargo/registry/src/index.crates.io-*/log-0.4.34/src/lib.rs:1318-1326` 核实）

- [ ] **Step 1: 写失败测试**

创建 `rust/aegis/src/main/logging.rs`，**只写测试与最小骨架**（骨架故意不安装 logger，好让测试先红）：

```rust
use std::io::Write;

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let _ = writeln!(std::io::stderr(), "[{}] {}", record.level(), record.args());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

fn parse_level(raw: &str) -> log::LevelFilter {
    log::LevelFilter::Info
}

fn level_from_env() -> log::LevelFilter {
    parse_level(&std::env::var("WWPS_LOG").unwrap_or_default())
}

pub fn init_logger() {
    let _ = level_from_env();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「没有安装日志器」的机器可判定特征：`log::logger()` 在未安装时返回 `NopLogger`，
    /// 而 `NopLogger::enabled()` 恒为 false —— 这正是 139 处 `log::*` 被静默丢弃的原因。
    /// 注意不能用 `log::max_level()`：单独的 `set_max_level` 就能把它设为非 Off，
    /// 即使一个 logger 都没有。
    #[test]
    fn init_logger_installs_a_logger_whose_records_are_enabled() {
        init_logger();
        let meta = log::Metadata::builder().level(log::Level::Info).build();
        assert!(
            log::logger().enabled(&meta),
            "没有安装日志器：所有 log::* 都会被静默丢弃"
        );
    }

    #[test]
    fn parse_level_accepts_known_names_case_insensitively() {
        assert_eq!(parse_level("off"), log::LevelFilter::Off);
        assert_eq!(parse_level("ERROR"), log::LevelFilter::Error);
        assert_eq!(parse_level(" warn "), log::LevelFilter::Warn);
        assert_eq!(parse_level("Info"), log::LevelFilter::Info);
        assert_eq!(parse_level("debug"), log::LevelFilter::Debug);
        assert_eq!(parse_level("Trace"), log::LevelFilter::Trace);
    }

    #[test]
    fn parse_level_defaults_to_info() {
        assert_eq!(parse_level(""), log::LevelFilter::Info);
        assert_eq!(parse_level("nonsense"), log::LevelFilter::Info);
    }
}
```

在 `rust/aegis/src/main/mod.rs` 末尾追加（保持字母序，插在 `runtime` 与 `simplex` 之间）：

```rust
pub mod logging;
```

在 `rust/aegis/src/main.rs` 的 `main()` 首行插入调用：

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 必须最先执行：无 logger 时 log::* 退化为 no-op，此处之前的任何日志都会被静默丢弃。
    main::logging::init_logger();

    harden_process();
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test logging
```
Expected: **2 failed / 1 passed** ——
`init_logger_installs_a_logger_whose_records_are_enabled` FAIL 于「没有安装日志器」；
`parse_level_accepts_known_names_case_insensitively` FAIL（骨架恒返回 `Info`）；
`parse_level_defaults_to_info` PASS（骨架恰好满足它，属预期）。
关键是第 1 条必须红 —— 它就是守卫本身。

- [ ] **Step 3: 写最小实现**

把 `rust/aegis/src/main/logging.rs` 的 `parse_level` 与 `init_logger` 换成真实现，并把文件头文档注释补上：

```rust
//! 进程唯一的日志器。
//!
//! 在此之前 aegis 从未安装过日志器：`log` crate 在无 logger 时把 `log::logger()` 保持为
//! `NopLogger`（`enabled()` 恒 false），139 处 `log::*` 在生产二进制里全部退化为 no-op。
//! 运维唯一能取到 SimpleX contactId 的途径（`runtime.rs` 的「未授权联系人」告警）因此
//! 永远不出现，`--set-simplex-admin` 无值可填，onboarding 无法闭环。
//!
//! 写入 stderr 而非 stdout：安装器把 `--generate-totp-secret` 的整段 stdout 当成 TOTP 密钥
//! （见 `main.rs` 顶部注释），stdout 必须保持干净。systemd 默认把 stderr 收进 journal，
//! `Binary Integrity Hash:` 走 `eprintln!` 是既有先例。

use std::io::Write;

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // 不用 eprintln!：它会在 broken pipe 上 panic。日志失败绝不能让进程退出。
        let _ = writeln!(std::io::stderr(), "[{}] {}", record.level(), record.args());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// 解析 `WWPS_LOG` 的级别名；无法识别时回落到 Info。
///
/// `log::LevelFilter` 已实现 `FromStr`（大小写不敏感，见 log 0.4.34 `src/lib.rs:674-685`），
/// 所以这里只补一个 `trim`（`FromStr` 不做 trim），不重复实现级别名表。
fn parse_level(raw: &str) -> log::LevelFilter {
    raw.trim().parse().unwrap_or(log::LevelFilter::Info)
}

/// 默认 Info：139 处调用站点无法逐个人工分级，Info 保住运维必需的行而不至于灌爆 journal。
/// 这个旋钮存在的理由是 3am 排查要能调级别，而不必重新编译或重发签名二进制。
/// 只认这一个变量，不引入 `RUST_LOG` 之类的外部约定。
fn level_from_env() -> log::LevelFilter {
    parse_level(&std::env::var("WWPS_LOG").unwrap_or_default())
}

/// 安装日志器。必须在 `main()` 的**第一条**语句调用。
///
/// `set_logger` 重复调用返回 Err（测试里会）—— 忽略：日志器是进程全局的，
/// 第二次安装无效但无害。
pub fn init_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level_from_env());
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test logging
```
Expected: 4 passed（`init_logger_installs_a_logger_whose_records_are_enabled` +
`parse_level_accepts_known_names_case_insensitively` + `parse_level_defaults_to_info` + 无其他）

- [ ] **Step 5: 跑完整 Rust 质量门**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test
cargo test --doc
```
Expected: clippy 无 warning；nextest **924 passed, 1 skipped**（920 + 4）；doc test ok。

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git add rust/aegis/src/main/logging.rs rust/aegis/src/main/mod.rs rust/aegis/src/main.rs
git commit -m "fix(aegis): 安装 stderr 日志器，使 139 处 log::* 不再被静默丢弃"
```

---

### Task 2: P2 — `has_simplex_config` 接受「只有 simplex_port」

**Files:**
- Modify: `rust/aegis/src/main/simplex.rs:55-59`（函数）与 `:178-181`（测试）

**Interfaces:**
- Consumes: Task 1 无依赖（可并行）
- Produces: `has_simplex_config(&EncryptedConfig, &[String]) -> bool` 新语义 —— 无 flag 时只看 `simplex_port`

- [ ] **Step 1: 改测试（先红）**

把 `rust/aegis/src/main/simplex.rs` 的测试 `returns_false_when_only_port_present` 整个替换为：

```rust
    /// 管理员留空是合法状态（首次安装时 contactId 尚未存在），因此「只配端口」必须
    /// 被识别为 SimpleX。否则会退化到 Telegram 分支，而无 token 时 `Bot::new` 直接 panic。
    #[test]
    fn returns_true_when_only_port_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        assert!(has_simplex_config(&cfg, &[]));
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test returns_true_when_only_port_present
```
Expected: FAIL（当前仍要求 `simplex_admin_id`）

- [ ] **Step 3: 改实现**

把 `rust/aegis/src/main/simplex.rs` 的 `has_simplex_config` 换成：

```rust
/// 是否需要接入 SimpleX。
///
/// 无 flag 时的探测只看 `simplex_port`：管理员留空是**合法状态**（首次安装时 contactId
/// 尚未存在，见 `decode_port_and_admin` 与 `main.rs` 的形态判定），所以要求
/// `simplex_admin_id` 同时存在会把「已配端口但还没填管理员」错判成非 SimpleX 部署，
/// 进而退化到 Telegram 分支（无 token 时 `Bot::new` 直接 panic）。
pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--simplex");
    explicit || encrypted_config.simplex_port.is_some()
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex
```
Expected: 全绿，含 `returns_true_when_only_port_present`、`returns_false_when_no_simplex_config`、
`returns_true_when_flag_present`、`returns_true_when_both_fields_present`

- [ ] **Step 5: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **924 passed, 1 skipped**（本任务只把 1 条测试改名并反转，**不增加**测试数量，因此仍是 Task 1 后的 924）。

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git add rust/aegis/src/main/simplex.rs
git commit -m "fix(simplex): 无 flag 探测不再要求 simplex_admin_id（管理员留空是合法状态）"
```

---

### Task 3: P3 — 未知 `-`/`--` 参数不得静默启动 bot

**Files:**
- Modify: `rust/aegis/src/main/cli.rs`（`CliMode` 枚举、`try_cli_mode`、`execute_cli_mode`、`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 无
- Produces: `CliMode::Unknown(String)`；`try_cli_mode` 对 `args[1]` 的未知 `-` 前缀参数返回 `Some(CliMode::Unknown(_))`

**已核实的安全面**（勿再质疑）：`tests/` 里启动二进制的参数只有
`["--setup", "dummy_token", "123456", <secret>]`、`["--generate-totp-secret"]`、`["-v"]`、`["--setup-stdin"]`
—— 全在白名单内。自升级只走 `self_replace::self_replace`（`src/core/system/upgrade.rs:594`）替换文件，
不 re-exec 传参，所以新校验不会打断升级路径。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/main/cli.rs` 的 `#[cfg(test)] mod tests` 末尾追加：

```rust
    /// aegis 已作为 systemd 服务运行时，静默回落到「正常启动」会起第二个实例、
    /// 消费同一份 WS 事件流并重复执行命令。拼写错误必须立刻可见。
    #[test]
    fn unknown_dash_argument_is_rejected_not_booted() {
        for arg in ["--set-simplex-admin=42", "--bogus", "-x", "--help"] {
            let mode = try_cli_mode(&args(&["aegis", arg]));
            assert!(
                matches!(mode, Some(CliMode::Unknown(ref f)) if f.as_str() == arg),
                "{arg} 必须被识别为未知参数，而不是回落到正常启动"
            );
        }
    }

    /// 平台 flag 必须穿透到 main.rs 的 resolve_platform_selection；
    /// `--discord` 保留穿透是为了让那条带迁移指引的专有报错继续生效。
    #[test]
    fn platform_flags_fall_through_to_normal_boot() {
        for arg in [
            "--simplex",
            "--tg-simplex",
            "--matrix",
            "--all",
            "--tg-only",
            "--discord",
        ] {
            assert!(try_cli_mode(&args(&["aegis", arg])).is_none(), "{arg} 必须返回 None");
        }
    }

    /// `--setup` 的 args[2..] 是定位参数，可能是任意字符串（例如以 `--` 开头的 token）；
    /// 未知校验只针对 args[1]。
    #[test]
    fn setup_positional_args_are_not_treated_as_unknown_flags() {
        let mode = try_cli_mode(&args(&["aegis", "--setup", "--weird-token", "1", "SECRET"]));
        assert!(matches!(mode, Some(CliMode::Setup { .. })));
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test cli
```
Expected: **编译失败**（`CliMode::Unknown` 不存在）。编译失败即本步的 RED —— 不要因此改动测试。

- [ ] **Step 3: 实现**

在 `rust/aegis/src/main/cli.rs` 的 `CliMode` 枚举里，`SetSimplexAdmin` 之后追加：

```rust
    /// `args[1]` 是无法识别的 `-`/`--` 参数。
    ///
    /// 绝不放行：aegis 已作为 systemd 服务运行时，静默回落到正常启动会起第二个实例、
    /// 重复消费同一份事件流并重复执行命令。
    Unknown(String),
```

`try_cli_mode` 的 `match` 尾部（`"--set-simplex-admin"` 分支之后）改为：

```rust
        "--set-simplex-admin" => Some(CliMode::SetSimplexAdmin(args.get(2).cloned())),
        // 合法但不走 CLI 模式的平台 flag：必须返回 None，让 main.rs 的
        // resolve_platform_selection 处理（其中 --discord 有专门的迁移指引报错）。
        "--simplex" | "--tg-simplex" | "--matrix" | "--all" | "--tg-only" | "--discord" => None,
        // 未知的 -/-- 参数一律报错。只校验 args[1]：--setup 的 args[2..] 是定位参数，
        // 可能是任意 token 字符串。
        other if other.starts_with('-') => Some(CliMode::Unknown(other.to_string())),
        _ => None,
```

`execute_cli_mode` 的 `match` 里追加（放在 `CliMode::SetSimplexAdmin` 分支之后）：

```rust
        CliMode::Unknown(flag) => Err(anyhow::anyhow!(
            "未知参数 {flag:?}。可用参数: --simplex | --tg-simplex | --matrix | --all | --tg-only | \
             --generate-totp-secret | --version | --setup <token> <admin_id> <totp_secret> | \
             --setup-stdin | --set-simplex-admin <contactId>"
        )),
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test cli
```
Expected: 全绿（含既有 `recognises_set_simplex_admin_with_value`、`rejects_non_numeric_contact_id`）

- [ ] **Step 5: 端到端确认退出码与用法提示**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo build --profile fast-test
./$CARGO_TARGET_DIR/fast-test/aegis --set-simplex-admin=42; echo "exit=$?"
```
Expected: 打印含「未知参数」与「可用参数」的 stderr 行，`exit=1`
（若 `fast-test` profile 产物路径不同，用 `cargo build --profile fast-test 2>&1 | grep -o '/[^ ]*aegis$'` 定位）

- [ ] **Step 6: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **927 passed, 1 skipped**（924 + 本任务新增 3 条）。

- [ ] **Step 7: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git add rust/aegis/src/main/cli.rs
git commit -m "fix(aegis): 未知 --参数 直接报错而非静默启动第二个 bot 实例"
```

---

### Task 4: P5 — 平台不再含 SimpleX 时停用 `wwps-simplex`

**Files:**
- Modify: `go/installer/main.go:1143-1146`（`deploySimplexService` 早退分支）+ 新增两个函数
- Modify: `go/installer/i18n/zh.json`、`en.json`、`ja.json`
- Test: `go/installer/main_test.go`（`usesSimplexService`）+ `go/installer/i18n/i18n_test.go`（文案守卫）

**Interfaces:**
- Consumes: 无
- Produces:
  - `usesSimplexService(platform string) bool`
  - `disableSimplexServiceIfPresent()`（无参无返回）
  - i18n key：`simplex.service_disabled`、`simplex.disable_failed`（含 `%s`）

- [ ] **Step 1: 写失败测试**

在 `go/installer/main_test.go` 末尾追加：

```go
func TestUsesSimplexService(t *testing.T) {
	tests := map[string]bool{
		"tg":         false,
		"matrix":     false,
		"tg-matrix":  false,
		"simplex":    true,
		"tg-simplex": true,
	}
	for platform, want := range tests {
		if got := usesSimplexService(platform); got != want {
			t.Errorf("usesSimplexService(%q) = %t, want %t", platform, got, want)
		}
	}
}
```

在 `go/installer/i18n/i18n_test.go` 末尾追加（沿用既有 `TestSimplexAddressKeysKeepFormatVerb` 的写法）：

```go
// P5 文案守卫：main.go 的 disableSimplexServiceIfPresent 按这两个名字查询，
// 而 TestAllKeysExist 只能发现「zh 有而 en/ja 缺」，发现不了「三个语言文件同时拼错」。
// disable_failed 带 %s，必须保留占位符。
func TestSimplexDisableKeysExist(t *testing.T) {
	keys := []string{"simplex.service_disabled", "simplex.disable_failed"}
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		for _, key := range keys {
			if v, ok := locale.data[key]; !ok || strings.TrimSpace(v) == "" {
				t.Errorf("%s: key %q 缺失或为空（main.go 会按这个名字查找）", locale.name, key)
			}
		}
		if !strings.Contains(locale.data["simplex.disable_failed"], "%s") {
			t.Errorf("%s: simplex.disable_failed 缺少 %%s 占位符: %q",
				locale.name, locale.data["simplex.disable_failed"])
		}
	}
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
go test ./... 2>&1 | tail -20
```
Expected: `go/installer` 编译失败（`usesSimplexService` 未定义）；`go/installer/i18n` FAIL 于两个 key 缺失/为空。

- [ ] **Step 3: 实现（Go 侧）**

在 `go/installer/main.go` 中，把 `deploySimplexService` 的开头改为：

```go
// deploySimplexService 安装 simplex-chat 并写好它的 systemd 单元；
// 平台不再包含 SimpleX 时改为停用既有单元。
// 端口优先级：调用方显式给的 > 已存在单元里回读的 > 默认端口。
func deploySimplexService(platform, port string) {
	if !usesSimplexService(platform) {
		disableSimplexServiceIfPresent()
		return
	}
```

紧接在 `deploySimplexService` **之前**插入两个新函数：

```go
// usesSimplexService 报告某个平台形态是否需要 wwps-simplex 单元。
func usesSimplexService(platform string) bool {
	return platform == "simplex" || platform == "tg-simplex"
}

// disableSimplexServiceIfPresent 停止并取消 wwps-simplex 的开机自启。
//
// 平台不再包含 SimpleX 时，早退前必须处理掉这个单元：它此前是 enabled 且正在运行的，
// 会一直占着端口和 SQLite 库，开机还会自启（此前 deploySimplexService 对它静默 return，
// 全文件再无任何 stop/disable）。
//
// 单元文件与数据目录都**保留** —— 切回 simplex 时仍可用，且不销毁 bot 身份（simplex_store）。
func disableSimplexServiceIfPresent() {
	if _, err := os.Stat(simplexServiceFile); err != nil {
		// 从未部署过 simplex：不要对不存在的单元调 systemctl，那只会把报错喷进安装输出。
		return
	}
	if err := runCmdSilent("systemctl", "disable", "--now", simplexServiceName); err != nil {
		printYellow(i18n.T("simplex.disable_failed", err.Error()))
		return
	}
	printGreen(i18n.T("simplex.service_disabled"))
}
```

- [ ] **Step 4: 实现（三语文案）**

在 `go/installer/i18n/zh.json` 中，紧邻既有的 `"simplex.service_ok"` 之后各加一行：

```json
	"simplex.service_disabled": "已停止并取消 wwps-simplex 的开机自启（单元与数据保留，切回 SimpleX 仍可用）",
	"simplex.disable_failed": "停用 wwps-simplex 失败（不影响本次部署）: %s",
```

在 `go/installer/i18n/en.json` 同一位置：

```json
	"simplex.service_disabled": "Stopped wwps-simplex and disabled its autostart (unit and data kept, so switching back to SimpleX still works)",
	"simplex.disable_failed": "Failed to disable wwps-simplex (does not affect this deployment): %s",
```

在 `go/installer/i18n/ja.json` 同一位置：

```json
	"simplex.service_disabled": "wwps-simplex を停止し自動起動を無効化しました（ユニットとデータは保持、SimpleX へ戻せます）",
	"simplex.disable_failed": "wwps-simplex の無効化に失敗しました（今回のデプロイには影響しません）: %s",
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
gofmt -l . && go vet ./... && go test ./... 2>&1 | tail -20
```
Expected: `gofmt -l` 无输出；`go vet` 无输出；两个包都 `ok`

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/i18n_test.go \
        go/installer/i18n/zh.json go/installer/i18n/en.json go/installer/i18n/ja.json
git commit -m "fix(installer): 平台不再含 SimpleX 时停用 wwps-simplex，不再静默留着自启"
```

---

### Task 5: P4 — 交互式重跑安装器可重新配置

**Files:**
- Modify: `go/installer/main.go:1364-1369`（`installAegis` 的 `configExists` 分支）+ 新增 `shouldReconfigure`
- Modify: `go/installer/i18n/zh.json`、`en.json`、`ja.json`
- Test: `go/installer/main_test.go` + `go/installer/i18n/i18n_test.go`

**Interfaces:**
- Consumes: Task 4 无依赖（同文件，必须串行执行）
- Produces:
  - `shouldReconfigure(answer string) bool`
  - i18n key：`install.reconfigure_prompt`（含 `%s`）、`install.reconfigure_warning`

- [ ] **Step 1: 写失败测试**

在 `go/installer/main_test.go` 末尾追加（并确认 `strings` 已在 import 里 —— 它已在）：

```go
func TestShouldReconfigure(t *testing.T) {
	for _, tt := range []struct {
		answer string
		want   bool
	}{
		{"", false},
		{"n", false},
		{"N", false},
		{"no", false},
		{"也许", false},
		{"y", true},
		{"Y", true},
		{" y ", true},
		{"yes", true},
		{"YES", true},
	} {
		if got := shouldReconfigure(tt.answer); got != tt.want {
			t.Errorf("shouldReconfigure(%q) = %t, want %t", tt.answer, got, tt.want)
		}
	}
}
```

在 `go/installer/i18n/i18n_test.go` 末尾追加：

```go
// P4 文案守卫：installAegis 按这两个名字查询；reconfigure_prompt 带 %s（当前平台），
// 缺占位符会让平台名被 fmt 丢弃。
func TestInstallReconfigureKeysExist(t *testing.T) {
	keys := []string{"install.reconfigure_prompt", "install.reconfigure_warning"}
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		for _, key := range keys {
			if v, ok := locale.data[key]; !ok || strings.TrimSpace(v) == "" {
				t.Errorf("%s: key %q 缺失或为空", locale.name, key)
			}
		}
		if !strings.Contains(locale.data["install.reconfigure_prompt"], "%s") {
			t.Errorf("%s: install.reconfigure_prompt 缺少 %%s 占位符: %q",
				locale.name, locale.data["install.reconfigure_prompt"])
		}
	}
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
go test ./... 2>&1 | tail -20
```
Expected: `go/installer` 编译失败（`shouldReconfigure` 未定义）；i18n 包 FAIL 于两个 key 缺失。

- [ ] **Step 3: 实现（Go 侧）**

在 `go/installer/main.go` 中，`installAegis` **之前**插入：

```go
// shouldReconfigure 判断交互式重跑安装时用户是否要求重新配置。
//
// 默认（空输入 / 直接回车）为否：重配会重新生成 TOTP 密钥并重新输入所有凭据，
// 不能是「手滑回车」的默认结果。接受 y / Y / yes / YES（大小写与前后空白不敏感）。
func shouldReconfigure(answer string) bool {
	switch strings.ToLower(strings.TrimSpace(answer)) {
	case "y", "yes":
		return true
	default:
		return false
	}
}
```

把 `installAegis` 里的 `if configExists { ... } else { ... }` 改为：

```go
	if configExists {
		printGreen(i18n.T("install.config_exists"))
		fmt.Print(i18n.T("install.reconfigure_prompt", platform))
		answer, _ := readLine()
		if shouldReconfigure(answer) {
			printYellow(i18n.T("install.reconfigure_warning"))
			var err error
			platform, simplexPort, err = firstTimeSetup(destPath)
			if err != nil {
				return
			}
			if _, err := os.Stat(configPath); err != nil {
				printRed(i18n.T("setup.failed", err.Error()))
				return
			}
		}
	} else {
		var err error
		platform, simplexPort, err = firstTimeSetup(destPath)
		if err != nil {
			return
		}
		if _, err := os.Stat(configPath); err != nil {
			printRed(i18n.T("setup.failed", err.Error()))
			return
		}
	}
```

（`configExists` 分支原先只有 `printGreen(i18n.T("install.config_exists"))` 一行，其余照抄不动。）

- [ ] **Step 4: 实现（三语文案）**

在 `go/installer/i18n/zh.json` 中，紧邻既有的 `"install.config_exists"` 之后各加一行：

```json
	"install.reconfigure_prompt": "\n是否重新配置？（当前平台: %s）[y/N] ",
	"install.reconfigure_warning": "⚠️  重新配置会重新生成 TOTP 密钥并重新输入所有凭据（Telegram / Matrix / SimpleX），原 2FA 条目将失效。",
```

在 `go/installer/i18n/en.json` 同一位置：

```json
	"install.reconfigure_prompt": "\nReconfigure now? (current platform: %s) [y/N] ",
	"install.reconfigure_warning": "WARNING: reconfiguring regenerates the TOTP secret and re-prompts every credential (Telegram / Matrix / SimpleX); the existing 2FA entry stops working.",
```

在 `go/installer/i18n/ja.json` 同一位置：

```json
	"install.reconfigure_prompt": "\n再設定しますか？（現在のプラットフォーム: %s）[y/N] ",
	"install.reconfigure_warning": "⚠️  再設定すると TOTP シークレットが再生成され、すべての認証情報（Telegram / Matrix / SimpleX）を再入力します。既存の 2FA エントリは無効になります。",
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
gofmt -l . && go vet ./... && go test ./... 2>&1 | tail -20
```
Expected: 无 gofmt/vet 输出；两包 ok

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/i18n_test.go \
        go/installer/i18n/zh.json go/installer/i18n/en.json go/installer/i18n/ja.json
git commit -m "feat(installer): 交互式重跑可重新配置（默认否，需显式确认）"
```

---

### Task 6: 全量回归 + 真实主机验证

**Files:**
- Modify: 无（只跑验证；失败则回到对应任务修）

**Interfaces:**
- Consumes: Task 1–5 全部产出
- Produces: 交付证据

- [ ] **Step 1: 全量本地门（Rust + Go）**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings \
  && cargo nextest run --cargo-profile fast-test && cargo test --doc
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
gofmt -l . && go vet ./... && go test ./...
```
Expected: Rust **927 passed, 1 skipped**（920 + 4 + 1 + 3 = 928 累计增量中，i18n 的 2 条属于 Go 包）；
以实际 `Summary` 行为准，且**必须 0 failed / 0 skipped 增量**。Go 两包 ok。

- [ ] **Step 2: 构建待测二进制**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo build --release
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening/go/installer
CGO_ENABLED=0 go build -o installer-test .
```
Expected: 两个二进制产出成功

- [ ] **Step 3: 真实主机 P1 硬断言（onboarding 闭环）**

按 `HANDOFF.md` 第四节部署测试要点操作（仓库安装器只会下载**已发布**的 aegis，
必须把本分支自建二进制 `install -m 0755` 覆盖 `/etc/wwps/aegis/aegis` 后
`setcap cap_ipc_lock+eip` 并重启）：

```bash
ssh root@23.165.248.200 'journalctl -u wwps-aegis --since "5 min ago" -o cat | grep -c "SimpleX bot 地址:"'
```
Expected: **≥ 1**（这是 P1 的核心断言 —— 修复前该计数恒为 0）
随后用一个非管理员 SimpleX 身份向 bot 发消息，再执行：
```bash
ssh root@23.165.248.200 'journalctl -u wwps-aegis --since "5 min ago" -o cat | grep -c "未授权联系人 contactId="'
```
Expected: **≥ 1**，且输出里能看到真实 contactId → 用它跑
`aegis --set-simplex-admin <contactId>` → `systemctl restart wwps-aegis` → bot 开始响应

- [ ] **Step 4: 真实主机 P3 / P5 断言**

```bash
ssh root@23.165.248.200 'cd /etc/wwps/aegis && ./aegis --set-simplex-admin=42; echo "exit=$?"; pgrep -c -f "/etc/wwps/aegis/aegis$"'
```
Expected: 打印「未知参数」用法提示，`exit=1`，`pgrep -c` 仍为 **1**（没有起第二个实例）

P5：用只含 token 的 keyval 从 simplex 切到 tg 后：
```bash
ssh root@23.165.248.200 'systemctl is-enabled wwps-simplex; systemctl is-active wwps-simplex'
```
Expected: `disabled`（或非 `enabled`）+ `inactive`（或非 `active`）——**修复前两者都是 enabled/active**

- [ ] **Step 5: 确认回归对照未退化**

```bash
ssh root@23.165.248.200 'systemctl show -p NRestarts -p ActiveState wwps-aegis'
```
Expected: `ActiveState=active`（不是 `activating`）、`NRestarts=0`（不是递增）—— 与合并前的 A/B 对照一致

- [ ] **Step 6: 提交（仅在 Step 3–5 有修复时）**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/onboarding-hardening
git status --short
```
Expected: 干净（无未提交改动）。若有，按对应任务补提交。

---

## rust-skills 核对（265 条规则逐类过滤后的实际命中）

### 已遵守且是刻意设计（勿在实现时“简化掉”）

| 规则 | 本计划如何满足 |
| --- | --- |
| `obs-library-facade` “库只用派发器，绝不安装 subscriber” | 日志器装在 **bin**（`src/main/logging.rs`），不在 `src/lib.rs`。`src/main/mod.rs` 只被 `src/main.rs` 引入，lib 侧不装任何 logger |
| `mem-write-over-format` | `writeln!(std::io::stderr(), "[{}] {}", ...)` 直接格式化进 stderr，不走 `format!` 分配临时 String |
| `conv-fromstr-parsing` / `anti-stringly-typed` | `parse_level` 直接用 `LevelFilter: FromStr`（见 Step 3 注释），不手写级别名 match |
| `own-borrow-over-clone` / `own-slice-over-vec` | 签名沿用 `&str`、`&[String]`、`&Metadata`、`&Record`，无新增 clone/所有 |
| `unsafe-send-sync-manual` | 不手写 `Send`/`Sync`：`static LOGGER: StderrLogger` 是 ZST，`Log: Sync + Send`（`log-0.4.34/src/lib.rs:1284`）由编译器自动满足 |
| `unsafe-*` 全类 | 本计划不新增任何 `unsafe` 块 |
| `test-cfg-test-module` / `test-use-super` / `test-descriptive-names` | 测试均在 `#[cfg(test)] mod tests` 内、`use super::*;`、名字描述行为而非实现 |
| `doc-module-inner` / `doc-all-public` | `logging.rs` 有 `//!` 模块文档；`init_logger` 有 `///` 及其“必须在 main 首行”的契约 |
| `pat-exhaustive-enum` | `execute_cli_mode` 的 match 对新变体 `Unknown` 显式加分支，不留 catch-all 掩盖新变体 |
| `name-*` | `StderrLogger` / `parse_level` / `level_from_env` / `init_logger` / `usesSimplexService` / `shouldReconfigure` 均合命名约定（`name-is-has-bool` 也满足：布尔返回用 `uses…`/`should…`） |
| `lint-rustfmt-check` | Global Constraints 里的 `cargo fmt --check` |

### 刻意偏离（已权衡，勿“修正”）

| 规则 | 偏离 | 理由 |
| --- | --- | --- |
| `obs-tracing-over-log` | 仍用 `log` 而非 `tracing` | 仓内已有 **139** 处 `log::*`，迁到 `tracing` 是跨模块重写，远超 P1–P5 范围；本计划只让 `log` 真正生效 |
| `obs-levels-filter` | 用 `WWPS_LOG` 而非 `RUST_LOG` | 只认一个自有变量，不引入外部命名约定；`FromStr` 巳兼容全部标准级别名 |
| `perf-io-buffering` | **故意不加** `BufWriter` | release profile 是 `panic = "abort"`；带缓冲会在崩溃时丢掉最后几行（正是排障最需要的那几行）。无缓冲 stderr = 每记录一次 `write(2)`，且不会丢尾部 |
| `err-lowercase-msg` | 中文报错串沿用首字母大写与尾部标点 | 与仓内既有报错（如 `main.rs:191` 的 `--discord` 文案）保持一致；此规则针对英文报错 |

### `obs-no-sensitive-data` 审查（开启日志前的强制前置，已执行）

139 处 `log::*` 全部逐条审阅（34 `info` + 4 `debug` + 81 `warn` + 20 `error`，含 11 处多行宏的完整参数）：
**零处打印凭据** —— 内容均为端口 / 路径 / 文件名 / 计数 / SNI cache key / 错误串。
带敏感词的调用（token/password/passphrase/secret/key/credential/totp/recovery）**命中 0**。
`singbox/config.rs:424` 的“检查 TLS 密钥存在性失败”只打错误对象，不打密钥。
故默认 Info 不会新增凭据泄露面。

**额外发现（PR 描述应当写入）**：被静默丢弃的不只 simpleX 那两行 —— 20 处 `log::error!` 也全死了，
包括 `main/runtime.rs:250/437/473` 的 `❌ 初始化调度器失败`。即调度器初始化失败在生产里也是**静默**的。P1 的收益远大于“让 contactId 可见”。

## 交付后的分支处置

按 `finishing-a-development-branch` 技能：验证测试全绿后选择处置方式（PR / merge / keep）。
PR 描述须**如实**写明：P1 是既存缺陷（日志器从未存在），本计划修的是「本分支的文案指向了一个兑现不了的承诺」。

## 已知未覆盖（诚实边界，勿在 PR 中隐去）

1. **P1 的守卫测试证明 `init_logger` 有效，不证明 `main()` 调用了它。** 后者只有 Task 6 Step 3 的
   真实主机 journal 断言能证明。已在 spec §7 记录；若未来有人删掉 `main.rs` 里的调用，单测不会红。
2. **P2 的接受后果**：`token + simplex_port + 无 admin + 无 flag` 的配置会从「Telegram only」
   变为「纯 SimpleX，token 被忽略」。逃生通道是显式 `--tg-only`。见 spec §4 D2。
3. **P4 不是定点改字段**：重配会重新生成 TOTP 并重新输入全部凭据。
   只改 Telegram token 的无副作用路径（`aegis --set-tg-token`）明确不做。
4. **CodeGraph 索引为混合陈旧快照**，`codebase-memory-mcp` 无已索引项目（`check_index_coverage` 无法运行）。
   本计划所有行号均取自工作树源码直读；spec §7 已披露。
