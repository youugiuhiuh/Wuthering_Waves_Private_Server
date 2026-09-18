# P1–P5 修复设计（aegis 日志器 + onboarding 收尾）

> 状态：已批准，实施中 ｜ 日期：2026-09-18 ｜ 分支：`fix/onboarding-hardening`（基线 main = `b720f5e`）
> 来源：`HANDOFF.md` 第三节未决问题 P1–P5
>
> **前提变更**：PR #337 已 squash 合并进 main（`b720f5e`），原分支 `fix/simplex-onboarding` 正文
> 与 main 内容一致（`git diff main fix/simplex-onboarding` 为空）。本文档的行号引用均以 main = `b720f5e` 为准。
> 索引注记：CodeGraph 索引为合并前的**混合陈旧**快照（`deploySimplexService` 报 1086 = 旧 main `9d3a981`；
> `installAegis` 报 1330 = 新 main）。仅作符号导航，行号与「某逻辑不存在」类断言一律以工作树源码为准。

## 1. 问题（已本人复核到 file:line，非转述）

### P1 — aegis 从未安装日志器，139 处 `log::*` 全部是空操作

| 断言 | 证据 |
| --- | --- |
| 全仓无日志器接线 | `grep -rn 'set_logger\|set_boxed_logger\|impl log::Log' rust/aegis/src` → **0 命中**；`env_logger` 全仓只出现在 `examples/test_sni.rs:7` |
| `env_logger` 不是运行时依赖 | `rust/aegis/Cargo.toml`：`log = "0.4.34"` 在 `[dependencies]`；`env_logger = "0.11.11"` 在 `[dev-dependencies]` |
| 关键行是 `log::*` | `src/main/simplex.rs:49` `log::info!("SimpleX bot 地址: {address}")` 是 `record_address` 的第一条语句，而地址文件确实落盘 |
| 运维唯一取 contactId 的途径是 `log::warn!` | `src/main/runtime.rs:303` 的「未授权联系人 contactId={} 尝试发消息，已忽略」 |

`log` crate 在无 logger 时把 `max_level` 保持为 `Off`，宏在编译期短路为 no-op —— 139 处调用在生产二进制里**全部静默丢弃**。
本分支新写的安装器文案与部署文档却让运维 `grep 未授权联系人` 取 contactId，**指向一个运行时兑现不了的承诺**。
`--set-simplex-admin` 因此无值可填，onboarding 仍未闭环。

### P2 — `has_simplex_config` 的无 flag 探测路径漏掉「只有端口」

`src/main/simplex.rs:55-59`：
```rust
explicit || (encrypted_config.simplex_port.is_some() && encrypted_config.simplex_admin_id.is_some())
```
管理员留空在本分支已是**合法状态**（`--simplex` 允许无管理员启动，`src/main.rs:100-106` 只对 `--tg-simplex` 保留硬校验）。
但无 flag 时该合取项仍要求 `admin_id`，于是「只配端口 + 无 flag」不被识别为 simplex →
退化到 `(false, false)` 分支 → `telegram: true` → 无 token 时 `Bot::new` panic。
测试 `returns_false_when_only_port_present`（`simplex.rs:178`）正把这个错误行为钉死。

### P3 — `--set-simplex-admin=42` 静默退化为正常启动

`src/main/cli.rs:30-45` 对 `args[1]` 只做精确字符串匹配，`_ => None`。
等号形式不命中任何分支 → `try_cli_mode` 返回 `None` → `main.rs:36` 跳过 CLI 分支 → **正常启动 bot**。
aegis 已作为 systemd 服务运行时，这会起第二个实例、消费同一份 WS 事件、重复执行命令。
这是「未知参数一律放行」对**所有** flag 的共性隐患，本分支新增的 flag 新增了一类拼写错误面。

### P4 — 安装器交互式重跑无法重新配置

`go/installer/main.go:1348-1372`：`config.enc` 存在 → `recoveryPlatformForService` 从旧单元反推平台 → 打印 `install.config_exists` →
**`firstTimeSetup` 从不被调用**，`config.enc` 一字节不改。菜单（`main.go:2078-2098`）只有 install / uninstall / exit，没有「重新配置」。
想改 Telegram token / Matrix 凭据 / TOTP 只剩两条路，都不安全：
- 卸载重装：`uninstallPaths` 含 `/etc/wwps`，连 `simplex_store`（bot 身份与地址）一起删
- `--setup-keyval` / `--setup-stdin`：`run_setup`（`bootstrap.rs:253`）从零构造 `EncryptedConfig`，未传字段一律写 `None`，
  且缺 `totp_secret` 时安装器会重新生成 → 2FA 被静默轮换

（全量覆盖语义本身是**有意**的平台切换通道，本设计不改它。）

### P5 — 切平台时 `wwps-simplex` 单元不被清理

`go/installer/main.go:1143-1146`：`deploySimplexService` 对非 simplex 平台直接 `return`。
全文件 `simplexServiceName` 只有 4 处：常量声明 + `enable` + `restart` + 上面的判断，**没有任何 stop / disable**。
于是 simplex → tg 迁移后 `wwps-simplex.service` 仍 enabled 且在运行，开机自启，一直占着端口和 SQLite 库。

## 2. 目标

1. aegis 的 139 处 `log::*` 在生产二进制里真正可见（至少 Info 级），使 `SimpleX bot 地址:` 与 `未授权联系人 contactId=N` 能被 `journalctl` 读到。
2. 「只配 simplex_port + 无 flag」被正确识别为 simplex 平台，不再 panic / 退化。
3. 未知 `--flag` 不再静默启动第二个 bot 实例。
4. 交互式重跑安装器可以重新配置（默认保持现状，显式确认才重配）。
5. 平台不再包含 simplex 时，`wwps-simplex.service` 被停止并取消开机自启。

## 3. 非目标

- 不改 `run_setup` / `--setup` / `--setup-stdin` / `--setup-keyval` 的全量覆盖语义
- 不改 `--tg-simplex` 的硬校验（无管理员仍必须启动即失败）
- 不引入运行时日志依赖、不做日志轮转、不写日志文件（只走 stderr → journald）
- 不删 `simplex_store` / 数据目录（切换平台只停服务，不销毁身份）
- 不做 P6（README 断链、遗留工作树、simplex-chat 版本锁、明文 API key）
- 不改 `--discord` 的既有拒绝路径

## 4. 设计

### D1（P1）手写 stderr logger，零新增依赖

新增 `rust/aegis/src/main/logging.rs`（bin-only，`mod main` 只被 `src/main.rs` 引入，不进 lib，测试不会跑两遍）：

```rust
struct StderrLogger;
impl log::Log for StderrLogger {
    fn enabled(&self, _: &log::Metadata) -> bool { true }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

pub fn init_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level_from_env());
}

fn level_from_env() -> log::LevelFilter { /* WWPS_LOG=off|error|warn|info|debug|trace, 默认 Info */ }
```

`src/main/mod.rs` 加 `pub mod logging;`；`src/main.rs` 的 `main()` **第一条**语句改成 `main::logging::init_logger();`。

关键取向：
- **stderr 而非 stdout**：`main.rs:34` 的注释已经说明 stdout 必须干净 —— 安装器把 `--generate-totp-secret` 的整段 stdout 当成 TOTP 密钥。
  `Binary Integrity Hash:` 走 `eprintln!` 是既有先例，journald 会照收。
- **默认 Info**：139 处调用站点无法逐个人工分级；Info 保住了运维必需的两行，又不至于灌爆 journal。
- **`WWPS_LOG` 环境变量**：3am 排查要能调级别而不必重新编译或重发签名二进制（这是「校准旋钮」，不是配置项膨胀）。
  只认这一个变量，不引入 `RUST_LOG` 之类的外部约定。
- 不选 `env_logger`：为一个 15 行的 logger 给 `opt-level="z"` + `strip` + `obfstr` + anti-debug 的强化二进制增加运行时依赖不划算。
- 不选「只把 3 行关键日志改成 `eprintln!`」：另外 136 处仍然是死的，同类缺陷会复发。

**守卫测试**（这类「缺失型」缺陷审查与既有单测都看不见）：
```rust
#[test]
fn init_logger_makes_log_records_live() {
    init_logger();
    assert_ne!(log::max_level(), log::LevelFilter::Off, "没有安装日志器：所有 log::* 都会被静默丢弃");
    assert!(log::log_enabled!(log::Level::Info));
    assert!(log::log_enabled!(log::Level::Warn));
}
```
诚实边界：该测试证明 `init_logger` 有效，**不能**证明 `main()` 调用了它。后者靠 code review + 真实主机 journal 断言（§5）。

### D2（P2）去掉 `simplex_admin_id.is_some()` 合取项

```rust
explicit || encrypted_config.simplex_port.is_some()
```
同步改文档注释；测试 `returns_false_when_only_port_present` → `returns_true_when_only_port_present`。

**接受的后果（需明确记录）**：`token + simplex_port + 无 admin + 无 flag` 的配置，此前 `(false,false)` → Telegram only，
此后 `(false,true)` → **纯 SimpleX，Telegram token 被忽略**。这需要人工编辑过配置才会出现；
逃生通道是显式 `--tg-only`。选择接受而不是加「有 token 就不算 simplex」的启发式 ——
那正是 `resolve_platform_selection` 明确拒绝的「猜优先级」立场。

### D3（P3）`args[1]` 未知 `-` 前缀参数直接报错

`CliMode` 新增 `Unknown(String)`；`execute_cli_mode` 对应分支返回 `Err`（`main` 打印到 stderr 并以非零码退出）。

```rust
match args[1].as_str() {
    // ...既有 CLI 分支...
    "--simplex" | "--tg-simplex" | "--matrix" | "--all" | "--tg-only" | "--discord" => None,
    other if other.starts_with('-') => Some(CliMode::Unknown(other.to_string())),
    _ => None,
}
```

- 平台 flag 必须留在 `None`：`main.rs:199-203` 依赖它们穿透到 `resolve_platform_selection`。
- `--discord` 也留在 `None`，让既有那条带迁移指引的拒绝信息继续生效（`main.rs:191`）。
- **只校验 `args[1]`**：`--setup <token> <admin_id> <totp_secret>` 的 `args[2..]` 是定位参数，可能是任意 token 字符串。
- 合法但非 CLI 的 `--` 参数（`--tg-only`）与不存在但有专有报错的（`--discord`）都显式列出，
  剩余未知一律报错 —— 报错信息必须带用法，否则等于把静默启动换成了静默失败。

**部署影响评估**：`platformFlagFor`（`go/installer/main.go:1935-1947`）只会写 `--matrix` / `--simplex` / `--tg-simplex` / `--all`，
TG-only 写空串；`runCmdOutputBytes` 只用 `--generate-totp-secret`。既有部署不会因新校验启动失败。

### D4（P5）非 simplex 平台时停用并取消自启

`deploySimplexService` 的早退分支改为：

```go
if platform != "simplex" && platform != "tg-simplex" {
    disableSimplexServiceIfPresent() // 单元文件存在才调用，避免 systemctl 对不存在的单元报错污染输出
    return
}
```
`disableSimplexServiceIfPresent`：`os.Stat(simplexServiceFile)` 成功 → `systemctl disable --now wwps-simplex`（失败只 warn）。
**不删单元文件、不删数据目录** —— 切回 simplex 时仍可用，且不销毁 bot 身份。抽成 `usesSimplexService(platform) bool` 纯函数以便单测。

### D5（P4）交互式重跑时显式询问是否重新配置

`installAegis` 的 `configExists` 分支，在打印 `install.config_exists` 之后询问：

```
检测到已有配置（当前平台: <platform>）。
是否重新配置？将重新生成 TOTP 密钥并重新输入所有凭据（Telegram / Matrix / SimpleX）。[y/N]
```
- 默认 `N` → 保持现状（只重写单元文件并重启），**不改变既有行为**
- `y` → 调用 `firstTimeSetup(destPath)`，平台重新选择（因此也能切换平台，使 D4 的清理路径在交互模式下可达）

不新增菜单项：菜单 1 就是安装，重跑安装器即是入口，少一个入口少一处不一致。
抽 `shouldReconfigure(answer string) bool`（接受 `y`/`Y`/`yes`/`YES`）以便单测。
新增 i18n key 需同时补齐 `zh.json` / `en.json` / `ja.json`（`i18n_test.go` 有一致性约束）。

**明确不满足的部分**：这不是「定点改单个字段」。要无副作用地只改 Telegram token，
正确做法是再加一个 `aegis --set-tg-token`（复用 `set_simplex_admin_id` 的定点替换骨架）。
本设计不做 —— 它是新功能（strict / brainstorming 量级），且 P1–P5 的目标是让 onboarding 闭环，
不是造一套通用配置编辑器。

## 5. 验证

| 层 | 命令 / 断言 |
| --- | --- |
| Rust 本地门 | `cargo fmt` ｜ `cargo clippy --all-targets --all-features -- -D warnings` ｜ `cargo nextest run --cargo-profile fast-test`（基线 920 passed / 1 skipped，本设计预计 +7~10）｜ `cargo test --doc` |
| Go 本地门 | `go/installer` 下 `go fmt ./... && go vet ./... && go test ./...`（两包 ok） |
| 真实主机（P1 闭环，硬断言） | 重跑部署后 `journalctl -u wwps-aegis` 必须**出现** `SimpleX bot 地址:`；连一个非管理员联系人后必须出现 `未授权联系人 contactId=N` |
| 真实主机（P3） | `aegis --set-simplex-admin=42` 退出码非零且打印用法；`systemctl is-active wwps-aegis` 期间不出现第二个 aegis 进程 |
| 真实主机（P5） | 用 `--setup-keyval`（只给 token）从 simplex 切到 tg 后，`systemctl is-enabled wwps-simplex` 非 enabled 且 `is-active` 非 active |
| 回归对照 | P1 前 aegis 1.6.1 崩溃循环（`NRestarts` 递增）vs 本分支 `active` / `NRestarts=0` 不得回退 |

## 6. 任务拆分（供 writing-plans 展开）

| # | 问题 | 文件 | 依赖 |
| --- | --- | --- | --- |
| T1 | P1 logger | `src/main/logging.rs`(新)、`src/main/mod.rs`、`src/main.rs` | — |
| T2 | P2 | `src/main/simplex.rs` | — |
| T3 | P3 | `src/main/cli.rs` | — |
| T4 | P5 | `go/installer/main.go`(deploySimplexService) | — |
| T5 | P4 | `go/installer/main.go`(installAegis) + `i18n/{zh,en,ja}.json` | — |

冲突：T4 与 T5 同文件，必须串行；T5 依赖 T4 吗？不依赖，但两者都在 `main.go`。
T1/T2/T3 文件互不相交，可并行。**唯一真正的依赖是 T1 完成前 PR #337 的目标不成立**，因此 T1 优先。

## 7. 未解决 / 需人工裁决

1. P1 的 logger 默认级别：Info（本设计）还是 Warn？Info 会引入 matrix-sdk / teloxide 通过 `log` 的潜在噪声（未实测），
   代价是 journal 体积；Warn 会让现有 40+ 处 `log::info!` 仍是死的。
2. D5 的确认提示默认 `N`（安全）还是 `y`（方便）？本设计取 `N`。
3. ~~P1–P5 是否全部并入 PR #337~~ —— **已裁决**：PR #337 已合并，P1–P5 统一走新分支 `fix/onboarding-hardening`（基于 `b720f5e`）。
