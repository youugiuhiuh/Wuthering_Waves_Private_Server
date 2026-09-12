# Xray 连通性检测端点直连规则 — 设计文档

- **日期**：2026-09-12
- **基线提交**：`eda8415` (1.4.2)
- **模式**：strict（新增功能、影响 >3 文件、涉及存量配置迁移）
- **范围**：仅 Xray（`wwps-core`）。**sing-box 代码明确排除，不做任何改动。**

---

## 1. 问题

`https://www.gstatic.com/generate_204` 与
`http://connectivitycheck.gstatic.com/generate_204` 被路由到 `blocked`
（blackhole），导致客户端连通性探测失败：节点本身可用，但客户端显示
"无网络"或判定节点不可用。

> **关于写法**：路由匹配的是 SNI / Host 主机名，故规则中应写
> `www.gstatic.com`，**不能**写 `https://www.gstatic.com/generate_204`。
> `https://` 与 `/generate_204` 是 scheme 与 URL 路径，不参与 routing 匹配。

### 1.1 根因

`ROUTING_RULES` 中 `cn_domain` 规则（`rust/aegis/src/core/xray/routing.rs:34-39`）：

```rust
RuleDef {
    id: "cn_domain",
    rule_type: "domain",
    targets: &["geosite:cn"],
    outbound: "blocked",
    default_enabled: true,
},
```

该规则默认启用，outbound 为 `blocked`（`blackhole`，见
`config.rs:857-860` 生成的 outbounds）。运行时生成的规则为：

```json
{"type":"field","ruleTag":"cn_domain","outboundTag":"blocked","domain":["geosite:cn"]}
```

Xray routing **顺序匹配、首条命中即停**。`geosite:cn` 收录了
`www.gstatic.com` 与 `connectivitycheck.gstatic.com`，因此探测请求命中
`cn_domain` → blackhole。

`domainStrategy: "IPIfNonMatch"` 使问题进一步固化：即使域名规则未命中，
`cn_ip`（`geoip:cn`）仍会兜底拦截。

### 1.2 证据（双数据源交叉验证）

两侧数据源独立解析，结论一致：

| 数据源 | 解析方式 | `CN` 分类条目数 |
|---|---|---|
| `geosite-cn.srs`（sing-box） | `sing-box rule-set decompile` | ~110606 |
| `geosite.dat`（Xray） | 手工解析 protobuf | 111167 |

命中结果（两种数据源完全一致）：

| 域名 | 在 `geosite:cn` | 匹配类型 | 匹配项 |
|---|---|---|---|
| `www.gstatic.com` | ✅ | `Full`（精确） | `www.gstatic.com` |
| `connectivitycheck.gstatic.com` | ✅ | `Full`（精确） | 自身 |
| `connect.rom.miui.com` | ✅ | `Domain`（后缀） | `miui.com` |
| `www.google.cn` | ✅ | `Domain`（后缀） | `cn`（整个 `.cn` TLD） |
| `cp.cloudflare.com` | ❌ | — | — |
| `msftconnecttest.com` | ❌ | — | — |
| `detectportal.firefox.com` | ❌ | — | — |
| `captive.apple.com` | ❌ | — | — |

**注意（1）**：初版候选清单中的 `cp.cloudflare.com`、`msftconnecttest.com`、
`www.google.cn` 均属未经证实的推测，经验证后已剔除。本项目使用的 geosite
库不将海外探测端点归入 `cn`。

**注意（2）：gstatic 条目全部是精确（Full）匹配，不是后缀匹配。**

```
Full(exact)  www.gstatic.com
Full(exact)  connectivitycheck.gstatic.com
Full(exact)  fonts.gstatic.com  ssl.gstatic.com  csi.gstatic.com  g0~g3.gstatic.com
Domain(suffix)  bbgstatic.com / elongstatic.com / lgstatic.com   ← 无关站点，非 gstatic 系
```

**裸 `gstatic.com` 本身不在 `geosite:cn` 中**（已验证 `exact: False, suffix: False`）。
因此 `www.gstatic.com` 这条精确条目只拦该主机本身；同家族的
`fonts.gstatic.com` / `ssl.gstatic.com` 是各自独立的精确条目。

### 1.3 附带发现（本次不处理）

`cn_domain` 命中 `domain_suffix: cn`，意味着**整个 `.cn` 顶级域**均被 blackhole。
这是既有设计意图（"禁回国流量"，见 `routing.rs:187-192` 测试名
`test_rule_def_has_cn_rules_default_enabled` 及其断言消息），本次不改动，
仅记录。

---

## 2. 决策记录

| 议题 | 决策 | 理由 |
|---|---|---|
| 规则表达方式 | 显式域名列表 | 不依赖 `geosite.dat` 库版本；`geosite:connectivity-check` 等分类在旧库中可能不存在，运行时查库失败会静默不命中 |
| 是否修改 `cn_ip`/`cn_domain` 的 outbound | 否 | 会彻底改变节点用途（禁回国的反审查配置），推翻既有设计意图 |
| 是否一并修复 sing-box | 否 | 用户明确排除 |
| 存量配置迁移语义 | 无条件植入并启用 | 与 `ensure_base_config()` 的"默认配置"语义一致；`default_enabled: true` |
| 迁移触发时机 | `get_all_with_status()` | 用户打开路由菜单时迁移，不重载核心、不打断现有连接 |
| `00_base.json` 不存在时 | 不处理（已知遗留） | 保持与既有 `get_all_with_status()` 行为一致，不扩大范围 |
| `www.google.cn` | 剔除 | 其命中源于 `.cn` TLD 全域封锁，单独开洞语义不当；保持最小修复范围 |
| `connect.rom.miui.com` | 剔除 | 非 gstatic 系；其命中源于 `miui.com` 后缀，属小米域名，与本次目标（gstatic 探测端点）无关 |
| `connectivitycheck.gstatic.com` | 保留 | 已证为浏览器/OS 探测端点（见 §3.5），非普通资源 CDN |
| `fonts.` / `ssl.` / `g0~g3.` / `csi.gstatic.com` | 不加 | 同属 gstatic 但是资源 CDN 而非探测端点，加了属超范围开洞 |

---

## 3. 设计

### 3.1 新增规则（`rust/aegis/src/core/xray/routing.rs`）

插入 `ROUTING_RULES` **首位**（顺序即优先级，必须早于 `cn_ip`/`cn_domain`）：

```rust
RuleDef {
    id: "connectivity_check",
    rule_type: "domain",
    targets: &[
        "www.gstatic.com",
        "connectivitycheck.gstatic.com",
    ],
    outbound: "direct",
    default_enabled: true,
},
```

生成结果：

```json
{"type":"field","ruleTag":"connectivity_check","outboundTag":"direct",
 "domain":["www.gstatic.com","connectivitycheck.gstatic.com"]}
```

`direct` outbound 已存在于 `config.rs:857-860` 生成的 outbounds 中，无需新增。

#### 为什么写主机名而不是完整 URL

Xray 的 `domain` 匹配目标是 **SNI / Host 主机名**。因此：

| 写法 | 是否正确 | 说明 |
|---|---|---|
| `www.gstatic.com` | ✅ | 正确。`domain:` 前缀对裸主机名及其子域生效 |
| `https://www.gstatic.com/generate_204` | ❌ | 错误。scheme 与路径不参与 routing 匹配 |
| `domain:www.gstatic.com`（显式前缀） | ✅ | 同理，与本项目现有一致 |

本项目中 `rule_type: "domain"` 生成的即 `domain` 数组，Xray 默认按域名及其子域匹配，
与 `geosite:cn` 中的写法语义一致。

### 3.2 存量配置迁移

Xray 侧目前**无任何迁移机制**，需新增。仿 sing-box 的
`ensure_rule_sets_value` / `ensure_rule_sets_in_base` 命名风格，但独立实现：

```rust
/// 纯函数：确保 rules 含 connectivity_check（插在首位，幂等）。返回是否有变更。
pub fn ensure_direct_rules_value(v: &mut Value) -> bool

/// 加锁包装：读 → 迁移 → 写盘
pub async fn ensure_direct_rules_in_base() -> Result<()>
```

行为：

1. 按 `ruleTag == "connectivity_check"` 判定是否已存在
2. 不存在 → `insert(0, ...)`（**首位**，因优先级依赖顺序）
3. 已存在 → 不做任何事，返回 `false`

在 `get_all_with_status()` 开头调用，与 sing-box 现有模式对齐。

**不重载核心**：迁移只写盘，不触发 `reload_core()`，避免打断现有连接。

### 3.3 断言同步

`routing.rs`：
- `test_rule_def_constants_count`：`7` → `8`

`config.rs`（`test_ensure_base_config_structure`，`:1276-1278`）：
```rust
assert_eq!(rules.len(), 3);           // → 4
assert_eq!(tags, vec!["private_ip", "cn_ip", "cn_domain"]);
// → vec!["connectivity_check", "private_ip", "cn_ip", "cn_domain"]
```

**并新增意图断言**（本 bug 回归防线）：

```rust
#[test]
fn test_connectivity_check_must_precede_cn_rules() {
    let pos = |id: &str| ROUTING_RULES.iter().position(|r| r.id == id).unwrap();
    assert!(pos("connectivity_check") < pos("cn_ip"));
    assert!(pos("connectivity_check") < pos("cn_domain"));
}
```

理由：顺序是本修复的核心约束。仅改期望值会让"为什么必须首位"只存在于
commit message 中，而 commit message 会随时间腐烂。

### 3.4 i18n

`rust/aegis/src/resources/i18n/zh.yml` 与 `en.yml` 各新增 1 行：

```yaml
routing_rule_connectivity_check: "连通性检测直连"              # zh
routing_rule_connectivity_check: "Connectivity Check Direct"  # en
```

**仅新增 key，不修改任何既有条目。**

`handlers/xray.rs:596` 与 `handlers/xray.rs:635` 使用
`format!("xray.routing_rule_{}", def.id)` 动态拼接 key，handler 代码无需改动。

**共享资源说明**：`handlers/singbox.rs:940` 复用同一 `xray.routing_rule_*`
命名空间。新增 key 对 sing-box 渲染逻辑无影响（sing-box 的 `RuleDef` 集合不含
该 id，永不查找此 key）。这属于共享资源文件的新增，不是 sing-box 代码改动。

`ja.yml` 不改（缺失 key 时 i18n 层回退到 key 本身，不报错）。

### 3.5 两个域名的身份（为何保留）

| 域名 | 使用者 | 性质 |
|---|---|---|
| `www.gstatic.com/generate_204` | Chrome、历史上 Android | 探测端点；但该域同时承载其他资源 |
| `connectivitycheck.gstatic.com/generate_204` | Chrome（captive portal 专用）、Android 6.x、Android CaptivePortalLogin、Chromecast | **专为连通性检测而设** |

Chromium 曾将 captive portal 检测从 `www.gstatic.com` 迁至
`connectivitycheck.gstatic.com`，理由之一正是 `www.gstatic.com` 还承载大量
其他资源。因此 `connectivitycheck.gstatic.com` 是**更纯粹的探测端点**，
两者均应保留。

注：Android 部分分支后来改用 `www.google.com/generate_204`，属版本/平台差异，
不影响本设计对上述两个 gstatic 域名的放行。

---

## 4. 明确不做

- `rust/aegis/src/core/singbox/routing.rs` — 不碰
- `rust/aegis/src/core/singbox/config.rs`（含 `:650` 断言）— 不碰
- `rust/aegis/src/core/xray/config.rs` 的 `ensure_base_config()` 生成逻辑 — 不碰
- `ja.yml` — 不改
- `handlers/xray.rs`、`handlers/singbox.rs` — 不改（动态 key）
- `cn_ip` / `cn_domain` 的 outbound 语义 — 不改

---

## 5. 已知遗留

1. **`00_base.json` 不存在时菜单崩溃**：`get_all_with_status()` 经
   `handlers/xray.rs:582` 的 `?` 传播 `read_to_string` 的错误。
   `ensure_base_config()` 仅在启动时创建该文件，而菜单可随时被点开。
   本次按最小范围原则不处理。
2. **`.cn` 全域封锁**：见 §1.3。
3. **sing-box 存在相同缺陷**：`singbox/routing.rs` 的 `cn_domain`
   （`rule_set: geosite-cn`，outbound `block`）同样吞掉 `www.gstatic.com`。
   已在 §1.2 验证。本次不修，单独跟踪。

---

## 6. 测试策略（TDD：先 RED 后 GREEN）

| 测试 | 层级 | 文件 |
|---|---|---|
| `ensure_direct_rules_value` 幂等（已存在不重复插入） | 单元 | `xray/routing.rs` |
| `ensure_direct_rules_value` 插入位置为 index 0 | 单元 | `xray/routing.rs` |
| `ensure_direct_rules_value` 空 `rules` 数组可用 | 单元 | `xray/routing.rs` |
| `connectivity_check` 优先于 `cn_ip`/`cn_domain` | 单元（回归防线） | `xray/routing.rs` |
| `rule_def_to_json` 产出 `domain` + `direct` | 单元 | `xray/routing.rs` |
| `targets` 恰为 2 项且均为 gstatic 系 | 单元 | `xray/routing.rs` |
| 默认规则数量与顺序 | 单元 | `xray/config.rs` |

i18n key 无既有校验测试，不新增。

---

## 7. 文件清单（4 个）

| 文件 | 改动 |
|---|---|
| `rust/aegis/src/core/xray/routing.rs` | 新增 `RuleDef`（首位）；新增迁移函数；改 `len()` 断言；新增优先级断言与迁移单测 |
| `rust/aegis/src/core/xray/config.rs` | 更新 `rules.len()` 与 `tags` 断言 |
| `rust/aegis/src/resources/i18n/zh.yml` | +1 行 |
| `rust/aegis/src/resources/i18n/en.yml` | +1 行 |

---

## 8. 验证方式

1. `cargo test -p aegis`（或项目实际测试命令）
2. `cargo fmt --check`
3. `cargo clippy`（按 `rust-lint-format` 技能强制要求）
4. 手工：迁移函数对"已存在"和"不存在"两种存量配置的行为
