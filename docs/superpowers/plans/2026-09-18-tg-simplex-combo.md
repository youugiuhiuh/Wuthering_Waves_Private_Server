# Telegram + SimpleX 组合部署实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 aegis 与安装器支持第五种部署形态 `Telegram + SimpleX`（TG 为主适配器、SimpleX 为敏感内容落点），语义与现有 `Telegram + Matrix` 完全对等。

**Architecture:** 复用既有 `RoutingAdapter`（primary + 可选 secondary，命中 `is_sensitive` 时改投 secondary）。Rust 侧新增 `--tg-simplex` 显式 flag，在适配器装配处用 `RoutingAdapter` 包住 TG 主适配器并把 secondary 目标改写为 `simplex_admin_id`（Matrix 适配器忽略目标，SimpleX 适配器会把目标当 `contactId` 解析，因此必须改写）。Go 安装器放宽选择器合法性为「SimpleX 只与 Telegram 组合」，并新增 `--tg-simplex` 的单元映射与非交互推导。

**Tech Stack:** Rust（aegis，teloxide / matrix-sdk / simploxide-client）、Go 1.26（go/installer，bubbletea TUI）、三个 locale 的 JSON i18n。

**Spec:** `docs/superpowers/specs/2026-09-18-tg-simplex-combo-design.md`

## Global Constraints

- 工作目录：本计划在隔离工作树 `/home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex`（分支 `feat/tg-simplex`）中执行。
- **Rust 门禁（rust-lint-format 技能，四道全过才可声称完成）**，在 `rust/aegis/` 下：
  `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc`
- **Rust 编译缓存**：每条 cargo 命令都必须带 `CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`（复用主仓库已有 20G 缓存；不带则冷编译 15 分钟以上）。建议每步开始先 `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`。
- **Go 门禁（go-lint-format 技能）**，在 `go/installer/` 下：
  `go fmt ./... && go test ./... && staticcheck ./...`
- 不新增任何依赖（无需 `cargo add` / `go get`）。
- i18n：`go/installer/i18n/{en,zh,ja}.json` 三个 locale 必须同步加键（`i18n.T` 缺键会原样返回键名，不会报错，漏加会静默显示键名）。
- 提交信息用仓库既有的中文 conventional commit 风格。
- 每个 Task 的验证步骤必须真实执行并看到期望输出后才能勾选。

## File Structure

| 文件 | 责任 | 本计划中的改动 |
|---|---|---|
| `rust/aegis/src/common/routing.rs` | 敏感内容转发器（primary/secondary 分流） | 新增 secondary 目标覆盖 |
| `rust/aegis/src/main.rs` | 平台选择决策表 + 适配器装配 + 启动通知 | 新增 `--tg-simplex` 与组合装配 |
| `rust/aegis/src/main/runtime.rs` | 各平台网关启动与 scheduler 初始化 | SimpleX 初始化门禁加 `!enable_telegram` |
| `go/installer/main.go` | 安装器 CLI/TUI、平台→单元映射、非交互推导 | 组合合法性、映射表、推导纯函数 |
| `go/installer/main_test.go` | 安装器单测 | 改写 4 个旧契约测试 + 新增用例 |
| `go/installer/i18n/{en,zh,ja}.json` | 提示文案 | 补编号选项与新错误文案 |
| `docs/2026-09-16-simplex-platform.md` | SimpleX 平台运维文档 | 重写「平台互斥语义」表 |
| `README.md` | 用户快速上手 | 平台表补一行 |

依赖关系：Task 1 → Task 2（Task 2 用到 Task 1 的 API）。Task 3 → Task 4（同文件顺序修改）。Task 5、Task 6 收尾。Task 2 与 Task 3 之间无依赖，但**同一工作树内不得并行写入**。

---

### Task 1: RoutingAdapter 支持 secondary 目标覆盖

**Files:**
- Modify: `rust/aegis/src/common/routing.rs:8-46`（结构体 + `new` + `send_message`）、`rust/aegis/src/common/routing.rs:190-260`（`mod routing_tests`）

**Interfaces:**
- Consumes: 无（本任务是最底层）
- Produces:
  - `RoutingAdapter::with_secondary_target(self, target: TargetId) -> Self`（Task 2 使用）
  - `RoutingAdapter::new(primary: Arc<dyn BotAdapter>, secondary: Option<Arc<dyn BotAdapter>>) -> Self`（**签名不变**）

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/common/routing.rs` 的 `mod routing_tests` 内、`sends_all_to_primary_when_no_secondary` 之后追加两个测试：

```rust
        #[tokio::test]
        async fn sensitive_uses_secondary_target_override() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_send_message().never();

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary
                .expect_send_message()
                .times(1)
                .withf(|target, _| target.0 == "999")
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
                .with_secondary_target(TargetId("999".to_string()));
            routing
                .send_message(
                    &TargetId("42".to_string()),
                    MessageContent {
                        text: "vless://abc123".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn normal_message_keeps_original_target() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .withf(|target, _| target.0 == "42")
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary.expect_send_message().never();

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
                .with_secondary_target(TargetId("999".to_string()));
            routing
                .send_message(
                    &TargetId("42".to_string()),
                    MessageContent {
                        text: "normal system message".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test sensitive_uses_secondary_target_override
```

期望：编译失败，报 `no method named 'with_secondary_target' found for struct 'RoutingAdapter'`。

- [ ] **Step 3: 写最小实现**

把 `routing.rs:8-17` 的 `RoutingAdapter` 结构体与 `new` 替换为：

```rust
pub struct RoutingAdapter {
    primary: Arc<dyn BotAdapter>,
    secondary: Option<Arc<dyn BotAdapter>>,
    /// secondary 的发送目标覆盖。SimpleX 适配器会用 `parse_chat_id(target)` 解读目标
    /// （`gateways/simplex/adapter.rs`），而 Matrix 适配器忽略目标、固定发往自己的 room
    /// （`gateways/matrix/adapter.rs`）。只有需要改写的 secondary（TG + SimpleX）才设置它。
    secondary_target: Option<TargetId>,
}

impl RoutingAdapter {
    pub fn new(primary: Arc<dyn BotAdapter>, secondary: Option<Arc<dyn BotAdapter>>) -> Self {
        Self {
            primary,
            secondary,
            secondary_target: None,
        }
    }

    /// 覆盖 secondary 的发送目标；未设置时沿用调用方传入的 target。
    pub fn with_secondary_target(mut self, target: TargetId) -> Self {
        self.secondary_target = Some(target);
        self
    }
}
```

把 `send_message` 的 `match` 分支（原 `routing.rs:41-46`）替换为：

```rust
        match &self.secondary {
            Some(secondary) if is_sensitive(&content.text) => {
                let secondary_target = self.secondary_target.as_ref().unwrap_or(target);
                secondary.send_message(secondary_target, content).await
            }
            _ => self.primary.send_message(target, content).await,
        }
```

- [ ] **Step 4: 运行测试确认通过（含既有 3 个兼容用例）**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test routing
```

期望：`routing` 相关用例（含新增 2 个与既有 3 个）全部 PASS。

- [ ] **Step 5: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git add rust/aegis/src/common/routing.rs
git commit -m "feat(aegis): RoutingAdapter 支持 secondary 目标覆盖"
```

---

### Task 2: Rust `--tg-simplex` 选择、装配与 runtime 门禁

**Files:**
- Modify: `rust/aegis/src/main.rs:12`（import）、`:86-95`（适配器装配）、`:136-156`（决策表注释）、`:171-182`（flag 判定）、`:336-460`（`mod platform_selection_tests`）
- Modify: `rust/aegis/src/main/runtime.rs:232-240`（SimpleX 初始化门禁）

**Interfaces:**
- Consumes: `RoutingAdapter::with_secondary_target`（Task 1）
- Produces: `--tg-simplex` → `PlatformSelection { telegram: true, matrix: false, simplex: true }`

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/main.rs` 的 `mod platform_selection_tests` 内、`all_with_simplex_prefers_simplex` 之后追加：

```rust
    #[test]
    fn tg_simplex_flag_is_telegram_plus_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-simplex"]), false, false),
            Ok(sel(true, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-simplex"]), true, true),
            Ok(sel(true, false, true))
        );
    }

    #[test]
    fn tg_simplex_flag_wins_over_all_and_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--tg-simplex"]), true, true),
            Ok(sel(true, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex", "--tg-simplex"]), false, true),
            Ok(sel(true, false, true))
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test tg_simplex_flag
```

期望：两个用例 FAIL，`left: Ok(PlatformSelection { telegram: true, matrix: false, simplex: false })`（`--tg-simplex` 未被识别，回落到默认 telegram）。

- [ ] **Step 3: 实现 flag 判定**

在 `main.rs:174` 的 `let tg_only = ...` 之后追加一行：

```rust
    let use_tg_simplex = args.iter().any(|a| a == "--tg-simplex");
```

并在 `if use_simplex {` 这一行**之前**插入：

```rust
    // `--tg-simplex` 必须先于 `--simplex` 判定：两者同时给出时取「TG 主 + SimpleX」，
    // 沿用既有「按判定顺序首个命中者胜」的先例（如 `--all --simplex` 取 simplex）。
    if use_tg_simplex {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: false,
            simplex: true,
        });
    }
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test platform_selection_tests
```

期望：全部 PASS（含既有 `all_with_simplex_prefers_simplex`）。

- [ ] **Step 5: 更新决策表注释**

把 `main.rs` 中从 `/// | 显式 flag | has_matrix | has_simplex | 结果 |` 这一行开始，到
`/// SimpleX；无 token 时还会触发 \`Bot::new\` 的 panic。` 这一行为止的整段（原 `:140-156`，
即决策表 + `含 --discord` 说明 + 自动探测说明 + SimpleX 段落）替换为：

```rust
/// | 显式 flag | has_matrix | has_simplex | 结果 |
/// |---|---|---|---|
/// | 无 | 否 | 否 | telegram |
/// | 无 | 是 | 否 | matrix |
/// | 无 | 否 | 是 | simplex |
/// | 无 | 是 | 是 | 错误（配置歧义） |
/// | `--matrix` | 任意 | 任意 | matrix（压制 simplex 自动启用） |
/// | `--simplex` | 任意 | 任意 | simplex（压制 matrix 与 telegram） |
/// | `--tg-simplex` | 任意 | 任意 | telegram + simplex（TG 主，敏感内容落 SimpleX） |
/// | `--all` | 任意 | 任意 | telegram + matrix（永不包含 simplex） |
/// | `--tg-only` | 任意 | 任意 | telegram（不做自动启用） |
///
/// 含 `--discord` 的参数组合直接报错（平台已移除）。
///
/// 自动探测（无 flag）最多只启用一个平台：`matrix_*` 与 `simplex_*` 同时齐备时
/// 直接报错，而不是悄悄二选一。
///
/// SimpleX 作为**唯一**平台时必须关闭 Telegram：SimpleX 适配器会成为主适配器，而
/// Telegram dispatcher 用同一个 `state.adapter` 派发，二者同开会把 Telegram 回复发到
/// SimpleX；无 token 时还会触发 `Bot::new` 的 panic。`--tg-simplex` 是唯一允许二者
/// 共存的形式：此时 Telegram 仍是主适配器，SimpleX 只作为敏感内容落点，且目标被
/// 改写为 `simplex_admin_id`（见 `main.rs` 中适配器装配处）。
```

- [ ] **Step 6: 实现适配器装配**

先在 `main.rs:12` 把 `use aegis::common::{BotAdapter, MessageContent, TargetId};` 改为：

```rust
use aegis::common::{BotAdapter, MessageContent, RoutingAdapter, TargetId};
```

再把 `use anyhow::Result;` 改为：

```rust
use anyhow::{Context, Result};
```

最后把 `main.rs:86-95` 的整段 `let adapter = ...;` 替换为：

```rust
    let adapter = if let Some(handle) = simplex_handle.as_ref().filter(|_| !selection.telegram) {
        // 纯 SimpleX：raw 适配器即主适配器（行为与 SimpleX standalone 接入时一致）。
        handle.adapter.clone()
    } else if let Some(handle) = simplex_handle.as_ref() {
        // Telegram + SimpleX：Telegram 为主，敏感内容改投 SimpleX 管理员联系人。
        // SimpleX 适配器用 parse_chat_id(target) 解读目标（Matrix 适配器忽略目标），
        // 因此必须把 secondary 目标改写为 simplex_admin_id，否则会发往 TG 的 chat id。
        let primary = main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?;
        // connect_simplex 在装配之前已强制校验 simplex_port / simplex_admin_id 齐备，
        // 这里的 context 仅为防御性写法，正常路径不可达。
        let simplex_admin_id = app_config
            .decrypted
            .simplex_admin_id
            .context("启用 SimpleX 作为敏感内容落点时必须配置 simplex_admin_id")?;
        Arc::new(
            RoutingAdapter::new(primary, Some(handle.adapter.clone()))
                .with_secondary_target(TargetId(simplex_admin_id.to_string())),
        ) as Arc<dyn BotAdapter>
    } else {
        main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?
    };
```

- [ ] **Step 7: 收敛 runtime 的 SimpleX scheduler 初始化门禁**

把 `rust/aegis/src/main/runtime.rs:237-238` 的：

```rust
        let simplex_admin = state.simplex_admin_id();
        if let Some(admin_contact) = simplex_admin {
```

改为：

```rust
        let simplex_admin = state.simplex_admin_id();
        // TG + SimpleX 组合下 Telegram 分支已启动唯一一个 scheduler，此处必须跳过，
        // 否则定时通知与启动通知会各发两遍。事件循环不受影响，仍照常接收命令。
        if let Some(admin_contact) = simplex_admin.filter(|_| !enable_telegram) {
```

注意：不要改动下方的 `if simplex_admin != Some(msg.contact_id)` —— `simplex_admin` 是 `Option<i64>`（`Copy`），事件循环仍需要它。

同时把紧随其后的 `} else {` 改为 `} else if !enable_telegram {`（消息文案不动）：

```rust
        } else if !enable_telegram {
            log::error!(
                "SimpleX 未配置 simplex_admin_id，调度器与启动通知不会发送；\
                 请管理员先向 bot 发一条消息，从日志中取得 contactId 后写入配置"
            );
        }
```

原因：`filter` 会让条件在 TG + SimpleX 下取到 `None`，若仍走 `else`，健康部署每次启动都会打出一条
**假的**「未配置 simplex_admin_id」错误日志。加上 `!enable_telegram` 后，该日志仅在它陈述为真时
输出（`simplex_admin` 为 `None` 且 Telegram 未启用，即纯 SimpleX 且管理员联系人缺失），
TG 场景下由 Telegram 分支独占 scheduler，无需这条日志。

- [ ] **Step 8: 运行 Rust 四道门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

期望：`fmt` 无输出；`clippy` 无 warning；`nextest` 的 `Summary` 显示 `0 failed`（基线 890 passed / 1 skipped；Task 1 新增 2 个，本任务新增 2 个，应为 894 passed）；`doc` 全部 ok。

- [ ] **Step 9: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git add rust/aegis/src/main.rs rust/aegis/src/main/runtime.rs
git commit -m "feat(aegis): 新增 --tg-simplex 组合部署形态"
```

---

### Task 3: Go 非交互平台推导

**Files:**
- Modify: `go/installer/main.go:1506`（必填字段文案改常量）、`:1592` 之后（新增常量与纯函数）、`:1404-1421`（`installFromStdin` 推导）、`:1538-1548`（`installFromKeyVal` 推导）
- Modify: `go/installer/main_test.go`（新增 `TestPlatformForNonInteractive`）
- Modify: `go/installer/i18n/en.json`、`zh.json`、`ja.json`（新增 `install.platform_ambiguous_three`）

**Interfaces:**
- Consumes: 无
- Produces: `platformForNonInteractive(hasToken, hasMatrix, hasSimplex bool) (string, error)`，返回 `"tg"` / `"matrix"` / `"simplex"` / `"tg-matrix"` / `"tg-simplex"`

- [ ] **Step 1: 写失败测试**

在 `go/installer/main_test.go` 的 `TestServicePlatformForSetup` 之后追加：

```go
func TestPlatformForNonInteractive(t *testing.T) {
	tests := []struct {
		name                              string
		hasToken, hasMatrix, hasSimplex   bool
		want                              string
		wantErr                           bool
	}{
		{"telegram only", true, false, false, "tg", false},
		{"matrix only", false, true, false, "matrix", false},
		{"simplex only", false, false, true, "simplex", false},
		{"telegram+matrix", true, true, false, "tg-matrix", false},
		{"telegram+simplex", true, false, true, "tg-simplex", false},
		{"three platforms", true, true, true, "", true},
		{"no fields", false, false, false, "", true},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := platformForNonInteractive(tc.hasToken, tc.hasMatrix, tc.hasSimplex)
			if (err != nil) != tc.wantErr {
				t.Fatalf("platformForNonInteractive(%t, %t, %t) err = %v, wantErr = %t",
					tc.hasToken, tc.hasMatrix, tc.hasSimplex, err, tc.wantErr)
			}
			if got != tc.want {
				t.Fatalf("platformForNonInteractive(%t, %t, %t) = %q, want %q",
					tc.hasToken, tc.hasMatrix, tc.hasSimplex, got, tc.want)
			}
		})
	}
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go test ./... -run TestPlatformForNonInteractive
```

期望：编译失败，`undefined: platformForNonInteractive`。

- [ ] **Step 3: 新增 i18n 键（三个 locale）**

在 `go/installer/i18n/en.json` 的 `"install.mem_protect_ok"` 之后插入：

```json
  "install.platform_ambiguous_three": "Cannot deploy three platforms at once. Use Telegram + Matrix or Telegram + SimpleX, and remove one platform's fields from the payload.",
```

在 `zh.json` 的对应位置插入：

```json
  "install.platform_ambiguous_three": "不能同时部署三个平台。请改用 Telegram + Matrix 或 Telegram + SimpleX，并从载荷中移除其中一份平台字段。",
```

在 `ja.json` 的对应位置插入：

```json
  "install.platform_ambiguous_three": "3 つのプラットフォームを同時に導入できません。Telegram + Matrix または Telegram + SimpleX を選び、いずれか一方のプラットフォームのフィールドを削除してください。",
```

- [ ] **Step 4: 新增纯函数与共享常量**

在 `go/installer/main.go` 的 `servicePlatformForSetup` 函数之后插入：

```go
// missingPlatformFieldsMsg 由非交互安装的两条路径与 parseKeyVal 共用同一条文案。
const missingPlatformFieldsMsg = "缺少必填字段: 至少需要配置 Telegram (token/admin_id)、Matrix (matrix_homeserver) 或 SimpleX (simplex_port/simplex_admin_id) 之一"

// platformForNonInteractive 由非交互安装（stdin JSON / key=value）的字段存在性推导部署形态。
// 组合必须显式：只有 token 与 simplex_port 同时存在才是 tg-simplex；三个平台字段齐备
// 直接报错而不猜优先级，与 aegis 侧「配置歧义」的立场一致。
func platformForNonInteractive(hasToken, hasMatrix, hasSimplex bool) (string, error) {
	switch {
	case hasToken && hasMatrix && hasSimplex:
		return "", fmt.Errorf("%s", i18n.T("install.platform_ambiguous_three"))
	case hasToken && hasSimplex:
		return "tg-simplex", nil
	case hasToken && hasMatrix:
		return "tg-matrix", nil
	case hasToken:
		return "tg", nil
	case hasSimplex:
		return "simplex", nil
	case hasMatrix:
		return "matrix", nil
	default:
		return "", fmt.Errorf("%s", missingPlatformFieldsMsg)
	}
}
```

- [ ] **Step 5: 接入 stdin 路径**

把 `go/installer/main.go` 中 `installFromStdin` 里的这段（约 `:1404-1421`）：

```go
	platform := "tg"
	simplexPort, _ := inputData["simplex_port"].(string)
	// Discord 平台已移除，按「键存在」即拒绝（不只看非空值）：手写 payload 里带一个空的
	// discord_token 也必须失败，不能静默落到 tg 默认平台。
	if _, ok := inputData["discord_token"]; ok {
		printRed(i18n.T("install.discord_removed"))
		os.Exit(1)
	}
	if _, ok := inputData["simplex_port"].(string); ok {
		platform = "simplex"
	} else if _, ok := inputData["matrix_homeserver"].(string); ok {
		if token, ok := inputData["token"].(string); ok && token != "" {
			platform = "tg-matrix"
		} else {
			platform = "matrix"
		}
	}
```

替换为：

```go
	simplexPort, _ := inputData["simplex_port"].(string)
	// Discord 平台已移除，按「键存在」即拒绝（不只看非空值）：手写 payload 里带一个空的
	// discord_token 也必须失败，不能静默落到 tg 默认平台。
	if _, ok := inputData["discord_token"]; ok {
		printRed(i18n.T("install.discord_removed"))
		os.Exit(1)
	}
	// 组合由字段组合显式决定：token + simplex_port 即 tg-simplex。
	// 判定用「非空」而非「键存在」：显式传空的 simplex_port 不再被当成 SimpleX 部署。
	token, _ := inputData["token"].(string)
	matrixHS, _ := inputData["matrix_homeserver"].(string)
	platform, err := platformForNonInteractive(token != "", matrixHS != "", simplexPort != "")
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}
```

- [ ] **Step 6: 接入 keyval 路径**

把 `go/installer/main.go` 中 `installFromKeyVal` 里的这段（约 `:1538-1548`）：

```go
	platform := "tg"
	if cfg.Token == "" {
		if cfg.SimplexPort != "" {
			platform = "simplex"
		} else if cfg.MatrixHS != "" {
			platform = "matrix"
		}
	}
	if platform == "tg" && (cfg.MatrixHS != "" || cfg.MatrixUser != "") {
		platform = "tg-matrix"
	}
```

替换为：

```go
	// matrix_username 与 matrix_homeserver 任一存在即视为配置了 Matrix（沿用既有口径）。
	platform, err := platformForNonInteractive(
		cfg.Token != "",
		cfg.MatrixHS != "" || cfg.MatrixUser != "",
		cfg.SimplexPort != "",
	)
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}
```

- [ ] **Step 7: 复用共享常量**

把 `parseKeyVal` 中的必填字段错误（约 `:1506`）：

```go
		return nil, fmt.Errorf("缺少必填字段: 至少需要配置 Telegram (token/admin_id)、Matrix (matrix_homeserver) 或 SimpleX (simplex_port/simplex_admin_id) 之一")
```

替换为：

```go
		return nil, fmt.Errorf("%s", missingPlatformFieldsMsg)
```

- [ ] **Step 8: 运行 Go 门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go fmt ./... && go test ./... && staticcheck ./...
```

期望：`go fmt` 无输出；`go test` 全部 `ok`；`staticcheck` 无输出。

> 覆盖说明：`installFromStdin` / `installFromKeyVal` 是含 `os.Exit` 与二进制下载的巨型
> 函数，无现有测试、也不在本计划中重构。两处 4 行接入的正确性（是否传入了正确的三个
> 布尔量）由编译期类型与代码审查保证；推导逻辑本身已被 Step 1 的
> `platformForNonInteractive` 单测覆盖。

- [ ] **Step 9: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/en.json go/installer/i18n/zh.json go/installer/i18n/ja.json
git commit -m "feat(installer): 非交互安装支持 telegram+simplex 组合"
```

---

### Task 4: Go 选择器与平台映射支持 tg-simplex

**Files:**
- Modify: `go/installer/main.go:365-410`（`Update` 的切换逻辑）、`:406-410`（`platformSelection`）、`:441-454`（`parsePlatformChoice`）、`:1561-1577`（`platformSetupForChoice`）、`:1579-1592`（`servicePlatformForSetup`）、`:1813-1823`（`platformFromService`）、`:1842-1851`（`platformFlagFor`）、`:1857-1863`（`writeSystemdService` 描述名）、`:1087-1089`（`deploySimplexService` 门禁）
- Modify: `go/installer/main_test.go`（改写 4 个旧契约测试 + 新增 3 个 + 扩展 3 个）
- Modify: `go/installer/i18n/{en,zh,ja}.json`（`firsttime.platform_prompt`、`firsttime.platform_text_prompt`）

**Interfaces:**
- Consumes: `platformForNonInteractive`（Task 3，本任务不使用）
- Produces: 平台标识字符串 `"tg-simplex"` → aegis flag `--tg-simplex`，且 `deploySimplexService` 对其生效

- [ ] **Step 1: 改写字面契约测试（RED）**

**1a.** 把 `TestPlatformSelectorSimplexIsStandalone` 整体替换为：

```go
func TestPlatformSelectorSimplexKeepsTelegram(t *testing.T) {
	m := newPlatformSelector()
	m.telegram = true
	m.cursor = 2
	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.simplex || !m.telegram || m.matrix {
		t.Fatalf("selecting simplex must keep telegram and clear matrix: %#v", m)
	}
}
```

**1b.** 把 `TestParsePlatformChoiceRejectsSimplexCombo` 整体替换为：

```go
func TestParsePlatformChoiceSimplexCombos(t *testing.T) {
	tg, matrix, simplex, err := parsePlatformChoice("telegram+simplex")
	if err != nil {
		t.Fatalf("telegram+simplex must be accepted: %v", err)
	}
	if !tg || matrix || !simplex {
		t.Fatalf("telegram+simplex expected, got tg=%t matrix=%t simplex=%t", tg, matrix, simplex)
	}
	if _, _, _, err := parsePlatformChoice("simplex+matrix"); err == nil {
		t.Fatal("parsePlatformChoice(\"simplex+matrix\") must be rejected: Matrix 与 SimpleX 互斥")
	}
}
```

注意：这里只接受 `telegram+simplex` 这一种 token 顺序。`parsePlatformChoice` 从不去重排序（既有代码同样
拒绝 `matrix+telegram`），本计划不引入顺序无关语义；`simplex+matrix` 被拒是因为组合本身非法，
与顺序无关。空格变体由 Step 6e 的 `"telegram + simplex"` 覆盖。

**1c.** 把 `TestPlatformSelectorSimplexIsExclusive` 的 `cases` 字面量替换为：

```go
	cases := []struct {
		name             string
		telegram, matrix bool
		simplex          bool
		wantValid        bool
	}{
		{"simplex alone", false, false, true, true},
		{"simplex with matrix", false, true, true, false},
		{"simplex with telegram", true, false, true, true},
		{"telegram+matrix+simplex", true, true, true, false},
		{"matrix alone", false, true, false, true},
		{"telegram+matrix", true, true, false, true},
		{"telegram alone", true, false, false, true},
	}
```

**1d.** 把 `TestPlatformSelectorTogglingSimplexClearsOthers` 整体替换为：

```go
func TestPlatformSelectorTogglingSimplexClearsMatrixOnly(t *testing.T) {
	m := newPlatformSelector()
	m.telegram = true
	m.matrix = true
	m.cursor = 2
	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.simplex || m.matrix || !m.telegram {
		t.Fatalf("selecting simplex must clear matrix and keep telegram: %#v", m)
	}
}
```

- [ ] **Step 2: 运行确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go test ./... -run 'TestPlatformSelectorSimplexKeepsTelegram|TestParsePlatformChoiceSimplexCombos|TestPlatformSelectorSimplexIsExclusive|TestPlatformSelectorTogglingSimplexClearsMatrixOnly'
```

期望：4 个用例 FAIL（选择器仍清空 telegram；`simplex+telegram` 仍被拒；`simplex with telegram` 仍判非法）。

- [ ] **Step 3: 实现选择器逻辑**

把 `Update` 里的 `case "space":` 整块替换为：

```go
	case "space":
		switch m.cursor {
		case 0:
			// Telegram 可作为主平台与 SimpleX 组合，因此不再清空 SimpleX。
			m.telegram = !m.telegram
		case 1:
			m.matrix = !m.matrix
			if m.matrix {
				m.simplex = false
			}
		case 2:
			m.simplex = !m.simplex
			if m.simplex {
				m.matrix = false
			}
		}
```

把 `platformSelection` 整体替换为：

```go
func (m platformSelector) platformSelection() (bool, bool, bool, bool) {
	// SimpleX 只允许与 Telegram 组合（TG 主 + SimpleX 敏感内容落点）；Matrix 与 SimpleX
	// 是两个互斥的独立平台，三者同选同样非法。
	valid := (m.telegram || m.matrix || m.simplex) && !(m.simplex && m.matrix)
	return m.telegram, m.matrix, m.simplex, valid
}
```

- [ ] **Step 4: 实现平台映射**

`parsePlatformChoice` 的 `case "telegram+matrix":` 之后插入：

```go
	case "telegram+simplex":
		return true, false, true, nil
```

`platformSetupForChoice` 的 `case "5":` 之后插入：

```go
	case "6":
		return true, false, true, nil
```

`servicePlatformForSetup` 整体替换为：

```go
func servicePlatformForSetup(tg, matrix, simplex bool) string {
	switch {
	case tg && simplex:
		return "tg-simplex"
	case simplex:
		return "simplex"
	case tg && matrix:
		return "tg-matrix"
	case tg:
		return "tg"
	case matrix:
		return "matrix"
	default:
		return ""
	}
}
```

`platformFromService` 的 `case bytes.Contains(service, []byte("--simplex")):` **之前**插入：

```go
	case bytes.Contains(service, []byte("--tg-simplex")):
		return "tg-simplex"
```

`platformFlagFor` 的 `case "simplex":` 之后插入：

```go
	case "tg-simplex":
		return "--tg-simplex"
```

`writeSystemdService` 的 `switch platform` 中、`case "simplex":` 之后插入：

```go
	case "tg-simplex":
		descName = "WWPS Telegram + SimpleX Bot"
```

`deploySimplexService` 的门禁行：

```go
	if platform != "simplex" {
		return
	}
```

替换为：

```go
	if platform != "simplex" && platform != "tg-simplex" {
		return
	}
```

- [ ] **Step 5: 更新 i18n 提示文案**

`firsttime.platform_prompt`（三个 locale）改为：

- en：`"Select platform (1=Telegram, 2=Matrix, 4=Telegram + Matrix, 5=SimpleX, 6=Telegram + SimpleX): "`
- zh：`"选择平台 (1=Telegram, 2=Matrix, 4=Telegram + Matrix, 5=SimpleX, 6=Telegram + SimpleX): "`
- ja：`"プラットフォームを選択 (1=Telegram, 2=Matrix, 4=Telegram + Matrix, 5=SimpleX, 6=Telegram + SimpleX): "`

`firsttime.platform_text_prompt`（三个 locale）改为：

- en：`"Select platform (Telegram, Matrix, Telegram+Matrix, SimpleX, Telegram+SimpleX): "`
- zh：`"选择平台 (Telegram, Matrix, Telegram+Matrix, SimpleX, Telegram+SimpleX): "`
- ja：`"プラットフォームを選択 (Telegram, Matrix, Telegram+Matrix, SimpleX, Telegram+SimpleX): "`

- [ ] **Step 6: 扩展既有映射测试**

**6a.** `TestPlatformFromService` 的 map 中追加（在 `--all` 行之后）：

```go
		"ExecStart=/etc/wwps/aegis/aegis --tg-simplex": "tg-simplex",
```

**6b.** `TestPlatformSetupForChoice` 的 map 中追加：

```go
		"6": {tg: true, simplex: true},
```

**6c.** `TestServicePlatformForSetup` 的切片中追加：

```go
		{tg: true, simplex: true, want: "tg-simplex"},
		{tg: true, matrix: true, simplex: true, want: "tg-simplex"},
```

**6d.** `TestWriteSystemdServiceSimplexUsesFlag` 的 map 中追加：

```go
		"tg-simplex": "--tg-simplex",
```

**6e.** `TestParsePlatformChoice` 的 map 中追加：

```go
		"telegram + simplex": {tg: true, simplex: true},
```

- [ ] **Step 7: 新增 TG 切换不清空 SimpleX 的测试**

在 `TestPlatformSelectorTogglingSimplexClearsMatrixOnly` 之后追加：

```go
func TestPlatformSelectorTogglingTelegramKeepsSimplex(t *testing.T) {
	m := newPlatformSelector()
	m.simplex = true
	m.cursor = 0
	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.telegram || !m.simplex || m.matrix {
		t.Fatalf("telegram toggle must not clear simplex: %#v", m)
	}
}
```

- [ ] **Step 8: 运行 Go 门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go fmt ./... && go test ./... && staticcheck ./...
```

期望：`go fmt` 无输出；`go test` 全部 `ok`；`staticcheck` 无输出。

- [ ] **Step 9: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/en.json go/installer/i18n/zh.json go/installer/i18n/ja.json
git commit -m "feat(installer): 平台选择器与单元映射支持 telegram+simplex"
```

---

### Task 5: 文档、i18n 文案与注释与新契约对齐

**Files:**
- Modify: `docs/2026-09-16-simplex-platform.md:89-113`
- Modify: `README.md:47`、`README.md:144-149`
- Modify: `go/installer/i18n/en.json`、`zh.json`、`ja.json`（`firsttime.simplex_desc1` / `firsttime.simplex_desc3`）
- Modify: `go/installer/main.go:1084`（`deploySimplexService` 注释）
- Modify: `go/installer/main_test.go`（重命名一个已失真的测试名）

**Interfaces:**
- Consumes: Task 2 的 flag 名与 Task 4 的平台标识
- Produces: 无（纯文字）

**为什么一个任务：** 本任务的主题是单一的——**所有文字（文档、用户可见 i18n、代码注释、测试名）都必须停止声称 SimpleX 是 standalone-only**。这些残留都是 Task 4 审查发现的同类项，合并处理比分成四次修正波更省。

- [ ] **Step 0: 修正已失真的 i18n 说明文案**

`firsttime.simplex_desc1` / `firsttime.simplex_desc3` 目前写的是「SimpleX 以独立平台方式运行」与「**不能**与 Telegram 或 Matrix 同时运行」，而它们在交互安装选中 SimpleX 时直接展示给用户，已被 `--tg-simplex` 证伪。三个 locale 全部替换（`desc2` 仍准确，不动）：

- en：`firsttime.simplex_desc1` → `"💡 SimpleX can run standalone (--simplex) or as the sensitive-content sink for Telegram (--tg-simplex)."`；`firsttime.simplex_desc3` → `"   Note: SimpleX cannot be combined with Matrix, and all three at once is not supported."`
- zh：`firsttime.simplex_desc1` → `"💡 SimpleX 可独立运行（--simplex），也可作为 Telegram 的敏感内容落点（--tg-simplex）。"`；`firsttime.simplex_desc3` → `"   注意：SimpleX 不能与 Matrix 组合，也不支持三者同时运行。"`
- ja：`firsttime.simplex_desc1` → `"💡 SimpleX は単独（--simplex）でも、Telegram の機密内容の宛先（--tg-simplex）としても動作します。"`；`firsttime.simplex_desc3` → `"   注意：Matrix との併用はできず、3 つ同時の実行もサポートされません。"`

注意：`ja.json` 全部使用 `\uXXXX` 转义（既有约定），新值必须按同样方式转义。

- [ ] **Step 0b: 修正已失真的代码注释**

`go/installer/main.go:1084` 的：

```go
// deploySimplexService 安装 simplex-chat 并写好它的 systemd 单元；非 simplex 平台是空操作。
```

改为：

```go
// deploySimplexService 安装 simplex-chat 并写好它的 systemd 单元；非 simplex / tg-simplex 平台是空操作。
```

- [ ] **Step 0c: 重命名已失真的测试名**

`TestPlatformSelectorSimplexIsExclusive` 已不再准确（SimpleX 现在可合法与 Telegram 组合，只是仍与 Matrix 互斥）。仅改函数名，不动任何用例体：

```go
func TestPlatformSelectorValidityMatrix(t *testing.T) {
```

- [ ] **Step 0d: 跑 Go 门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go fmt ./... && go test ./... && staticcheck ./...
```

期望：`go fmt` 无输出；两个包 `ok`（含跨 locale 键齐测试）；`staticcheck` 无输出。

- [ ] **Step 1: 重写 SimpleX 文档的「平台互斥语义」**

把 `docs/2026-09-16-simplex-platform.md` 中 `## 平台互斥语义` 一节（从该标题到「## 手工验收清单」之前）整体替换为：

```markdown
## 平台选择语义

一次启动运行**一种**部署形态：Telegram、Matrix、SimpleX，或两种组合之一
（Telegram + Matrix、Telegram + SimpleX）。**自动探测（无 flag）最多只启用一个平台**，
组合必须显式传 flag：

| 命令行 | config.enc 状态 | 结果 |
|---|---|---|
| 无 flag | 仅 `matrix_*` | Matrix |
| 无 flag | 仅 `simplex_*` | SimpleX |
| 无 flag | 两者都没有 | Telegram |
| 无 flag | 两者都有 | **启动失败**（配置歧义） |
| `--matrix` | 任意 | 仅 Matrix（不再自动启用 SimpleX） |
| `--simplex` | 任意 | 仅 SimpleX（同时关闭 Telegram 与 Matrix） |
| `--tg-simplex` | 任意 | Telegram + SimpleX（TG 主，敏感内容落 SimpleX） |
| `--all` | 任意 | Telegram + Matrix（永远不含 SimpleX） |
| `--tg-only` | 任意 | 仅 Telegram（不做任何自动启用） |
| `--discord` | 任意 | **启动失败**（平台已移除，错误信息给出迁移路径） |

要点：

- **SimpleX 独立运行时仍然强制关闭 Telegram**：此时 SimpleX 适配器是主适配器，而
  Telegram dispatcher 用同一个 `state.adapter` 派发，二者同开会把 Telegram 的回复发到
  SimpleX；无 token 时还会触发 `Bot::new` 的 panic。`--tg-simplex` 是唯一允许二者共存的
  形式，此时 Telegram 仍是主适配器。
- **`--tg-simplex` 的语义与 `--all` 对等**：Telegram 是控制入口，`is_sensitive` 命中的
  文本改投 SimpleX 管理员联系人（`simplex_admin_id`）。SimpleX 侧仍可发命令，回包回到
  SimpleX。scheduler 与启动通知全局只启动一个实例，不会重复发送。
- 敏感内容的判定与 `--all` 一致（`vmess://` / `vless://` / `trojan://` / `ss://` /
  `hysteria://` / `hysteria2://` / `tuic://` 以及 `"privateKey"` / `"secretKey"` /
  `"password":` 字段）。**仅转发文本**，文件下发仍走主平台。
- `matrix_*` 与 `simplex_*` **同时齐备**时，无显式 flag 会直接报错退出，而不是悄悄二选一。
  报错信息给出两条出路：显式传 `--matrix` 或 `--simplex`，或从 `config.enc` 中移除其中
  一份配置。迁移平台时请清掉旧平台字段。
- `--discord` 平台已**移除**：显式传入会在启动时硬失败，错误信息给出迁移路径（改用
  `--matrix` / `--simplex` / `--tg-simplex` / `--tg-only`，或从 systemd 单元中移除 `--discord`）。
- `--tg-simplex` 的判定排在 `--simplex` **之前**，因此 flag 冲突时 `--simplex --tg-simplex`
  与 `--all --tg-simplex` 都得到 Telegram + SimpleX（沿用 aegis 既有的「按判定顺序首个
  命中者胜」行为，未新增冲突校验）。

- [ ] **Step 1b: 修正文档头部对 Discord 的过期列举**

`docs/2026-09-16-simplex-platform.md:3` 仍把已移除的 Discord 列为受支持平台，与本任务新写的
章节自相矛盾。把该行中的「SimpleX 与前三个平台（Telegram / Matrix / Discord）的结构差异是」
改为「SimpleX 与 Telegram / Matrix 的结构差异是」（该行其余文字不动）：

```markdown
记录 aegis 第四个平台（SimpleX Chat）的部署结构、版本约束与手工验收清单。SimpleX 与 Telegram /
Matrix 的结构差异是：没有中心服务器 token，身份是**本机 `simplex-chat` 数据库里的联系人**，收发全部经由该进程暴露的 **WebSocket bot API**。因此部署多出一个常驻进程与一个 systemd 单元。
```
```

- [ ] **Step 2: 更新 README 平台表**

把 `README.md` 的平台表：

```markdown
| Platform | Flag | Notes |
| --- | --- | --- |
| Telegram | _(default)_ | |
| Matrix | `--matrix` | |
| SimpleX | `--simplex` | standalone; requires a local `simplex-chat` v7.0.0 — see [`docs/2026-09-16-simplex-platform.md`](docs/2026-09-16-simplex-platform.md) |
```

替换为：

```markdown
| Platform | Flag | Notes |
| --- | --- | --- |
| Telegram | _(default)_ | |
| Matrix | `--matrix` | |
| SimpleX | `--simplex` | standalone; requires a local `simplex-chat` v7.0.0 — see [`docs/2026-09-16-simplex-platform.md`](docs/2026-09-16-simplex-platform.md) |
| Telegram + Matrix | `--all` | sensitive messages are routed to the Matrix room |
| Telegram + SimpleX | `--tg-simplex` | Telegram is the control surface; sensitive messages are routed to the SimpleX admin contact |
```

- [ ] **Step 3: 更新 README 的敏感路由说明**

把 `README.md:47` 的：

```markdown
Follow the prompts to enter your Telegram Bot Token and Admin ID. Optionally configure Matrix for sensitive notification routing.
```

替换为：

```markdown
Follow the prompts to enter your Telegram Bot Token and Admin ID. Optionally configure Matrix or SimpleX for sensitive notification routing — the installer lets you pick Telegram, Matrix, SimpleX, Telegram + Matrix, or Telegram + SimpleX.
```

- [ ] **Step 4: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git add docs/2026-09-16-simplex-platform.md README.md go/installer/i18n/en.json go/installer/i18n/zh.json go/installer/i18n/ja.json go/installer/main.go go/installer/main_test.go
git commit -m "docs(simplex): 让文档、i18n 文案与注释对齐 telegram+simplex 契约"
```

---

### Task 6: 全量门禁与验收核对

**Files:**
- 无代码改动（纯验证；若发现缺陷，按严重程度回到对应 Task 修复）

**Interfaces:**
- Consumes: Task 1-5 的全部产出
- Produces: 可交付的验证证据

- [ ] **Step 1: Rust 四道门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt --check && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

期望：`fmt --check` 无输出（干净）；`clippy` 无 warning；`nextest` 的 `Summary` 为
`894 tests run: 894 passed, 1 skipped`（基线 890 passed + Task 1 新增 2 个 + Task 2 新增 2 个）；
`doc` 全部 ok。把 `Summary` 一行原样记录到完成报告里。

- [ ] **Step 2: Go 门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go fmt ./... && go test ./... && staticcheck ./...
```

期望：`go fmt` 无输出；`go test` 两个包 `ok`；`staticcheck` 无输出。

- [ ] **Step 3: 交叉验证管理员身份不变量（设计文档 §5.5）**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex_admin_id_is_recognized_as_admin
```

期望：1 个用例 PASS。该测试已存在于 `aegis/src/app/state.rs`，本计划不改动它 ——
组合形态依赖「管理员身份同时接受 `admin_id` 与 `simplex_admin_id`」这条不变量，此处只做确认。

- [ ] **Step 4: 交叉验证安装器 flag 契约**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex/go/installer
go test ./... -run 'TestPlatformFromService|TestPlatformFlagFor|TestServicePlatformForSetup|TestPlatformForNonInteractive' -v 2>&1 | grep -E '^(=== RUN|--- (PASS|FAIL)|ok|FAIL)' | head -40
```

期望：`--tg-simplex` 相关子用例全部 PASS，无 FAIL。

- [ ] **Step 5: 确认 i18n 三 locale 键齐**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
python3 -c "
import json
keys = ['install.platform_ambiguous_three','firsttime.platform_prompt','firsttime.platform_text_prompt']
data = {f: json.load(open('go/installer/i18n/%s.json' % f)) for f in ('en','zh','ja')}
missing = [(f,k) for f in data for k in keys if k not in data[f]]
assert not missing, missing
assert all('6=' in data[f]['firsttime.platform_prompt'] for f in data), '编号提示缺少 6'
assert all('Telegram+SimpleX' in data[f]['firsttime.platform_text_prompt'] for f in data), '文本提示缺少组合'
print('i18n OK:', keys)
"
```

期望：输出 `i18n OK: [...]`，无 AssertionError。

- [ ] **Step 6: 确认工作树干净、提交历史完整**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/tg-simplex
git status --short
git log --oneline main..HEAD
```

期望：`git status --short` 无输出（无未提交改动）；`git log` 显示 6 个提交（设计文档 1 + 本计划新增 5）。

- [ ] **Step 7: 报告**

输出完成报告，包含：四道 Rust 门禁的实际输出摘要（含 `Summary` 原文）、Go 门禁实际输出、上述交叉验证结果、以及设计文档 §11 中**无法在本机自动验证**的条目清单（需要真实主机与真实 Telegram/SimpleX 账号的部分），明确标注为「待人工验收」。

---

## 附：设计文档 §11 验收清单的人工项

以下条目依赖真实主机与真实平台账号，只能由人工在部署后核对，不由本计划的自动化步骤覆盖：

- Telegram 侧 `/menu` 可用，菜单正常渲染
- SimpleX 侧 `/menu` 有响应，回包落在 SimpleX
- 触发含 `vless://` 的导出（敏感）→ 只在 SimpleX 出现，TG 侧无
- 普通状态消息（启动通知）→ 只在 TG 出现，SimpleX 侧无
- `journalctl -u wwps-aegis` 中 scheduler 初始化只出现一次，定时通知不翻倍
- `systemctl status wwps-simplex` active，`ss -ltnp` 仅见 `127.0.0.1:<port>`
- 重跑安装器：平台从既有单元回读为 `tg-simplex`，不降级为 tg 或 simplex
