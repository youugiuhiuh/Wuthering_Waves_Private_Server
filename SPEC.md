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

**成功标准（一句话）**：拆分后两文件各 < 1000 行（预期 `routing.rs` ≈ 630、`custom_direct.rs` ≈ 790），四条质量门全绿，且**既有测试零增删行**（40 项测试原样通过，测试名不变、断言字符不变）。

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
| 上述项的 25 个单测 + 其专属测试夹具（`norm_ok`/`norm_err`/`cd_domains`/`cd_rule`/`ensure_cd`/`remove_cd`/`upsert_cd`/`tags_of`/`tag_index`/`direct_chain`/`strs`/`many`） | `routing.rs::tests` | — | — |

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
2. **不变量断言**：`git diff -U0 -- rust/aegis/src/core/xray/routing.rs | grep '^[-+]' | grep 'assert'` 必须**为空**；测试函数名的增删计数必须为 0（`grep -c 'fn test_' routing.rs` 拆分后应从 40 降为 15，`custom_direct.rs` 为 25，合计 40）。
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
3. `grep -c 'fn test_'`：`routing.rs` = 15、`custom_direct.rs` = 25、合计 = 40（与拆分前一致）。
4. `git diff` 中**无**任何 `assert` 行的增删；`custom_direct.rs` 的函数体与拆分前逐字一致（`git diff --no-index` 视角下仅位置变化）。
5. 四条质量门全绿；全量测试通过数 = 拆分前（1097 passed / 1 skipped）。
6. 仅 3 个代码文件被改：`routing.rs`、`custom_direct.rs`（新增）、`mod.rs`；`handlers/` 零改动。
7. `custom_direct.rs` 内**零** `anyhow::bail!`/错误路径改动：`add_custom_direct_entry` / `remove_custom_direct_at` 的「重复/越界不写盘」语义原样保留。
