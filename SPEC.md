# Spec: 路由自定义放行（Xray）

> 状态：**待批准**。批准前不进入 BUILD。

## 范围决定（已确认）

- **只做本模块**（`custom-allowlist`）。原 `google-direct` 模块（补 Google 白名单漏项如 `recaptcha.net`）**已移出范围**——管理员日后遇到 Google 类误伤，直接用本模块加域名即可，无需改代码、无需发版。
- **sing-box 侧本次不改**。sing-box 的 `route.rules`、`m_sb_routing` 菜单、`.srs` 转换一律不触碰。已知代价：两个核心行为不一致——Xray 放行的域名在 sing-box 上仍会被 `geosite-cn` 拦。记为后续独立任务。
- **v1 只做 `direct`（放行）**，不做自定义 `block`；**只收域名**，不收 IP/CIDR；**不纳入 `geosite:` 前缀**（见 Open Question 1）。

---

# Module: custom-allowlist

## Objective

**问题**：`cn_domain`（`geosite:cn`）、`cn_ip`（`geoip:cn`）、`ads`（`category-ads-all`）等规则会误伤正常服务。每次误伤都只能由开发者改硬编码白名单 → 发版 → 用户升级。案例：`decodo.cn`（decodo.com 的中国站登录入口）被 `geosite:cn` 中的裸 TLD `cn` 命中而 blackhole，首页可开、登录必失败。

**目标**：把「加白名单」从**发版能力**降级为**运维能力**——管理员在 Telegram 网关里输入域名（如 `decodo.com` / `decodo.cn`），该域名（及其子域）立即走 `direct`，优先级高于 `cn_ip` / `cn_domain` / `ads`。

**用户**：本项目的部署者（单管理员，通过 Telegram bot 操作）。

**成功标准（一句话）**：管理员在 bot 里输入 `decodo.cn` → 30 秒内 `dashboard.decodo.cn` 登录可用，全程无需改代码、无需发版、无需手动编辑服务器上的 JSON。

**非目标**：不是分流客户端（不提供规则订阅/分享）；不替代 `geosite` 分类；不做 IP 段放行（v1）；不改 sing-box 侧（v1）。

## Assumptions I'm making

1. **v1 只覆盖 Xray**。sing-box 侧本次不改（`route.rules`、`m_sb_routing` 菜单、`.srs` 转换都不动）。代价：Xray 放行的域名在 sing-box 上仍会被 `geosite-cn` 拦——已知不一致，记为后续独立任务。
2. **只做 `direct`（放行），不做自定义 `block`**。自定义屏蔽可由用户自行开启现成的 `ads`/`cn_domain` 规则达成。
3. **入口格式见下方「输入格式规范」章节**。收纯主机名（`youtube.com`）与 `domain:` / `full:` 前缀；**不支持** `regexp:`、`keyword:`、IP/CIDR、`geosite:`。
4. **入口界面**：仅在现有 `m_routing`（Xray）菜单下加「自定义放行」子菜单，含「添加 / 列表 / 删除 / 自检」。
5. **持久化位置**：直接写进 `00_base.json` 的 `routing.rules`，以 `ruleTag: "custom_direct"` 标识；不新建独立文件。这样天然随 `00_base.json` 备份/回滚，且 `update_geodata` 不会覆盖它（它只写 `.dat`/`.db`/`.srs`）。
6. **规则位置**：Xray 侧插在 `connectivity_check` **之后**、`private_ip` 之前（索引 1）。两条都是 `direct`，先后无行为差异，因此选择不破坏现有 `ensure_direct_rules_*` 测试的插入点。
7. **空列表不落盘**：用户列表为空时不写 `custom_direct` 规则（避免污染配置、避免改变现有测试断言的长度）。
8. **匹配语义**：`domain:decodo.cn` 同时匹配 `decodo.cn` 与其**所有子域**（`dashboard.decodo.cn`）。因此用户只需输入裸域名 `decodo.cn`，不必写 `*.`。
9. **删除交互：按序号**（`routing_custom_del:<idx>`）。
10. **「生效自检」按钮纳入 v1，但能力受限**（见下）：它只能确定性地判定**本项目自己维护的列表**（`custom_direct` / `connectivity_check`），**无法**在本地判定 `geosite:cn` / `geoip:cn` / `category-ads-all` 是否命中——那些由核心内部解析 `.dat`/`.db`，本项目不解析它们。

## 菜单位置与按钮（已核实）

**是，与现有封锁/直连规则在同一个菜单里。** 实测 `handle_routing_menu`（`handlers/xray.rs:585`）当前结构为：

```
📋 路由规则管理
活跃规则: N 条
[✅ Google 服务直连]      [✅ 私有IP封锁]      [✅ 中国IP封锁]
[✅ 中国域名封锁]          [⬜ 私有域名封锁]    [⬜ BT协议封锁]
[⬜ 广告域名封锁]          [⬜ OpenAI直连]
[⬅ 返回]
```

即：8 条规则的开关按钮（带 ✅/⬜ 状态）+ 一行返回按钮。

**本功能的按钮插在「返回」之前，自成一行**（导航型按钮，**不带** ✅/⬜）：

```
[✏️ 自定义放行 (3)]        ← 新增：显示当前条数
[⬅ 返回]
```

点击进入子菜单：

```
✏️ 自定义放行
当前 3 条。这些域名（含子域）直接放行，优先级高于「中国域名封锁」「中国IP封锁」「广告域名封锁」。
⚠️ 仅对域名生效：若客户端直接用 IP 连接，仍会被 geoip:cn 拦截。

[➕ 添加域名]   [📄 查看列表]
[🔍 生效自检]
[⬅ 返回]
```

设计要点：
- 开关型按钮（✅/⬜）与导航型按钮（无图标、带条数）**视觉上可区分**，避免误以为可以「关闭」。
- 条数直接显示在按钮上，不进子菜单也能看到状态。
- 子菜单标题里就必须写明「仅对域名生效」这条边界（Success Criteria 之外的体验要求）。

## 输入格式规范（用户可输入什么）

**规范化目标**：把用户输入变成 Xray `domain` 数组中的一项。Xray 的 `domain:` 前缀 = **匹配该域名及其所有子域**。

### 接受的形式

| 用户输入 | 规范化结果 | 匹配范围 |
|---|---|---|
| `youtube.com` | `domain:youtube.com` | `youtube.com` + **所有子域** |
| `www.youtube.com` | `domain:www.youtube.com` | `www.youtube.com` + 其子域 |
| `*.youtube.com` | `domain:youtube.com` | 同上（剥离 `*.`） |
| `.youtube.com` | `domain:youtube.com` | 同上（剥离前导点） |
| `domain:youtube.com` | `domain:youtube.com` | 原样保留 |
| `full:youtube.com` | `full:youtube.com` | **仅**精确匹配，不含子域 |

**推荐输入就是 `youtube.com` 这种裸域名**——它已经覆盖全部子域，用户不需要写 `*.`。

### 归一化顺序（必须按此顺序）

1. `trim()` 首尾空白
2. 转小写（`YOUTUBE.COM` → `youtube.com`）
3. 去尾部点（`youtube.com.` → `youtube.com`）
4. 非 ASCII → punycode（`中文.com` → `xn--fiq228c5hs.com`）
5. 剥离 `*.` 或前导 `.`
6. 无前缀 → 加 `domain:`

### 拒绝的输入（附提示要点）

| 输入 | 拒绝理由 |
|---|---|
| `https://youtube.com/path` | 含 scheme / 路径 |
| `youtube.com:443` | 含端口 |
| `youtube.com/path` | 含路径 |
| `192.168.1.1` / `10.0.0.0/8` | v1 不做 IP（须明确告知） |
| `a b.com` / 空串 | 含空白 / 为空 |
| `_dmarc.youtube.com` | label 含非法字符 `_` |
| `-foo.com` / `foo-.com` | label 以 `-` 开头/结尾 |
| `localhost` / `cn`（单 label） | 非合法公网域名；且 `domain:cn` 会放行**全部** `*.cn`（正是本功能要对抗的误伤源） |
| `regexp:...` / `keyword:...` / `geosite:...` | v1 不支持 |

### 长度与数量上限

- 单条：总长 ≤ 253，每个 label ≤ 63
- 列表：≤ 100 条（超出时提示先删除）
- 幂等：同一条重复输入不产生重复项

> **裸 TLD 特例已由「单 label 拒绝」规则覆盖**：`cn` 会被拒，不会变成放行全部 `.cn` 的 `domain:cn`。

## 文案规范（三语，需人工审阅）

现有输入引导的先例：`domain.input_prompt` = `"请输入你的域名，例如 example.com"`（`xray.rs:3050` 发送）。本功能沿用同样风格，但**必须把格式说明写进提示文案**（用户要求）。

### 关键：输入引导文案（必须包含格式示例与不支持项）

**zh**
```
✏️ 请输入要放行的域名。

格式：直接输入域名即可，例如 decodo.cn
• 裸域名会自动包含其所有子域（decodo.cn 含 dashboard.decodo.cn）
• 也接受 *.decodo.cn、domain:decodo.cn
• 仅精确匹配写 full:decodo.cn

不支持：https:// 开头、带路径/端口、IP、正则、geosite:
⏳ 120 秒内有效
```

**en**
```
✏️ Enter the domain to allow.

Format: just the domain, e.g. decodo.cn
• A bare domain covers all its subdomains (decodo.cn includes dashboard.decodo.cn)
• *.decodo.cn and domain:decodo.cn are also accepted
• Use full:decodo.cn for exact match only

Not supported: https://, paths, ports, IPs, regex, geosite:
⏳ Valid for 120s
```

**ja**
```
✏️ 許可するドメインを入力してください。

形式: ドメインをそのまま入力（例: decodo.cn）
• 裸のドメインは全サブドメインを含みます（decodo.cn は dashboard.decodo.cn を含む）
• *.decodo.cn、domain:decodo.cn も可
• 完全一致のみは full:decodo.cn

非対応: https://、パス、ポート、IP、正規表現、geosite:
⏳ 120秒以内有効
```

### 其余必需 key（命名沿用 `xray.routing_*`）

| key | zh（示例） |
|---|---|
| `xray.routing_custom_btn` | `✏️ 自定义放行 (%{count})` |
| `xray.routing_custom_title` | `✏️ <b>自定义放行</b>\n\n这些域名（含子域）直接放行，优先级高于「中国域名封锁」「中国IP封锁」「广告域名封锁」。\n⚠️ 仅对域名生效：客户端直接用 IP 连接时仍会被 geoip:cn 拦截。\n\n当前 %{count} 条` |
| `xray.routing_custom_add` | `➕ 添加域名` |
| `xray.routing_custom_list` | `📄 查看列表` |
| `xray.routing_custom_check` | `🔍 生效自检` |
| `xray.routing_custom_added` | `✅ 已放行 %{domain}（含子域）` |
| `xray.routing_custom_exists` | `ℹ️ %{domain} 已在列表中` |
| `xray.routing_custom_removed` | `🗑 已移除 %{domain}` |
| `xray.routing_custom_empty_list` | `列表为空。点「添加域名」开始。` |
| `xray.routing_custom_full` | `❌ 已达上限 100 条，请先删除部分条目` |
| `xray.routing_custom_del_bad_index` | `❌ 序号无效，未做任何修改` |
| `xray.routing_custom_invalid_scheme` | `❌ 不要带 https:// 或路径，只填域名，例如 decodo.cn` |
| `xray.routing_custom_invalid_single_label` | `❌ 请填写完整域名（至少两段），例如 decodo.cn。单段如 cn 会放行全部 .cn 域名，已拒绝。` |
| `xray.routing_custom_invalid_ip` | `❌ v1 不支持 IP/CIDR 放行，请填域名` |
| `xray.routing_custom_invalid_generic` | `❌ 格式不合法：%{reason}` |
| `xray.routing_custom_check_hit` | `✅ 命中自定义放行（第 %{idx} 项）：%{entry}` |
| `xray.routing_custom_check_miss` | `❌ 未命中自定义放行列表。\n注意：其余规则（geosite:cn / geoip:cn / category-ads-all）无法在本地判定，请查看核心日志中的 ruleTag。` |

> 三语 key 必须齐全（parity 测试会卡）；上表只给 zh 示例，en/ja 需同步翻译并在 T5 交付时人工审阅。

## ⚠️ 「生效自检」的真实能力边界

理想中是「对某域名回报命中哪条 `ruleTag`」。但本项目**不解析** `geosite.dat` / `geoip.dat` / `.db`——那是核心内部的事。所以自检只能：

| 规则 | 自检能否判定 |
|---|---|
| `custom_direct`（本功能维护的列表） | ✅ 确定性判定 |
| `connectivity_check`（静态 5 域名） | ✅ 确定性判定 |
| `private_ip` / `cn_ip` / `cn_domain` / `ads` / `bt` / `openai` | ❌ **无法判定**（需核心内部 geo 数据 + 需解析目标 IP） |

**因此自检的输出定义为**：

- 「命中自定义放行列表（第 N 项）」或「未命中自定义放行列表」
- 若未命中，追加提示：*「其余规则（`geosite:cn` / `geoip:cn` / `category-ads-all`）无法在本地判定，请查看核心日志中的 `ruleTag`」*

这仍然有用（能立刻验证「我刚加的域名是否生效」），但**不得**宣称能报出 `cn_domain` 之类的命中。若你要的是完整判定，需要单独立项（解析 geo 数据），不在 v1。

## Tech Stack

- Rust（edition 见 `rust/aegis/Cargo.toml`），Telegram bot 网关
- 本次仅涉及 Xray（`00_base.json`）；**sing-box 不在 v1 范围内**
- 配置为 JSON（`serde_json`），规则顺序敏感（首条命中即停）
- 测试：`cargo nextest`（`fast-test` profile，LLVM 为准）+ `cargo test --doc`

## Commands

```bash
cd rust/aegis
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test
cargo test --doc
```

以上四条为**强制质量门**（见 `rust-lint-format`），任一失败不得宣称完成。

## Project Structure

```
SPEC.md                                   ← 本文件
tasks/plan.md                             ← 实施计划 + 任务清单（含验收标准/验证步骤）
tasks/todo.md                             ← 任务勾选清单（下游工具约定）
rust/aegis/src/core/xray/routing.rs       ← Xray 规则定义 + 规范化/落盘/迁移（纯函数 + 单测）
rust/aegis/src/shared/handlers/xray.rs    ← Xray 菜单/回调（handle_routing_menu 等）
rust/aegis/src/shared/handlers/message.rs ← 文本输入状态机（AwaitDomain 复用点）
rust/aegis/src/shared/handlers/mod.rs     ← route_callback 回调前缀路由
rust/aegis/src/app/state.rs               ← 待输入状态存储
rust/aegis/src/resources/i18n/{zh,en,ja}.yml ← 文案（三语必须同步）
rust/aegis/tests/                         ← 集成测试
（以下为 v1 范围外，本次不动）
rust/aegis/src/core/singbox/routing.rs    ← sing-box 规则定义 + 落盘/迁移
rust/aegis/src/shared/handlers/singbox.rs ← sing-box 菜单/回调
```

## Code Style

沿用既有模式：**把顺序/幂等逻辑写成不碰文件系统的纯函数，单测覆盖；I/O 只做薄封装。**

```rust
/// 纯函数：确保 routing.rules 含 custom_direct（位于 connectivity_check 之后，幂等）。
/// 返回是否发生变更。空列表时不插入（避免污染配置）。
pub fn ensure_custom_direct_value(v: &mut Value, domains: &[String]) -> bool {
    const RULE_ID: &str = "custom_direct";
    if domains.is_empty() {
        // 空列表 = 删除既有规则（若存在）
        return Self::remove_rule_by_tag(v, RULE_ID);
    }
    let canonical = Self::custom_direct_rule_json(domains);
    Self::upsert_after(v, "connectivity_check", RULE_ID, canonical)
}
```

约定：
- 规则 `id`/`ruleTag` 用 snake_case（`custom_direct`）；回调 data 前缀用 `routing_custom_*`。
- 错误用 `anyhow::Result` + `.context("中文说明")`；日志中文。
- 注释解释**为什么**（尤其「为什么必须在这个位置」），不解释语法。
- 单个 patch 不超过 ~200 行；单任务不超过 ~5 个文件。

## Testing Strategy

- **单元测试**（`#[cfg(test)]` 同文件）：纯函数优先——域名规范化/校验、`ruleTag` 定位与插入位置、幂等、空列表删除、用户列表与既有规则共存不被覆盖。
- **回归防线**：新增菜单按钮必须被 `route_callback` 路由，否则 `handlers/mod.rs::test_every_menu_button_data_is_routed` 会失败——**该测试必须保持通过**。
- **I/O 层约束**：`00_base.json` 路径是硬编码的 `/etc/wwps/...`，单测**不能**写生产路径。因此把全部顺序/幂等逻辑放在纯函数里，I/O 只做「读 → 纯函数 → 写」的薄封装，不为其写文件系统测试（沿用 `test_ensure_direct_rules_in_base_errors_when_base_missing` 的「路径不存在则跳过」惯例）。
- **集成测试**（`rust/aegis/tests/`）：三语文案 parity（参照 `hy2_i18n_parity.rs`）。
- 覆盖要求：新增纯函数分支全覆盖；不追求行覆盖率数字。
- TDD：先写失败测试并**确认失败**，再写最小实现。

## Boundaries（custom-allowlist）

**Always**
- 改任何 Rust 代码前跑完四条质量门。
- 新增用户可见文案必须同步 zh/en/ja 三语。
- 规则位置逻辑（首条命中即停）必须有单测锁死，并说明为何在该索引。
- 写盘复用既有 `CONFIG_LOCK`，避免并发写坏 `00_base.json`。
- 顺序/幂等逻辑放纯函数，便于在无法写生产路径的前提下测试。

**Ask first**
- 修改 `ensure_direct_rules_value` 的既有语义或 `connectivity_check` 的索引（会动既有测试）。
- 改变 v1 的格式范围（加入 `geosite:` / regex / IP）。
- 新增依赖。
- 单任务涉及 >3 个文件。

**Never**
- 把 `custom_direct` 放到 `cn_ip` / `cn_domain` **之后**（首条命中即停，规则会静默失效——这正是历史 bug）。
- 在未加锁的情况下读改写 `00_base.json`。
- 用未转义的用户输入拼正则。
- 直接编辑 `.dat` / `.db` / `.srs`。
- 删除或跳过既有失败测试来「变绿」。
- v1 内改动 sing-box 的 `00_base.json`（`route.rules`）或 `m_sb_routing` 菜单。
- 「生效自检」按钮写盘或触发 core reload（必须只读）。
- 宣称自检能判定 `geosite:cn` / `geoip:cn` / `category-ads-all` 的命中。

## Success Criteria（custom-allowlist）

1. 在 bot 中输入 `decodo.cn` → `00_base.json` 的 `routing.rules` 出现 `ruleTag: "custom_direct"`，`domain` 含 `domain:decodo.cn`，且**索引位于 `cn_domain` 之前**（紧跟 `connectivity_check`）。
2. 该主机（含子域 `dashboard.decodo.cn`）经核心出站为 `direct`，不再 blackhole。
3. 重复添加同一域名**幂等**（不产生重复条目）。
4. 删除后规则消失；列表清空后 `custom_direct` 规则整体移除。
5. 非法输入按「输入格式规范」被拒绝并给出明确提示，**不写盘**。
6. 三语菜单文案齐全；`test_every_menu_button_data_is_routed` 与全部既有测试通过。
7. 四条质量门全绿。
8. `update_geodata` 执行后用户列表**仍在**。
9. **按序号删除**：`routing_custom_del:<idx>` 精确删除对应项，其余项顺序不变；越界/非法序号给出提示且**不写盘**。
10. **生效自检**：只读、不写盘、不 reload；对给定域名确定性回报「是否命中 `custom_direct`（第 N 项）」与「是否命中 `connectivity_check`」，并对无法判定的规则给出「请查核心日志」提示。
11. 输入格式按规范表逐条验证（接受 6 种形式、拒绝 9 类非法输入）。
12. 既有 `connectivity_check` 规则的内容与索引**不受本功能影响**（`ensure_direct_rules_value` 的既有测试全部保持通过）。

## Open Questions

1. **是否纳入 `geosite:` 前缀**（v1 默认不纳入）。纳入后用户一条 `geosite:google` 就能自行覆盖整类场景；代价是可写 `geosite:cn` 整体绕过「中国域名封锁」（管理员的自由，但需在提示文案说明）。**当前决定：不纳入**，留待后续。
2. **自检能力是否够用**？见上「真实能力边界」——它无法判定 geo 类规则。若你要完整判定，需单独立项解析 geo 数据。**当前决定：接受受限版本**。

## 已核实的边界（写实现时必须遵守）

- **只按域名放行，无法覆盖纯 IP 访问**：Xray `domainStrategy: IPIfNonMatch` 先匹配域名规则；若客户端直接用 IP 连接或 SNI/Host 非该域名，`cn_ip`（`geoip:cn`）仍会 blackhole。必须在提示文案中向用户明示。
- **`geosite:cn` 含裸 TLD `cn`**（v2fly `data/cn` → `include:tld-cn`），故**所有 `*.cn` 都命中 `cn_domain`**。这正是 `decodo.cn` 案例的根因，也是本功能的首要验收场景。
- **`geosite:cn` 大量条目是 `Full`（精确匹配，不含子域）**：实测 `gstatic.com`（裸）**不**被拦，而 `www.gstatic.com` / `csi.gstatic.com` / `ssl.gstatic.com` **分别**被拦。这解释了「为什么误伤总是逐个冒出来」，也说明白名单用 `domain:`（覆盖子域）比 `full:` 更实用。
- **`decodo.com` 不会自动跳转到 `.cn`**：实测无 meta-refresh、无 JS 跳转，首页所有 login/register 链接均指向 `dashboard.decodo.com`；`decodo.cn` 仅出现在手动的 Region 选择器（`China (中文)`）。因此若登录发生在 `dashboard.decodo.com`，`.cn` 拦截**不是**其原因。
- **规则顺序**：Xray `routing.rules` 与 sing-box `route.rules` 均为**首条命中即停**。
- **IP 条件救不了域名规则**：`IPIfNonMatch` 下第一轮无 IP，仅含 `ip` 条件的规则无法命中，因此**本功能必须用域名规则**，不能用 `ip` 条件。

---

# Module: routing-split（`routing.rs` 拆分）

> 状态：**待批准**。批准前不进入 BUILD。
> 类型：**纯重构**（行为保持不变）。不新增功能、不改文案、不改 JSON 形状、不改公开 API 语义。

## Objective

**问题**：`rust/aegis/src/core/xray/routing.rs` 已 **1325 行**，越过 ~1000 行单文件体检线。它同时承载三件互不相关的事，其中 `custom_direct`（custom-allowlist 功能的实现载体）占 ~790 行（含其单测），是超线主因。

**目标**：把 `custom_direct` 全链路（域名规范化 + 纯逻辑 + `RoutingManager` 的 custom_direct 相关 I/O 方法 + 其单测）**逐字搬**到新文件 `rust/aegis/src/core/xray/custom_direct.rs`；`routing.rs` 只留规则表定义 + `connectivity_check` 迁移/开关。

**成功标准（一句话）**：拆分后两文件各 < 1000 行（实测 `routing.rs` 576、`custom_direct.rs` 791），四条质量门全绿，且**既有测试零增删行**（40 项测试原样通过，测试名集合不变、断言行零改动）。

## 非目标

- 不改任何行为：不新增/删除测试，不改规则顺序、不改文案、不改 JSON 键、不改 `00_base.json` 布局。
- 不动 sing-box 侧、不动 `handlers/*`、不动 i18n、不动 `Cargo.toml`（无新依赖）。
- 不进一步拆分 `connectivity_check`（它属于规则表，留在 `routing.rs`）。
- 不做重命名、不做"顺手清理"、不做可见性收紧（除下文列出的 2 处必需放宽）。

## 拆分边界（已用 CodeGraph 核实调用方）

### 移入 `custom_direct.rs`（~790 行）

| 项 | 现状 | 外部调用方 | 可见性 |
|---|---|---|---|
| `CustomDomainError` | pub enum | `handlers/message.rs`（6 处） | 保持 `pub` |
| `is_ip_or_cidr` | private fn | 仅本文件 | 保持 private |
| `normalize_custom_domain` | pub fn | `handlers/message.rs:603` | 保持 `pub` |
| `CUSTOM_DIRECT_LIMIT` | pub(crate) const | 仅本文件 | 保持 `pub(crate)` |
| `CustomAddOutcome` | pub enum | `handlers/message.rs:13` | 保持 `pub` |
| `append_unique` / `remove_at` | pub(crate) fn | 仅本文件 + 本文件测试 | 保持 `pub(crate)` |
| `normalize_host` / `entry_matches_host` | private fn | 仅本文件 | 保持 private |
| `match_custom_direct` | pub fn | `handlers/message.rs:762` | 保持 `pub` |
| `matches_connectivity_check` | pub fn | `handlers/message.rs:778` | 保持 `pub` |
| `RoutingManager::{custom_direct_rule_json, remove_rule_by_tag, upsert_after, ensure_custom_direct_value, custom_direct_domains_from, persist_base_json, add_custom_direct_entry, remove_custom_direct_at, list_custom_direct_domains}` | 9 个 inherent 方法 | `handlers/*`（仅 `add_/remove_/list_` 三个） | 全部保持原可见性 |
| 上述项的 23 个单测 + 其专属测试夹具（`norm_ok`/`norm_err`/`cd_domains`/`cd_rule`/`ensure_cd`/`remove_cd`/`upsert_cd`/`tags_of`/`tag_index`/`direct_chain`/`strs`/`many`） | `routing.rs::tests` | — | — |

> `matches_connectivity_check` 随迁的理由：它与 `match_custom_direct` 共用 `normalize_host`/`entry_matches_host`，原文件已把两者放在同一注释分组「纯函数自检判定」；拆开需把两个 helper 提为 `pub(super)`，反而扩大接口。它读的 `ROUTING_RULES` 由 `routing.rs` 提供（单向依赖 `custom_direct → routing`）。

### 留在 `routing.rs`（~630 行）

`CONFIG_LOCK`、`RuleDef`、`ROUTING_RULES`、`RoutingManager` 本体、`read_rules` / `read_base_json` / `write_rules` / `rule_def_to_json` / `ensure_direct_rules_value` / `ensure_direct_rules_in_base` / `get_all_with_status` / `toggle`，以及 15 个规则表单测（`test_rule_def_*`、`test_connectivity_check_*`、`test_ensure_direct_rules_*`）与夹具 `base_with_rules`。

### 必需的可见性改动（仅 2 处，最小）

1. `static CONFIG_LOCK` → `pub(super) static CONFIG_LOCK`（`custom_direct.rs` 的写盘/读盘需复用同一把锁）。
2. `RoutingManager::read_base_json` → `pub(super) async fn`（`custom_direct.rs` 唯一的读入口）。

`base_with_rules` 是 `routing.rs::tests` 的私有夹具，`custom_direct.rs` 的测试**另存一份等价副本**（测试夹具跨测试模块无法共享；这是本次唯一新增的重复文本，不属行为代码）。

## 接口稳定性（关键决定）

`routing.rs` 顶部保留一行转发，**不修改 `handlers/message.rs`**：

```rust
// 拆分后仍从本模块导出，保持既有调用方路径不变（handlers/message.rs 有 6 处引用）
pub use super::custom_direct::{
    CustomAddOutcome, CustomDomainError, match_custom_direct, matches_connectivity_check,
    normalize_custom_domain,
};
```

理由：`RoutingManager` 本体留在 `routing.rs`，其方法调用方无需改动；只有 4 个自由项 + 1 个枚举需要转发。这样本次改动**只碰 3 个代码文件**（`routing.rs`、新增 `custom_direct.rs`、`mod.rs`），不触碰 `handlers/`，diff 可逐行审读。

## Commands

```bash
cd rust/aegis
wc -l src/core/xray/routing.rs src/core/xray/custom_direct.rs   # 两文件均须 < 1000
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test
cargo test --doc
```

`xray` 域的快速回路：`cargo nextest run --cargo-profile fast-test xray`（拆分前基线 = **40 passed**）。

## Testing Strategy（纯重构的 TDD 等价物）

无新行为 ⇒ 无新测试。**既有 40 项测试就是这次重构的契约**，验证方式是"移动而非修改"：

1. **基线（RED 的等价物）**：拆分前记录 `cargo nextest run --cargo-profile fast-test xray` = 40 passed（已验证）。
2. **不变量断言（两层）**：
   - 弱一层：`git diff -U0 -- rust/aegis/src/core/xray/routing.rs | grep '^[-+]' | grep assert` 中删除的那 85 行必须**原样出现**在 `custom_direct.rs`（23 个已迁走的测试携带它们）。
   - 强一层（实测采用）：把 `HEAD:routing.rs` 与拆分后两文件的**全部非空行去缩进后做多重集差集**。消失集必须**恰为 2 行**（`CONFIG_LOCK` 与 `read_base_json` 的两处可见性改动），其余差异只能是新增的转发 / 模块文档 / `impl` 包裹 / 测试模块脚手架 / `base_with_rules` 夹具副本。任何函数体或断言行的丢失、改写都会在此暴露。
   - 测试函数名集合必须与拆分前**完全相等**（`grep -o 'fn test_[a-z_]*'`）。
3. **GREEN**：`xray` 域 40 passed 且全量测试数不变。

## Code Style

- 移动 = 剪切粘贴：函数体、注释、doc comment、断言**一字不改**。仅允许改：`use` 行；`RoutingManager` 方法所在的 `impl` 块归属；`mod` 声明。
- `custom_direct.rs` 顶部写文件头注释，说明「为什么存在这个文件」（从 `routing.rs` 拆出，服务 custom-allowlist 功能）。
- 注释解释**为什么**，不解释语法；延续既有中文注释风格。
- 单个 patch ≤ ~200 行 ⇒ 拆成 4 片（见 `tasks/plan.md` Phase R），每片后跑一次 `xray` 域测试。

## Boundaries（routing-split）

**Always**
- 每片移动后立刻跑 `cargo nextest run --cargo-profile fast-test xray`，确认 40 passed 且测试名集合不变。
- 保持 `custom_direct` 规则插入点语义不变（`connectivity_check` 之后）；`test_custom_direct_index_precedes_cn_domain_regression` 必须原样通过。
- 保持 `CONFIG_LOCK` 只有一处定义（同时只有一个实例）。

**Ask first**
- 若要新增测试、改既有断言、或改动任何函数签名/行为。
- 若发现必须触碰 `handlers/`（当前评估：不需要）。
- 若两文件仍超 1000 行而需二次拆分。

**Never**
- 删除、跳过、重写既有测试断言来"变绿"。
- 在同一 patch 里混合行为变更或无关重构（不重命名、不顺手抽公共 helper）。
- 引入新依赖或新模块层（不搞 `routing/` 目录化）。

## Success Criteria（routing-split）

1. `src/core/xray/custom_direct.rs` 存在，承载上表全部项；`routing.rs` 只剩规则表 + `connectivity_check` + 转发。
2. `wc -l` 两文件均 < 1000。
3. `grep -c 'fn test_'`：`routing.rs` = 17、`custom_direct.rs` = 23、合计 = 40（与拆分前一致）。
4. 行级多重集差集的消失集恰为 2 行（即上文的可见性改动），且 `custom_direct.rs` 内所有函数体与拆分前逐字一致；无任何断言被改写。
5. 四条质量门全绿；全量测试通过数 = 拆分前（1097 passed / 1 skipped）。
6. 仅 3 个代码文件被改：`routing.rs`、`custom_direct.rs`（新增）、`mod.rs`；`handlers/` 零改动。
7. `custom_direct.rs` 内**零** `anyhow::bail!`/错误路径改动：`add_custom_direct_entry` / `remove_custom_direct_at` 的「重复/越界不写盘」语义原样保留。

---

# Module: essential-direct（外網必需服務直連）

> 状态：**已批准（2026-10-06，用户确认「要但新建按钮」＋「cc 五条也改 `domain:` 前缀」）**。
> 类型：**新功能（新增内建规则 + 新菜单按钮）＋ 两个既有缺陷修复**（迁移不 reload、`toggle` 落位）。
> 归属：影响 >3 文件、属安全/路由逻辑 ⇒ **strict 模式**（worktree → plan → TDD → review → ship）。

## Objective

**问题**：`geosite:cn`（Loyalsoldier 的 `geosite.dat`）把大量**外网必需**的 Google/Apple/Microsoft 服务端點收进了「中国域名」名单。本机实测命中并被黑洞的实例：

| 域名 | 实测证据 | 用户可见后果 |
|---|---|---|
| `www.recaptcha.net` | HAR：登录链唯一失败请求 `net::ERR_CONNECTION_CLOSED`；core 日志 `14:16:32 [2607:f8b0:4005:815::2003]:443 -> blocked`（`getent` 证实该 IP 即 `www.recaptcha.net`） | **登录/注册彻底失败** |
| `app-measurement.com` | `.220` core 日志 `-> blocked` **42 次** | Firebase 分析被黑洞、App 反复重试 |
| `safebrowsing.googleapis.com` | `.220` core 日志 `-> blocked` 6 次 | Chrome 安全浏览失效 |
| `r2---sn-j5o76n7{s,l}.googlevideo.com` | `.220` core 日志 `-> blocked`；命中 `geosite:cn` 的 **regex** 条目 | YouTube 影片 CDN 被封 |
| `pagead-googlehosted.l.google.com` 等 | `.44` core 日志大量 `142.251.x:80 -> blocked` | 广告/追踪（**本模块明确不放行**） |

上游**已知且不修**：

- **#484**《请求移除 fonts.gstatic.com 为 cn 规则》2026-01-25 开启，**至今 open**；
- **#478**《gstatic.com 大陆直连导致 gemini 显示不全》owner 回复（comment 3708851066，2026-01-05）：`将 geosite:google-cn 设置为代理，并放在 geosite:cn 上面`；同期社区留言「这个设计很不好，导致用 google 的人都有问题」。

**目标**：把「被 `geosite:cn` 误收的外网必需服务端點」做成**内建、默认启用、可在菜单单独开关**的规则 `essential_direct`，免去管理员逐台手动 `custom_direct`。

**成功标准（一句话）**：任一部署升级后打开一次路由菜单，`essential_direct` 自动落位在 `cn_ip`/`cn_domain` 之前并重启核心；`www.recaptcha.net` 的登录请求由 `blocked` 变为 `direct`；全程无需手工编辑 JSON。

## 为什么是「新规则 + 新按钮」而不是扩 `connectivity_check`

**既有守护测试禁止扩它**（`routing.rs:311 test_connectivity_check_targets_are_probe_endpoints_only`）：

```rust
assert_eq!(rule.targets, &[5 个探測端點], "域名清单必须恰为这 5 项");
for t in rule.targets {
    assert!(!t.starts_with("csi.") && !t.starts_with("update.") && !t.starts_with("safebrowsing.")
        && !t.starts_with("tac.") && !t.starts_with("clientservices.") && !t.starts_with("fontfiles.")
        && ... , "不应放行其余资源 CDN / 遥测域名");
}
```

→ 方案 A（扩充 `connectivity_check`）会当场让该测试变红。**新建独立规则**既满足用户要求，又不违反既有不变量；且 `handle_routing_menu` 直接迭代 `ROUTING_RULES`，新规则**自动获得菜单按钮**（`routing_toggle:essential_direct`），无需改菜单渲染逻辑。

## 已核实的事实（设计依据，全部有据可查）

1. **`geosite:google-cn` 不能直接采用**（本机 `geosite.dat` 解析）：该分类 112 条里 **28 条是广告/追踪**（`doubleclick.net`、`googlesyndication.com`、`googleadservices.com`、`googletagmanager.com`、`google-analytics.com`、`pagead-googlehosted.l.google.com`、`imasdk.googleapis.com`、`app-measurement.com` …）⇒ 违反「不放行广告/追踪」。
2. **`geosite:google-cn` 还漏掉本次真正的故障域名**：它只收 `full:recaptcha.net`（精确匹配，**不含子域**），而 `geosite:cn` 收的是 `full:www.recaptcha.net` ⇒ 只加 `google-cn` 修不好登录。
3. **`geosite:apple-cn` 可用**：165 条，覆盖 Apple 清单 20/21（只漏 `init.itunes.apple.com`），且**零广告/追踪**。
4. **`geosite:microsoft-pki` 可用**：6 条（`crl/ocsp.microsoft.com` 等）；**无 `microsoft-cn`** ⇒ 微软端點需手写。
5. **Xray 官方文档语义**（`XTLS/Xray-docs-next/docs/en/config/routing.md`）：
   - `domain:` = 匹配该域名**及其子域**（推荐用法）；
   - **裸字符串 = `keyword:` 子字符串匹配**（可省略前缀）——**本专案此前误以为等于 `domain:`**；
   - `geosite:xxx` 是 domain 列表中的合法条目形式；
   - 路由**自上而下、首条命中即停**；全部不命中时用**第一个 outbound**。
6. **被误收的 Google IP 不在 `geoip:CN`**（本机 `geoip.dat` 实测：`142.251.218.195`、`2607:f8b0:4005:815::2003` 等均 `NO`）⇒ 问题纯由 `cn_domain` 造成，**用 domain 规则即可修**，且必须排在 `cn_domain` 之前。

## 规则定义

```rust
RuleDef {
    id: "essential_direct",
    rule_type: "domain",
    outbound: "direct",
    default_enabled: true,
    // 位置：ROUTING_RULES 中紧接 connectivity_check 之后（索引 1）
}
```

- `connectivity_check` **内容不动**（仅把 5 条正規化为 `domain:` 前缀，见下），语义保持「连通性探測 + Google 静态资源」。
- 全部条目**必须**显式前缀：`domain:`（子域语义）或 `geosite:`；**禁止裸字符串**（会被 Xray 当子字符串，产生 `www.gstatic.com.evil.com` 这类误命中，且与自检函数语义不一致）。

### 条目清单（39 条）

> 39 = 37 条 `domain:` + 2 条 `geosite:`。

**Google / YouTube 功能必需（29）**

```
domain:recaptcha.net
domain:safebrowsing.googleapis.com
domain:safebrowsing-cache.google.com
domain:update.googleapis.com
domain:dl.google.com
domain:dl.l.google.com
domain:tools.google.com
domain:clientservices.googleapis.com
domain:performanceparameters.googleapis.com
domain:tac.googleapis.com
domain:crashlyticsreports-pa.googleapis.com
domain:firebase-settings.crashlytics.com
domain:update.crashlytics.com
domain:checkin.gstatic.com
domain:csi.gstatic.com
domain:g0.gstatic.com
domain:g1.gstatic.com
domain:g2.gstatic.com
domain:g3.gstatic.com
domain:fontfiles.googleapis.com
domain:redirector.gvt1.com
domain:redirector.gcpcdn.gvt1.com
domain:redirector.offline-maps.gvt1.com
domain:redirector.snap.gvt1.com
domain:beacons.gvt2.com
domain:beacons2.gvt2.com
domain:beacons3.gvt2.com
domain:googlevideo.com          # 覆盖 geosite:cn 的 YouTube CDN regex，避免枚举轮换节点
domain:youtube-dubbing.com
```

**Apple（2）**

```
geosite:apple-cn                    # 165 条，覆盖 ocsp/crl/mesu/swscan/swdist/swcdn/gs-loc/cl2-cl5/init.ess/guzzoni/...
domain:init.itunes.apple.com        # apple-cn 唯一漏项
```

**Microsoft（8）**

```
geosite:microsoft-pki               # crl/ocsp.microsoft.com 等 6 条
domain:download.microsoft.com
domain:download.visualstudio.microsoft.com
domain:officecdn.microsoft.com
domain:storeedge.microsoft.com
domain:storeedgefd.dsx.mp.microsoft.com
domain:dcg.microsoft.com
domain:sdx.microsoft.com
```

### 明确**不**纳入（硬约束：不放行广告/追踪）

`app-measurement.com`、`imasdk.googleapis.com`、`adservice.google.com`、`pagead-googlehosted.l.google.com`、`ssl-google-analytics.l.google.com`、`www-google-analytics.l.google.com`、`www-googletagmanager.l.google.com`、`google-analytics.com`、`googletagmanager.com`、`googleadservices.com`、`googlesyndication.com`、`googletagservices.com`、`doubleclick.net`、`googleoptimize.com`。

也不列 apex `google.com` / `googleapis.com` / `gstatic.com`（避免把其下广告端點一并放行）。**该约束以测试断言固化**（denylist + 前缀检查），防止日后手滑加回。

### 开关持久化（`routing.rulesDisabled`）

`toggle()` 在停用某规则时，除从 `routing.rules` 移除该规则外，还把规则 id 追加进同文件的 `routing.rulesDisabled`（字符串数组，去重）；重新启用时写回 canonical 规则（`rule_def_to_json`）并从该数组移除 id。整个读-改-写与迁移、custom_direct 共用既有 `CONFIG_LOCK`，且不改动 `00_base.json` 其他键；「用户显式停用」因此随文件一起备份/回滚，无需新增状态文件。

迁移 `ensure_direct_rules_value` 与 `rulesDisabled` 的交互：

- 停用中的规则（id ∈ `rulesDisabled`）即使 `default_enabled == true` 也不得被插入，且「缺失」不计为变更——否则打开一次菜单就把它塞回来，用户永远关不掉（`connectivity_check` 的既有缺陷同此）。
- 若同一 id 同时出现在 `rules` 与 `rulesDisabled`（例如管理员手改 JSON），以 `rules` 为准：保留规则并从 `rulesDisabled` 移除该 id；此移除本身算一次变更（触发写盘 + reload）。
- `rulesDisabled` 缺省即「无停用记录」；迁移不会主动创建该键，仅在需要移除冲突标记时写回。

## 两个既有缺陷的修复（本模块必须一并做）

### 修復 1：迁移后不重启核心 ⇒ 新规则在存量机器上等于不存在

`ensure_direct_rules_in_base()`（打开路由菜单时触发）**写盘后不 reload**；而 `ensure_direct_rules_value()` 已支持「内容过时 → 覆盖」。结果：菜单显示 ✅、核心仍跑旧规则，新域名继续被 `cn_domain` 黑洞——与本次线上排查到的现象同类。

**改法**：`ensure_direct_rules_value` 返回是否变更（已有语义），`ensure_direct_rules_in_base` 在**变更时**写盘并 `reload_core()`，与 `write_rules()` 对齐；无变更时零副作用（幂等，不打扰现网连接）。

### 修復 2：`toggle()` 用 `push` ⇒ 任何 `direct` 规则被打开后都落在 `cn_domain` 之后（死规则）

**改法**：把迁移的不变量从「确保 `connectivity_check` 在首位」泛化为

> **所有 `outbound == "direct"` 的规则，必须按 `ROUTING_RULES` 顺序位于所有 blocked 规则之前**（存在性 / 内容 = 当前定义 / 位置）。

迁移是唯一修复路径（`toggle` 保持 push 不改），因此切开关后重新打开菜单即自动修正。**收益**：新按钮可用；既有 `openai`（direct、默认关）被打开后也不再是死规则。

## 自检一致性

`matches_connectivity_check` 泛化为 `matches_builtin_direct(host) -> Option<&'static str>`（返回命中的规则 id），`custom_check_reply` 用该规则 id 取 i18n 名回显；否则出现「已白名单、自检却说未命中」的误导（本次线上排查正是被这类不一致拖慢）。

## `connectivity_check` 5 条正規化（已批准）

`www.gstatic.com` → `domain:www.gstatic.com`（其余同理）。理由：Xray 裸字符串是**子字符串**语义，`www.gstatic.com.evil.com` 也会命中；`domain:` 收紧为 apex＋子域，且与自检函数语义一致。守护测试保留其原意（仍断言「恰为这 5 项」、仍禁 `csi./update./safebrowsing.` 等），但比对前剥掉 `domain:` 前缀。

## Tech Stack

Rust 2024（workspace `rust/aegis`）、Xray-core 26.9.30（`wwps-core`）、rust-i18n（`src/resources/i18n/{zh,en,ja}.yml`）、tokio、serde_json、cargo-nextest。**无新依赖。**

## Commands

```bash
cd rust/aegis
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --cargo-profile fast-test            # 权威门（基线见 tasks/plan.md）
cargo test --doc                                       # 兜底门
cargo nextest run --cargo-profile fast-test xray       # 本模块快速回路
```

## Project Structure（本次改动范围）

```
rust/aegis/src/core/xray/routing.rs          # 新 RuleDef + 迁移泛化 + reload + 测试
rust/aegis/src/core/xray/custom_direct.rs    # 自检泛化 matches_builtin_direct + 测试夹具
rust/aegis/src/shared/handlers/message.rs    # 自检回报用命中规则名
rust/aegis/src/shared/handlers/xray.rs       # 菜单注释/按钮测试
rust/aegis/src/resources/i18n/{zh,en,ja}.yml # routing_rule_essential_direct
SPEC.md / tasks/plan.md / tasks/todo.md      # 文档
```

## Code Style

```rust
// 条目一律带显式前缀：读的人不必去查 Xray 裸字符串是子字符串还是子域语义。
targets: &[
    "domain:recaptcha.net",              // 登录链路必需（#478 同类问题的直系案例）
    "domain:safebrowsing.googleapis.com",
    "geosite:apple-cn",                  // 由 geodata 维护，免手写 165 条
],
```

## Testing Strategy

**全部为单元测试（纯函数优先）+ 既有夹具**；无 I/O 依赖（`00_base.json` 路径硬编码 `/etc/wwps/...`，故迁移逻辑必须留在纯函数层被测，I/O 包装仅薄调用）。

| 测试 | 断言要点 |
|---|---|
| `test_essential_direct_rule_shape` | id/`rule_type=domain`/`outbound=direct`/`default_enabled=true` |
| `test_essential_direct_precedes_cn_rules` | `pos(connectivity_check) < pos(essential_direct) < pos(cn_ip)`、`< pos(cn_domain)` |
| `test_essential_direct_targets_use_explicit_prefix` | 每条须以 `domain:` 开头或为白名单内的 `geosite:` 条目；无裸域名 |
| `test_essential_direct_excludes_ads_and_tracking` | denylist 14 条 + 前缀（`pagead`/`doubleclick`/`adservices`/`syndication`/`googletagmanager`/`-analytics`/`app-measurement`/`imasdk`）全不出现 |
| `test_essential_direct_contains_evidence_backed_hosts` | `domain:recaptcha.net`、`domain:googlevideo.com`、`geosite:apple-cn`、`geosite:microsoft-pki` 必须在场 |
| `test_essential_direct_targets_unique_lowercase_no_scheme` | 去重、全小写、无 `://`、无 `/`、无空格、无 IP、无 `regexp:` |
| `test_ensure_direct_rules_inserts_essential_direct` | 旧 base（仅 cc + blocked）→ `[connectivity_check, essential_direct, private_ip, cn_ip, cn_domain]` |
| `test_ensure_direct_rules_moves_misplaced_direct_before_blocked` | 末尾的 `openai`/`essential_direct` 被提到 blocked 之前且内容=canonical |
| `test_ensure_direct_rules_idempotent_all_canonical` | 第二次调用返回 `false`，JSON 不变 |
| `test_ensure_direct_rules_updates_stale_targets_at_index_zero`（既有，更新） | 旧 5 条 → 新 `domain:` 5 条 |
| `test_connectivity_check_targets_are_probe_endpoints_only`（既有，更新） | 剥前缀后仍「恰为这 5 项」且仍禁 CDN/遥测 |
| `test_custom_direct_index_precedes_cn_domain_regression`（既有，更新） | `cd == tag_index(essential_direct) + 1`，且仍 `< cn_ip/cn_domain` |
| `test_matches_builtin_direct_reports_rule_id` | `www.recaptcha.net → Some("essential_direct")`、`fonts.gstatic.com → Some("connectivity_check")`、`www.doubleclick.net → None` |
| `test_matches_builtin_direct_rejects_substring_false_positive` | `www.gstatic.com.evil.com → None`（正規化后核心与自检一致） |
| `test_rule_def_constants_count`（既有，更新） | xray `ROUTING_RULES.len()` 8 → **9**；singbox 仍 6 |
| 菜单按钮 | `routing_toggle:essential_direct` 存在且文案走 i18n；三语 key 齐备 |

## Boundaries

**Always**
- 先写失败测试（RED）再写实现；每个任务一个原子提交。
- 迁移逻辑保持纯函数可测；I/O 包装只做「读—纯函数—有变更才写盘+reload」。
- 新条目带显式 `domain:`／`geosite:` 前缀。
- 收尾跑四道门：`cargo fmt`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo nextest run --cargo-profile fast-test`、`cargo test --doc`。

**Ask first**
- 若要新增/删除任何清单条目（尤其把广告/追踪类加回）。
- 若需改动 sing-box 侧或 `custom_direct` 的管理员输入语法。
- 若 diff 超 ~200 行/片或需 >3 文件/片。

**Never**
- 删除、跳过或放松既有断言来「变绿」。
- 引入新依赖、把 `geosite:google-cn` 整包放行。
- 在同一 patch 混合无关重构。

## Success Criteria（essential-direct）

1. `ROUTING_RULES` 含 `essential_direct`（39 条），索引紧接 `connectivity_check` 之后，早于 `cn_ip`/`cn_domain`；xray 规则数 = 9。
2. 清单零广告/追踪（denylist + 前缀断言通过）；全部条目带显式前缀；无重复/大写/IP/路径/正则。
3. `connectivity_check` 5 条为 `domain:` 前缀；其守护测试仍断言「恰为这 5 项」。
4. 迁移：旧 base 打开菜单后被插入 `essential_direct` 并**重启核心**；重复执行零副作用（返回 `false`、不写盘、不 reload）；错位的 direct 规则被提到 blocked 之前（`openai` 回归）。
5. 自检：`www.recaptcha.net` 回报命中 `essential_direct`；`www.doubleclick.net` 回报未命中；`www.gstatic.com.evil.com` 不再被裸前缀误命中。
6. 三语 `xray.routing_rule_essential_direct` 齐备；菜单出现「外網必需服務直連」按钮且可开关。
7. 四道质量门全绿，通过数 = 基线 + 新增测试数（无既有测试减少）。
8. 真机：升级后打开一次路由菜单 → `journalctl -u wwps-core` 出现重启并读取新 `00_base.json`；登录页 `www.recaptcha.net` 由 `-> blocked` 变为 `>> direct`。

## Open Questions

- 无（用户已确认：新建按钮、纳入 Apple/Microsoft、排除广告追踪、`cc` 五条改 `domain:` 前缀）。
- 遗留（超范围，另行立项）：sing-box 侧同类问题（`.srs` 的 `geosite-cn` 仍会拦这些域名）；`geosite:google-cn` 只收 `full:` 导致子域漏网——可向上游提 issue。

## References

- <https://github.com/Loyalsoldier/v2ray-rules-dat/issues/484>（fonts.gstatic.com 未修，open）
- <https://github.com/Loyalsoldier/v2ray-rules-dat/issues/478#issuecomment-3708851066>（owner：改用 `geosite:google-cn` 并置于 `geosite:cn` 之前）
- <https://github.com/XTLS/Xray-docs-next/blob/main/docs/en/config/routing.md>（domain 匹配语义、geosite 条目、首条命中即停、默认 outbound）
