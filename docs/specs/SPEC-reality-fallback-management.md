# Spec: REALITY 回落防护（配置化 + 监控 + 告警 + 运维中心）— V1

Status: **deferred**（2026-11-21 搁置，理由见下）
Supersedes: `SPEC-reality-fallback-limits.md` §5（生成器写死区间）与 §13（随机化的实现细节）
不动: `SPEC-reality-fallback-limits.md` 的结论部分（为什么要限速、真机吞吐数据）

> **为何搁置**：封锁上线后实测 160/160 个 REALITY 入站全部带限速、零缺失、零越界；代码里只有
> `build_reality_vless_inbound` 一处生成点，sing-box 侧没有 REALITY。已观测到的滥用（DMIT LAX 2% 租户、
> 单夜 2 TB）是「默认扫描器 + 单连接刷大文件」，现行每连接限速已把它挡在门外。
> 剩下的攻击面只有「多流并发摊薄」——那是每连接限速的固有性质，不是缺陷；为它建配置化 UI + nft 计量
> 属于过度设计。本文件保留为设计储备，等真的出现「持续、多流、按源异常」的证据再启动。


**V1 决策（2026-11-21 已确认）**

| 决策 | 取值 |
| :--- | :--- |
| 默认档位 | `random` |
| 档位数量 | 三档：`off` / `random` / `strict`（自定义推迟到 V1.1） |
| 随机粒度 | 字节级，不做 KiB/256 KiB 对齐 |
| 监控默认 | 开 |
| 告警 | 只发 bot，不写 journal |
| 阈值 | 尚未定论，先给**不会误报**的初值，跑 7 天再用实测回填 |

---

## 0. Capability Map

| Module id | 职责 | 依赖 |
| :--- | :--- | :--- |
| `fb-config` | 档位模型、配置文件读写、随机/严格解析、生成器注入、存量迁移与显式应用 | — |
| `fb-monitor` | nft 目标集合与计数器生命周期、采样、状态文件、速率换算 | — |
| `fb-alert` | 阈值判定、状态机、去重、bot 投递 | `fb-monitor` |
| `fb-ui` | 运维中心菜单、回调路由、i18n | `fb-config`, `fb-monitor`, `fb-alert` |

构建顺序：`fb-config` ∥ `fb-monitor` → `fb-alert` → `fb-ui`

`fb-config` 与 `fb-monitor` 无共享可变状态，可分两个子代理并行实现；`fb-alert` 只读 monitor 的输出，`fb-ui` 最后统一接线。

---

## 1. Objective

把 2026-11-21 用二进制常量实现的「REALITY fallback 限速」升级为运维中心里可配置、可观测、可告警的一等功能。

**为什么现在做：**

1. **限制是有效的，但写死了。** 实测 512 KiB/s 档位把 32 MiB 传输拖到 58 秒；参数已随机化到区间，但仍是编译期常量，改一次要重新发版。
2. **滥用不可见。** Xray 不暴露 fallback 字节数，运维只能靠 DMIT 的账单知道被蹭了。需要旁路计量。
3. **SNI 不是秘密。** 2026-11-21 在 `aqtwqmgx:53862`（真实 `serverNames=hsptests.com`）用假 SNI `bogus.example` 实测：回落照常转发，返回证书是 `CN=wordpress.com`。源码对应 `tls.go:212`（SNI 不匹配即跳出认证）与 `tls.go:169`（转发固定用 `config.Dest`）。**攻击者只需要端口，SNI 随便填，证书会自己告诉他目标是谁。** 任何「靠隐藏」的方案都不成立，只能靠限制与计量。

**用户故事：**

- 作为运维，我在「运维中心 → 安全防护」里看到今天有多少流量走向回落目标，以及有几条活跃回落连接。
- 作为运维，我可以在 `关闭 / 随机 / 严格` 三档之间切换，并对全部 REALITY 入站一键应用。
- 作为运维，当回落流量或连接数异常时，bot 主动告警一次（不重复轰炸），并建议"要不要升到严格档"，但不替我改。

---

## 2. 术语

| 词 | 含义 |
| :--- | :--- |
| 回落 / fallback | 未通过 REALITY 认证的连接被原样转发到 `realitySettings.target` |
| 目标 / target | 回落目的地（域名:端口），本地解析成 IP 集合 |
| 档位 / mode | `off` / `random` / `strict` |
| 显式应用 | 用户在 UI 点「应用」，无条件重写全部 REALITY 入站的限速字段 |
| 自愈 | aegis 启动时补齐缺失字段、迁移旧常量，但**不**改用户值 |

---

## 3. 技术栈与命令

Rust 2024 edition，tokio，serde_json；nftables ≥ 1.0（线上实测 1.1.3）；aegis 以 root 运行（已验证 `systemctl show -p User wwps-aegis` → `root`）。

```bash
cd rust/aegis && cargo build --release            # 生产：CARGO_PROFILE_RELEASE_LTO=false
cargo nextest run                                  # 全量（当前基线 1400 passed / 1 skipped）
cargo nextest run -E 'test(fb_)'                   # 本特性
cargo fmt --all -- --check
cargo clippy --all-targets
```

---

## 4. 项目结构

```
rust/aegis/src/core/xray/
  config.rs                 # 改：区间常量 → profile 解析；patch/ensure 接受 profile
  fallback_profile.rs       # 新：档位模型、解析、预设、迁移判定（纯函数，可单测）
rust/aegis/src/core/security/
  fallback_monitor.rs       # 新：nft 表生命周期、采样、状态文件、速率换算
  fallback_alert.rs         # 新：阈值判定与状态机（纯函数）+ 投递
rust/aegis/src/shared/handlers/
  ops.rs                    # 改：新增 a_fb_* 回调分发
  menu.rs                   # 改：m_security 增加入口；新增 m_reality_fb 子菜单
resources/i18n/{zh,en,ja}.yml  # 改：新增 menu.fb_* / ops.fb_* 文案
docs/specs/SPEC-reality-fallback-management.md  # 本文件
```

运行时文件（不加密，无密钥）：

```
/etc/wwps/aegis/fallback.json    # 档位 + 告警阈值
/etc/wwps/aegis/fbmon.json       # 采样状态（上次计数、速率、告警状态）
```

---

## 5. 数据模型

### 5.1 `fallback.json`

```json
{
  "version": 1,
  "mode": "random",
  "monitor": { "enabled": true },
  "alert": {
    "enabled": true,
    "egress_gib_per_day": 50.0,
    "concurrent_conns": 20,
    "probe_conns_per_min": 500
  }
}
```

约束（加载时校验，越界回落默认值并记 warn）：

| 字段 | 范围 |
| :--- | :--- |
| `egress_gib_per_day` | 1 … 10000 |
| `concurrent_conns` | 2 … 10000 |
| `probe_conns_per_min` | 10 … 1000000 |

**阈值是暂定值，明确不为"正确值"。** 现网基线：`refletke` 0.31 GB/天、`aqtwqmgx` 4.25、`otxmdmoz` 3.87、`mqmvnjpq` 7.97。用 50 GiB/天 + 20 条并发，四台在正常状态下都不会触发。跑满 7 天后再按「3 × p99」回填，见 §12。

### 5.2 预设

| mode | download after | download rate | download burst | upload after | upload rate | upload burst |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `off` | — | — | — | — | — | — |
| `random` | 5 242 880 … 10 485 760 | 196 608 … 393 216 | 1 048 576 … 3 145 728 | 0 | 49 152 … 98 304 | 0 |
| `strict` | 0 | 131 072 | 0 | 0 | 32 768 | 0 |

**随机粒度是字节级**，不做 KiB 对齐。`random` 抽样规则：

- `afterBytes`：区间内均匀取整（1 字节粒度）
- `bytesPerSec`：区间内均匀取整（1 字节粒度）
- `burstBytesPerSec`：区间内均匀取整（1 字节粒度）

不做对齐的理由：对齐只服务于"人看着舒服"，而攻击者能测出来的是**限速后的实际吞吐**，不是配置里的数字；真正要避免的是「全网同一个值」，字节级精度把单节点指纹空间从几百种放大到十万级。副作用是配置里会出现 `263117` 这种数字，属预期。

### 5.3 与旧值的关系

`LEGACY_FALLBACK_*` 两个常量保留，仅用于「这是上一版写死的值，可以迁移」的判定。判定条件仍是精确匹配三字段且键数恰为 3。

---

## 6. 行为规格

### 6.1 解析（纯函数）

```rust
pub fn resolve(cfg: &FallbackConfig, rng: &mut impl Rng) -> Option<FallbackProfile>
```

- `off` → `None`
- `random` → 每次调用独立抽样（同一进程内不同入站必须不同）
- `strict` → 固定值

### 6.2 生成器

`build_reality_vless_inbound` 调用 `resolve()`，`None` 时不写两个 `limitFallback*` 字段。生成器不做迁移判定。

### 6.3 自愈（aegis 启动）

`ensure_base_config()` 保持现有语义，只把「写什么」换成 `resolve()` 的结果：

- 字段缺失 → 按当前档位补（`off` 时不补）
- 字段精确等于 `LEGACY_*` → 迁移为当前档位
- 其它值（用户手改）→ 不动

迁移与补全都计为「有改动」，沿用现有逻辑（有改动且核心在跑 → 重启核心；或 conf mtime 晚于核心启动 → 重启核心）。

**`off` 档不删除已有字段。** 删除属于显式操作，只在用户点「应用」时发生。

### 6.4 显式应用

`apply_profile(dir, profile: Option<&FallbackProfile>) -> (usize /*files*/, usize /*inbounds*/)`

- 遍历全部 `*_inbounds.json`（跳过 `00_*`）
- 对每个含 `realitySettings` 的入站无条件写入（`Some`）或移除（`None`）两个键
- 原子写 + 保留权限位（复用现有实现）
- 任一文件改动 → 调用方重启 `wwps-core`（一次，不是每文件一次）

UI 文案必须明确：**会覆盖所有 REALITY 入站上的现有值**。这是唯一会覆盖用户值的路径。

### 6.5 监控（`fb-monitor`）

**nft 表**（独立表，`policy accept`，不改任何流量行为）：

```
table inet wwps_fbmon {
  set targets_v4  { type ipv4_addr; flags interval; }
  set targets_v6  { type ipv6_addr; flags interval; }
  set ports_v4    { type inet_service; }
  set ports_v6    { type inet_service; }

  chain out {
    type filter hook output priority -150; policy accept;
    ip  daddr @targets_v4 counter
    ip6 daddr @targets_v6 counter
  }

  chain in_new {
    type filter hook prerouting priority -150; policy accept;
    tcp dport @ports_v4 ct state new counter
    meta nfproto ipv6 tcp dport @ports_v6 ct state new counter
  }
}
```

为什么这样能测准：认证通过的连接由本机终结，**只有回落才会让本机主动连目标 IP**。因此 `output` 链到目标集合的字节数 ≈ 回落出向流量（误差仅限本机自身对这些 IP 的 DNS/健康检查，可忽略）。

**目标集合构建**：启动、应用档位、以及每 60 分钟，从全部入站的 `realitySettings.target` 取 `host:port`，对域名做一次 DNS（A/AAAA），写回集合。解析失败保留旧值并记 warn，不清空。

**端口集合**：全部 REALITY 入站的 `port`（含范围展开）。

**采样**：每 60 秒 `nft -j list table inet wwps_fbmon`，取 `bytes`/`packets`，与 `fbmon.json` 的上次值求差；若当前值 < 上次值（重启/表重建）则基线上移、不产生负值。活跃连接数直接解析 `/proc/net/tcp` 与 `/proc/net/tcp6`（不 spawn `ss`），统计 `dst` 命中目标集合的 `ESTABLISHED`。

**状态文件** `fbmon.json`：

```json
{
  "ts": 1763000000,
  "last": { "out_v4": 0, "out_v6": 0, "new_v4": 0, "new_v6": 0 },
  "hourly": [{ "ts": 1762996400, "egress": 0, "new_conns": 0 }],
  "today": { "date": "2026-11-21", "egress": 0, "new_conns": 0 },
  "conn_est": 0,
  "alert_state": "normal"
}
```

保留最近 48 个小时桶与当天累计；跨天时把 `today` 清零。

**禁用监控**：删除 `wwps_fbmon` 表并停止采样。不留空表。

**容错**：`nft` 不存在、权限不足、内核不支持，一律只记 warn 并把监控标记为「不可用」（UI 显示原因），**不得影响 aegis 启动或其他功能**。

### 6.6 性能预算（硬约束）

监控跑在代理节点上，开销必须可忽略：

| 项 | 预算 | 量级依据 |
| :--- | :--- | :--- |
| 每 tick 总耗时 | < 15 ms | 1 次 `nft -j list`（4 条规则）≈ 2–3 ms；解析 `/proc/net/tcp{,6}` 2 000 连接 ≈ 3–5 ms；原子写 JSON < 1 ms |
| 稳态 CPU | < 0.05 % 单核 | 15 ms / 60 s |
| 内存增量 | < 200 KiB | 状态文件 + 48 个小时桶 |
| 数据面开销 | 单包一次计数器自增 | 内核态，无 verdict、无 userspace 处理 |

实现约束：

- 单一采样任务，`tokio::time::interval` + `MissedTickBehavior::Skip`，**不允许重入**；上一 tick 未完成则跳过本 tick。
- 命令调用用 `tokio::process::Command`（不阻塞 runtime）；解析与写入均在同任务内完成，不额外起线程。
- 一次 tick 只读一次 nft 表；禁止每连接、每包任何调用。
- 计数器存**绝对值**，速率由差值算，因此丢 tick 只影响粒度、不会重复计数。
- 首次启动在核心启动后 60 秒才开始采样，避开启动抖动。

**上线前后各测一次**（`refletke`）：`iperf3 -c <peer> -t 10` 与 `curl` 大文件吞吐，差异需 < 1 %。

### 6.7 告警（`fb-alert`）

纯函数先行，便于单测：

```rust
pub fn evaluate(sample: &Sample, cfg: &AlertConfig, prev: &AlertState) -> (AlertState, Option<Alert>)
```

暂定阈值：

| 信号 | warn | critical |
| :--- | :--- | :--- |
| 当日回落出向流量 | > 50 GiB | > 200 GiB |
| 活跃回落连接 | > 20 | > 100 |
| 每分钟新建连接（端口集合） | > 500 | > 2000 |

状态机：

- `normal → warn` 需连续 2 个采样命中；`warn → critical` 需连续 2 个；降级需连续 5 个未命中。
- 同一等级 6 小时内只发一次（`alert_state.last_sent`）。
- 跨天清零 `today` 时，若此前是 `warn`，降回 `normal`。

告警文案包含：当前值、阈值、档位、建议（切 `strict`），以及"在运维中心点『应用档位』即可生效"的提示。**不自动应用。**

投递：只走既有 `adapter.send_message` 到管理员目标；不写 journal；失败只记 warn（并且不重试，避免雪崩）。

### 6.8 UI

`m_security` 子菜单新增按钮 → `m_reality_fb`：

```
🛡 <b>REALITY 回落防护</b>
档位: 随机（下载 5–10 MiB 全速，之后 192–384 KiB/s）
今日回落: 1.2 GiB · 活跃连接: 0 · 监控: 开
最近告警: 无

[关闭] [随机] [严格]
[应用档位] [刷新]
[监控: 开/关] [告警: 开/关]
[⬅️ 返回运维中心]
```

- 点击档位按钮 → 立即写 `fallback.json`（只改配置，不动入站）+ 提示「已选择 X，点『应用档位』生效」
- 「应用档位」→ 二次确认（复用现有 `approval` 流程）→ `apply_profile` → 重启核心 → 显示改动数量
- 监控与告警开关即时生效（写配置 + 启停采样任务）

回调数据前缀统一 `a_fb_`，并在 `shared/handlers/mod.rs` 的路由白名单里登记（现有 `a_fw` 的位置）。

i18n：所有文案进 `zh/en/ja.yml` 的 `menu.*` 或 `ops.*`，不写死中文。

---

## 7. 接口

```rust
// core/xray/fallback_profile.rs
pub enum Mode { Off, Random, Strict }
pub struct FallbackConfig { pub mode: Mode, pub monitor_enabled: bool, pub alert: AlertConfig }
pub struct FallbackProfile { pub download: Limits, pub upload: Limits }
pub struct Limits { pub after_bytes: u64, pub bytes_per_sec: u64, pub burst_bytes_per_sec: u64 }

impl FallbackConfig {
    pub fn load() -> Self;                 // 缺失/损坏 → random + 监控开 + 告警开
    pub fn save(&self) -> Result<()>;      // 原子写
}
pub fn resolve(cfg: &FallbackConfig, rng: &mut impl Rng) -> Option<FallbackProfile>;
pub fn is_legacy(v: &Value) -> bool;

// core/xray/config.rs（改）
pub async fn apply_profile(dir: &Path, profile: Option<&FallbackProfile>) -> (usize, usize);

// core/security/fallback_monitor.rs
pub async fn sync_sets(dir: &Path) -> Result<()>;
pub async fn sample() -> Result<Sample>;
pub fn spawn_sampler(config_dir: PathBuf) -> JoinHandle<()>;

// core/security/fallback_alert.rs
pub fn evaluate(sample: &Sample, cfg: &AlertConfig, prev: &AlertState) -> (AlertState, Option<Alert>);
```

---

## 8. Code Style

```rust
/// 解析当前档位；`off` 返回 `None`，调用方据此不写字段。
pub fn resolve(cfg: &FallbackConfig, rng: &mut impl Rng) -> Option<FallbackProfile> {
    match cfg.mode {
        Mode::Off => None,
        Mode::Random => Some(FallbackProfile {
            download: Limits::random(rng, DOWNLOAD_AFTER, DOWNLOAD_RATE, DOWNLOAD_BURST),
            upload: Limits::random(rng, (0, 0), UPLOAD_RATE, (0, 0)),
        }),
        Mode::Strict => Some(FallbackProfile::strict()),
    }
}
```

约定：纯逻辑与 IO 分离；随机抽样接受 `&mut impl Rng`（测试用固定种子）；对外错误用 `anyhow`，内部判定返回 bool/usize。

---

## 9. Testing Strategy

**单元（`cargo nextest`，与实现同文件 `mod tests`）**

- `resolve` 三种档位的输出范围；`off` 为 `None`；同种子可复现、不同种子有差异；随机值必须落在字节级区间内（不假设对齐）。
- `apply_profile`：Some 覆盖全部（含用户手改值）、None 全部删除、权限位保持、原子写。
- 自愈：缺失补、旧常量迁移、用户值保留、`off` 档不删。
- `evaluate`：每个等级的进入/退出迟滞、6 小时去重、跨天重置、计数器重置后的基线。
- `fbmon.json` 差值换算与状态迁移；`/proc/net/tcp` 解析（用固定样本字符串）。

**集成（临时目录）**

- 造 3 个 inbound 文件（1 个非 REALITY、1 个 XHTTP、1 个手工值），跑 `apply_profile` 与自愈，断言内容与权限。

**真机（每台一次，脚本化）**

- `nft list table inet wwps_fbmon` 存在且 `policy accept`；`inet firewalld` 不受影响。
- 用一个已知回落的 32 MiB 下载，比较 nft 计数增量与 `curl` 报告的字节数，误差 < 5 %。
- 关闭监控后表被删除；重新开启后重建且计数从 0 开始（不产生负值）。
- 采样开销：`/usr/bin/time` 或 `time` 包住单次 tick，< 15 ms；开监控前后 `iperf3` 差异 < 1 %。

**UI**

- 键盘构建为纯函数测试（仓库已有此模式，如 `one_click_*` 测试）；回调路由表断言 `a_fb_*` 全部登记。

---

## 10. Boundaries

- **Always**：`cargo fmt --all -- --check`；`cargo clippy --all-targets`；`cargo nextest run` 全绿；nft 失败不得影响 aegis 启动。
- **Ask first**：删除或重建 `inet firewalld`；修改 `WWPS` 之外的表；把告警改成自动应用档位；新增 crate 依赖。
- **Never**：在 nft 链上设 `drop`/`reject`；改 `policy accept`；把 `fallback.json` 放进加密配置；在 UI 里无确认地覆盖用户值；在采样路径里 spawn `ss`/`nft` 之外的进程。

---

## 11. Success Criteria

1. 新装节点：生成的 REALITY 入站带随机限速，取值落在 `random` 区间，**且不要求 KiB 对齐**。
2. `off` + 应用 → 全部 REALITY 入站无 `limitFallback*` 键；核心重启一次。
3. `strict` + 应用 → 全部入站为严格值；与用户手改值并存时，应用后以档位为准。
4. 只重启 aegis（不点应用）→ 用户手改值保持不变。
5. 监控开启后 60 秒内出现第一个采样；人为制造 500 MiB 回落流量，`今日回落` 误差 < 5 %。
6. 超过暂定阈值 → 收到一次告警；6 小时内不重复；回落后自动降级。
7. 关闭监控 → `wwps_fbmon` 表消失；重新开启 → 重建且不产生负值。
8. 采样 tick < 15 ms；开监控前后 `iperf3` 吞吐差异 < 1 %。
9. 全量测试 ≥ 1400 passed；clippy 对本特性文件零诊断；fmt 通过。

---

## 12. Rollout / 调参

**Rollout（按模块，每块一个 PR，≤200 行）**

1. `fb-config`：模型 + 解析 + 生成器接线 + 迁移（无新 UI，行为与今天一致）。
2. `fb-monitor`：nft 表 + 采样 + 状态文件（只读观测，零行为变化）。
3. `fb-alert`：判定 + bot 投递。
4. `fb-ui`：菜单 + i18n。

每步先部署 `refletke`（流量最小）观察 24 小时，再推其余四台。

**阈值调参（7 天后，单独任务）**

1. 导出四台的 `fbmon.json` 小时桶，算 p50/p95/p99。
2. 取 `max(p99 × 3, 当前值)` 作为 warn，`max(p99 × 10, warn × 4)` 作为 critical。
3. 只改 `fallback.json` 默认值（代码里的默认常量同步），不动现有节点已保存的配置——避免升级把运维调过的值覆盖。
4. 若 7 天内出现「warn 但人工确认是正常业务」，把该节点加入白名单字段（V1.1 再议）。

**Rollback**

- 代码：回退二进制即可；`fallback.json` 缺失时按默认 `random`，与当前行为等价。
- nft：`nft delete table inet wwps_fbmon` 一条命令，不影响任何流量。
- 档位：`off` 档可完全关闭限速（显式应用后生效）。

---

## 13. Out of Scope（Deferred）

- **自定义档位 + 数值输入向导**：先只给三档；V1.1 再加。
- **按 IP 的连接数限制**：同一端口同时服务真实用户与回落，L4 无法区分。
- **自动升档**：告警只建议，不执行；限速会改变真实用户体验。
- **跨节点聚合**：各节点独立部署，无中心。
- **按目标 IP 分桶计数**：单计数器足够判定总量。
- **同 ASN 选池**（上游建议的根治方案）：属 `sni_probe` 选池策略，另立 spec。
- **告警写 journal**：V1 只发 bot。
- **`/proc/net/nf_conntrack` 精确会话统计**：当前用 `/proc/net/tcp` 足够，且更便宜。

---

## 14. Open Questions

1. 暂定阈值（50 GiB/天、20 并发、500 新建/分）是否接受作为 7 天观察期的初值？我按"绝不误报"选的，代价是早期可能漏报小流量蹭用。
2. `refletke` 的 60 no-referrer 目标里有一个是 WordPress 托管站（实测返回 `*.wordpress.com` 证书），这类目标的 IP 集合广义上是共享 CDN 出口，计数会偏高。要不要在集合构建时排除"证书与目标域名不一致"的目标？（需要额外一次 TLS 探测，成本在应用档位时一次性发生。）
3. 监控 tick 的 60 秒间隔是否合适？改成 30 秒能把突发看得更细，但把"每 tick < 15 ms"的预算压到 0.05 % CPU → 0.1 %。
