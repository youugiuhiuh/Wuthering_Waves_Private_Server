# Tasks: WARP 模式开关 + 缺陷修复 + 双路出口探测器

> 属于 `feat/warp-scope-toggle` 分支。
> ⚠️ 同仓库 `tasks/todo.md` 属于 `custom-allowlist` 特性，不要覆盖。

**执行方式**: TDD（RED → GREEN → REFACTOR），每任务一提交
**测试命令（权威）**: `cargo nextest run --cargo-profile fast-test`
**工作目录**: `rust/aegis/`

## 进度

| 任务 | 状态 |
|---|---|
| T1 路径常量 + 探测骨架 | ✅ 5bb4255 |
| T6 UDP 探测 | ✅ 5bb4255 |
| T7 TCP 探测 | ✅ 5bb4255 |
| T8 4 态组合 + 并行 | ✅ 5bb4255 |
| T2–T5, T9–T15 | ⬜ 待做 |

**当前套件**: 1126 passed, 1 skipped（基线 1119）

---

## T1: ✅ DONE (commit 5bb4255) 路径常量 + 探测模块骨架

**Description**: `paths.rs` 加探测常量（Q1：走常量模块）。创建 `warp_probe.rs` 骨架与类型。

**验收标准:**
- [x] `paths::warp` 新增 `WG_RX_BYTES_PATH`、`UDP_PROBE_ENDPOINT`、`TCP_PROBE_ENDPOINT`、`TCP_PROBE_SNI`
- [x] `core/network/warp_probe.rs` 定义 `UdpVerdict` / `TcpVerdict` / `ProbeReport`
- [x] `network/mod.rs` 声明并 re-export
- [x] 常量测试断言字面值

**验证:** `cargo nextest run --cargo-profile fast-test paths::` + `cargo check --all-targets`

**依赖**: 无
**文件:** `src/core/paths.rs`、`src/core/network/warp_probe.rs`(新)、`src/core/network/mod.rs`

```rust
pub const WG_RX_BYTES_PATH: &str = "/sys/class/net/wg0/statistics/rx_bytes";
pub const UDP_PROBE_ENDPOINT: &str = "162.159.192.1:2408";
pub const TCP_PROBE_ENDPOINT: &str = "162.159.198.2:443";
pub const TCP_PROBE_SNI: &str = "consumer-masque.cloudflareclient.com";
```

---

## T2: `WarpScope` enum + 循环（TDD）

**Description**: 新增 `WarpScope { Domain, Global }`，与 `WarpMode` 正交。

**验收标准:**
- [ ] `next()`：Domain → Global → Domain
- [ ] `Default` = `Domain`
- [ ] `as_str()` 返回 i18n key（非硬编码文案）

**验证:** `cargo nextest run warp::`

**依赖**: 无
**文件:** `src/core/xray/warp.rs`、`src/core/xray/mod.rs`

**RED:**
```rust
#[test]
fn test_warp_scope_next_cycles() {
    assert_eq!(WarpScope::Domain.next(), WarpScope::Global);
    assert_eq!(WarpScope::Global.next(), WarpScope::Domain);
}
#[test]
fn test_warp_scope_default_is_domain() {
    assert_eq!(WarpScope::default(), WarpScope::Domain);
}
```

---

## T3: `WarpAccountConfig.scope` 字段（serde 兼容）

**Description**: 加 `#[serde(default)] pub scope: WarpScope`。

**验收标准:**
- [ ] 旧 JSON（无 `scope`）→ 反序列化成功且 `== Domain`
- [ ] 新 JSON 往返保真
- [ ] `register_account()` 设 `WarpScope::default()`
- [ ] 既有 `test_warp_account_config_*` 仍通过

**验证:** `cargo nextest run warp_api::`

**依赖**: T2
**文件:** `src/core/network/warp_api.rs`

**RED:**
```rust
#[test]
fn test_scope_defaults_when_absent_legacy_file() {
    let legacy = r#"{"private_key":"k","public_key":"p","address_v4":"1.2.3.4",
                     "address_v6":"::1","reserved":[1,2,3],"client_id":"id"}"#;
    let cfg: WarpAccountConfig = serde_json::from_str(legacy).unwrap();
    assert_eq!(cfg.scope, WarpScope::Domain);
}
```
⚠️ `WarpScope` 需 `Serialize + Deserialize`。

---

## T4: `build_routing_rules(scope, rules)`（Q5：配置项控制）

**Description**: 抽出 rules 构建函数，支持两态。配置项 `WARP_SCOPE_ENFORCE`（默认 `false`）控制是否生成 catch-all。

**验收标准:**
- [ ] `Domain` → 现有行为（domain 规则）
- [ ] `Global` → catch-all `{"outboundTag":"warp"}`（**无 domain 键**）
- [ ] 空规则 + Domain → 只产出 warp-in 规则
- [ ] 默认 false → 实际仍为 Domain 行为
- [ ] 无 dead_code（clippy 通过）

**验证:** `cargo nextest run warp::` + `cargo clippy -D warnings`

**依赖**: T2
**文件:** `src/core/xray/warp.rs`

---

## T5: 签名变更 + Q2 跳过 reload

**Description**: `get_*` 加 `WarpScope`；`update_*` 加 `scope_only: bool`。

**验收标准:**
- [ ] `get_warp_routing_rules() -> (Vec<String>, WarpMode, WarpScope)`
- [ ] `update_warp_routing_rules(rules, mode, scope, scope_only)`
- [ ] `scope_only == true` → **不调用** `reload_core()`
- [ ] `scope_only == false` → 行为不变
- [ ] 6 处调用点全部适配

**验证:** `cargo nextest run warp::` + `cargo check --all-targets`

**依赖**: T2, T3, T4
**文件:** `src/core/xray/warp.rs`、`src/shared/handlers/warp.rs`、`src/core/xray/installer.rs`

---

## T6: ✅ DONE (commit 5bb4255) UDP 探测（TDD）

**Description**: `probe_udp()` 发合法 WG 握手包，3s 内收到任何回应即 `Available`。

**验收标准:**
- [x] 包格式：首字节 `0x01`，总长 148
- [x] 收到回应 → `Available`；超时 → `Blocked`
- [x] async + timeout，不 panic
- [x] 测试覆盖两态

**验证:** `cargo nextest run warp_probe::`

**依赖**: T1
**文件:** `src/core/network/warp_probe.rs`

```rust
// WireGuard handshake initiation: type=1, reserved=0, 4B sender_idx, 144B random = 148
let mut pkt = vec![1u8, 0, 0, 0];
pkt.extend_from_slice(&[0u8; 4]);
pkt.extend_from_slice(&random_144);
```

---

## T7: ✅ DONE (commit 5bb4255) TCP 探测（TDD）

**Description**: `probe_tcp()` 连 `TCP_PROBE_ENDPOINT:443`，TLS 握手，检查 ALPN。

**验收标准:**
- [x] ALPN 含 `h2` → `AvailableH2`
- [x] 连接/握手失败 → `Blocked`
- [x] 连上但 ALPN 非 h2 → `OpenNoH2`
- [x] 测试覆盖三态

**验证:** `cargo nextest run warp_probe::`

**依赖**: T1
**文件:** `src/core/network/warp_probe.rs`

**注意**: 复用项目已有 `tokio-rustls` + `rustls`(ring) 与 `install_crypto_provider()`。

---

## T8: ✅ DONE (commit 5bb4255) 4 态结论 + 并行（Q3：并行 3s）

**Description**: `run_probe()` 用 `tokio::join!` 并行跑两路。

**验收标准:**
- [x] 总超时 3s（并行，非串行 6s）
- [x] 4 种组合各有结论文案 key
- [x] 穷举断言 4 态

**验证:** `cargo nextest run warp_probe::`

**依赖**: T6, T7
**文件:** `src/core/network/warp_probe.rs`

```
UDP ✅ + TCP ✅ → WgUsable
UDP ❌ + TCP ✅ → TcpOnly        ← 当前服务器处境
UDP ✅ + TCP ❌ → UdpOnly
UDP ❌ + TCP ❌ → FullyBlocked
```

---

## T9: Bug A — `install()` 保留既有配置（TDD）

**Description**: 重装前先读现有 `(rules, mode, scope)`，写回时保留。

**验收标准:**
- [ ] 已有配置 → install 后三者**不变**
- [ ] 无既有配置 → `Default` + 空规则 + `Domain`
- [ ] 测试覆盖两分支

**验证:** `cargo nextest run installer::` + `cargo nextest run self_destruct`（回归红线）

**依赖**: T3, T5
**文件:** `src/core/xray/installer.rs`

---

## T10: Bug B — `status()` 真实探测（TDD）

**Description**: 保留文件判定，新增读 `WG_RX_BYTES_PATH` 三态输出。

**验收标准:**
- [ ] 无文件 → 🔴 未注册
- [ ] 有文件 + `rx_bytes == 0` → 🟡 隧道未建立
- [ ] 有文件 + `rx_bytes > 0` 且**有增长** → 🟢 正常
- [ ] `rx_bytes > 0` 但**无增长** → 🟡（防"曾经通现在断"假阳性）
- [ ] 读文件失败不 panic

**验证:** `cargo nextest run installer::`

**依赖**: T1
**文件:** `src/core/xray/installer.rs`

---

## T11: handler — `a_warp_switch_scope`

**Description**: 新增回调，翻转 scope，走 `scope_only = true`（不重启核心）。

**验收标准:**
- [ ] 分支存在
- [ ] 调 `update_*(..., scope_only = true)`
- [ ] 成功 → `Redirect("m_warp")`
- [ ] 失败 → `answer_callback` 报错（不静默）

**验证:** `cargo nextest run handlers::warp`

**依赖**: T5
**文件:** `src/shared/handlers/warp.rs`

---

## T12: handler — `a_warp_probe`

**Description**: 新增探测器按钮，跑并行探测渲染 4 态。

**验收标准:**
- [ ] 分支存在
- [ ] 先 `answer_callback` 提示「探测中」
- [ ] 结果含 UDP/TCP 两行 + 建议文案
- [ ] 超时/失败有兜底

**验证:** `cargo nextest run handlers::warp`

**依赖**: T8
**文件:** `src/shared/handlers/warp.rs`

---

## T13: 菜单按钮 + i18n 三语

**Description**: `m_warp` 加「流量范围」「出口探测」按钮；补三语 key。

**验收标准:**
- [ ] 菜单显示当前 scope（标注未生效）
- [ ] 两按钮 callback data 正确
- [ ] `zh/en/ja` 三文件 key **完全一致**
- [ ] 无硬编码文案
- [ ] 现有键未被改动

**验证:** `cargo nextest run` 全绿 + grep 比对三语 key 集合

**依赖**: T11, T12
**文件:** `src/shared/handlers/warp.rs`、`src/resources/i18n/{zh,en,ja}.yml`

---

## T14: 全量质量门禁（rust-lint-format 强制）

**验收标准:**
- [ ] `cargo fmt` 无 diff
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 零警告
- [ ] `cargo nextest run --cargo-profile fast-test` ≥ 1119 通过
- [ ] `cargo test --doc` 通过

**依赖**: T1–T13
**文件:** —

---

## T15: code-review-and-quality 审查

**验收标准:**
- [ ] 无 Critical / High 未解决
- [ ] 交付物 1–4 全部落实
- [ ] 非目标未越界

**依赖**: T14
**文件:** —
