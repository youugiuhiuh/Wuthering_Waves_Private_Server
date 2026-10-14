# Spec 2 前置调研：usque / WARP-over-TCP-443

> 状态：**实证完成**。以下全部为真机实测结果，非推断。
> 实测环境：racknerd-534f8f8（107.174.245.44），2026-10-14
> 全部探针产物已清理，生产服务未受影响（4 个 wwps-* 服务 active，用户连接数 4）。

## 结论一句话

**在出站 UDP 2408 被封的机器上，usque + MASQUE/HTTP2 走 TCP/443 可以建立完整的 WARP 出口，且实测比直连更快。**

## 1. 决定性验证：隧道确实承载真实流量

```
直连出口:   ip=107.174.245.44                  warp=off
经 usque:   ip=2a09:bac5:cad3:3228::4ff:3f    warp=on
```

`warp=on` 且出口 IP 变为 Cloudflare 段。这是在 UDP 2408 被上游丢弃的机器上实现的。

## 2. 性能实测（出乎意料地好）

| 指标 | 直连 | 经 usque 隧道 | 说明 |
|---|---|---|---|
| 延迟（5 次均值） | ~262 ms | **~38 ms** | 隧道 **快 6.9 倍** |
| 吞吐（10 MB） | — | **9.01 MB/s（~72 Mbps）** | 单连接 |
| CPU | — | 3.0% | `usque socks --http2` |
| 内存 (RSS) | — | 20 MB | |

延迟更低的原因：直连走 `colo=SEA` 但路由绕远；WARP 的 MASQUE 接入点路由更优。

## 3. 版本与供应链事实

| 项目 | 值 |
|---|---|
| 版本 | **v4.2.1**（2026-10 最新） |
| 资产 | `usque_4.2.1_linux_amd64.zip`（5,132,812 B） |
| 校验 | **仓库提供 `checksums.txt`**（sha256） |
| 实测校验 | ✅ 我下载后 `sha256sum` 与 `checksums.txt` **完全一致** |
| 二进制 | Go 静态编译，12.9 MB，`amd64` |
| **签名** | ❌ **无 GPG / minisign** —— 只有 sha256 |

**供应链风险**：无签名，只有 sha256。但 sha256 来自同一 GitHub Release，属于「同一信任域自证」，防不了仓库被投毒。缓解：pin 版本 + pin sha256（硬编码在代码里），而非每次拉 latest。这与本项目既有做法一致（`core_upgrade.rs` 也用 sha256；上游 Xray 同样无 minisign，见其注释）。

## 4. 环境前置条件（本机全部满足）

| 条件 | 实测 |
|---|---|
| 架构 | `x86_64` ✅ |
| glibc | 2.43 ✅ |
| `curl` / `unzip` | 均存在 ✅ |
| `github.com:443` | ✅ 通 |
| `objects.githubusercontent.com:443` | ✅ 通 |
| `api.cloudflareclient.com:443`（注册用） | ✅ 通 |

## 5. 交互陷阱（必须在设计里处理）

`usque register` **需要交互确认 ToS**：

```
You must accept the Terms of Service (https://www.cloudflare.com/application/terms/) to register.
Do you agree? (y/n):
```

不处理会得到 `failed to read user input: EOF`。实测 `printf "y\n" | usque register` 可行。

⚠️ **这意味着自动化注册需要往子进程 stdin 写入 `y`** —— 设计时必须显式实现，不能假设 register 是无交互的。

## 6. 运行模式选择

usque 提供的模式（`usque --help`）：

| 模式 | 说明 | 是否适用 |
|---|---|---|
| `socks` | SOCKS5 代理（支持 UDP associate） | ✅ 已实测可行 |
| **`l4-socks`** | **TCP-only** SOCKS5，更快 | ⚠️ 丧失 UDP |
| `http-proxy` | HTTP CONNECT | 备选 |
| `l4-http-proxy` | TCP-only HTTP | 备选 |
| `nativetun` | 建真实 TUN 接口 | ⚠️ 需 root + 会改路由表 |
| `portfw` | 端口转发 | ❌ 不适用 |

实测启动日志会提示：
```
Hint: l4-socks is faster for TCP-only SOCKS use cases.
```
但本项目需要 UDP 转发（用户有 QUIC/DNS/游戏流量），故**倾向 `socks`（非 l4）**。性能代价需实测确认。

## 7. 关键运行时行为（来自上游文档 + 实测）

### 7.1 空闲会断，靠「有出站流量」才重连

实测日志：
```
Tunnel idle. Waiting for outbound activity before reconnecting...
Detected outbound activity (64 bytes). Reconnecting...
Establishing MASQUE connection to 162.159.198.2:443
Connected to MASQUE server
```

上游 README「Known Issues」明确：
- 远端会因长时间空闲以 `H3_NO_ERROR` 断开
- **v1.5.0 起改为「只在有真实出站流量时重连」**（`--always-reconnect` 可恢复旧行为）
- 上游 issue #49：有用户报告「用一阵子后卡死需重启进程」
- 上游 PR #103：修了一个 tunnel 死锁

**→ 这是最大的运维风险。必须设计健康检查 + 自动重启，不能只靠 `Restart=always`。**

### 7.2 ALPN 与端点（修正我上一轮的一个误解）

usque 用自己的 `endpoint_h2_v4`，**不依赖 ALPN 协商**（日志显示直连 `162.159.198.2:443` 成功）。
我上一轮探测用 `162.159.197.1`（ALPN=h2）是对的——两者不矛盾：一个能协商 h2，一个不说 ALPN 但照样跑 HTTP/2。

配置里相关字段：
```
endpoint_v4       162.159.198.2
endpoint_v6       2606:4700:103::2
endpoint_h2_v4    162.159.198.2
endpoint_h2_v6    (空)
```

### 7.3 没有 WireGuard 支持

usque 是 MASQUE 客户端，**不支持 WireGuard**。所以它不是「给现有 WARP 加个 TCP 开关」，而是**替换实现**。

## 8. 配置内容（实测 register 产物，已脱敏）

```
private_key       <164 字符，ECDSA P-256 ASN.1 DER base64>
endpoint_v4       162.159.198.2
endpoint_v6       2606:4700:103::2
endpoint_h2_v4    162.159.198.2
endpoint_h2_v6    (空)
endpoint_pub_key  <178 字符>
id                2f165280-0944-49d4-a214-95e5df07b81f   (device id)
access_token      <36 字符>
ipv4              172.16.0.2
ipv6              2606:4700:110:8568:75b2:c861:d39:3587
```

注意：**这与现有 `warp_account.json` 的字段完全不同**（那是 WireGuard 的 `private_key`/`public_key`/`reserved`/`client_id`）。两者不能共用同一个文件语义。

## 9. 与现有架构的关系

```
现状（已合并的 Spec 1）:
  Xray [proxy-wireguard] ──UDP 2408──> ✗ 被机房丢弃
  Xray [proxy-warp/warp outbound]      wg0 rx_bytes 恒为 0

Spec 2 目标:
  Xray ──> [socks5://127.0.0.1:PORT] ──> usque ──TCP 443/H2──> Cloudflare ──> 目标
           （新的 outbound）              （新系统组件）
```

即：**Xray 的 wireguard outbound 被替换为一个指向本地 usque 的 socks outbound**。

## 10. 待决策项（进入 interview）

见 `tasks/usque-tcp/OPEN-QUESTIONS.md`

## 11. 已确认的风险清单

| 风险 | 严重度 | 证据 |
|---|---|---|
| 无签名，仅 sha256 | 中 | Release 只有 `checksums.txt` |
| 空闲断连 + 已知死锁 | **高** | 上游 Known Issues + issue #49 + PR #103 |
| register 需交互确认 ToS | 低 | 实测 |
| 新增第三方二进制依赖 | 中 | 需版本 pin + 升级策略 |
| 与现有 WireGuard 路径并存的复杂度 | 中 | 两套账号/配置格式 |
| `l4-socks` 丧失 UDP 支持 | 中 | 需选模式 |

---

# 增补：SOCKS 模式实测对比（用户要求「先实测性能再定」）

> 实测环境同上。全部产物已清理，生产服务未受影响。

## 结论：**必须用 `socks`，`l4-socks` 不可用**

不是因为性能，而是因为 **`l4-socks` 根本不支持 TCP**。

## 模式 × `--http2` 支持矩阵（实测 `usque <mode> --help`）

| 模式 | 支持 `--http2`？ | 在本机房可用？ |
|---|---|---|
| **`socks`** | ✅ 支持 | ✅ **可用**（含 UDP ASSOCIATE） |
| `http-proxy` | ✅ 支持 | ✅ 备选 |
| `nativetun` | ✅ 支持 | ⚠️ 需改路由表 |
| **`l4-socks`** | ❌ **不支持** | ❌ **QUIC/UDP-only，本机房必死** |
| `l4-http-proxy` | ❌ 不支持 | ❌ 同上 |

实测报错：
```
$ ./usque l4-socks --http2 -p 10812 -b 127.0.0.1
Error: unknown flag: --http2
```

`l4-socks` 不带 `--http2` 时能起（走 QUIC），日志显示 `L4 SOCKS proxy listening on 127.0.0.1:10814`，
但通过它请求只拿到 **1076 B/s**（几乎为零）——正是 UDP 被封的表现。

**→ 上游 README 里「l4-socks is faster for TCP-only SOCKS use cases」的提示，前提是 QUIC 可用。**
在本机房（出站 UDP 被封）下，`l4-socks` 完全不可用。

## `socks`（含 UDP）性能实测 —— 3 轮 × 20 MB

| 轮次 | 速度 | 耗时 |
|---|---|---|
| #1 | 10.48 MB/s | 1.91 s |
| #2 | 9.20 MB/s | 2.17 s |
| #3 | 11.79 MB/s | 1.70 s |

- 延迟（5 次）：`0.063 / 0.039 / 0.041 / 0.039 / 0.048` s ≈ **39 ms**
- 资源：CPU 22.4%（下载中），RSS 19.1 MB

对比直连延迟 ~262 ms，隧道约 **6.7 倍更快**。

## 对 Spec 2 的含义

1. 模式固定为 **`socks --http2`**，无争议。
2. 上游「用 l4 更快」的建议在本部署下**不成立**，需在设计文档里显式记录原因，
   否则未来有人会「优化」成 l4-socks 并把节点搞挂。
3. UDP 转发能力得以保留（`socks` 支持 UDP ASSOCIATE），
   对 3 个 Hysteria2 + 10 个 KCP 入站的 UDP 流量友好。
