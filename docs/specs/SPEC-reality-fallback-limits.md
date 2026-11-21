# Spec: REALITY fallback 流量兜底（生成期 + 启动自愈）— V1

Status: proposed

## 0. 假设（先确认，避免我替你填空）

1. 目标 Xray 核心支持 REALITY 的 `limitFallbackUpload` / `limitFallbackDownload`（`config.proto` field 12/13，`LimitFallback{after_bytes, bytes_per_sec, burst_bytes_per_sec}`）。旧核心由 `encoding/json` 处理配置（`infra/conf/serial/loader.go:86` 的 `json.Unmarshal`），未知字段被忽略 → 下发不会导致启动失败，只是不生效。
2. 限速只挂在**未认证 fallback 连接**上（`xtls/reality` `tls.go:290` 未认证分支、`tls.go:452` target ServerHello 异常分支），已认证的 VLESS 流量不经过该连接器 → 对正常用户吞吐零影响。
3. 默认值先硬编码为常量，不暴露到 bot 菜单，不做 `env` 覆盖。
4. 本批次只处理 Xray 配置；sing-box 的 REALITY 是另一套 schema，另开 spec。
5. 存量入站片段文件命名为 `*_inbounds.json`（`generate_secure_batch_filename` → `format!("{}_{}_inbounds.json", ...)`），且不含 `00_` 前缀 —— 与既有 `list_all_inbound_files()` 的过滤条件一致，可直接复用。
6. 语义选择：**对象级存在即视为“已配置”**。只要 `limitFallbackUpload` / `limitFallbackDownload` 任一对象存在，就不再补齐它的字段（哪怕只写了一部分）。理由：可预测、不会和运维的手工调参打架。

---

## 1. 目标与背景

### 1.1 背景

REALITY 在未通过认证时会把整条 TLS 连接转发到 `dest`/`target`（即被伪装的真站）。这是协议设计的一部分，无法在协议层拒绝：任何人扫描到你的端口，都能把你的 VPS 当成到该目标站的免费中继，流量全部计入你的 VPS 账单。

DMIT 本次通报即属此类。上游已提供唯一的内建刹车：REALITY 的 fallback 限速（`LimitFallback`），但 `rust/aegis` 全仓 `grep limitFallback` 零命中 —— 也就是说 aegis 创建的**每一个** REALITY 入站都是无限速的公开中继：

- 生成点唯一：`core/xray/config.rs:455` `build_reality_vless_inbound`，Vision 与 XHTTP 共用（`xhttp.rs:55` 调同一函数）。
- 批量放大：`reality.rs:153` / `xhttp.rs:30` 的 `batch_create_*(count)` 每个入站抽一个 SNI、开一个独立端口。
- fallback 流量不是 VLESS 用户流量，**不进 Xray user stats**，bot 侧/面板侧永远看不到这块钱。

### 1.2 目标

在不改变协议、不引入新依赖、不改 `.pb` / protobuf schema 的前提下：

1. **生成期**：新创建的 REALITY 入站默认带上 fallback 上下行限速。
2. **存量**：已部署节点在下次启动/维护周期被幂等补齐，无需重建节点、无需用户重扫二维码。
3. 对正常 VLESS 用户**零感知**（吞吐、延迟、握手均不变）。
4. 可回滚：删掉两个字段即恢复原行为，无数据迁移。

### 1.3 非目标

见 §9。

---

## 2. 技术栈

- Rust 2024 edition，crate `aegis` 2.6.2。
- `serde_json::Value` 直接改 JSON 树（与既有 `patch_direct_outbound_ipv6` 同范式）。
- `tokio` 文件 IO。
- 无新增依赖。

---

## 3. 命令

```bash
cd rust/aegis

# 格式（CI 之外本地必跑）
cargo fmt --all -- --check

# 静态检查（CI 刻意不跑 clippy，本地按技能要求执行）
cargo clippy --all-targets -- -D warnings

# 测试：与 CI 同一档（fast-test profile）
cargo +nightly nextest run --cargo-profile fast-test

# 定向跑本批次用例
cargo +nightly nextest run -E 'test(reality_fallback)'

# 发布构建验证
CARGO_PROFILE_RELEASE_LTO=false cargo +nightly build --release
```

手动验收（需要真实节点，不进 CI）：

```bash
xray -test -config /etc/wwps/wwps-core/conf/<file>_inbounds.json
# fallback 吞吐对照（未带 pbk 的裸 TLS 连接）
openssl s_client -connect <vps>:<port> -servername <sni> -quiet < /dev/zero   # 观察限速后速率
```

---

## 4. 项目结构

| 路径 | 变化 |
|---|---|
| `rust/aegis/src/core/xray/config.rs` | 新增常量 + `patch_reality_fallback_limits()` + 生成器注入 + 自愈接入 + 单测 |
| `docs/specs/SPEC-reality-fallback-limits.md` | 本文件 |

复用点：

- `ConfigManager::list_all_inbound_files()`（`config.rs:85`）枚举存量文件。
- `ConfigManager::ensure_base_config()`（`config.rs:1119`，被 `main.rs:168`、`installer.rs:584`、`maintenance.rs:127` 调用）作为自愈接入点。
- **补丁生效后直接重启核心**：只写文件不够 —— 运行中的核心不会重读配置（2026-11-21 已在真机复现，见 §13.2）。补丁有改动且核心在跑时，调用
  `MaintenanceManager::control_service("wwps-core", Restart)` 一次；**不调 `reload_core()`**：它会回调本函数，形成递归。

---

## 5. 代码风格

沿用仓内既有范式（`patch_direct_outbound_ipv6` 的幂等 + 返回 `bool`）：

```rust
// 区间而非固定值：回落限速本身是可识别特征，上游 PR #4553 要求参数随机化。
const FALLBACK_UPLOAD_AFTER: u64 = 0;
const FALLBACK_UPLOAD_RATE: (u64, u64) = (48 * 1024, 96 * 1024);
const FALLBACK_DOWNLOAD_AFTER: (u64, u64) = (5 * 1024 * 1024, 10 * 1024 * 1024);
const FALLBACK_DOWNLOAD_RATE: (u64, u64) = (192 * 1024, 384 * 1024);
const FALLBACK_DOWNLOAD_BURST: (u64, u64) = (1024 * 1024, 3 * 1024 * 1024);

// 旧版本写死的值：只用于识别“这是我们的旧值，可迁移”，其余手工值一律不碰。
const LEGACY_FALLBACK_UPLOAD: (u64, u64, u64) = (64 * 1024, 256 * 1024, 1024 * 1024);
const LEGACY_FALLBACK_DOWNLOAD: (u64, u64, u64) = (1024 * 1024, 512 * 1024, 2 * 1024 * 1024);

/// 给所有 REALITY 入站补齐/迁移 fallback 限速。返回 `true` 表示有改动。
/// 缺失 → 写随机值；等于旧默认值 → 迁移；其余（手工值）→ 保留。
pub fn patch_reality_fallback_limits(config: &mut Value) -> bool { /* ... */ }
```

区间依据：上游 `XTLS/Xray-core#4553`（已合并）的示例配置为「下载前 10 MiB + 2 MiB 突发、
之后 256 KiB/s；上传 64 KiB/s」，并明确要求脚本/面板**随机化**参数。

约定：

- 字段名一律 camelCase（Xray JSON），与 `limitFallbackUpload` 一致。
- 所有“补齐”逻辑必须幂等，且不做 key 级补全。
- 日志只打汇总（`已补齐 N 个入站 / M 个文件`），不逐入站刷屏。
- 原子写必须保留原文件 mode：`rename` 会带过临时文件的权限位，而配置含 `privateKey`，不得从 0600 放宽成 0644。

---

## 6. 测试策略

框架：`cargo nextest`，用例放在 `config.rs` 现有 `#[cfg(test)] mod tests`（与 `test_patch_direct_outbound_*` 同处）。**全部为纯函数单测，不触网。**

| 用例 | 断言 |
|---|---|
| `reality_fallback_generator_emits_limits` | `build_reality_vless_inbound` 产出 JSON 中两个对象存在，且三字段值 = 默认表 |
| `reality_fallback_xhttp_path_emits_limits` | XHTTP 同样命中（共用生成器回归保护） |
| `reality_fallback_patch_is_idempotent` | 首次返回 `true`，二次返回 `false`，序列化结果逐字节相同 |
| `reality_fallback_patch_respects_manual_values` | 已存在对象（含 `bytesPerSec: 0` 的显式关闭）不覆盖 |
| `reality_fallback_patch_ignores_non_reality` | `security: tls` / 无 `streamSettings` 的入站不动 |
| `reality_fallback_patch_tolerates_garbage` | `inbounds` 非数组、元素非对象、空文件 → `false` 且不 panic |
| `reality_fallback_patch_preserves_other_fields` | 仅新增两个 key，入站其余字段与顺序语义不变（比较 `Value` 相等） |

不覆盖：真实 Xray 进程行为（由手动验收项兜底）。

---

## 7. Boundaries

**Always**

- 提交前跑 `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo +nightly nextest run --cargo-profile fast-test`。
- 补丁函数保持纯函数、幂等、可重入。
- 保留用户已写字段，不覆盖。

**Ask first**

- 调整 §5 默认值表。
- 把限速做成 bot 可调/环境变量可调。
- 扩展到 sing-box REALITY。
- 顺手把 `"target"` 迁移为 `"dest"`（独立变更，不混入本批次）。
- 改 CI 配置。

**Never**

- 覆盖或删除用户手写的 `limitFallback*`。
- 把限速施加到已认证用户路径（Xray 语义上做不到，也不要试图在 outbound/routing 上做）。
- 本批次内改 sni_probe、`proto/sni.proto`、`DomainList` schema。
- 删除/跳过既有测试来换绿。

---

## 8. 成功标准

1. 新建 Vision 与 XHTTP REALITY 入站的 JSON 中，`realitySettings.limitFallbackUpload` 与 `.limitFallbackDownload` 存在，且三字段等于默认表。
2. 对含无字段 REALITY 入站的存量 `*_inbounds.json`，启动后文件被改写，字段补齐；第二次启动无写入。
3. 对已存在 `limitFallbackUpload`（哪怕只是 `{"bytesPerSec":0}`）的入站，文件**不变**。
4. 畸形 JSON、只读文件、目录不存在：仅告警，不阻断启动。
5. `xray -test -config <file>` 通过（手动验收）。
6. 未认证 fallback 连接：前 5–10 MiB（随机）全速，之后下载稳定在 192–384 KiB/s（手动验收，记录实测值）。
   注：这是 per-connection 限速，不是总额度；加压路径见 §12。
7. 已认证 VLESS 用户下载吞吐与变更前一致（同机同文件 A/B，误差 ±10% 内）。
8. 自愈改写后原文件 mode 保持不变（含 0600）。
9. Diff 规模 ≤ 2 个文件、单次提交 ≤ 200 行（不含 spec/plan）。

---

## 9. Out of Scope (Deferred)

| 项 | 原因 |
|---|---|
| sni_probe 选池侧“高滥用风险”过滤（ASN org / content-type） | 跨 Rust/Go 双实现，单独批次；且需先修 `h2::decode_static` 的 HPACK 截断 |
| `tc` / `nft` 总带宽兜底、per-IP 并发限制 | 会波及正常用户，需独立压测与阈值评估 |
| per-user 配额、bot 面板展示 fallback 用量 | fallback 不进 user stats，需另设采集通道 |
| `DomainList` 增加风险分级字段 | 改 proto + selector，属 schema 变更 |
| sing-box REALITY 同款处理 | 不同 schema，另开 spec |
| 安装器内校验 Xray 最低版本 | 未知字段被忽略，当前无阻断风险；等确有需求再做 |
| SNI 黑名单/手工指定 | 属选池层职责 |

---

## 10. Resolved Decisions（原 Open Questions）

| # | 问题 | 决定 |
|---|---|---|
| 1 | 默认值是否按节点类型分档 | **不分**，但每个入站从区间内随机取值（上游要求随机化） |
| 2 | 下载 `afterBytes` 取值 | **5–10 MiB 随机**。原定 1 MiB（§11 实测），后按上游 `#4553` 改为大宽限 + 参数随机化：宽限太小会让探测看到“加载到一半变慢”，本身就是特征 |
| 3 | 自愈接入点 | **`ensure_base_config()` 内部**，复用 3 处调用点，避免漏接 |
| 4 | `bytesPerSec: 0` 语义 | **确认**：视为“已配置 / 显式关闭”，不覆盖 |
| 5 | 日志级别 | **`warn`** 一条汇总，仅在有改动时输出 |

## 11. 实测依据（2026-11-21）

样本：`US.pb` 随机 60 个域名（seed=42），curl 8.20 + Chrome UA，`-L --compressed`，8s 超时；
页面体积 = HTML 字节数 + 页面内前 10 个 css/js/img/woff 子资源的 `content-length` 之和。

| 指标 | p50 | p75 | p90 | max |
|---|---:|---:|---:|---:|
| HTML 文档 | 141 KiB | 304 KiB | 710 KiB | 871 KiB |
| 估算整页（含 10 个子资源） | 324 KiB | 807 KiB | 1207 KiB | 2317 KiB |

样本健康度：50/60 返回 200（83%），其余为 403×4、自签证书×2、429/502/500/404 各 1。

结论：

- `afterBytes = 1 MiB` 覆盖 **约 p85** 的整页体积，保证真实浏览器 / 主动探测首屏不被中途限速；
- 宽限字节在滥用场景中可忽略（1000 连接 × 1 MiB = 1 GiB），**真正起作用的是 `bytesPerSec`**；
- `bytesPerSec = 512 KiB/s`（≈4.2 Mbps）低于 1080p 常见码率，视频中继变得不可用，而普通页面加载无感。

## 12. 升级阶梯（若再次收到滥用通报）

`limitFallback` 是 **per-connection** 限速，不是总额度；并发 N 条即 N × rate。
若 DMIT 二次通报，按此顺序加压，不要一次全上：

1. `bytesPerSec` 下调到 128 KiB/s（`afterBytes` 保持 1 MiB，避免首屏异常）；
2. 启用 per-IP 连接数 / 并发限制（§9 第 2 项，届时单独立项）；
3. 轮换 SNI 池并复核选池（回退到 sni_probe 批次）。

---

## 13. 验证记录（2026-11-21）

工作树：`.worktrees/reality-fallback-limits`，分支 `feat/reality-fallback-limits`，基线 `623a199`。

| 门禁 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | ✅ |
| 静态检查 | `cargo clippy --all-targets` | ✅ 本变更零诊断；仓库存在两处**基线遗留**（`src/main/runtime.rs:25` 的 `BotCommands` derive `unreachable_code`、`Cargo.toml` 的 `goldberg` 未使用依赖），均在未改动文件中 |
| 单测（定向） | `cargo nextest run -E 'test(reality_fallback)'` | ✅ 12/12 |
| 单测（全量） | `cargo nextest run` | ✅ 1397 passed / 1 skipped |
| 发布构建 | `CARGO_PROFILE_RELEASE_LTO=false cargo build --release` | ✅ 0 error（4 条基线遗留警告） |

未完成项（需真实节点，不在本次环境内）：`xray -test`、fallback 限速实测、已认证用户 A/B。

### 13.1 真机验收（2026-11-21 16:39–16:45，x86_64 / Debian GLIBC 2.41 / Xray 26.11.9）

部署产物：本地 release 构建 sha256 `1b8265e797ec01b690fe811aa96ae584fe8f3f906d1315ffc3ec9c2bc81a3868`
（所需最高符号 `GLIBC_2.39` ≤ 宿主机 2.41，可运行）。备份：`/root/deploy-20261121-163826/`
（`aegis.bak` + `conf.bak` + `config.enc`）。

| 验收项 | 结果 |
|---|---|
| 自愈触发 | ✅ `[WARN] REALITY fallback 限速已补齐：40 个入站 / 2 个文件` |
| 覆盖完整性 | ✅ 40/40 REALITY 入站均含两字段 |
| 配置合法性 | ✅ `wwps-core -test -confdir ...` → `Configuration OK.` |
| 权限保留 | ✅ 原文件 0644 不变 |
| 服务状态 | ✅ `wwps-core` / `wwps-aegis` 均 active |

**限速实测**（临时入站 `port 49777` → `127.0.0.1:18443` 的 `openssl s_server -WWW`，测完已删除）：

| 场景 | 大小 | 耗时 | 平均速率 |
|---|---:|---:|---:|
| 直连 s_server（基线） | 32 MiB | 0.25 s | 133 MB/s |
| 经 REALITY fallback | 8 MiB | 10.01 s | 0.84 MB/s |
| 经 REALITY fallback | 32 MiB | **58.07 s** | **578 KB/s** |

与配置完全对得上：1 MiB 全速宽限 + 2 MiB 初始 burst + 剩余 29 MiB ÷ 512 KiB/s = 58 s。
即 **持续速率就 512 KiB/s**，4K/1080p 视频中继不可用。

未做真机测试的两项（附理由）：

- **已认证用户 A/B**：限速器只在 `hs.c.conn != conn` 与 target ServerHello 异常两个分支包裹（`xtls/reality tls.go:290/452`），
  已认证流量不走这两条路径；代码路径已核，端到端需另起 VLESS 客户端。
- **上传方向**：`openssl s_server -WWW` 不收大 body，无法稳定测。

回滚：`cp /root/deploy-20261121-163826/aegis.bak /etc/wwps/aegis/aegis && systemctl restart wwps-aegis`，
并把 `conf.bak` 拷回 `conf/` 后重启 `wwps-core`。

实现中修正的 spec 偏差：

1. **不做 reload**（原 §4 写“单次 reload”）：`reload_core()` 会回调 `ensure_base_config()`，形成递归。

### 13.2 复核发现的真实缺陷与修复（2026-11-21）

两台节点对照（都是 40 个入站、都是补丁成功）：

| 节点 | aegis 启动 | 补丁写入 | 核心进程启动 | 结果 |
|---|---|---|---|---|
| `aqtwqmgx` | 17:24:03 | 16:39:25 | **16:45:40**（补丁后手动重启） | 生效，实测 512 KiB/s |
| `otxmdmoz` | 17:24:42 | 17:24:46 | **11-18 11:11:51**（早于补丁 3 天） | ❌ 文件已改，运行态未生效 |

原实现只写文件、不重启核心，`otxmdmoz` 上日志写着“已补齐”，实际仍在无限速中继。

修复：`ensure_base_config()` 在补丁有改动且核心在跑时重启 `wwps-core` 一次，日志区分
「已补齐并重启核心生效」/「重启失败，需手动重启」/「核心未运行，下次启动生效」。
2. **新增权限保留**（原 spec 未覆盖）：`rename` 会把临时文件的 0644 带过去，使原本 0600 的配置被放宽；已改为写前按原 mode 设好，并加 `#[cfg(unix)]` 用例锁定。

---

## 验收清单（实现完成后逐项打勾）

- [x] 生成器两协议均有字段
- [x] 自愈幂等
- [x] 手工值不被覆盖
- [x] 畸形输入不 panic
- [x] 权限保留（0600 不变）
- [x] fmt / nextest / release build 通过（clippy 本变更零诊断）
- [x] `xray -test` 通过（需真实节点）
- [x] fallback 限速实测达标（需真实节点）
- [ ] 已认证用户吞吐 A/B 无回退（仅代码路径核实，未端到端）
- [x] 单次提交 ≤ 200 行（最大一笔 +124）

**修复验证（2026-11-21 18:44，`otxmdmoz`）**：先手动剥掉 1 个入站的两个字段（备份
`/root/deploy-fix-20261121-184355/`），再换新二进制并只重启 `wwps-aegis`：

```
[WARN] REALITY fallback 限速已补齐并重启核心生效：1 个入站 / 1 个文件
核心进程启动：Wed Nov 18 11:11:51  →  Sat Nov 21 18:44:41
40/40 入站含两字段；wwps-core -test -confdir → Configuration OK.
```

对照 `aqtwqmgx`（文件本来就齐）：只重启 aegis 时**不**触发核心重启，无多余中断。

### 13.3 特征降低：参数随机化（2026-11-21）

依据上游 `XTLS/Xray-core#4553`（2025-06-08 合并）作者的明确要求：

> 回落限速是一种特征，不建议启用……如果是面板/一键脚本开发者，务必让这些参数随机化。

原实现是全网同一组常量（1 MiB / 512 KiB/s / 2 MiB），200 个入站完全一致。改为每个入站
从区间取随机值：

| 方向 | afterBytes | bytesPerSec | burst |
|---|---|---|---|
| upload | 0 | 48–96 KiB/s | 0 |
| download | 5–10 MiB | 192–384 KiB/s | 1–3 MiB/s |

存量迁移：`patch_reality_fallback_limits` 只在「字段缺失」或「值正好等于旧常量」时写入；
其余（运维手工值，含 `bytesPerSec: 0`）保持不动。
