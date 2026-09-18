# SimpleX Onboarding 修复设计

> 状态：待实施 ｜ 日期：2026-09-18 ｜ 分支：`fix/simplex-onboarding`
> 决策已批准：D1 地址落盘文件 + 日志；D2 新增 `--set-simplex-admin`，不动 `run_setup`

## 1. 问题

三个独立缺陷让 SimpleX 平台的 onboarding 无法完成。

### 1.1 bot 地址没有任何出口

- `rust/aegis/src/main/simplex.rs`（全文 110 行）没有任何输出 bot 地址的语句
- `go/installer/main.go` 亦无：`simplex.*` 的 i18n key 只有 `download_start` / `installed` / `port_invalid` / `install_failed` / `service_failed` / `service_ok` / `unit_write_failed`
- 上游 `simploxide-client` 已提供 `Bot::address()`（`simploxide-client-0.14.0/src/bot/mod.rs:378`，落到命令 `/_show_address <userId>`），aegis 也持有该句柄（`SimplexHandle.bot`，被标了 `#[expect(dead_code)]`），但从未调用

**后果**：管理员无从得知 bot 的 SimpleX 地址，因此无法建立连接。

### 1.2 `simplex_admin_id` 没有可用的补填路径

- **首次安装时该值必然未知**：`firstTimeSetup`（询问 contactId）跑在 `deploySimplexService`（下载并启动 simplex-chat）之前
- **重跑安装器无法补填**：`installAegis()`（`go/installer/main.go:1301-1314`）在 `config.enc` 存在时跳过 `firstTimeSetup`，`runAegisSetup` 从不被调用，`config.enc` 一字节不改
- **菜单没有"重新配置"**：`go/installer/main.go:2019-2028` 只有 install / uninstall / exit
- **卸载也不可行**：`uninstallPaths` 含 `/etc/wwps`（`main.go:63`），会连 `simplex_store`（bot 身份与地址）一起删除
- **唯一能改写的路径是 `--setup-keyval` / `--setup-stdin`，但它是全量覆盖**：`run_setup`（`rust/aegis/src/bootstrap.rs:253`）从参数从零构造 `EncryptedConfig`，未传字段一律写 `None`
  - 只传 `simplex_admin_id` → `simplex_port` 被清空 → aegis 解密时报「缺少 simplex_port」退出
  - `tg-simplex` 部署下还会一并清空 `token` / `admin_id`
  - `totp_secret` 缺失时 `installFromKeyVal` / `installFromStdin` 会调 `generateTOTPSecret()` 重新生成 → 2FA 被静默轮换

**后果**：aegis 长期停在"未配置 simplex_admin_id"分支 —— 能收消息但只记日志、不派发，调度器与启动通知全部不发送。bot 实际不可用。

### 1.3 安装提示语在要求一个不可能的操作

`i18n/{zh,en,ja}.json` 的 `firsttime.simplex_admin_help_step1/2` 让用户"启动 simplex-chat，让管理员联系人主动连接 bot / 在 bot 日志中查看 contactId"。在提示出现的时刻，simplex-chat 尚未下载。

## 2. 目标

装完 SimpleX 部署后，管理员能在**不手工编辑加密配置、不损失任何其他配置字段**的前提下完成：取到 bot 地址 → 连接 bot → 取到自己的 contactId → 写入配置 → bot 生效。

## 3. 非目标

- 不放开 simplex-chat 版本锁（`MAX_SUPPORTED_VERSION = 7.0.0.99`，上游 `main` 亦未放开）
- 不修改 `run_setup` / `--setup` / `--setup-stdin` / `--setup-keyval` 的任何既有语义
- 不做切平台时 `wwps-simplex` 单元的 stop / disable 清理（独立缺陷）
- 不修 README 指向已删文档（`docs/2026-09-16-simplex-platform.md`，1.6.0 提交删除）的断链（独立缺陷，另开 rapid 提交）
- 不实现入站文件下载、群聊支持、bot 命令菜单

## 4. 设计

### 4.1 决策记录

| # | 决策 | 选择 | 否决的方案与理由 |
| --- | --- | --- | --- |
| D1 | 地址出口 | 落盘文件 + 日志 | ①只打日志：installer 需抓日志，耦合 i18n 文案；②installer 自连 WebSocket 查询：需给 Go 侧加 WebSocket 依赖 |
| D2 | 补填方式 | 新增 `--set-simplex-admin` | 把 `run_setup` 改成 merge 语义：merge 后**无法清空字段**，切平台时旧 `matrix_*` 残留会触发 aegis 的「配置歧义」启动错误 —— 比现状更糟 |

### 4.2 组件 A — aegis 暴露 bot 地址

**新增路径常量**（`rust/aegis/src/core/paths.rs`，`pub mod bot` 内，与 `KEY_FILE` 同块）：

```rust
pub const SIMPLEX_ADDRESS_FILE: &str = "/etc/wwps/aegis/simplex_address";
```

**新增私有辅助函数**（`rust/aegis/src/main/simplex.rs`）：

```rust
/// 以 0600 原子写入地址文件：tmp + fsync + rename。
fn write_address_file(path: &std::path::Path, address: &str) -> std::io::Result<()>
```

- 内容为 `{address}\n`
- 目标已存在时必须**截断写**（`create(true).truncate(true)`），否则短地址覆盖长地址会留尾巴
- tmp 路径用 `path.with_extension("tmp")`；`sync_all()` 后 `fs::rename`
- 权限必须**显式**设置两次：建 tmp 时带 `.mode(0o600)`，并在 `sync_all()` 之前再 `f.set_permissions(Permissions::from_mode(0o600))`。
  只靠建文件时的 `mode()` 不够：它对**已存在**的 tmp 不生效，而 `truncate(true)` 会复用该文件 —— 上次崩溃残留的 tmp 若是 0644，rename 出去的就是 0644。
- ⚠️ **不要照抄 `bootstrap.rs::clear_matrix_recovery_key` 的写法**：那一处用的是 `File::create`，落成默认 0644，靠调用方另行收紧。本文件必须自保证 0600。

**接线点**：`connect_simplex` 中 `connect().await` 成功、构造 adapter 之前：

```rust
match bot.address().await {
    Ok(address) => {
        log::info!("SimpleX bot 地址: {address}");
        let path = std::path::Path::new(aegis::core::paths::bot::SIMPLEX_ADDRESS_FILE);
        if let Err(e) = write_address_file(path, &address) {
            log::warn!("写入 SimpleX 地址文件失败（不影响 bot 运行）: {e}");
        }
    }
    Err(e) => log::warn!("读取 SimpleX bot 地址失败（不影响 bot 运行）: {e}"),
}
```

两条错误路径都只 `warn` —— 地址拿不到不应让 bot 起不来。`BotBuilder::connect()` 内部已走完 `setup_auto_accept`（`bot/mod.rs:217`），所以返回时 `auto_accept_with` 已确保地址存在。

`SimplexHandle.bot` 上的 `#[expect(dead_code)]` 保持不变（本组件用的是局部变量 `bot`）。

### 4.3 组件 B — 新增 `--set-simplex-admin`

**`rust/aegis/src/bootstrap.rs`** 新增（放在 `clear_matrix_recovery_key` 之后）：

```rust
/// 就地更新配置中的 simplex_admin_id，保留其余字段（含各自的密文）不变。
///
/// 与 run_setup 的区别：run_setup 从参数从零构造 EncryptedConfig，未传字段一律写 None；
/// 本函数以磁盘上的现有配置为底做定点替换，因此不会清空 simplex_port / totp_secret /
/// matrix_*，也不会轮换 TOTP。
///
/// 原子写（tmp + fsync + rename），与 clear_matrix_recovery_key 一致。
pub fn set_simplex_admin_id(config_dir: &Path, admin_id: i64) -> Result<()>
```

行为：

1. `admin_id <= 0` → `anyhow::bail!("contactId 必须是正整数，收到 {admin_id}")`
   （SimpleX 的 id 以 `NonZeroI64` 表示；`0` 会在 `gateways/simplex/adapter.rs` 的 `parse_chat_id` / `MessageId` 处理路径上被拒，那里为 `panic = "abort"` 的 release profile 做了防御）
2. 读 `config_dir.join(CONFIG_FILE)` 并 `serde_json::from_slice::<EncryptedConfig>`
3. `SecurityManager::new(&config_dir.join(KEY_FILE))?` 后 `security.encrypt(admin_id.to_string().as_bytes())?`
4. `encrypted_config.simplex_admin_id = Some(ciphertext)`
5. 原子写回，权限保持 0600
6. `println!("✅ SimpleX 管理员 contactId 已写入配置: {admin_id}")`

**`rust/aegis/src/main/cli.rs`**：

```rust
pub enum CliMode {
    Stdout(String),
    Setup { token: Option<String>, admin_id: Option<String>, totp_secret: Option<String> },
    SetupStdin,
    SetSimplexAdmin(Option<String>),
}

/// 解析 contactId 参数。正整数校验放在 bootstrap 层（那里才知道 id 的语义）。
pub fn parse_contact_id(raw: &str) -> anyhow::Result<i64> {
    raw.trim().parse::<i64>()
        .map_err(|e| anyhow::anyhow!("contactId 必须是整数，收到 {raw:?}: {e}"))
}

// try_cli_mode 内新增分支：
"--set-simplex-admin" => Some(CliMode::SetSimplexAdmin(args.get(2).cloned())),

// execute_cli_mode 内新增分支：
CliMode::SetSimplexAdmin(raw) => {
    let raw = raw.context("用法: aegis --set-simplex-admin <contactId>")?;
    let id = parse_contact_id(&raw)?;
    set_simplex_admin_id(&config_dir(), id)
}
```

**关键约束**：`main.rs:36` 是 `if let Some(mode) = try_cli_mode(&args) { return execute_cli_mode(mode).await; }` —— 返回 `None` 会**继续正常启动 bot**。因此畸形的 `--set-simplex-admin` 绝不能返回 `None`，必须返回 `Some(CliMode::SetSimplexAdmin(..))` 让 `execute_cli_mode` 以 `Err` 终止。上面"保留原始字符串、在执行期校验"的写法同时满足该要求，并与既有 `CliMode::Setup { token: Option<String>, .. }` 的风格一致。

新增 import：`use crate::bootstrap::{config_dir, run_setup, run_setup_from_stdin, set_simplex_admin_id};` 与 `use anyhow::Context;`。

### 4.4 组件 C — installer 打印地址

**`go/installer/main.go`** 新增：

```go
// simplexAddressFile 必须与 aegis 的 core/paths.rs::bot::SIMPLEX_ADDRESS_FILE 一致。
var simplexAddressFile = filepath.Join(installDir, "simplex_address")

// readSimplexAddress 读取并校验地址文件；不存在 / 空 / 仅空白视为未命中。
func readSimplexAddress(path string) (string, bool)

// pollSimplexAddress 有界轮询地址文件；attempts <= 0 视为不轮询。
func pollSimplexAddress(path string, attempts int, interval time.Duration) (string, bool)

// printSimplexOnboarding 在部署收尾后打印地址与后续步骤。平台不含 simplex 时为空操作。
func printSimplexOnboarding(platform string)
```

`pollSimplexAddress` 接收 `path` 参数而非直接用包级变量，是为了可测：测试用 `t.TempDir()`，不依赖 `/etc/wwps/aegis` 在测试机上存在。

`printSimplexOnboarding` 逻辑：

```go
if platform != "simplex" && platform != "tg-simplex" {
    return
}
if addr, ok := pollSimplexAddress(simplexAddressFile, 20, 500*time.Millisecond); ok {
    printGreen(i18n.T("simplex.address_ready", addr))
    printYellow(i18n.T("simplex.address_paste_hint"))
} else {
    printYellow(i18n.T("simplex.address_pending"))
}
printYellow(i18n.T("simplex.admin_fill_hint"))
```

**调用点两处**（两条安装路径都要覆盖），均紧随 `printGreen(i18n.T("install.success"))` 之后：

- `installAegis()` 末尾
- `finishDeploy()` 末尾

轮询是必需的：`systemctl restart` 对 `Type=simple` 单元立即返回，安装器打印 success 时 aegis 才刚 fork，尚未连接 WebSocket、地址文件还没生成。

**不改动 `platformForNonInteractive` / `parseKeyVal` 的校验**：`simplex_admin_id` 留空在两条非交互路径上本就合法（`parseKeyVal` 只要求 `token` / `matrixHS` / `SimplexPort` 之一非空）。

### 4.5 组件 D — i18n 文案

**`go/installer/i18n/{zh,en,ja}.json`** 新增 4 个 key：

| key | zh |
| --- | --- |
| `simplex.address_ready` | `✅ bot 地址: %s` |
| `simplex.address_paste_hint` | `   复制上面这一行，粘到 SimpleX 客户端的输入框里连接 bot（或发 /c <地址>）` |
| `simplex.address_pending` | `⏳ 暂未取到 bot 地址（aegis 可能仍在启动）。稍后执行：journalctl -u wwps-aegis \| grep "bot 地址"` |
| `simplex.admin_fill_hint` | `   连上后给 bot 发任意一条消息，再执行：aegis --set-simplex-admin <ID>（ID 见 journalctl -u wwps-aegis \| grep 未授权联系人）` |

改写 4 个既有 key（把"去启动 simplex-chat"改成"留空"）：

| key | 新 zh |
| --- | --- |
| `firsttime.simplex_admin_title` | `\n👤 SimpleX 管理员 contactId（首次安装请留空）` |
| `firsttime.simplex_admin_help_step1` | `  首次安装时 bot 还没启动，拿不到 contactId —— 直接回车留空即可。` |
| `firsttime.simplex_admin_help_step2` | `  安装完成后安装器会打印 bot 地址；连上并发送一条消息后，用 aegis --set-simplex-admin 补填。` |
| `firsttime.simplex_admin_prompt` | `请输入 SimpleX 管理员 contactId（留空即可，稍后补填）：` |

`en` / `ja` 同步翻译，语义一致。

## 5. 接口清单

| 符号 | 位置 | 签名 |
| --- | --- | --- |
| `SIMPLEX_ADDRESS_FILE` | `rust/aegis/src/core/paths.rs::bot` | `&str` |
| `write_address_file` | `rust/aegis/src/main/simplex.rs` | `fn(&Path, &str) -> std::io::Result<()>`（私有） |
| `set_simplex_admin_id` | `rust/aegis/src/bootstrap.rs` | `pub fn(&Path, i64) -> anyhow::Result<()>` |
| `parse_contact_id` | `rust/aegis/src/main/cli.rs` | `pub fn(&str) -> anyhow::Result<i64>` |
| `CliMode::SetSimplexAdmin` | `rust/aegis/src/main/cli.rs` | `SetSimplexAdmin(Option<String>)` |
| `simplexAddressFile` | `go/installer/main.go` | `var string` |
| `readSimplexAddress` | `go/installer/main.go` | `func(string) (string, bool)` |
| `pollSimplexAddress` | `go/installer/main.go` | `func(string, int, time.Duration) (string, bool)` |
| `printSimplexOnboarding` | `go/installer/main.go` | `func(string)` |

`src/bootstrap.rs` 同时编进 lib（`pub mod bootstrap`）与 bin（`mod bootstrap`），因此 `set_simplex_admin_id` 的测试会随两个 target 各跑一次 —— 与既有 bootstrap 测试行为一致。

## 6. 测试策略

### aegis

1. `set_simplex_admin_id` **保留所有其他字段**：写一份含 `token` / `totp_secret` / `matrix_*` / `simplex_port` 的 `EncryptedConfig`，调用后断言这些字段的字节**逐一不变**，且 `simplex_admin_id` 能解回传入值
2. `set_simplex_admin_id` 拒绝 `0` 与负数
3. `set_simplex_admin_id` 不留 `.tmp` 残留，且文件 mode 仍为 0600
4. `set_simplex_admin_id` 在 `config.enc` 缺失时返回 `Err`（不 panic）
5. `write_address_file` 内容为 `{addr}\n` 且 mode 0600
6. `write_address_file` 覆盖写：先写长内容再写短内容，结果不含旧内容尾巴
7. `write_address_file` 在 `<target>.tmp` **预先存在且权限为 0644** 时，结果文件仍为 0600（守住上面那个 `mode()` 对已存在文件不生效的坑）
8. `parse_contact_id`：`"42"` → 42；`" 42 "` → 42；`"abc"` / `""` → `Err`
9. `try_cli_mode(&["aegis", "--set-simplex-admin", "42"])` → `Some(SetSimplexAdmin(Some("42")))`
10. `try_cli_mode(&["aegis", "--set-simplex-admin"])` → `Some(SetSimplexAdmin(None))` —— **必须是 `Some`**，否则会退回正常启动
11. 既有 894 个测试保持通过（回归门）

### installer（Go）

12. `readSimplexAddress`：正常文件命中；含尾随空白 trim 后命中；空文件 / 仅空白 / 不存在 → 未命中
13. `pollSimplexAddress`：文件已存在 + `attempts=1` → 立即命中；文件不存在 + `attempts=1` → 未命中且不 panic
14. i18n 三个语言文件的 key 集合一致（若既有 i18n 包已有同类守卫测试则复用，否则新增一条 key 对齐断言）

## 7. 风险与权衡

| 风险 | 缓解 |
| --- | --- |
| 地址文件被非管理员读取 | 目录 `0700` + 文件 `0600`；内容只是连接链接，泄漏等价于泄漏邀请链接（该风险已在既有部署文档记录） |
| 10s 轮询延长安装时长 | 读到即提前返回；最坏 10s，且超时**不算失败**，不 `os.Exit` |
| `--set-simplex-admin` 依赖 key 文件 | 复用 `SecurityManager::new(&dir.join(KEY_FILE))`，与 `save_lang_to_config` 一致；key 缺失时返回 `Err` 并打印原因 |
| 文件路径两侧可能漂移 | `paths.rs` 与 `main.go` 各自加一行交叉引用注释 |

## 8. 验收标准

- [ ] `cargo nextest run` 全绿（≥894 + 新增）
- [ ] `cargo clippy --all-targets -- -D warnings` 无告警
- [ ] `cargo fmt --check` 通过
- [ ] `go test ./...` 全绿（含新增用例）
- [ ] `go vet ./...` 无告警
- [ ] 纯 simplex 部署下安装器打印出 `simplex:/...` 地址
- [ ] 未配置管理员时 aegis 仍正常启动（只 warn，不中断）
- [ ] 跑一次 `--set-simplex-admin 42` 后，`config.enc` 中 `simplex_port` 与 `totp_secret` 的密文逐字节不变
