# Xray 00_base.json 重构设计

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Xray (wwps-core) 引入 00_base.json 基础配置，消除 batch 文件中的重复 log/dns/outbounds/routing，统一与 sing-box 的架构模式。

**Architecture:** Xray `-confdir` 按字典序加载 JSON 文件并合并（数组追加、对象深度合并）。00_base.json 提供基础设施配置，batch 文件只需包含 inbounds 片段。

**Tech Stack:** Rust, tokio, serde_json, anyhow

---

## 背景

Xray 当前每个 batch 配置文件都包含完整的 log/dns/outbounds/routing，Xray -confdir 覆盖策略导致只有最后一个文件的这些字段生效，其余全是冗余。需要引入 00_base.json 与 sing-box 架构统一。

## 目标架构

```
/etc/wwps/wwps-core/conf/
├── 00_base.json                  ← log/dns/routing/outbounds（基础设施）
├── batch_reality_xxx.json       ← 仅 {"inbounds": [...]}
├── batch_xhttp_xxx.json         ← 仅 {"inbounds": [...]}
├── batch_kcp_xxx.json           ← 仅 {"inbounds": [...]}
└── 10_warp_routing.json          ← 保持自包含（专属 WireGuard outbound）
```

## 00_base.json 内容

```json
{
  "log": {
    "loglevel": "warning"
  },
  "dns": {
    "servers": [
      "https+local://1.1.1.1/dns-query",
      "https+local://8.8.8.8/dns-query"
    ],
    "tag": "dns"
  },
  "routing": {
    "domainStrategy": "IPIfNonMatch",
    "rules": [
      {
        "type": "field",
        "protocol": ["bittorrent"],
        "outboundTag": "blocked"
      },
      {
        "type": "field",
        "ip": ["geoip:private"],
        "outboundTag": "blocked"
      }
    ]
  },
  "outbounds": [
    {
      "protocol": "freedom",
      "settings": {},
      "tag": "direct"
    },
    {
      "protocol": "blackhole",
      "settings": {},
      "tag": "blocked"
    }
  ]
}
```

设计说明：
- log: 仅 loglevel，不写文件（stdout → journald）
- dns: 保留当前 DNS 服务器配置（1.1.1.1 + 8.8.8.8）
- routing: BT 屏蔽 + 私有 IP 屏蔽（木有 CN IP 屏蔽，允许中国玩家直连）
- outbounds: 直接 + 黑洞，tag 用 "blocked"（与现有代码一致）

## 修改清单

| # | 文件 | 修改内容 |
|---|------|----------|
| 1 | `config.rs` | 新增 `ensure_base_config()` — 创建 00_base.json，幂等 |
| 2 | `config.rs` | `create_standalone_config()` 只写 `{"inbounds": [...]}` 片段 |
| 3 | `config.rs` | 删除 `update_existing_config()` 函数 |
| 4 | `config.rs` | 三个 `batch_create_*` 去掉 `standalone` 参数 |
| 5 | `config.rs` | `list_all_inbound_files()` 增加 `00_` 前缀过滤 |
| 6 | `installer.rs` | `install_wwps_core_service()` 调用 `ensure_base_config()` |
| 7 | `maintenance.rs` | `reload_core()` 为 wwps-core 调用 `ensure_base_config()` |
| 8 | `main.rs` | 删除 `standalone_mode` 变量，简化 `batch_create_*` 调用 |

## 不修改

- `10_warp_routing.json` 及相关代码 — WARP 配置自包含
- sing-box 的 `00_base.json` 相关代码
- `delete_all_configurations()` 等删除函数 — 通过 `list_all_inbound_files()` 过滤自然兼容
- `BatchCreationResult` 结构

## 设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| base config 文件名 | `00_base.json` | 与 sing-box 统一 |
| 旧文件迁移 | 不做 | Xray 覆盖合并不崩溃，冗余不影响功能 |
| 07_VLESS 模式 | 删除 | 有 base 后不再需要共享文件模式 |
| standalone 参数 | 删除 | 统一 inbound-only 片段模式 |
| WARP 配置 | 保持自包含 | 有专属 outbound |
| 路由规则 | BT + 私有 IP 屏蔽 | 允许中国玩家直连 |
| 日志输出 | stdout (journald) | 与 sing-box 一致 |

## 风险

- 旧完整配置文件继续工作（冗余但不崩溃）
- `list_all_inbound_files()` 现有 `*_inbounds.json` 过滤已排除 `00_base.json`
- `10_warp_routing.json` 不匹配 `*_inbounds.json` 模式，不受影响