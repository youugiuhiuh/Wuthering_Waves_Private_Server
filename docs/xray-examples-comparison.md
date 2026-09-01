# Xray-examples 与本项目对照：适用于直连的配置

> 对照对象：[XTLS/Xray-examples](https://github.com/XTLS/Xray-examples)（2026-08 拉取）
> 结论先行：本项目（WWPS）采用的协议栈全部属于**直连型**（客户端直连 VPS 公网 IP，不套 CDN、不经过中转）。
> 下方按「直接匹配 / 其他直连可选 / 不适用于直连」三档列出。

## 1. 本项目实际生成的入站协议栈

| 组件 | 协议 | 传输 | 安全 | 备注 |
|------|------|------|------|------|
| wwps-core（Xray-core） | VLESS | TCP | REALITY + flow `xtls-rprx-vision` | 生成 `vless_reality_vision` 入站 |
| wwps-core（Xray-core） | VLESS | XHTTP（mode auto，随机 path） | REALITY | 生成 `vless_xhttp_reality` 入站 |
| wwps-core（Xray-core） | VLESS | mKCP | TLS + finalmask | 生成 `vless_kcp` 入站 |
| wwps-box（Sing-box） | Hysteria2 | QUIC/HTTP3 | TLS + brutal | 支持端口跳跃 |
| wwps-box（Sing-box） | TUIC | QUIC | TLS | — |

代码位置：`rust/aegis/src/core/xray/config.rs`（`Proto::{Vision,XHTTP,Kcp}`、`build_reality_vless_inbound`、`build_kcp_inbound`）、`rust/aegis/src/core/singbox/hysteria2.rs`、`tuic.rs`。

## 2. 直接匹配本项目（优先参考，全部直连）

| Xray-examples 目录 | 与本项目对应 | 参考价值 |
|------|------|------|
| `VLESS-TCP-XTLS-Vision-REALITY/` | wwps-core `vless_reality_vision` 同款（REALITY + XTLS-Vision） | 极简服务端/客户端配置，含 dest 与 serverNames 留空绕过限速的说明；本项目生成链路 `vless://...#xtls-rprx-vision` 与其同构 |
| `VLESS-XHTTP-Reality/minimal-steal_others/` | wwps-core `vless_xhttp_reality` 同款（REALITY + XHTTP mode auto） | 含 `server-block-cn.jsonc` **禁回国流量路由**——与项目路由模块（`core/xray/routing.rs`）思路一致，值得对照；client 端 `xhttp` 参数与项目 `generate_client_link` 完全对齐 |
| `VLESS-TCP-REALITY (without being stolen)/` | REALITY 防偷跑专题 | dest 被扫描滥用的防护手段（不用 CF 站点做 dest），对应项目 REALITY 目标 SNI 选择策略（配合 `sni_tester` 探测可用 SNI） |
| `VLESS-mKCPSeed/` | wwps-core `vless_kcp` 同款传输（mKCP） | 本项目额外叠加 TLS + finalmask，seed 机制同源，可对照 kcpSettings 参数 |
| `VLESS-gRPC-REALITY/` | REALITY 家族，传输为 gRPC | 本项目未实现 gRPC 传输；若未来为 QUIC 被阻断的客户端提供回退通道可参考 |
| `Hysteria2/` | wwps-box Hysteria2 同款 | 本项目在此基础上做端口跳跃（`hy2_batch`）；示例只含服务端 inbound 与客户端 outbound，可作最小对照 |
| `VLESS-TCP-TLS (minimal by rprx)/` `(maximal by rprx)/` | 经典 TLS 直连（无 REALITY） | 若某运营商 SNI 导致 REALITY 不可用时的回退方案参考 |

## 3. 社区视角：中转 vs 直连

> 依据：chika0801/Xray-examples `warning.md`（rprx 评论合集）、XTLS/Xray-examples issues #67/#75、Xray-core discussions（#1719/#1811/#1891/#2017 等）、社区中转教程。

**社区对两类场景的协议分工有明确共识：**

| 场景 | 协议选择 | 依据 |
|------|---------|------|
| **直连**（客户端 → VPS 出墙） | REALITY 系（Vision / H2 / gRPC / XHTTP）、Hysteria2、TUIC | 新一代抗封锁协议设计目标即直连；REALITY **不能过免费 CDN**（TLS 服务器认证依赖目标站可直连），本质上只适用于直连 |
| **中转**（客户端 → 中转机 → 落地机） | SS / VMess（机场主力）、REALITY H2 / gRPC | rprx 原话：*“要中转的话不能用 Vision，但其实可以 REALITY H2 / gRPC”*；*“机场会用 SS 或 VMess 中转 XTLS 出墙”*；SS22 中转客户端兼容性好、部署简单 |
| **不可中转** | XTLS Vision（flow=xtls-rprx-vision） | Vision 只支持纯净入站或另一个 Vision 入站（issue #1612），不能套在中转链路里；**中转时去掉 flow 或用 REALITY H2/gRPC** |

**社区对协议现状的评价（影响直连选型）：**

- **WSS 已被社区判 deprecated**：ALPN 恒为 `http/1.1` 一眼 WSS，内层 WS 多一次握手时序独特，TLS in TLS 特征明显（#1750）。rprx 明确 *“不要用 WSS…直连有 N 种姿势，已无任何必要”*——`VLESS-WSS-Nginx` 仅剩套 CDN 场景，且套 CDN 有 gRPC 更优。
- **REALITY 是当前直连首选**：TLS 级回落（自己偷自己），解决 CA 与回落指纹问题；偷白名单域名（非被墙、非国内镜像）即使有 TLS in TLS 特征也不封（#2317）。
- **Hysteria/TUIC（UDP 类）**：不一定封，但可能遭 QoS 限速，体验因人而异；混淆后的 UDP 包“一眼假”，只是用的人少所以暂时不是靶子（#1767）。
- **SS/Trojan 裸 TLS（旧协议）**：GFW 已能识别（SS 全随机数秒封 IP、Trojan 隔天封端口），2023 年起仅剩中转/IPLC 场景仍在使用（#2317）。
- **中转配置易错点**：中转流量必须走 **outbounds + 路由规则** 而不是 inbounds 里写落地机地址（issue #67 大量踩坑）；SS22 中转与多用户共存不兼容（issue #75）。

**对 Xray-examples 目录的定位修正（按社区口径）：**

- 官方唯一中转模板 = `ReverseProxy/`（VLESS-TCP-XTLS-WS、Vmess-TCP、Shadowsocks-2022 三个子方案），架构为 portal（有公网 IP）+ bridge（内网），issues 里几乎所有“中转”话题都指向它。
- 其余全部为直连模板。`All-in-One-fallbacks-Nginx/` 社区常配合 CDN 做 443 全家桶，也可直连——两者皆可，不属纯中转。

## 4. 其他适用于直连（本项目未实现，可作扩展参考）

以下均为「客户端 → VPS 公网 IP 直连」方案，无 CDN 依赖；本项目当前未采用。社区已不推荐的（WSS）标注：

- **VLESS 家族**：`VLESS-TCP/`（无 TLS 裸奔，仅内网/测试）、`VLESS-TCP-TLS/`、`VLESS-TCP-TLS-WS/`、`VLESS-WSS-Nginx/`（Nginx 终止 TLS；⚠️ 社区已判 deprecated，仅套 CDN 场景残留）、`VLESS-TLS-SplitHTTP-CaddyNginx/`、`VLESS-TLS-SplitHTTP-H3/`、`VLESS-XHTTP3-Nginx/`（XHTTP/3，需 Nginx QUIC）、`VLESS-GRPC/`（Caddy）、`VLESS-HTTP-Caddy/`（H2C/H3 三变体）、`VLESS-TCP-TLS-proxy protocol/`（PROXY protocol，用于反代链路保留真实 IP）
- **VMess 家族**（全部直连）：`VMess-TCP/`、`VMess-TCP-TLS/`、`VMess-Websocket/`、`VMess-Websocket-TLS/`、`VMess-HTTP/`、`VMess-HTTP2/`、`VMess-mKCPSeed/`
- **Trojan**：`Trojan-TCP-TLS (minimal)/`、`Trojan-gRPC-Caddy2／Nginx/`
- **Shadowsocks**：`Shadowsocks-2022/`、`Shadowsocks-AEAD/`、`Shadowsocks-TCP/`
- **其他**：`Socks5-TLS/`
- **全家桶**：`All-in-One-fallbacks-Nginx/`（443 单端口 fallbacks 聚合 TLS/WS/gRPC/H2/XTLS，需自有域名证书 + Nginx decoy 站点）——直连可用，但架构与项目「按需多端口、单协议生成」的设计不同，仅作了解

## 5. 不适用于直连（排除，勿采用）

| 目录 | 排除原因 |
|------|---------|
| `ReverseProxy/`（`VLESS-TCP-XTLS-WS`、`Vmess-TCP`、`Shadowsocks-2022` 三个子方案） | 官方**中转**模板：bridge/portal 架构，需要另一台有公网 IP 的设备做 portal（社区 issues #67/#75 即此方案踩坑） |
| `Serverless-for-Iran/` | serverless 中继 + 反审查特化（fragment/噪声），非直连场景 |
| `MITM-Domain-Fronting/` | 域前置，依赖接受前置的站点与 CDN，非直连 |

## 6. 社区直连推荐清单（对应本项目现状）

> 依据：Xray-core discussions #4113/#4118（XHTTP 官方提案）、#2317（2023 协议现状）、社区实测对比（2026）、warning.md。

| 场景 | 社区推荐 | 本项目对应 | 状态 |
|------|---------|-----------|------|
| 主力直连（TCP 低丢包、低延迟） | VLESS + REALITY + **Vision**（S 级伪装，完整处理 TLS in TLS） | wwps-core `vless_reality_vision` | ✅ 已实现 |
| 新传输（上下行分离、流式） | VLESS + REALITY + **XHTTP**（mode auto，可取代 gRPC） | wwps-core `vless_xhttp_reality` | ✅ 已实现 |
| 弱网/高丢包（>30% 丢包仍跑满） | **Hysteria2**（Brutal 拥塞控制） | wwps-box Hysteria2（含端口跳跃） | ✅ 已实现 |
| 移动网络/低延迟交互 | **TUIC v5**（0-RTT 握手） | wwps-box TUIC | ✅ 已实现 |
| 客户端兼容补充 | iOS 支持 REALITY 客户端少 → 额外部署 Hysteria2 兜底 | 与上方 wwps-box 一致 | ✅ 已实现 |
| 禁回国流量 | 服务端屏蔽全部境内 IP（防标记） | `ROUTING_RULES`（core/xray/routing.rs） | ⚠️ 规则已实现但**非默认启用**（cn_ip/cn_domain 默认关，仅 private_ip 默认开，需 toggle） |
| SNI 探测/选目标站 | 白名单大厂域名、非被墙、非国内镜像；IP 直连可测通 | `sni_tester` 工具 | ✅ 已实现 |
| 客户端指纹 | uTLS fingerprint=chrome（REALITY 安全默认） | 链路生成固定 `fp=chrome` | ✅ 已实现 |

**结论：本项目协议栈 = 社区直连推荐清单的完整落地，无需新增协议。** 社区补充建议：

1. **端口**：避开默认值（443 最易被扫），配合 `policy` 自定义 handshake/connIdle 超时（默认 60s/300s 是特征）；本项目随机端口 10000-60000 符合。
2. **IP 干净度**：REALITY/Vision 封禁多因 IP 被邻居/段波及，封了先换端口→换 IP→换服务商，别怀疑协议本身。
3. **共存玩法**（可选）：单 443 端口 Nginx SNI 分流 REALITY+XHTTP+Hysteria2+Web 站点五合一（#4118），多用户聚合场景可参考，单 VPS 轻量部署无需。
4. **XHTTP 上游直连、下游 CDN** 混用是 XHTTP 专属能力，本项目 XHTTP 全 REALITY 直连已够用，需要时再研究。

## 7. 建议
1. **直接对照项（第 2 节）已覆盖本项目全部协议栈**，日常维护（路由规则、参数调优、防偷跑）以此为准。
2. `VLESS-XHTTP-Reality/minimal-steal_others/server-block-cn.jsonc` 的禁回国流量路由，与项目 `ROUTING_RULES`（`core/xray/routing.rs`）可做一次逐条核对，避免遗漏境内 IP 段。
3. **直连是当前架构的正确选择**：REALITY/Hysteria2/TUIC 均为社区认定的抗封锁直连协议（REALITY 物理上无法套 CDN），符合轻量 VPS 部署目标。
4. **本项目不应引入中转**：当前栈没有 Vision 之外的“可中转”链路需求；若未来需要中转（如加落地机），遵循社区规则——中转段禁用 Vision flow，改用 REALITY H2/gRPC 或 SS-2022，且中转流量走 outbounds + 路由而非 inbounds 直写落地地址。
5. **建议默认开启禁回国**：社区（warning.md）视屏蔽境内 IP 为基本实践（防代理 IP 被标记），但本项目 cn_ip/cn_domain 默认关、需手动 toggle——可考虑把 `cn_ip` 提为 `default_enabled: true`（或安装引导时默认开启）。
