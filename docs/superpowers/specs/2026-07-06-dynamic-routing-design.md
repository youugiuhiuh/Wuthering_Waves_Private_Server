# Xray 动态路由规则管理设计

## 概述

将 Xray `00_base.json` 中的 `routing.rules` 改为用户通过 Telegram Bot 可动态开启/关闭的管理系统。默认仅开启 `geoip:private`（私有IP封锁），其余规则（CN IP/CN域名/BT/广告/OpenAI直连）由用户按需启用。

## 设计原则

参考 [zxcvos/Xray-script](https://github.com/zxcvos/Xray-script) 的路由管理实现：

- 规则的存在/不存在即代表启用/禁用
- 无需额外配置文件存储开关状态
- 直接操作 `00_base.json` 的 `routing.rules` 数组

## 规则定义

每个规则在代码中定义为常量，包含以下字段：

```rust
struct RuleDef {
    id: &'static str,             // 唯一标识，如 "cn_ip"
    rule_type: &'static str,      // "ip" | "domain" | "protocol"
    targets: &'static [&'static str], // 匹配目标，如 ["geoip:cn"]
    outbound: &'static str,       // "blocked" | "direct"
    default_enabled: bool,        // 首次安装是否默认开启
}
```

| id | 名称 | 类型 | 目标 | 出站 | 默认 |
|----|------|------|------|------|------|
| `private_ip` | 私有IP封锁 | ip | `geoip:private` | blocked | 是 |
| `cn_ip` | 中国IP封锁 | ip | `geoip:cn` | blocked | 否 |
| `cn_domain` | 中国域名封锁 | domain | `geosite:cn` | blocked | 否 |
| `private_domain` | 私有域名封锁 | domain | `geosite:private` | blocked | 否 |
| `bt` | BT协议封锁 | protocol | `bittorrent` | blocked | 否 |
| `ads` | 广告域名封锁 | domain | `geosite:category-ads-all` | blocked | 否 |
| `openai` | OpenAI直连 | domain | `geosite:openai` | direct | 否 |

## 架构分层

```
Telegram Bot Handler (adapters/telegram/handlers/xray/routing.rs)
  ↓ toggle(id)
Core Logic (core/xray/routing.rs) — RoutingManager
  ↓ 读写
00_base.json (routing.rules 数组)
  ↓ reload
Xray-core
```

## 核心实现

### RoutingManager（`core/xray/routing.rs`，新建）

```rust
pub struct RoutingManager;

impl RoutingManager {
    /// 从 00_base.json 读取当前启用的规则 ID 列表
    pub fn get_enabled_ids() -> Vec<String>;

    /// 获取所有规则及当前状态，(ruleDef, enabled)
    pub fn get_all_with_status() -> Vec<(&RuleDef, bool)>;

    /// 切换规则状态：
    /// - 存在 → 从 rules 数组中删除
    /// - 不存在 → 插入到末尾
    /// 写回 00_base.json → reload_core()
    pub async fn toggle(rule_id: &str) -> Result<bool>;
}
```

### Toggle 流程

1. 读 `00_base.json` → 解析 `routing.rules` 数组
2. 遍历查找 `ruleTag == id`：
   - 找到 → 删除该对象（禁用）
   - 未找到 → 从 `ROUTING_RULES` 常量构造新对象并插入末尾（启用）
3. 写回 `00_base.json`
4. 调用 `MaintenanceManager::reload_core()` 热重载

### ensure_base_config 变更

首次创建 `00_base.json` 时，仅写入 `default_enabled=true` 的规则（即只有 `private_ip`）。

不再幂等跳过——每次调用都保证规则状态与用户配置一致。

## Telegram Bot 接口

**触发**: `/routing` 命令或管理菜单入口

**UI**: `InlineKeyboard` 列表，每个规则一行

```
📋 路由规则管理

✅ 私有IP封锁       [禁用]
⬜ 中国IP封锁       [启用]
⬜ 中国域名封锁     [启用]
⬜ 私有域名封锁     [启用]
⬜ BT协议封锁       [启用]
⬜ 广告域名封锁     [启用]
⬜ OpenAI直连       [启用]

活跃规则: 2 条
```

点击按钮 → `callback_query` → `toggle(id)` → 编辑消息显示新状态 + `"已开启/关闭 {name}"`

## 国际化（i18n）

支持 zh/en/ja 三种语言，使用 `rust_i18n::t!()` 宏。

需要新增的 i18n key：

| key | zh | en |
|-----|----|----|
| `routing.title` | 路由规则管理 | Routing Rules |
| `routing.active_count` | 活跃规则: %{count} 条 | Active rules: %{count} |
| `routing.toggled_on` | ✅ 已开启 %{name} | ✅ %{name} enabled |
| `routing.toggled_off` | ❌ 已关闭 %{name} | ❌ %{name} disabled |
| `routing.reload_failed` | Xray 重载失败 | Xray reload failed |
| `routing.rule_names.private_ip` | 私有IP封锁 | Block Private IP |
| `routing.rule_names.cn_ip` | 中国IP封锁 | Block China IP |
| `routing.rule_names.cn_domain` | 中国域名封锁 | Block China Domain |
| `routing.rule_names.private_domain` | 私有域名封锁 | Block Private Domain |
| `routing.rule_names.bt` | BT协议封锁 | Block BitTorrent |
| `routing.rule_names.ads` | 广告域名封锁 | Block Ads |
| `routing.rule_names.openai` | OpenAI直连 | OpenAI Direct |

## 文件变更

| 操作 | 文件 | 说明 |
|------|------|------|
| 新建 | `core/xray/routing.rs` | `RoutingManager` + `ROUTING_RULES` 常量 |
| 修改 | `core/xray/mod.rs` | `pub mod routing;` |
| 修改 | `core/xray/config.rs` | `ensure_base_config` 改为动态生成 |
| 新建 | `adapters/telegram/handlers/xray/routing.rs` | Bot handler |
| 修改 | `adapters/telegram/handlers/xray/mod.rs` | 注册 handler |
| 修改 | `locales/zh.yml` | 添加 routing 相关 i18n key |
| 修改 | `locales/en.yml` | 添加 routing 相关 i18n key |

## 与现有 PR (#140) 的关系

当前 PR `feat/xray-routing-cn-block` 将 `geoip:cn` + `geosite:cn/private` 硬编码进 `00_base.json`。
动态路由上线后：
- PR 作为过渡方案合入（已提交至 main）
- 动态路由将 `ensure_base_config` 改为只写 `private_ip` 默认规则
- 用户通过 Bot 启用其他规则覆盖硬编码行为

## 未纳入范围

- 自定义规则（用户新增任意规则）
- Nginx SNI 分流
- WARP 分流路由
- 上下行分离

## 测试覆盖

- `test_toggle_enables_rule`: toggle 后规则出现在 00_base.json
- `test_toggle_disables_rule`: 再次 toggle 后规则被移除
- `test_default_only_private_ip`: 首次 ensure_base_config 只有 1 条规则
- `test_get_all_with_status`: 列出所有规则及状态正确
