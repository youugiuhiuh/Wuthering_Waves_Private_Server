# `rust/aegis` — rust-skills（265 规则）符合性审计

> **状态**：仅分析，未做任何代码修改
> **范围**：`rust/aegis/src/**/*.rs`，生产代码（已剥离 `#[cfg(test)]` 模块）
> **证据基线**：45k LoC / 104 源文件 / 600 个 pub item / 89 文件含测试 / aegis v1.7.2 (edition 2024)
> **方法**：CodeGraph（`codegraph_status` / `explore` / `impact` / `files`）+ 定向文件系统度量
> **规则来源**：[rust-skills](../.agents/skills/rust-skills/SKILL.md) 26 类别 / 265 规则，规则 ID 对应 `rules/<id>.md`
> **审计日期**：2026-03-10 / 分支 `main` / HEAD `cb556f9`

---

## 0. 工具与索引状态说明

| 项 | 状态 |
|---|---|
| `codebase-memory-mcp` CLI | ❌ 二进制未安装（仅残留 `~/.cache/codebase-memory-mcp`），按 AGENTS.md 回退规则改用 CodeGraph |
| CodeGraph 索引 | ⚠️ **2248 条引用因中断的索引任务未解析**，`caller`/`impact` 边不完整 —— 本报告依赖结论已用文件系统度量交叉验证 |
| CodeGraph 版本 | v1.6.0（v1.6.2 可用） |

---

## 1. 总体评级

| 优先级 | 类别 | 违规 | 判定 |
|---|---|---|---|
| CRITICAL | `own-` 所有权 | 0 | ✅ |
| CRITICAL | `err-` 错误处理 | 2 | ✅ **优秀** |
| CRITICAL | `unsafe-` Unsafe | 2 | ⚠️ 全局豁免绕过检查 |
| CRITICAL | `mem-` 内存 | 0 | ✅ |
| HIGH | `async-` Async/Await | 3 | 🔴 必修 |
| HIGH | `api-` API 设计 | 2 | ⚠️ |
| HIGH | `opt-` 编译优化 | 0 | ✅ 刻意取舍 |
| MEDIUM | `type-` 类型安全 | 2 | 🔴 |
| MEDIUM | `pat-` 模式匹配 | 2 | 🟡 |
| MEDIUM | `obs-` 可观测性 | 2 | 🟡 |
| MEDIUM | `num-` 数值安全 | 1 | 🟡 |
| MEDIUM | `perf-` 性能 | 1 | 🟢 观察 |
| MEDIUM | `doc-` 文档 | 2 | 🟡 |
| MEDIUM | `test-` 测试 | 0 | ✅ 良好 |
| LOW | `lint-` Lint | 3 | 🟡 |
| LOW | `proj-` 项目结构 | 2 | 🟡 |
| — | `trait-` 泛型设计 | 0 | ✅ |
| — | `const-` / `serde-` / `macro-` / `closure-` / `coll-` / `name-` | 0 | ✅ |

**最值得表扬**：`err-` 全类别近乎完美。生产代码仅 **19 `unwrap()` / 7 `expect()`**（45k LoC），`Box<dyn Error>` = **0**（`anti-` 明令禁止项），`thiserror::AppError` + `anyhow`(318) 分工清晰，完全符合 `err-thiserror-lib` / `err-anyhow-app`。这在 bot 类项目中相当罕见。

**最严重问题**：`async-no-lock-across-await` 违规 7 处。

---

## 2. 汇总表

| 类别 | 优先级 | 违规项 | 判定 |
|---|---|---|---|
| `async-` | HIGH | 🔴 3 | 必修 |
| `unsafe-` | CRITICAL | 🔴 2 | 必修 |
| `type-` | MEDIUM | 🔴 2 | 必修 |
| `err-` | CRITICAL | 🟡 2 | 建议 |
| `pat-` | MEDIUM | 🟡 2 | 建议 |
| `obs-` | MEDIUM | 🟡 2 | 建议 |
| `num-` | MEDIUM | 🟡 1 | 建议 |
| `api-` | HIGH | 🟡 2 | 建议 |
| `doc-` | MEDIUM | 🟡 2 | 可选 |
| `lint-` | LOW | 🟡 3 | 可选 |
| `proj-` | LOW | 🟡 2 | 可选 |
| `mem-` / `perf-` | MEDIUM/CRITICAL | 🟢 2 | 观察 |
| `own-` / `trait-` / `opt-` / `test-` | — | 🟢 0 | 符合 |

---

## 3. 必修项（🔴 8 项）

### `async-` — Async/Await（HIGH）

#### 🔴 A-1 `async-no-lock-across-await` / `anti-lock-across-await` — 7 处

| 文件 | 锁 | 上下文 |
|---|---|---|
| `core/security/ufw.rs` | `UFW_MUTEX.lock().await` | ×2，UFW 命令执行 |
| `core/xray/routing.rs` | `CONFIG_LOCK.lock().await` | ×2，配置读写 |
| `core/singbox/routing.rs` | `CONFIG_LOCK.lock().await` | ×2，配置读写 |
| `core/system/scheduler/mod.rs` | `SCHEDULER.lock().await` | 定时任务持久化 |

**危害**：临界区内若含 `std::fs` 写盘 / `Command::spawn` / HTTP（`routing.rs` 与 `scheduler` 正是如此），单次慢 IO 会阻塞 tokio worker 池中该子系统全部调用者。与 `async-tokio-runtime`（多线程 runtime）意图直接冲突。

**修复**：锁内仅做内存计算 / 克隆出所需数据 → 释放锁 → `await` IO → 必要时重新加锁回写。配合 `async-clone-before-await`（await 前先克隆 Arc/数据）。

**影响面**：4 文件，独立可测，**低风险**。→ 阶段 1

---

#### 🔴 A-2 `async-tokio-fs` — async 内同步文件 IO

```
core/system/maintenance.rs::ensure_singbox_rule_sets()
core/xray/config.rs::delete_all_configurations()
```

同步 `std::fs::*` 在 `async fn` 内直接阻塞 worker。改 `tokio::fs`，大文件操作用 `tokio::task::spawn_blocking` 包裹。

**影响面**：2 文件，低风险。→ 阶段 1

---

#### 🔴 A-3 `async-tokio-runtime` / `anti-lock-across-await` — async 上下文内 `block_on` 与 `thread::sleep`

```
core/system/monitor.rs          Handle::block_on(...)     ← async 内 block_on，tokio 明确禁止
core/security/acme.rs           std::thread::sleep(...)  ← 阻塞 worker 线程
```

`block_on` 在 async 上下文会**死锁或 panic**。`acme.rs` 属阻塞重试循环，应改 `tokio::time::sleep`。

**影响面**：2 文件，低风险；`monitor.rs` 需先确认调用点确在 async 上下文。→ 阶段 1

---

### `unsafe-` — Unsafe Code（CRITICAL）

#### 🔴 U-1 `lint-unsafe-doc` / `unsafe-safety-comment` — 全局豁免 unsafe 检查

```toml
[lints.rust]
unsafe_code = "allow"   # Cargo.toml
```

unsafe 分布：

| 文件 | unsafe 数 |
|---|---|
| `main/config.rs` | 7 |
| `core/security/acme.rs` | 5 |
| `bootstrap.rs` | 3 |
| `core/security/crypto.rs` | 3 |
| `core/xray/port_allocator.rs` | 1 |
| `core/xray/config.rs` | 1 |
| `core/security/firewall_scanner.rs` | 1 |
| `core/xray/kcp.rs` | 1 |
| `shared/destruct.rs` | 1 |
| `core/singbox/config.rs` | 1 |
| `main/runtime.rs` | 1 |

**危害**：clippy 完全失去 unsafe 兜底，导致 3 条 CRITICAL 规则同时失效 ——
`unsafe-safety-comment`（每块需 `// SAFETY:`）、`unsafe-minimize-scope`（unsafe 块最小化）、`unsafe-miri-ci`（CI 跑 miri）。

**修复**：`allow` → `warn`；逐个补 `// SAFETY: <不变量说明>`；确认无 `mem::uninitialized` / `mem::zeroed`（`unsafe-maybeuninit`）。→ 阶段 2

---

#### 🔴 U-2 `unsafe-safety-comment` / `anti-unwrap-abuse` — `acme.rs` unsafe 聚集

`core/security/acme.rs`：**2239 行 / 85 unwrap / 5 unsafe**，全项目最大 unsafe 聚集地，且是**证书签发路径**（安全敏感）。同时存在 450 行巨型函数 `generate_secure_batch_filename`（命名与行为不符，见 J-2）。

**修复优先级**：先于其他 unsafe 项。→ 阶段 2

---

### `type-` — 类型安全（MEDIUM）

#### 🔴 T-1 `type-no-stringly` / `anti-stringly-typed` — 回调协议完全无类型

`shared/handlers/mod.rs:48` `route_callback` 用 **39 个**字符串字面量分支：

```rust
if data.starts_with("sx_approve:") || data.starts_with("sx_reject:") { /* Approval */ }
if data == "m_log" || data.starts_with("l_") { /* Log */ }
// … 共 39 个分支，返回 Option<CallbackRoute>
```

**危害**：
- 拼写错误 → 静默落 `None`，**无编译期错误**
- 新增功能须同步改 3 处（路由表 / 各 `handle` 的 match / 测试）
- 违反 `pat-exhaustive-enum` 精神

**讽刺之处**：项目内**已有**正确模式 —— `CallbackRoute` enum、`ServiceAction`、`PortAllocData`、`IdentityAction`、`DestructMessageAction`、`HandlerAction`、`TimeoutStatus`。`route_callback` 返回 `CallbackRoute` 却在内部裸字符串判别。

**修复**（`api-parse-dont-validate`）：`CallbackEvent.data: String` 进入时立即 `parse()` 成 `enum CallbackData`（实现 `FromStr`，失败即显式错误），下游全部穷尽匹配。已有测试 `test_every_menu_button_data_is_routed`（`handlers/mod.rs`）可作回归护栏。

**影响面**：路由表 + 8 个 handler 的 match 分支，>3 文件 → **strict 模式，需 SPEC**。→ 阶段 3

---

#### 🔴 T-2 `api-newtype-safety` / `type-newtype-validated` — 裸 String 标识符

`chat_id_str` / `domain` / `user_id` 全部以 `String` 在 handler 间传递，语义不明的字符串可互换。

**19 处 fn 签名接受 `String` 而非 `&str`**（同时触发 `own-slice-over-vec` 的字符串版本 / `anti-string-for-str`）：

```
app/state.rs::begin_destruct            app/state.rs::start_warp_input
app/state.rs::insert_schedule_input     app/state.rs::start_security_file_input
app/state.rs::start_domain_input  ×2    common/markup.rs::render_markup_buttons
core/security/acme.rs::normalize_percent_escapes
core/security/acme.rs::initialize_process_scope
core/singbox/hysteria2.rs::new          …（共 19）
```

**修复**：`struct ChatId(String)` / `struct Domain(String)` newtype（`api-newtype-safety`），配合 `type-repr-transparent`。可增量：先加 newtype 不改调用点，再逐步收紧。→ 阶段 5

---

## 4. 建议改进项（🟡）

### `err-` — 错误处理（CRITICAL）

> 整体优秀，仅两点。

#### 🟡 E-1 `anti-unwrap-abuse` / `err-no-unwrap-prod` — 生产 unwrap 残留

| 文件 | unwrap | expect |
|---|---|---|
| `shared/handlers/xray.rs` | **5** | – |
| `gateways/matrix/commands.rs` | 3 | – |
| `shared/destruct.rs` | 2 | – |
| `core/security/firewall_scanner.rs` | 2 | – |
| `core/xray/routing.rs` | 1 | 1 |
| `core/singbox/routing.rs` | 1 | – |
| `shared/handlers/schedule.rs` | 1 | – |
| `main/runtime.rs` | 1 | – |
| `core/xray/port_allocator.rs` | 1 | – |
| `core/xray/kcp.rs` | 1 | – |
| `core/system/operations.rs` | 1 | – |
| `core/security/acme.rs` | – | 2 |
| `core/xray/config.rs` | – | 1 |
| `shared/handlers/message.rs` | – | 1 |
| **合计** | **19** | **7** |

`shared/destruct.rs` 是**自毁流程**（安全关键），2 处 unwrap 需优先审计。

#### 🟡 E-2 `err-context-chain` — 缺少 `anyhow::Context`

`anyhow` 用了 318 处但 `.context()` / `.with_context()` 罕见，错误链断裂导致线上诊断困难。叠加 `panic="abort"` + `strip=true`（无符号表）后排查雪上加霜。

---

### `pat-` — 模式匹配（MEDIUM）

#### 🟡 P-1 `pat-exhaustive-enum` — catch-all `_` 分支泛滥

```
shared/handlers/xray.rs        28 处   ← 巨型 match
gateways/matrix/commands.rs     9 处
core/xray/kcp_mask.rs           8 处
shared/handlers/singbox.rs      4 处
main/runtime.rs                 3 处
shared/destruct.rs              3 处
core/xray/warp.rs               2 处
shared/handlers/ops.rs          2 处
… 共 33 个文件
```

#### 🟡 P-2 `err-result-over-panic` — `unreachable!` 14 处

```
core/xray/config.rs          unreachable!("Kcp should use build_kcp_inbound")       ×2
core/xray/config.rs          unreachable!("Hysteria2 should use build_hysteria2_inbound")
core/xray/config.rs          unreachable!("Kcp should use generate_kcp_client_link instead")
core/xray/config.rs          unreachable!("Hysteria2 should use generate_hysteria2_client_link instead")
core/xray/routing.rs         unreachable!("unknown rule_type: {}", rule.rule_type)
core/singbox/routing.rs      unreachable!("unknown rule_type: {}", rule.rule_type)
shared/handlers/xray.rs      unreachable!("KCP uses separate UI flow")               ×3
shared/handlers/xray.rs      unreachable!("KCP uses separate batch handler")         ×3
shared/handlers/xray.rs      unreachable!("Hysteria2 uses its own handler")         ×3
main/matrix.rs               IdentityAction::TryRecovery => unreachable!()   ← 缺消息
```

多数是**正确的穷尽性断言**（编译器无法证明跨函数不变量）。但 `main/matrix.rs` 的 `unreachable!()` 缺消息，配合 `panic="abort"` 会直接杀进程。建议改返回 `Result` 或至少补诊断信息（`err-lowercase-msg`）。

---

### `obs-` — 可观测性（MEDIUM）

#### 🟡 O-1 `obs-tracing-over-log` — 无 tracing，println 残留

```
tracing:: 使用      =   0 处
log:: 宏调用        = 172 处
println!/eprintln!  =  31 处（非日志初始化文件）
    main/matrix.rs               12 处  ← 库里裸输出
    main/cli.rs                   6 处
    bootstrap.rs                  5 处
    core/system/maintenance.rs    2 处
    main.rs                       2 处
    main/adapter.rs               2 处
    core/security/firewall_scanner.rs 1 处
    core/security/self_destruct.rs    1 处
```

无 `#[tracing::instrument]` → **无法构建因果链**。对"远程管理服务器、崩溃后进程消失"的场景，日志是唯一取证手段，当前能力不足。

#### 🟡 O-2 `obs-structured-fields` — 日志字段内插

172 处以 `log::warn!("获取公网 IPv4 失败: {}", err)` 形式把值插进消息字符串，无法结构化检索。结合 O-1 应整体迁移。

---

### `num-` — 数值安全（MEDIUM）

#### 🟡 N-1 `num-cast-try-from` — 26 处窄化 `as` 转换

```
core/xray/config.rs             6      core/xray/xhttp.rs                3
core/security/acme.rs           3      core/xray/hysteria2.rs            2
main/runtime.rs                 2      core/xray/installer.rs            2
core/xray/kcp.rs                2      core/xray/port_allocator.rs       2
core/security/crypto.rs         1      core/xray/reality.rs              1
core/xray/warp.rs               1      gateways/telegram/adapter.rs      1
```

集中在 `core/xray/` —— **端口号、缓冲区长度、协议字段**。溢出即端口错配 / 缓冲区越界。改 `TryFrom` 或 `try_into().context()?`。

**优先级**：端口相关（`port_allocator` / `config` / `kcp`）优先。→ 阶段 5

---

### `api-` — API 设计（HIGH）

#### 🟡 A-4 `api-must_use` — `#[must_use]` 仅 4 处 / 468 个 `pub fn`

`let _ = validate(input);` 静默丢弃 `Result` 无警告。

#### 🟡 A-5 `api-builder-pattern` — 复杂构造无 builder

`AppState::new(...)` 7 个位置参数（含 3 个 `Option`）；链式 `with_simplex_repin` / `with_simplex_code_verified_for`。`api-builder-pattern` + `api-default-impl` 可改善。→ 阶段 5

---

### `doc-` — 文档（MEDIUM）

#### 🟡 D-1 `doc-all-public` / `doc-errors-section` — 覆盖率 ≈17%

600 个 pub item，仅 106 个带 `///`。**但注释质量实际很高** —— 集中在"为什么"而非"是什么"：

- `main.rs` 的 `resolve_platform_selection` 完整决策表（含 `--discord` 已移除的显式拒绝）
- `handlers/mod.rs:45-47` 的 OpenRC 假设警告
- `acme.rs` 证书流程说明
- `fast-test` profile 的 aws-lc-sys/cranelift 链接失败踩坑记录

缺 `# Errors` / `# Panics`（14 处 `unreachable!` 无任何标注）。

---

### `lint-` — Lint（LOW）

| ID | 问题 |
|---|---|
| L-1 `lint-deny-correctness` | `main.rs` 顶层仅 `#![allow(clippy::vec_init_then_push)]`，**无任何 `deny`**。建议 `#![deny(clippy::correctness)]` |
| L-2 `lint-rustfmt-check` | CI 需确认是否含 `cargo fmt --check` / `clippy -D warnings` / nextest |
| L-3 `unsafe-miri-ci` | 项目有 unsafe 但 CI 无 miri job |

**CI 工作流现状**：

```
.forgejo/workflows/   public-release.yml  sign-release.yml
.github/workflows/    build-test.yml  minisign-key-check.yml  repo-sync.yml
```

---

### `proj-` — 项目结构（LOW）

#### 🟡 J-1 `proj-msrv-declare` — 无 `rust-version`

edition 2024 隐含 MSRV 下限但未显式声明，CI 无法校验。

#### 🟡 J-2 `proj-mod-by-feature` — 文件粒度过粗

| 文件 | 行数 | 问题 |
|---|---|---|
| `shared/handlers/xray.rs` | **3313** | 28 处 `_ =>`，全项目最大文件 |
| `core/security/acme.rs` | 2239 | 5 unsafe + 85 unwrap + 450 行巨型函数 |
| `core/system/maintenance.rs` | 1492 | — |
| `core/xray/config.rs` | 1355 | `generate_secure_batch_filename` 450 行（**命名与行为不符**） |
| `app/state.rs` | 1328 | 49 个 pub fn，CodeGraph impact **扇出 208 符号** |
| `shared/handlers/singbox.rs` | 1300 | 单函数 `handle()` **1205 行** |
| `shared/handlers/menu.rs` | 845 | 单函数 845 行 |
| `shared/handlers/warp.rs` | 582 | 单函数 582 行 |

`shared/handlers/` 9406 LoC 中 singbox + menu + warp + xray 独占 6970。

**巨型函数 Top（近似行数）**：

```
1205  shared/handlers/singbox.rs:97   pub async fn handle()
 845  shared/handlers/menu.rs:68      pub async fn handle()
 582  shared/handlers/warp.rs:9       pub async fn handle()
 450  core/xray/config.rs:193         generate_secure_batch_filename
 423  shared/handlers/message.rs:48   pub async fn handle_message()
 410  shared/handlers/ops.rs:524      pub async fn run_one_click()
 299  shared/destruct.rs:89           pub async fn intercept_message()
 254  main/runtime.rs:164             fn extract_media_info()
 209  core/singbox/config.rs:352      pub async fn ensure_base_config()
```

---

## 5. 观察项（🟢，暂不动）

| 规则 | 现状 | 判断 |
|---|---|---|
| `mem-with-capacity` | `with_capacity` 11 vs `Vec::new()` 58 | 非热路径可接受。`anti-premature-optimize`：**先 profile 再改** |
| `mem-box-large-variant` | `KcpMask` 7 个堆分配变体、`CoreEvent` 4 个 | `match` 时可能栈拷贝，值得 benchmark 验证 |
| `opt-lto-release` / `opt-codegen-units` | 已开 LTO + `codegen-units=1`；`opt-level="z"` | **刻意取舍**（反取证体积最小化：`opt-level="z"` + `panic="abort"` + `strip=true` + `lto="thin"`），非疏漏 |
| `opt-target-cpu` | 未设 | 单机常驻部署，不追 CPU 通用性 |
| `unsafe-safety-comment` 中 `unsafe fn` | 无 `unsafe fn` 定义 | 全为 unsafe 块，补注释即可 |

---

## 6. 符合良好的类别（✅）

- **`own-` / `mem-`（CRITICAL）**：无滥用 clone / `&String` 参数泛滥。`Arc<AppState>` 跨 handler 传递干净。
- **`trait-` 设计**：`BotAdapter` 为 dyn-compatible（`gateways` 三适配器异构，符合 `trait-dyn-vs-generic`）；`SelfDestructExecutor` 用 `BoxFuture` 返回（等价 `async fn in trait`）。
- **`type-enum-states` / `pat-let-else`**：状态机建模出色 —— `DestructStep`、`DomainInputStep`、`TimeoutStatus`、`CallbackRoute` 均显式枚举，非法状态不可表示。
- **`unsafe-extern-block` / `unsafe-no-mangle-unsafe`**：edition 2024 迁移彻底，无裸属性遗留。
- **`test-` 全类别**：`test-tokio-async` + `test-descriptive-names` + `test-use-super` + Mock trait 模式（`SelfDestructExecutor` / `BotAdapter` 均有 mock impl）齐备；竞态测试 `stale_domain_provider_callback_is_rejected` 用 `Barrier` + `#[serial]` 并标注 nextest 隔离要求。
- **`obs-no-sensitive-data`**：日志未见明文密钥/TOTP 泄漏。

---

## 7. 修复顺序建议

```
阶段 1（并发正确性，4-6 文件）  A-1 锁跨 await → A-2 tokio::fs → A-3 block_on/sleep
阶段 2（安全边界，1-2 文件）    U-2 acme.rs unsafe → U-1 全局 warn + SAFETY 注释
阶段 3（类型安全，>3 文件）      T-1 回调枚举化【strict 模式，需 SPEC】
阶段 4（工程质量）              L-1 deny correctness → J-1 rust-version → O-1 tracing
阶段 5（可选）                  D-1 文档 → N-1 端口窄化 → T-2 newtype → A-4/A-5
```

**关键约束**（依 AGENTS.md）：

- **阶段 1 / 2**：影响面小、语义清晰 → `normal` 模式
- **阶段 3（T-1）**：触及路由协议 + 8 个 handler → **strict 模式**，必须先出 `SPEC.md` + `tasks/plan.md` 并经批准
- **阶段 4 的 O-1（tracing）**：改动面最广（172 处日志），建议单独评估 ROI；考虑到 `panic="abort"` + `strip=true` 已削弱取证能力，优先级可上调
- 所有阶段完成后按 `rust-lint-format` 执行强制门禁：
  ```bash
  cargo fmt
  cargo clippy
  cargo nextest run --cargo-profile fast-test
  ```

---

## 8. 参考

- 规则目录：`../.agents/skills/rust-skills/rules/`
- 简明规范：`../.agents/skills/rust-patterns/SKILL.md`
- 项目规范：[AGENTS.md](../../AGENTS.md)