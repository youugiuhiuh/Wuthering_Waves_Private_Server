# sing-box 路由管理移植设计

日期：2026-09-02
状态：已批准（brainstorming 完成）

## 背景

Xray（wwps-core）已有完整的路由管理：`core/xray/routing.rs` 的 `RoutingManager` + 7 条 `ROUTING_RULES`（Telegram `m_routing` 菜单 + `routing_toggle:` 回调），写盘到 `00_base.json` 的 `routing.rules`，更新后 `reload_core()`。

sing-box（wwps-box）目前 `00_base.json` 只有 `route: {default_domain_resolver: "dns"}`，**无任何路由规则**，也无管理界面。

本次目标：为 sing-box 移植同等路由管理能力（6 条规则、Telegram 开关菜单、定时更新规则集文件）。

## 关键事实（已实测验证，sing-box v1.14.0）

1. **sing-box 1.12+ 已移除 `geosite`/`geoip` 规则字段**，必须使用 `rule_set`（.srs 二进制 / .json source 格式）。
2. **`rule_set` 位于 `route` 对象内**（`route.rule_set`），不是顶层字段。
3. **规则对象不接受自定义字段**（实测 `route.rules[1].ruleTag` 报 `json: unknown field "ruleTag"`）→ 开关状态必须用**语义匹配**（规范 JSON 深比较），不能复用 Xray 的 `ruleTag` 方案。
4. **`.db` 不能直接被 `rule_set` 引用**（只认 source/binary），需本机转换：
   - `sing-box geosite export <category> -f geosite.db -o xxx.json`
   - `sing-box rule-set compile --output xxx.srs xxx.json`
   - 实测：geosite-cn.json 445KB → .srs 55KB；geoip-cn.json 208KB → .srs 34KB
   - 编译/运行版本一致（installer 装最新版），规避已知 compile bug（#3740）
5. **规则实际生效已验证**：google 放行、baidu 被 block、114.114.114.114 被 block，日志 `rule_set=geoip-cn => route(block)`。
6. **`protocol: bittorrent` 匹配需要 inbound 开 sniff**，本项目 sing-box inbound 未开 → **bt 规则不移植**（用户决策）。
7. `ip_is_private` 为 sing-box 内置规则字段，无需规则集。

## 规则定义（6 条）

| id | 匹配 | outbound | 默认启用 |
|---|---|---|---|
| private_ip | `ip_is_private: true` | block | ✅ |
| cn_ip | rule_set `geoip-cn` | block | ✅ |
| cn_domain | rule_set `geosite-cn` | block | ✅ |
| private_domain | rule_set `geosite-private` | block | ⬜ |
| ads | rule_set `geosite-category-ads-all` | block | ⬜ |
| openai | rule_set `geosite-openai` | direct | ⬜ |

规则 JSON 形态（sing-box 简写语法）：

```json
{"ip_is_private": true, "outbound": "block"}
{"rule_set": ["geoip-cn"], "outbound": "block"}
```

## 规则集文件来源（官方）

| 仓库 | 文件 | 大小 | 更新频率 | 下载 URL（latest 重定向，实测 200） |
|---|---|---|---|---|
| SagerNet/sing-geosite | geosite.db | ~3.6MB | 每日 | `https://github.com/SagerNet/sing-geosite/releases/latest/download/geosite.db` |
| SagerNet/sing-geoip | geoip.db | ~3.8MB | 每月（12 日） | `https://github.com/SagerNet/sing-geoip/releases/latest/download/geoip.db` |

提取分类（5 个 .srs 写入 `/etc/wwps/wwps-box/rule-set/`）：
- geosite：`cn`、`private`、`category-ads-all`、`openai`
- geoip：`cn`

## 定时任务拆分

| 任务 | cron | 内容 |
|---|---|---|
| `GeoUpdate`（现有，`0 4 * * 0`） | 每周日 | Xray geodata（geoip.dat + geosite.dat）+ **sing-geosite**（域名库，每日更新源） |
| `GeoIpUpdate`（**新增**，`0 4 13 * *`） | **每月 13 日** | **sing-geoip**（IP 库）→ geoip-cn.srs |

官方每月 12 日发布 geoip，13 日拉取确保拿到最新。已有部署的 scheduler state 文件缺 `GeoIpUpdate` 任务 → 加载时补注册（迁移逻辑）。

## 文件改动（8 处）

| 文件 | 改动 |
|---|---|
| **新** `rust/aegis/src/core/singbox/routing.rs` | `RuleDef` + `ROUTING_RULES`(6) + `rule_def_to_json` + `RoutingManager`：`read_rules`/`write_rules`（读写 00_base.json 的 `route.rules`）/`get_all_with_status`（语义匹配判断启用）/`toggle`（按规范 JSON 深比较增删）+ `CONFIG_LOCK` + 写后 `reload_core()`；迁移：base 缺 `route.rule_set` 时补入 5 项 |
| `rust/aegis/src/core/singbox/config.rs` | `ensure_base_config()`：`route.rules` 写入默认启用规则 + `route.rule_set` 5 项（type local, format binary, path 指向 rule-set 目录） |
| `rust/aegis/src/core/system/maintenance.rs` | 新 `update_singbox_rules(include_geoip: bool)`：下载 .db → export 5 分类 → compile 5 个 .srs（带进度回调，复用现有 download_file）；新 `ensure_singbox_rule_sets()`（skip-if-exists，安装时用）；`update_geodata()` 扩展为同时更新 sing-geosite（域名） |
| `rust/aegis/src/core/system/scheduler/task_types.rs` | `TaskType::GeoIpUpdate` 枚举 + display name + execute 分支 |
| `rust/aegis/src/core/system/scheduler/mod.rs` | `default()` 注册 `GeoIpUpdate`（cron `0 4 13 * *`）；加载 state 时缺新任务补注册 |
| `rust/aegis/src/core/paths.rs` | singbox 模块加 `RULE_SET_DIR = "/etc/wwps/wwps-box/rule-set"` |
| `rust/aegis/src/core/singbox/installer.rs` | install 后调用 `ensure_singbox_rule_sets()` |
| `rust/aegis/src/shared/handlers/singbox.rs` | 主菜单加"路由管理"按钮 → `m_sb_routing` 菜单 + `sb_routing_toggle:ID` 回调（仿 xray handler：`handle_routing_menu`/`handle_routing_toggle`） |
| i18n `en/zh/ja.yml` | `singbox.routing_*` 键（标题/活跃数/开关提示/失败提示）；规则名复用 `xray.routing_rule_*` |

## 配置示例（00_base.json 目标形态）

```json
{
  "log": {"level": "warning"},
  "dns": {"servers": [...]},
  "route": {
    "default_domain_resolver": "dns",
    "rule_set": [
      {"tag": "geoip-cn", "type": "local", "format": "binary", "path": "/etc/wwps/wwps-box/rule-set/geoip-cn.srs"},
      {"tag": "geosite-cn", "type": "local", "format": "binary", "path": "/etc/wwps/wwps-box/rule-set/geosite-cn.srs"},
      {"tag": "geosite-private", "type": "local", "format": "binary", "path": "/etc/wwps/wwps-box/rule-set/geosite-private.srs"},
      {"tag": "geosite-category-ads-all", "type": "local", "format": "binary", "path": "/etc/wwps/wwps-box/rule-set/geosite-category-ads-all.srs"},
      {"tag": "geosite-openai", "type": "local", "format": "binary", "path": "/etc/wwps/wwps-box/rule-set/geosite-openai.srs"}
    ],
    "rules": [
      {"ip_is_private": true, "outbound": "block"},
      {"rule_set": ["geoip-cn"], "outbound": "block"},
      {"rule_set": ["geosite-cn"], "outbound": "block"}
    ]
  },
  "outbounds": [
    {"type": "direct", "tag": "direct"},
    {"type": "block", "tag": "block"}
  ]
}
```

## 测试计划（TDD）

- `routing.rs` 单测：ROUTING_RULES 数=6；默认启用集（private_ip/cn_ip/cn_domain）；`rule_def_to_json` 各形态（ip_is_private / rule_set / outbound）；语义匹配 enabled 判断；toggle 增删 + 幂等（temp dir）
- `config.rs` 测试更新：base 含 `route.rules`（3 条默认）+ `route.rule_set`（5 项）
- `maintenance.rs` 测试：转换命令构造（export/compile args 正确）；skip-if-exists；include_geoip 参数行为
- `scheduler` 测试：`TaskType::GeoIpUpdate` serde 兼容（旧 state 文件反序列化不破坏）；cron 表达式校验
- handler 测试：路由菜单回调分发（如现有 handler 测试模式）

## 明确不做（YAGNI）

- bt 规则（需 inbound sniff，超范围）
- rule_set 远程模式（type: remote）
- 自定义规则添加（仅开关内置 6 条，与 Xray 对齐）
- WARP 类 sing-box 分流（无对应需求）
