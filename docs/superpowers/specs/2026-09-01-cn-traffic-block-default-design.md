# 禁回国流量默认启用设计

日期：2026-09-01
状态：已批准（brainstorming 设计评审通过）
关联：docs/xray-examples-comparison.md（社区口径对照）

## 背景

本项目（WWPS）为海外 VPS 部署的代理控制栈（wwps-core = Xray-core，wwps-box = Sing-box）。
社区（XTLS warning.md、Xray-examples `server-block-cn.jsonc`）将「服务端屏蔽境内 IP/域名」视为基本实践：
用户经代理访问境内网站时，境内服务端记录的是 VPS IP，导致 **VPS IP 被标记为代理 IP**，进而被封 IP/封端口，节点对全体用户失效。

当前 `ROUTING_RULES`（`rust/aegis/src/core/xray/routing.rs`）中：
- `private_ip`（geoip:private → blocked）：`default_enabled: true`（默认唯一启用项）
- `cn_ip`（geoip:cn → blocked）：`default_enabled: false`
- `cn_domain`（geosite:cn → blocked）：`default_enabled: false`

`00_base.json` 生成时仅写入 `default_enabled` 的规则（`config.rs` ensure_base_config，`.filter(|r| r.default_enabled)`），
即新部署默认只禁私有 IP，**禁回国流量默认关闭**，需管理员 Telegram toggle 手动开启。

## 目标

将禁回国流量（`geoip:cn` + `geosite:cn` → blocked）设为**新部署的默认值**，使新节点开箱即用即获得防标记保护。

## 决策（brainstorming 已确认）

| # | 决策点 | 结论 | 理由 |
|---|--------|------|------|
| D1 | 屏蔽范围 | `cn_ip` + `cn_domain` **两条都默认开** | 社区口径（warning.md「屏蔽所有境内 IP」；官方 `server-block-cn.jsonc` 即 IP+域名两条规则，`domainStrategy: IPIfNonMatch`） |
| D2 | 存量部署 | **仅新部署生效**，存量靠管理员手动 toggle | `ensure_base_config` 幂等（`if exists return Ok`），不引入迁移逻辑；最小改动 |
| D3 | 覆盖范围 | **只动 xray（wwps-core）**，不动 singbox | singbox 无路由规则、需另引 sing-geoip/sing-geosite 数据集，改动大；本次最小 diff |

## 改动范围

| 文件 | 改动 |
|------|------|
| `rust/aegis/src/core/xray/routing.rs` | `cn_ip`、`cn_domain` 的 `default_enabled: false → true`；新增测试断言两条规则默认开 |
| `rust/aegis/src/core/xray/config.rs` | `test_ensure_base_config_structure`：`rules.len() == 1` → `3`，断言包含 `cn_ip`/`cn_domain` |

无其他文件改动：Telegram UI 规则列表走 `get_all_with_status` 通用枚举，无硬编码；i18n 无 cn 规则文案。

## 行为说明

- **新部署**：`ensure_base_config` 生成的 `00_base.json` routing.rules 含 3 条，顺序保持
  `ROUTING_RULES` 现有顺序：`private_ip` → `cn_ip` → `cn_domain`；`domainStrategy: IPIfNonMatch` 不变。
- **存量部署**：已存在的 `00_base.json` 不被改写，维持现状；管理员仍可通过 Telegram toggle 开关任意规则。
- **规则顺序无冲突**：`geosite:cn` 与 `geosite:openai`（direct）无交集，openai 不被误挡。
- **路由不影响节点连接**：Xray 路由规则只作用于「客户端已连上节点后」的出站选择（direct/blocked），
  不参与客户端 → 节点的 inbound 握手，因此开启禁回国**不会导致用户连不上节点**。

## 风险与边界

- 新部署用户经代理访问境内网站/App → 断流（设计意图；境内站点请直连，不走代理）
- REALITY 目标站为境外白名单域名（`sni_tester` 探测），不受 geoip/geosite 规则影响
- 不涉及 singbox（Hysteria2/TUIC 入站流量不受禁回国保护，D3 权衡）
- 无迁移逻辑（D2 权衡）

## 验证

- `cargo test`：routing.rs 新增用例 + config.rs 结构用例
- `cargo fmt && cargo clippy -- -D warnings`（rust-lint-format 技能强制门槛）

## 后续（不在本次范围）

- 若需覆盖 singbox：引入 sing-geoip/sing-geosite 数据集与 route 规则（独立设计）
- 若需存量自动迁移：base config 加规则版本标记，区分「从未配置」与「管理员主动关闭」（独立设计）
