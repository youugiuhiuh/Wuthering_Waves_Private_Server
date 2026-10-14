# Tasks: usque-tcp（WARP over TCP/443）

> 属于 `feat/usque-tcp` 分支。
> ⚠️ 同仓库 `tasks/plan.md` / `tasks/todo.md` 属于 `custom-allowlist`，不要覆盖。

**执行方式**: TDD（RED → GREEN → REFACTOR），每任务一提交
**测试命令（权威）**: `cargo nextest run --cargo-profile fast-test`
**工作目录**: `rust/aegis/`
**基线**: 1157 passed, 1 skipped

## 进度

| 任务 | 状态 | commit |
|---|---|---|
| T1 路径常量 + self_destruct | ✅ | c367888 |
| T2 pin 版本 + sha256 | ✅ | c367888 |
| T3 systemd unit 纯函数 | ✅ | c367888 |
| T4 sha256 校验 | ✅ | c367888 |
| T5 checksums 解析 | ✅ | c367888 |
| T6 出口引擎选择 | ✅ | c367888 |
| T7 探测缓存 TTL | ✅ | c367888 |
| T8 Xray socks outbound | ✅ | 33cf45e |
| T9 隧道健康判据 | ✅ | c367888 |
| T10 失败计数判死 | ✅ | c367888 |
| T11 安装状态探测 | ✅ | c367888 |
| T12 **成对卸载** | ✅ | c367888 |
| T13 enforce 改运行时 | ✅ | 33cf45e |
| T14 状态显示两套 | ✅ | 9ffad79 |
| T15 i18n 三语同步 | ✅ | 9ffad79 |
| T16 质量门禁 + 审查 | 🔄 | — |

---

## T1: usque 路径常量 + self_destruct 增项

**Description**: `paths.rs` 新增 `pub mod usque`，并把 `wwps-usque` 加入 `DESTRUCT_SERVICES`（决策 OQ4）。

**验收标准:**
- [x] `paths::usque` 含 `DIR` / `BIN` / `CONFIG_FILE` / `SERVICE_NAME` / `SERVICE_PATH` / `SOCKS_PORT`
- [x] `DESTRUCT_SERVICES` 含 `"wwps-usque"`
- [x] 常量测试断言字面值
- [x] 既有 `test_maintenance_paths` 仍通过（它断言 `DESTRUCT_SERVICES` 非空且含 `wwps-core`）

**验证:** `cargo nextest run paths::`

**依赖**: 无
**文件:** `src/core/paths.rs`

```rust
pub mod usque {
    pub const DIR: &str = "/etc/wwps/usque";
    pub const BIN: &str = "/etc/wwps/usque/usque";
    pub const CONFIG_FILE: &str = "/etc/wwps/usque/config.json";
    pub const SERVICE_NAME: &str = "wwps-usque";
    pub const SERVICE_PATH: &str = "/etc/systemd/system/wwps-usque.service";
    /// usque 的 SOCKS 监听端口（本地回环，供 Xray socks outbound 使用）
    pub const SOCKS_PORT: u16 = 1080;
}
```

---

## T2: pin 版本与 sha256（纯函数）

**Description**: 新建 `core/usque/pin.rs`，硬编码版本与 sha256（硬约束 5），提供从架构构造下载 URL 的纯函数。

**验收标准:**
- [x] `USQUE_VERSION` = `"4.2.1"`
- [x] `USQUE_SHA256_LINUX_AMD64` = `"4117e206..."`（64 位 hex）
- [x] `asset_name(arch) -> Option<String>` 纯函数：`x86_64` → `usque_4.2.1_linux_amd64.zip`；`aarch64` → `usque_4.2.1_linux_arm64.zip`；未知架构 → `None`
- [x] `download_url(asset) -> String` 拼正确 GitHub Release URL
- [x] `checksums_url() -> String`
- [x] 测试覆盖各架构 + 未知架构 + URL 形状
- [x] **断言 sha256 是 64 位小写 hex**（防手误粘贴截断的哈希）

**验证:** `cargo nextest run usque::pin`

**依赖**: 无
**文件:** `src/core/usque/mod.rs`（新）、`src/core/usque/pin.rs`（新）、`src/core/mod.rs`

**注意**: arm64 的 sha256 我**没有实测**。若 arch 为 arm64 而 pin 表里没有对应哈希，必须**明确失败**而不是跳过校验（安全优先）。

---

## T3: systemd unit 模板生成（纯函数）

**Description**: 新建 `core/usque/unit.rs`，把 unit 文本生成做成纯函数（便于断言），复用 `singbox/installer.rs:224` 的风格。

**验收标准:**
- [x] `service_unit(bin, config, port) -> String`
- [x] 含 `Restart=always`、`RestartSec=5`、`LimitNOFILE=51200`
- [x] `ExecStart` 含 **`socks --http2`**（硬约束 1 的机械保证）
- [x] `ExecStart` **不含** `l4-socks`
- [x] `-b 127.0.0.1`（不对外暴露 SOCKS）
- [x] `WantedBy=multi-user.target`
- [x] `[Unit]` 含 `After=network.target`
- [x] 测试断言以上全部（尤其「无 l4-socks」的防回归断言）

**验证:** `cargo nextest run usque::unit`

**依赖**: T1
**文件:** `src/core/usque/unit.rs`（新）

**RED（关键防回归测试）:**
```rust
#[test]
fn test_unit_uses_socks_http2_not_l4() {
    let u = service_unit("/etc/wwps/usque/usque", "/etc/wwps/usque/config.json", 1080);
    assert!(u.contains("socks --http2"), "必须用 socks --http2（TCP 模式）");
    assert!(!u.contains("l4-socks"), "l4-socks 无 --http2 支持，本机房必死，禁止使用");
}
```

---

## T4: sha256 校验（纯函数）

**Description**: `core/usque/pin.rs` 或独立 `verify.rs` 加哈希比较函数。

**验收标准:**
- [x] `verify_sha256(expected: &str, actual_hex: &str) -> bool` 大小写不敏感
- [x] 支持带 `sha256:` 前缀的输入（与既有 `parse_digest` 风格一致）
- [x] 长度不符直接 false（不 panic）
- [x] 测试：相等 / 不等 / 大小写 / 前缀 / 空 / 长度错误

**验证:** `cargo nextest run usque::`

**依赖**: T2
**文件:** `src/core/usque/pin.rs`

---

## T5: `checksums.txt` 解析（纯函数）

**Description**: 从 Release 的 `checksums.txt` 中提取指定资产的 sha256。

**验收标准:**
- [x] `parse_checksums(text, asset) -> Option<String>`
- [x] 格式为 `<hash>  <filename>`（两空格），兼容单空格与 tab
- [x] 找不到该资产 → `None`
- [x] 行含 `./` 前缀或路径前缀时仍能匹配（健壮性）
- [x] 测试覆盖多种格式 + 缺失资产

**验证:** `cargo nextest run usque::`

**依赖**: T2
**文件:** `src/core/usque/pin.rs`

**实测样例（真实 checksums.txt 片段）:**
```
4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313  usque_4.2.1_linux_amd64.zip
c88b061c2a567f30813d7505637d1fb6fe7ec5898b5b61fd05122409fa5ad925  usque_4.2.1_linux_arm64.zip
```

---

## T6: 出口引擎选择（纯函数）

**Description**: 新建 `core/usque/engine.rs`。`ProbeConclusion` → 出口类型。

**验收标准:**
- [x] `pub enum Egress { WireGuard, Masque, Direct }`
- [x] `select_egress(conclusion) -> Egress` 穷举 4 态：
  - `WgUsable` → `WireGuard`
  - `UdpOnly` → `WireGuard`
  - `TcpOnly` → `Masque`
  - `FullyBlocked` → `Direct`
- [x] 测试穷举 4 态（不是抽 2 个）
- [x] `Egress` 有 `is_warp_enabled()`（`Direct` 时 false）

**验证:** `cargo nextest run usque::engine`

**依赖**: 无（只依赖 Spec 1 的 `ProbeConclusion`）
**文件:** `src/core/usque/engine.rs`（新）

---

## T7: 探测缓存（TTL 10min，内存）

**Description**: 决策 OQ1/OQ2。内存缓存 + TTL。

**验收标准:**
- [x] 缓存 `ProbeReport`，TTL 10 分钟
- [x] `cached_or_probe()`：命中且未过期 → 返回缓存；否则跑 `run_probe()`
- [x] 过期判定用可注入的「当前时刻」以便测试（不依赖真实时钟）
- [x] 测试：未探测 / 命中 / 过期 / 强制刷新
- [x] **不在测试里打网络**（把 probe 做成可注入的闭包/trait）

**验证:** `cargo nextest run usque::engine`

**依赖**: T6
**文件:** `src/core/usque/engine.rs`

**设计提示**: 把 probe 抽成 `Fn() -> impl Future<Output = ProbeReport>` 便于注入假实现。

---

## T8: Xray `socks` outbound 生成（硬约束 6）

**Description**: `core/xray/warp.rs` 加分支：出口引擎为 `Masque` 时生成 `socks` outbound 而非 `wireguard`。

**验收标准:**
- [ ] `masque_outbound(addr, port) -> Value` 产出**扁平**形状：
      `{"tag":"proxy-warp","protocol":"socks","settings":{"address":"127.0.0.1","port":1080}}`
- [ ] **断言 `settings` 下直接有 `address`/`port`**，且**没有** `servers` 数组（硬约束 6）
- [ ] tag 仍为 `proxy-warp`（保持与既有 routing / freedom dialerProxy 兼容）
- [ ] 不含 `secretKey` / `reserved` 等 WireGuard 字段
- [ ] `build_routing_rules` 行为不变（`outboundTag` 仍指 `warp`）
- [ ] Spec 1 的既有测试全通过

**验证:** `cargo nextest run xray::warp`

**依赖**: T6
**文件:** `src/core/xray/warp.rs`

**RED:**
```rust
#[test]
fn test_masque_outbound_is_flat_socks_shape() {
    let o = ConfigManager::masque_outbound("127.0.0.1", 1080);
    assert_eq!(o["protocol"], "socks");
    assert_eq!(o["settings"]["address"], "127.0.0.1");
    assert_eq!(o["settings"]["port"], 1080);
    // 硬约束 6：不是 servers[] 形状
    assert!(o["settings"].get("servers").is_none(),
        "Xray socks outbound 的 address/port 直接挂在 settings 下，写成 servers[] 会静默失效");
    assert!(o["settings"].get("secretKey").is_none(), "不应含 WireGuard 字段");
}
```

---

## T9: 健康判据（纯函数）

**Description**: `core/usque/health.rs`。从 `cdn-cgi/trace` 响应体判定隧道是否工作。

**验收标准:**
- [x] `is_tunnel_healthy(trace_body: &str) -> bool`：含 `warp=on` → true
- [x] `warp=off` / 空体 / 无该字段 → false
- [x] **不区分行序、允许前后空白**（trace 输出顺序可变）
- [x] 测试用真实 trace 输出做 fixture（不打网络）
- [x] 断言 `warp=plus` 也算健康（Cloudflare 还有这个值）

**验证:** `cargo nextest run usque::health`

**依赖**: 无
**文件:** `src/core/usque/health.rs`（新）

**真实 fixture（实测）:**
```
fl=...
h=www.cloudflare.com
ip=2a09:bac5:cad3:3228::4ff:3f
ts=...
warp=on
colo=SEA
loc=US
```

---

## T10: 失败计数与判死（纯函数）

**Description**: 连续失败 N 次才判死（避免网络抖动误杀）。

**验收标准:**
- [x] `FailureTracker`：`record(success: bool)`，连续成功清零
- [x] `should_restart() -> bool`：连续失败 ≥ 3 时 true
- [x] 阈值 3、间隔 60s 为具名常量
- [x] 测试：第一次失败不触发 / 两次不触发 / 三次触发 / 中途成功清零
- [x] 单次成功必须清零（否则偶发失败累积后误杀）

**验证:** `cargo nextest run usque::health`

**依赖**: T9
**文件:** `src/core/usque/health.rs`

---

## T11: 安装状态探测（纯函数）

**Description**: `detect_installed_kinds`，为成对卸载提供判据。

**验收标准:**
- [x] `pub struct InstalledKinds { pub wireguard: bool, pub usque: bool }`
- [x] `detect_installed_kinds(wg_exists: bool, usque_exists: bool) -> InstalledKinds`
- [x] 穷举 4 种组合的测试
- [x] `InstalledKinds::none()` / `any()` / `describe()` 辅助方法

**验证:** `cargo nextest run usque::`

**依赖**: 无
**文件:** `src/core/usque/installer.rs`（新）

---

## T12: 成对卸载（用户追加需求）

**Description**: 改 `WarpInstaller::uninstall()`，同时清理 WireGuard 与 usque，幂等。

**验收标准:**
- [x] 返回 `UninstallOutcome { removed_wireguard, removed_usque }`
- [x] 清理 WireGuard：`warp_account.json` + `10_warp_routing.json`
- [x] 清理 usque：`stop` → `disable` → 删 unit → `daemon-reload` → 删 `CONFIG_FILE` → 删 `BIN`/`DIR`
- [x] **幂等**：两套都不存在 → 返回成功（`Ok`）而非错误
- [x] **任一 `stop` 失败不阻断**（已是 stopped 属正常）
- [x] 只装其中一套时正常卸载另一套，不报错
- [x] 结束后 `reload_core()`
- [x] `UninstallOutcome::i18n_key()` 返回 4 种组合各自的 key，且**互不相同**
- [x] 测试：`i18n_key()` 穷举 4 态 + 断言无重复

**验证:** `cargo nextest run xray::installer` + `cargo nextest run self_destruct`（回归红线）

**依赖**: T1, T11
**文件:** `src/core/xray/installer.rs`

**i18n key 设计（4 态）:**
```
(both)         -> warp.uninstalled_both
(wg only)      -> warp.uninstalled_wireguard
(usque only)   -> warp.uninstalled_usque
(none)         -> warp.uninstalled_none
```

---

## T13: 移除 `WARP_SCOPE_ENFORCE`，改运行时判定（OQ5）

**Description**: `Global` 是否生成 catch-all，改由「出口引擎是否可用」决定。

**验收标准:**
- [x] 删除 `enforce_global_warp()`（读环境变量的函数）
- [x] 新增 `egress_engine_available()` 或等价判据
- [x] `build_routing_rules` 的 `enforce` 参数语义更新为「出口可用」，并写进文档注释
- [x] `Direct`（无可用出口）时 `Global` **不**生成 catch-all（防黑洞）
- [x] Spec 1 的 `test_global_without_enforce_is_identical_to_domain` **仍需通过**（语义兼容）
- [x] 更新该测试的注释说明语义变化

**验证:** `cargo nextest run xray::warp`

**依赖**: T6, T8
**文件:** `src/core/xray/warp.rs`

---

## T14: `a_warp_status` 体现两套状态（决策 Q6/OQ3）

**Description**: 不做新菜单，但状态里要能看到 WireGuard + usque 两套的情况。

**验收标准:**
- [x] `WarpInstaller::status()` 同时报告：WireGuard（账号/隧道）与 usque（安装/健康）
- [x] `FullyBlocked` 时明确提示「出口完全受限」（OQ3）
- [x] 未装 usque 时不报错，只显示「未安装」
- [x] 测试覆盖各组合（用纯函数拆分）
- [x] i18n 三语新增键，key 集合一致

**验证:** `cargo nextest run xray::installer` + 脚本校验三语 key 集合

**依赖**: T9, T12
**文件:** `src/core/xray/installer.rs`、`src/resources/i18n/{zh,en,ja}.yml`

---

## T15: i18n 三语同步（卸载 + 状态）

**Description**: 补齐 T12/T14 的所有新键。

**验收标准:**
- [x] `zh`/`en`/`ja` 的 `warp` 块 key 集合**完全一致**
- [x] 无硬编码文案（全走 `t!`）
- [x] 现有键未被破坏性改动
- [x] 用脚本比对三方 key 集合

**验证:** `cargo nextest run` 全绿 + 脚本比对

**依赖**: T12, T14
**文件:** `src/resources/i18n/{zh,en,ja}.yml`

---

## T16: 全量质量门禁 + 审查

**验收标准:**
- [ ] `cargo fmt` 无 diff
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 零警告
- [ ] `cargo nextest run --cargo-profile fast-test` ≥ 1157 通过
- [ ] `cargo test --doc` 通过
- [ ] `code-review-and-quality` 五轴审查，无 Critical/High

**依赖**: T1–T15
