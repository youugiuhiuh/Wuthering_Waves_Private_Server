# Spec: usque-tcp（WARP over TCP/443）

> 状态：**待批准**。批准前不进入 BUILD。
> 模块 id：`usque-tcp`（kebab-case，后续 plan / 命令按此 id 选取）
> 前置：
> - `tasks/usque-tcp/FINDINGS.md` —— 真机实测（含性能与失败模式）
> - `tasks/usque-tcp/DECISIONS.md` —— 7 项已确认决策 + 5 条硬约束
> - Spec 1（已合并 `fafb0e1`）—— 提供了 `ProbeConclusion` 与 `WarpScope`

---

## Objective

**问题**：本部署所在机房（RackNerd）在出站方向丢弃**非 53 端口的 UDP**（实测 2408/500/1701/4500/8443 全部静默，而 UDP 53 与 TCP 443 正常）。Xray 的 `proxy/wireguard` 是 **UDP-only**（WireGuard 协议本身不支持跑在 TCP 上），因此 WARP 隧道永远建立不起来：`wg0` 的 `rx_bytes` 恒为 0、`tx_bytes` 持续增长，经 WARP 的流量全部静默超时。

**目标**：用 usque（Cloudflare WARP 的 MASQUE 开源实现）走 **TCP/443**，在同一台机器上建立可用的 WARP 出口；并让系统**按探测结果自动选择**出口实现。

**用户**：本项目的部署者（单管理员，经 Telegram bot 操作）。

**成功标准（一句话）**：在出站 UDP 被封的机器上，`ProbeConclusion::TcpOnly` 时用户流量经 WARP 出口可达，`cdn-cgi/trace` 返回 `warp=on`，全程无需管理员手工 SSH 干预。

**非目标（v1）**：
- 不做 TG **新**菜单（决策 Q6）——部署由 installer 完成，状态并入现有 `a_warp_status`
- 不改 sing-box 侧
- 不做 usque 自动升级（只做 pin 版本的安装与替换）
- 不启用 `nativetun` 模式（需改路由表 + root，风险高）

**超 v1 目标的追加需求**：卸载按钮语义扩大。见下方「卸载语义」章节。

---

## 真机实证（本 spec 的全部技术前提均已验证，非推断）

| 验证项 | 结果 |
|---|---|
| usque v4.2.1 在目标机上运行 | ✅ |
| `usque register` 经 TCP/443 注册 | ✅ |
| **隧道承载真实流量** | ✅ `ip=2a09:bac5:...  warp=on`（出口 IP 变为 Cloudflare） |
| 延迟 | ✅ ≈ **39 ms**（直连 262 ms） |
| 吞吐 | ✅ **9.2 – 11.8 MB/s** |
| sha256 校验可行 | ✅ 与 Release 的 `checksums.txt` 一致 |
| `socks --http2` | ✅ **可用** |
| `l4-socks` | ❌ **无 `--http2` 标志，QUIC-only，本机房必死** |

全部探针产物已清理，生产服务未受影响。

---

## 架构设计

### 数据流

```
                     探测结果自动选路
                            │
        ┌───────────────────┴───────────────────┐
        │                                       │
  WgUsable / UdpOnly                      TcpOnly / FullyBlocked*
        │                                       │
        ▼                                       ▼
  Xray [proxy-wireguard]                  Xray [socks → 127.0.0.1:1080]
  UDP 2408                                  │
        │                                   ▼
        │                            usque (socks --http2)
        │                              TCP 443 / HTTP-2
        │                                   │
        ▼                                   ▼
   Cloudflare WARP  ←────────────────→  Cloudflare WARP
        │                                       │
        └───────────────────┬───────────────────┘
                            ▼
                         目标站点

  * FullyBlocked 时两条路都不可用 → 保持 direct 出口（不启用 WARP）
```

### 关键设计点

**① 出口实现的选择是「派生的」，不是「存储的」**

`ProbeConclusion` 已经给出判据。但**探测是网络 I/O，不能放在每次配置生成的热路径上**。

设计：探测结果**缓存**（与 Spec 1 的「不缓存」相反 —— 因为用途不同：Spec 1 是用户主动点击的诊断按钮，追求实时；这里是配置生成的输入，追求确定性），并带 TTL。

```
ProbeConclusion::WgUsable      → WireGuard outbound
ProbeConclusion::UdpOnly       → WireGuard outbound
ProbeConclusion::TcpOnly       → usque socks outbound
ProbeConclusion::FullyBlocked  → direct（不启用 WARP，并告警）
```

**② usque 是独立 systemd unit，由 installer 部署**

```
/etc/wwps/usque/usque          二进制（pin 版本 + pin sha256）
/etc/wwps/usque/config.json    MASQUE 账号（register 产物）
/etc/systemd/system/wwps-usque.service
```

unit 模板沿用项目既有风格（`core/xray/installer.rs:689`）：

```ini
[Unit]
Description=wwps-usque (WARP over TCP/443)
After=network.target

[Service]
User=root
Type=simple
ExecStart=/etc/wwps/usque/usque socks --http2 -p 1080 -b 127.0.0.1 -c /etc/wwps/usque/config.json
Restart=always
RestartSec=5
LimitNOFILE=51200

[Install]
WantedBy=multi-user.target
```

**③ 健康检查必须能对付「卡死但不退出」—— 这是本 spec 最重要的设计**

上游已知问题（`FINDINGS.md` §7.1）：
- 空闲会被远端以 `H3_NO_ERROR` 断开
- issue #49：用户报告「用一阵子卡死后必须手动 kill 重启」
- PR #103：修过一个 tunnel 死锁

**systemd 的 `Restart=always` 只在进程退出时生效，对卡死完全无效。**

因此需要**三层**判定，且必须区分「进程在不在」与「功能通不通」：

```
第 1 层：进程存活        systemctl is-active wwps-usque
第 2 层：隧道功能        走 SOCKS 请求 cdn-cgi/trace，断言含 warp=on
第 3 层：判死并强杀      连续 N 次第 2 层失败 → systemctl restart wwps-usque
```

> 这一条直接沿用了项目既有的教训（`core/system/core_health.rs` 的文件头注释）：
> **「这两道防线保证的是『核心能起来且不崩』，不是『功能正常』。后者需要业务侧探测才能覆盖。」**
> 对 usque 而言，「功能正常」= 隧道真的转发了流量，这**只能**靠业务探测（第 2 层）确认。

**④ `register` 的交互式 ToS 确认必须处理**

实测：不写入 `y` 会得到 `failed to read user input: EOF`。

```
printf "y\n" | usque register
```

实现时必须显式往子进程 stdin 写入 `y`，并且要能识别注册失败（不能假设无交互）。

**⑤ 不做自动降级到 direct（v1 范围外）**

决策 Q4 选的是「主动健康检查 + 强杀重启」，不含「自动降级」。这意味着：**如果 usque 持续不健康且重启无效，用户流量会受影响。**

缓解：第 2 层探测失败时通过 bot 告警，由管理员决定是否切回 WireGuard / 关闭 WARP。记为后续独立任务。

---

## 卸载语义（用户追加需求）

**用户原话**：「我希望那个卸载 WARP 功能可以同时检查 WARP 和 usque 并卸载」

### 为什么必须成对卸载

Spec 1 的 `WarpInstaller::uninstall()` 只清 WireGuard 的两个文件：

```rust
let _ = fs::remove_file(warp_paths::ACCOUNT_FILE).await;   // warp_account.json
let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;   // 10_warp_routing.json
```

Spec 2 落地后多出 usque 的一整套资产。**只清一半会留下「幽灵状态」**：

| 残留 | 后果 |
|---|---|
| `wwps-usque.service` 仍在 | systemd 仍会拉起它，端口仍被占 |
| `/etc/wwps/usque/config.json` 仍在 | 包着一个已注册的 Cloudflare 账号（含 `access_token`） |
| Xray 指向 `127.0.0.1:1080` 的 socks outbound | 若 outbound 生成与账号文件解耦，会指向一个已死的端口 |
| 管理员以为「已经卸载了」 | 实际未清干净，且无人察觉 |

### 要求的行为（6 条）

1. **探测两套实现的实际安装状态**——不能靠「我以为装了什么」
2. **停用并移除 systemd unit**：先 `stop` 再 `disable`，删 unit 文件，然后 `daemon-reload`
3. **删除各自的数据文件**
   - WireGuard：`warp_account.json`、`10_warp_routing.json`
   - usque：`/etc/wwps/usque/config.json`、二进制、目录
4. **幂等：任一缺失都不得失败**
5. **清理后重载 Xray 核心**，使路由回到无 WARP 状态
6. **返回可读的结果**——本次实际卸载了哪些（供 bot 文案）

### 状态矩阵（需测试穷举）

| 输入状态 | 期望 |
|---|---|
| 两套都装 | 全部卸载，报告两条 |
| 只装 WireGuard | 只卸 WireGuard，报告一条，**不报错** |
| 只装 usque | 只卸 usque，报告一条，**不报错** |
| 都没装 | 报告「未检测到 WARP 实现」，**返回成功**（幂等） |
| unit 存在但进程未运行 | 仍应移除 unit 文件 |
| unit 文件缺失但数据目录存在 | 仍应删目录 |
| `stop` 失败（unit 已是 stopped） | 视为成功继续 |

### 设计：探测与执行分离

为了可测（路径硬编码为 `/etc/wwps/...`，单测跑不到），拆成：

```
纯函数：
  detect_installed_kinds(wg_exists: bool, usque_exists: bool) -> InstalledKinds
  → 可穷举 4 种组合

I/O 层：
  uninstall() 读取实际文件状态 → 调纯函数 → 按结果执行清理 → 汇总报告
```

返回值建议：

```rust
pub struct UninstallOutcome {
    pub removed_wireguard: bool,
    pub removed_usque: bool,
}

impl UninstallOutcome {
    pub fn any_removed(&self) -> bool
    pub fn i18n_key(&self) -> &'static str  // 4 种组合各自的文案 key
}
```

### 对 i18n 的影响

现有键（`warp` 命名空间）需扩展：

| 键 | 变化 |
|---|---|
| `warp.uninstall_service` | 文案改为「卸载所有 WARP 实现」 |
| `warp.uninstall_confirm` | 需说明会同时清理 WireGuard 与 MASQUE |
| `warp.uninstalling` | 不变 |
| `warp.uninstall_success` | 改为可带参数（报告卸了哪些） |
| `warp.uninstall_fail` | 不变 |
| **新增** | 4 种结果组合的文案 |

三语（`zh`/`en`/`ja`）必须同步，且 key 集合一致（可脚本校验）。

---

## Boundaries（模块边界）

### 本模块拥有

- usque 二进制与 `config.json` 的生命周期（下载、校验、注册、安装、卸载）
- `wwps-usque.service` 的生成与启停
- usque 的健康探测与强杀重启
- 出口引擎的选择逻辑（`ProbeConclusion` → outbound 类型）
- Xray 配置中 `socks` outbound 的生成（替代 wireguard outbound）

### 本模块不拥有（依赖既有）

| 依赖 | 来自 | 用法 |
|---|---|---|
| `ProbeConclusion` / `UdpVerdict` / `TcpVerdict` | Spec 1 `core/network/warp_probe.rs` | 出口选择判据 |
| `WarpScope`（Domain/Global） | Spec 1 `core/xray/warp.rs` | 决定「谁走 WARP」 |
| `WarpMode`（Default/IPv4/IPv6） | 既有 | WireGuard 路径的出口协议偏好 |
| `MaintenanceManager::reload_core()` | 既有 | 配置变更后重载 |
| `minisign` / sha256 校验工具 | 既有 `core/crypto/` | 二进制校验 |

### 明确不改动

- `rust/aegis/src/core/singbox/**`
- Spec 1 的 `warp_probe.rs` 探测逻辑（只消费其输出）
- `WarpScope` / `WarpMode` 枚举定义

---

## Assumptions I'm making

1. **`socks --http2` 的参数形式**：`-p` 端口、`-b` 绑定地址、`-c` 配置文件路径。
   已在实测中验证 `-p` / `-b` 可用；`-c` 来自 `--help`（`-c, --config string config file (default is config.json)`）。
2. **pin 的 sha256 是** `4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313`（v4.2.1 linux_amd64）。
3. **Xray 的 `socks` outbound 是无认证的本地 SOCKS5，且形状是「扁平」的。**

   官方文档（`xtls.github.io/en/config/outbounds/socks.html`）明确：

   ```json
   {
     "protocol": "socks",
     "settings": {
       "address": "127.0.0.1",
       "port": 1080
     }
   }
   ```

   ⚠️ **`address` / `port` 直接挂在 `settings` 下，不是嵌在 `servers` 数组里**——
   这与 `vless` / `vmess` / `trojan` / `wireguard` 的 `settings.servers[]` 风格**不同**。
   写错会静默失效（Xray 对未知字段宽容，见 `core_health.rs` 记录的真机教训）。
   实现时必须为此写断言测试。

   `user` / `pass` 仅在对端需要认证时才写；usque 默认无认证，故**不写**。
4. **探测缓存 TTL 取 10 分钟**：够短以跟随网络变化，够长以避免频繁发包。
5. **health check 的第 2 层复用 Spec 1 的探测常量**（`warp::TCP_PROBE_SNI` 等），不新增端点。
6. **health check 间隔 60s、连续失败 3 次才判死**：避免网络抖动导致误杀。
7. **usque 的进程名是 `usque`**，可用于 `pgrep -x`。
8. **注册失败可由退出码 + 不产生 `config.json` 双重判定**。

---

## 已定的设计决策（替代原 Open Questions）

全部 5 项已由用户确认（见 `DECISIONS.md`），并追加了 1 项新需求。

| # | 决策 | 选择 |
|---|---|---|
| OQ1 | 出口选择判据来源 | **(a) 探测缓存，TTL 10 分钟** |
| OQ2 | 探测缓存位置 | **(a) 内存** |
| OQ3 | `FullyBlocked` 时 | **(a) 保持 direct 出口 + bot 告警** |
| OQ4 | 纳入 `self_destruct` | **(a) 加入 `DESTRUCT_SERVICES`** |
| OQ5 | `WARP_SCOPE_ENFORCE` | **(b) 移除环境变量**，改为「出口引擎可用时 Global 即生效」 |
| NEW | 卸载行为 | **同时检查并卸载 WireGuard 与 usque**（见「卸载语义」章节） |

### OQ5 的连带改动

Spec 1 引入的 `enforce_global_warp()`（读环境变量）将被**替换为运行时判定**：

```
替换前：  enforce_global_warp()  -> env var WARP_SCOPE_ENFORCE
替换后：  egress_engine_available() -> 出口引擎是否真的能用
```

即：`Global` 是否生成 catch-all，取决于**运行时出口是否可用**，而非环境变量。
这比环境变量更准确（环境变量无法感知网络变化），也更难配错。

⚠️ 注意：这意味着 `build_routing_rules(rules, scope, enforce)` 的 `enforce` 参数
语义从「管理员是否显式允许」变成「出口是否可用」。签名可保留，语义需文档化。

### OQ3 的告警方式（细节）

`FullyBlocked` 时：
- 保持 direct 出口（不启用 WARP）
- **在 `a_warp_status` 里体现**（不主动推送）——与 Q6「不做新菜单」一致，
  避免引入新的通知路径

---

## 影响文件（预估 10 个）

| 文件 | 改动 |
|---|---|
| `core/usque/mod.rs`（新） | 模块声明 |
| `core/usque/installer.rs`（新） | 下载 / sha256 校验 / register（含 stdin `y`）/ 安装 / **卸载** |
| `core/usque/config.rs`（新） | `config.json` 读写、unit 模板生成 |
| `core/usque/health.rs`（新） | 三层健康检查 + 强杀重启 |
| `core/usque/engine.rs`（新） | `ProbeConclusion` → outbound 类型选择 + 内存缓存（TTL 10min） |
| `core/xray/warp.rs` | 生成 `socks` outbound 的分支；`enforce` 改由运行时判定 |
| `core/xray/installer.rs` | **`uninstall()` 成对清理 WireGuard + usque**；`UninstallOutcome` |
| `core/paths.rs` | usque 路径常量 + `DESTRUCT_SERVICES` 增 `wwps-usque` |
| `shared/handlers/warp.rs` | `a_warp_status` 体现 usque 健康；卸载文案 |
| `resources/i18n/{zh,en,ja}.yml` | 卸载相关键扩展 + usque 状态键（三语同步） |

---

## 验证方式

**单元测试（TDD）**
- sha256 校验：正确哈希通过 / 错误哈希拒绝
- pin 版本解析：从固定 URL 构造
- `ProbeConclusion` → outbound 类型映射的穷举（4 态）
- 探测缓存：命中 / 过期 / 未探测
- unit 模板生成：含必需字段（`Restart=always`、`ExecStart` 含 `socks --http2`）
- register 的 stdin 交互：断言写入了 `y`
- 健康判据：`warp=on` 判定、连续失败计数、达阈值触发重启
- **禁止 l4-socks** 的断言（防回归：见硬约束 1）
- **Xray socks outbound 是扁平形状**（`settings.address`/`settings.port`，非 `servers[]`）
- **卸载状态矩阵穷举**（7 种输入组合，见「卸载语义」章节）
- `UninstallOutcome` 的 `i18n_key()` 4 种组合互不相同
- `detect_installed_kinds()` 纯函数穷举 4 种组合
- **幂等**：两套都不存在时 `uninstall()` 返回成功而非错误

**集成测试**
- 真实的 `cdn-cgi/trace` 解析（用 fixture 字符串，不打网络）

**真机验证（合并后由管理员决定）**
- 装 usque → 探测 → `warp=on`
- 强杀 usque → 健康检查发现 → 自动重启

**质量门禁**：`cargo fmt` + `clippy -D warnings` + `nextest` + `doc tests`

---

## 风险

| 风险 | 缓解 |
|---|---|
| 无签名，仅 sha256 | pin 版本 + pin sha256 硬编码（决策 Q3） |
| 上游死锁导致卡死 | 三层健康检查 + 强杀（决策 Q4） |
| usque 挂了且重启无效 | bot 告警；自动降级记为后续任务 |
| 有人"优化"成 l4-socks | 硬约束 1 + 专门的防回归测试 |
| 两套 WARP 并存的复杂度 | 出口选择集中在一处（`engine.rs`），不散落 |
| `register` 可能被 Cloudflare 限流 | 明确错误处理；失败不破坏既有配置 |
