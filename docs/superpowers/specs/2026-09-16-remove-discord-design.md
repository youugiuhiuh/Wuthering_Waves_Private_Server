# Discord 平台移除设计（aegis / installer 全链路）

日期：2026-09-16
状态：待评审
关联：[SimpleX 平台接入设计](2026-09-16-simplex-platform-design.md)、[SimpleX 部署记录](../../2026-09-16-simplex-platform.md)

## 1. 背景与目标

aegis 的 `Platform` 枚举当前有四个成员：Telegram、Matrix、Discord、SimpleX。
Discord 是其中唯一没有实际运行记录、且以 `serenity` + `poise` 两套依赖为代价维持的
平台；其运行时接线里有大量 `#[allow(dead_code)]`（`has_discord_config`、
`register_slash_commands` 等在 `main.rs` 中已无调用点）。

目标：**全链路移除 Discord 平台**，覆盖三层：

1. aegis 运行时（`rust/aegis`，Rust）
2. 安装器（`go/installer`，Go：交互式平台选择器、配置解析、i18n）
3. `README.md` 的平台清单

非目标（明确不做）：

- 不修改 `Cargo.toml` 版本号（发布节奏不由本变更决定）
- 不改写历史文档：`docs/2026-09-16-simplex-platform.md` 与
  `docs/superpowers/{specs,plans}/2026-09-16-simplex-platform*.md` 中的
  Discord 引用是当时的事实记录，保留原样（本文档即为「Discord 何时被移除」的检索入口）
- 不动 `rust/aegis/src/resources/sni/*.pb`（其中的 `discord` 是域名词表，与集成无关）

## 2. 兼容性决策（本设计的核心）

现有部署可能同时带有两类 Discord 痕迹：systemd 单元里的 `--discord` 参数、
`config.enc` 里的 `discord_token` / `discord_admin_id` 密文字段。移除后不能出现
「用户以为还在跑 Discord，实际静默跑到了别的平台」。

### 2.1 `--discord` 参数 → 硬失败

`resolve_platform_selection` 在解析任何其它 flag 之前检查 `--discord`，命中即返回错误：

```rust
if args.iter().any(|a| a == "--discord") {
    return Err(
        "Discord 平台已移除，本版本不再支持 --discord。请改用 --matrix / --simplex / --tg-only，\
         或从 systemd 单元中移除 --discord。"
            .to_string(),
    );
}
```

调用点已有 `map_err(|e| anyhow::anyhow!("❌ {e}"))?`，因此错误串内不重复加 ❌。

**为什么不用「静默忽略」**：忽略 `--discord` 会让这次启动落到「无 flag → 自动探测」
分支，从而可能连上 Telegram 或 Matrix，属于静默换平台。硬失败是安全面行为。

### 2.2 `config.enc` 的 Discord 字段 → 直接删除

`EncryptedConfig` 每个字段都带 `#[serde(default)]`，且结构体未启用
`deny_unknown_fields`。因此从结构体中删除 `discord_token` / `discord_admin_id` 后，
**含这两个字段的老 `config.enc` 仍能正常反序列化**（未知字段被 serde 忽略），
老部署升级后可继续启动；下次 bootstrap 重写配置时这两个密文自然消失。

该承诺必须由一条回归测试固定（见 §8.2），不能只靠「serde 的行为我记得是这样」。

### 2.3 installer keyval 的 Discord 字段 → 硬失败

`parseKeyVal` 的 `default` 分支只打印 `keyval.unknown_field` 黄色警告并忽略。若把
`discord_token` / `discord_admin_id` 交给 default 处理，用户会带着「配置已生效」的
错觉完成安装。因此显式匹配这两个 key 并返回错误。

### 2.4 installer 平台编号 → 保留空洞

`platformSetupForChoice` 当前为 `1=telegram, 2=matrix, 3=discord, 4=tg+matrix, 5=simplex`。

决策：**保留编号空洞**，`case "3"` 返回「Discord 平台已移除」错误，`4` / `5` 不动。
理由：重编号会让未修改的旧脚本传 `"3"` 时从 Discord **静默变成 SimpleX**；保留空洞则
旧脚本明确失败。

## 3. Rust 侧改动

### 3.1 整文件删除

- `rust/aegis/src/gateways/discord/mod.rs`
- `rust/aegis/src/gateways/discord/adapter.rs`
- `rust/aegis/src/main/discord.rs`（含 `DiscordRawHandle`、`DiscordHandle`、
  `DiscordHandler`、`parse_slash`、`register_slash_commands`、`connect_discord`、
  `build_handle` 及 5 个测试）
- `rust/aegis/src/gateways/mod.rs` 的 `pub mod discord;`
- `rust/aegis/src/main/mod.rs` 的 `pub mod discord;`

### 3.2 逐点修改

| 文件 | 改动 |
|---|---|
| `src/common/trait.rs` | 删 `Platform::Discord` 枚举成员与 `PlatformCapabilities::DISCORD` 常量 |
| `src/main/config.rs` | `DecryptedConfig` 删 `discord_token` / `discord_admin_id`；删两段解密逻辑；删 `Ok((AppConfig { .. }))` 构造处的两个字段；测试字面量同步 |
| `src/bootstrap.rs` | `EncryptedConfig` 删两字段；`Drop for EncryptedConfig` 删两处 `zeroize`；`SetupInput` 删两字段；`run_setup` 删两参数；删 `discord_config_fields_round_trip`（simplex 往返测试覆盖同一路径）；其余测试字面量同步 |
| `src/app/state.rs` | `AppState` 删 `discord_admin_id` 字段；`AppState::new` 删第 2 个参数；`is_admin_user` 删该分支；删 `discord_admin_id_is_recognized_as_admin` 测试 |
| `src/main/runtime.rs` | `run()` 删 `discord_raw` 参数；删整个 `── Discord 网关 ──` 块；删 `discord_enabled` 并简化 SimpleX 保活条件为 `simplex_enabled && !enable_telegram && !enable_matrix` |
| `src/main.rs` | 删 `discord_raw` 构建、adapter 优先级中的 Discord 分支、`AppState::new` / `runtime::run` 的 discord 实参；`PlatformSelection` 删 `discord` 字段；`resolve_platform_selection` 加入 §2.1 硬失败并重写决策表注释（`--discord` 行删除，`--all` 行改为「永不包含 simplex」） |
| `src/main/matrix.rs`(3 处)、`src/main/simplex.rs`(1 处) | 测试字面量删 `discord_token` / `discord_admin_id` |
| `src/gateways/matrix/adapter.rs` | `assert_ne!(Platform::Matrix, Platform::Discord)` → 改用 `Platform::Simplex` |

### 3.3 依赖清理

```bash
cd rust/aegis
cargo remove serenity
cargo remove poise
```

`poise` 0.7 为 Discord 命令框架，全仓代码已无引用（仅 SNI 词表里出现同名词），
随 `serenity` 一并移除。`Cargo.toml` 中注释 `matrix-sdk/serenity` 改为 `matrix-sdk`。

按 `dependency-management` 技能要求，依赖增删只能通过 `cargo remove` 完成，
禁止手改 `Cargo.toml` 的依赖段。

## 4. Go installer 侧改动

| 位置 | 改动 |
|---|---|
| `platformSelector` 结构体 | 删 `discord` 字段 |
| `platformSelector.Update` | `cursor` 取模 `4 → 3`，`up` 用 `(cursor+2)%3`；空格分支删 `case 2`（discord）并把 simplex 落到 index 2 |
| `platformSelector.platformSelection()` | 返回 `(tg, matrix, simplex, valid)`；规则 `(tg \|\| matrix \|\| simplex) && !(simplex && (tg \|\| matrix))` |
| `platformSelector.View()` | `labels` / `choices` 各减为 3 项 |
| `parsePlatformChoice` | 返回 4 值，删 `"discord"` 分支 |
| `selectDeploymentPlatforms` | 返回值同步为 4 值 |
| `platformSetupForChoice` | 删 `"3"` 的 discord 分支，改为返回「Discord 平台已移除」错误（§2.4） |
| `servicePlatformForSetup` | 删 `discord` 参数与 `case discord` 分支 |
| `buildSetupPayload` | 删 `discordToken, discordAdminID` 参数及两段 JSON 写入；3 个调用点同步 |
| `parseKeyVal` | 显式匹配 `discord_token` / `discord_admin_id` 并硬失败（§2.3）；删 `cfg.DiscordToken` / `cfg.DiscordAdminID`；必填字段错误串删 Discord 说明 |
| 两处 `platform = "discord"` 判定 | keyval 路径与 JSON 路径各删一处 |
| 交互式 `── Discord section ──` | 整块删除（含 token / admin id 两次 `readSecureInputStr` 与两处 warning 打印） |

## 5. i18n

`go/installer/i18n/{en,zh,ja}.json` 各删除 17 个键：

```
firsttime.platform_selector_discord
firsttime.discord_section
firsttime.discord_desc1 / _desc2
firsttime.discord_prompt_yn
firsttime.discord_token_title / _help_step1 / _help_step2 / _help_format / _prompt
firsttime.discord_admin_title / _help_step1 / _help_step2 / _help_format / _prompt
firsttime.discord_intent_warning
firsttime.discord_guild_warning
```

三语必须同步删除，保持键集一致（现有 i18n parity 检查会失败否则）。
同时检查 `firsttime.platform_selector_help` 等相邻文案是否提及 Discord，若有则改写。

## 6. 文档

- `README.md`：第 138 行平台清单删「Discord」，第 148 行平台表删 `| Discord | --discord | standalone |` 行
- 历史文档保持原样（§1 非目标）

## 7. 测试策略（TDD）

先写失败测试并确认失败，再改实现。

### 7.1 Rust

| 测试 | 位置 | 说明 |
|---|---|---|
| `discord_flag_is_rejected` | `src/main.rs` 测试模块 | 替换 `discord_flag_is_discord_only`；断言 `--discord` 返回 Err，且错误信息含「已移除」 |
| `discord_and_simplex_flags_is_error` | 同上 | 删除（被上一条覆盖） |
| 其余 `platform_selection_tests` | 同上 | `sel()` helper 去 discord 参数，逐条更新 |
| 平台互斥断言 | `src/gateways/matrix/adapter.rs` | 改用 `Platform::Simplex` |
| 老配置兼容回归 | `src/main/config.rs` 测试模块 | 见 §7.2 |
| 往返测试 | `src/bootstrap.rs` | 只保留 simplex 往返 |

### 7.2 兼容性回归测试（必须）

在 `src/main/config.rs` 测试模块中新增：直接写入**含 `discord_token` / `discord_admin_id`
字段的原始 JSON**（结构体已无这两个字段，故必须用 `serde_json::json!` 或手写字符串构造），
然后断言 `load_and_validate()` 成功返回。

这条测试是 §2.2 承诺的唯一证据；它同时守住「未来给 `EncryptedConfig` 加
`deny_unknown_fields` 会静默打断老部署升级」这一回归。

### 7.3 Go

`go/installer/main_test.go`：

- 平台互斥用例删除 discord 维度，`platformSelector` 相关断言改为 3 平台
- `platformSetupForChoice`：`"3"` 断言返回错误且信息含「已移除」
- `servicePlatformForSetup`：签名与用例同步
- `parseKeyVal`：删除 with-discord-fields 子测试，新增 discord key 硬失败子测试
- 原有 `ExecStart=... --discord` 用例改为断言不再被识别为 discord 平台

### 7.4 命令

```bash
# Rust
cd rust/aegis && cargo fmt && cargo clippy -- -D warnings && cargo test

# Go
cd go/installer && go fmt ./... && go test ./... && staticcheck ./...
```

## 8. 验收标准

1. `grep -rn 'discord\|Discord' rust/aegis/src go/installer --include='*.rs' --include='*.go' --include='*.json'`
   零命中（`.pb` 域名词表除外）
2. `cargo tree | grep -E 'serenity|poise'` 无输出；`cargo build` 通过
3. 二进制收到 `--discord` → 非零退出并打印明确中文提示（手工验证一次）
4. §7.2 的老配置兼容回归测试通过
5. `go test ./...` + `staticcheck` 通过，三语 i18n 键集一致
6. `README.md` 无 Discord 引用

## 9. 风险

| 风险 | 处置 |
|---|---|
| 签名变化面广（`AppState::new` / `runtime::run` / `run_setup` / `buildSetupPayload`） | 全部由编译器暴露，逐点修复；测试字面量集中在少数文件 |
| 老部署升级后启动失败 | §2.2 直接删字段 + §7.2 回归测试；`--discord` 硬失败给出可操作提示 |
| 未来给 `EncryptedConfig` 加 `deny_unknown_fields` 打断升级 | §7.2 回归测试会立刻变红 |
| 三语 i18n 键集漂移 | 现有 parity 检查 + §5 同步删除 |

## 10. 明确不做

- 版本号 / CHANGELOG 变更
- 历史文档改写
- 「telegram 选中但 token 为空」的额外加固（与本移除无直接关系，属未请求行为）
