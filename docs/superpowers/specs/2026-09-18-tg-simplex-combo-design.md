# Telegram + SimpleX 组合部署设计（aegis 第五种部署形态）

日期：2026-09-18
状态：待评审

## 1. 背景

aegis 有两条彼此独立但语义上应当等价的转发链路，现状是它们并不等价：

1. **敏感内容转发器**：`RoutingAdapter`（`rust/aegis/src/common/routing.rs:8`）持有
   `primary` + `Option<secondary>`，`send_message` 命中 `is_sensitive(text)`
   （`routing.rs:43`）时改走 secondary。唯一构造点是
   `rust/aegis/src/main/adapter.rs:26`，secondary **硬编码为 Matrix**，
   `build_adapter` 连 simplex 参数都没有。
2. **平台选择**：`resolve_platform_selection`（`rust/aegis/src/main.rs:157`）
   的决策表规定 `--simplex` 强制 `telegram=false, matrix=false`
   （`main.rs:176-182`），`--all` 只产生 Telegram + Matrix 且「永不包含 simplex」
   （`main.rs:184-190`）。选 SimpleX 时 `main.rs:86-95` 直接
   `handle.adapter.clone()`，**绕过 `build_adapter`**，因此 SimpleX 部署下
   `RoutingAdapter` 根本不会被实例化。

结论：SimpleX 既不能当 secondary（敏感内容落点），当主平台时也没有 secondary。
安装器（`go/installer/main.go`）同样在三个位置把 SimpleX 写成互斥：
选择器合法性（`main.go:408-410`）、文本输入（`main.go:441-454`）、
平台→flag 映射（`main.go:1842-1851`）。SimpleX 的**部署流程本身是完整的**
（`simplexChatVersion = v7.0.0` 锁定、`wwps-simplex.service`、
`installSimplexChat` / `deploySimplexService`、端口回读、卸载清单），
缺的只是「组合」。

## 2. 目标

新增第五种部署形态 **`Telegram + SimpleX`**：Telegram 为主适配器（控制入口与
命令入口），SimpleX 为 secondary（`is_sensitive` 命中的内容落点）。SimpleX 仍可被
管理员当作命令入口使用，语义与现有 `Telegram + Matrix` **完全对等**。

选定方向为「TG 主 + SimpleX 从」（而非反向），因为反向会让敏感内容从 SimpleX 流回
Telegram，与「敏感内容外送」的目的相反。

## 3. 非目标（明确不做）

- **不扩展文件转发**：`send_file` / `send_image` / `send_voice` 继续恒走 primary
  （现有 `routing.rs:82` 行为，TG+Matrix 亦然）。仅转发 `is_sensitive(text)` 命中的文本。
- 不支持三方组合 `Telegram + Matrix + SimpleX`（`secondary` 保持单值，不做 fan-out）。
- 不支持 `Matrix + SimpleX`（缺 Telegram 主平台时该组合无意义）。
- 不实现 SimpleX 断线重连（既有已知限制）。
- 不修 `DESTRUCT_TARGETS` / `DESTRUCT_SERVICES` 未含 `wwps-simplex` 的既有缺口。
- 不改 `has_matrix_config` / `has_simplex_config` 的判定口径。

## 4. 契约（flag 与组合矩阵）

| 显式 flag | 结果 |
|---|---|
| 无 | 自动探测，**逻辑完全不变**（见 `main.rs:205-224`） |
| `--matrix` | 仅 Matrix（不变） |
| `--simplex` | 仅 SimpleX（不变，仍关闭 TG 与 Matrix） |
| `--all` | Telegram + Matrix（不变，仍永不含 SimpleX） |
| `--tg-only` | 仅 Telegram（不变） |
| **`--tg-simplex`（新增）** | Telegram 主 + SimpleX secondary |
| `--discord` | 硬报错（不变） |

三条不变量必须保持：

1. **组合永远需要显式 flag**。无 flag 时即使 `token` 与 `matrix_*` 齐备也只启
   Matrix —— 这是 `--all` 已有的先例，`--tg-simplex` 沿用同一立场，自动探测分支
   一个字不改。
2. **三方组合在任何入口都硬报错**，不猜优先级。
3. `--simplex` 与 `--tg-simplex` 用 `==` 比较（`main.rs:172` 的既有写法），
   两者互不构成子串歧义。

同时给出互相冲突的 flag 时沿用既有「按判定顺序首个命中者胜」的做法，不新增冲突校验
（安装器永不产生此类组合）：新增的 `--tg-simplex` 排在 `--simplex` 之前，因此
`--simplex --tg-simplex` 与 `--all --tg-simplex` 都得到 `tg + simplex`。该优先级须有测试覆盖。

## 5. Rust 侧改动

### 5.1 `rust/aegis/src/main.rs`

- `resolve_platform_selection`（`:157`）：在 `main.rs:174` 之后新增
  `let use_tg_simplex = args.iter().any(|a| a == "--tg-simplex");`，并在
  `use_simplex` 分支**之前**返回
  `PlatformSelection { telegram: true, matrix: false, simplex: true }`。
- 同步更新函数上方决策表注释（`:137-156`，含「永不包含 simplex」那句的措辞）。
- 更新 `#[cfg(test)]` 中的 `resolve_platform_selection` 用例集（`:344` 起），
  新增 `--tg-simplex` 用例与 `--all --tg-simplex` 优先级用例。

### 5.2 `rust/aegis/src/main.rs:86-95`（适配器装配）

现状：`simplex_handle` 为 `Some` 时直接用 raw adapter，否则调 `build_adapter`。
改为三分支：

1. `selection.simplex && !selection.telegram` → `handle.adapter.clone()`
   （纯 SimpleX，行为不变）。
2. `selection.telegram && selection.simplex` → 先 `build_adapter(token, true, false, &matrix_handle)`
   得到 TG primary，再用
   `RoutingAdapter::new(primary, Some(handle.adapter.clone()))`
   `.with_secondary_target(TargetId(simplex_admin_id.to_string()))` 包一层；
   `simplex_admin_id` 取 `app_config.decrypted.simplex_admin_id`，用
   `.context(...)` 在缺失时给出明确错误（正常路径不可达，仅为防御）。
3. 其余 → `build_adapter(...)` 现状。

正常情况下 `simplex_admin_id` 必然存在：`connect_simplex` 在本装配**之前**运行，
缺 `simplex_port` / `simplex_admin_id` 时已报「缺少 simplex_port」并终止，因此分支 2
的 `.context(...)` 仅为防御性写法，正常路径不可达。本设计**不新增语义错误分支**
（§8 的启动失败仍由 `connect_simplex` 现有报错承担）。

### 5.3 `rust/aegis/src/common/routing.rs`

- `RoutingAdapter` 新增字段 `secondary_target: Option<TargetId>`。
- 新增 `pub fn with_secondary_target(mut self, target: TargetId) -> Self`；
  **`new()` 签名保持不变**，现有 3 处测试与 `build_adapter` 的调用点零改动。
- `send_message`（`:41-46`）在命中 secondary 时用
  `self.secondary_target.as_ref().unwrap_or(target)` 作为目标。

这是本设计唯一的语义新增，原因是两个 secondary 适配器的 target 契约不同：

- `MatrixAdapter::send_message` 忽略 target（`gateways/matrix/adapter.rs:114` 的
  `_target`），固定发往构造时持有的 room；
- `SimplexAdapter::send_message` 会 `parse_chat_id(target)`
  （`gateways/simplex/adapter.rs:183-194`）。

若直接透传 Telegram 的 chat id，SimpleX 会尝试向一个不相关的 contactId 发送。
SimpleX 只有一个已授权联系人，因此 secondary target 恒为 `simplex_admin_id`。

### 5.4 `rust/aegis/src/main/runtime.rs`

- SimpleX 初始化块（`:232`）中 scheduler + 启动通知（`upgrade_success` /
  `bbr3_reboot_result` / `notify_online`）的门禁（`:238` 的
  `if let Some(admin_contact) = simplex_admin`）收敛为
  `if let Some(admin_contact) = simplex_admin.filter(|_| !enable_telegram)`，
  避免 Telegram 与 SimpleX 各起一个 scheduler 导致定时通知翻倍。
- SimpleX **事件循环保持常开**：管理员仍可从 SimpleX 发命令。事件回包走
  `handle.adapter`（raw SimpleX 适配器），不经过 `state.adapter`，因此回包必然
  回到 SimpleX，且不受敏感判定影响。
- `simplex_enabled && !enable_telegram && !enable_matrix` 的保活分支
  （`:503`）无需改动：TG+SimpleX 下由 Telegram Dispatcher 阻塞主流程。

### 5.5 无需改动但需在验收中确认

- `AppState::is_admin_user`（`app/src/state.rs:133`）已同时接受 `admin_id` 与
  `simplex_admin_id`。
- `enable_matrix && !enable_telegram` 的 Matrix-only 保活分支在 `matrix=false` 时跳过。

## 6. 数据流（`--tg-simplex` 运行时）

```
Telegram 用户消息
  └─► TG dispatcher ──► state.adapter = RoutingAdapter(TG primary,
                                                      secondary = SimplexAdapter,
                                                      secondary_target = simplex_admin_id)
        ├─ 非敏感（含 markup 渲染后文本）──► TG 适配器 ──► TG 管理员
        └─ is_sensitive 命中 ──► SimplexAdapter(target = simplex_admin_id) ──► SimpleX

SimpleX 用户消息
  └─► SimpleX 事件循环（handle.adapter = raw SimplexAdapter）──► 回包回 SimpleX

scheduler / 启动通知（单实例，在 Telegram 分支启动，target = TG admin_id）
  └─► 同一个 RoutingAdapter 分流（敏感的导出结果落 SimpleX）
```

与 `Telegram + Matrix` 的唯一差别是 secondary 的 target 需要重映射（§5.3）。

## 7. Go/installer 改动

| 位置 | 改动 |
|---|---|
| `main.go:365` `platformSelector.Update` | 切换 TG 不再清 SimpleX；SimpleX 只清 Matrix；Matrix 仍清 SimpleX |
| `main.go:406-410` `platformSelection` | 合法性改为 `(tg\|\|matrix\|\|simplex) && !(simplex && matrix)` |
| `main.go:441-454` `parsePlatformChoice` | 新增 `telegram+simplex` → `(true, false, true)`（`ReplaceAll` 已去空格） |
| `main.go:1561` `platformSetupForChoice` | 新增 `"6"` → `(true, false, true)`；`"3"` 的 Discord 空洞保留 |
| `main.go:1579` `servicePlatformForSetup` | 新增 `tg && simplex → "tg-simplex"`，**必须排在 `simplex` 分支之前** |
| `main.go:1842` `platformFlagFor` | 新增 `"tg-simplex" → "--tg-simplex"` |
| `main.go:1857-1863` `writeSystemdService` | 新增描述 `WWPS Telegram + SimpleX Bot` |
| `main.go:1813` `platformFromService` | 新增 `--tg-simplex` 分支（升级重跑时从既有单元回读平台）；`--tg-simplex` 不包含子串 `--simplex`，无碰撞，仍置于 `--simplex` 之前 |
| `main.go:1087` `deploySimplexService` | 门禁从 `platform != "simplex"` 改为 `platform != "simplex" && platform != "tg-simplex"` |
| `main.go:1377` `installFromStdin`（`:1404-1421` 推导） | `simplex_port` 与 `token` 同时存在 → `"tg-simplex"`；三个平台字段齐备 → 硬报错 |
| `main.go:1516` `installFromKeyVal`（`:1538-1548` 推导） | `token` + `simplex_port` → `"tg-simplex"`；三者齐备 → 硬报错 |
| `i18n/{en,zh,ja}.json` | `firsttime.platform_prompt` 与 `firsttime.platform_text_prompt` 补 `6=Telegram + SimpleX`；新增三方组合拒绝文案 |

**顺带修复的既有缺陷**：`main.go:1538-1548` 当前的推导顺序使 `token` +
`simplex_port` 落到 `"tg"`，写入的单元 flag 为空串，而 aegis 无 flag 自动探测会选中
SimpleX —— **Telegram 配置被静默丢弃**。新推导规则（显式判定组合）一并消除这条
静默换平台路径。

## 8. 错误处理

| 场景 | 行为 |
|---|---|
| `--tg-simplex` 但 config 缺 `simplex_port` / `simplex_admin_id` | 启动失败；由 `connect_simplex` 现有报错承担，不新增分支 |
| stdin / keyval 三个平台字段齐备 | 安装器硬报错，不猜优先级 |
| 无 flag 且 `matrix_*` 与 `simplex_*` 齐备 | 既有「配置歧义」报错不变 |
| 既有单元含 `--discord` | 既有硬报错不变 |
| `--discord` 与 `--tg-simplex` 同时给出 | 由 `--discord` 的硬报错先拦截 |

## 9. 测试策略（TDD：先 RED，再 GREEN）

Rust：

- `resolve_platform_selection`：`--tg-simplex` → `(tg, simplex)`；
  `--tg-simplex --all` 的优先级；`--tg-simplex` 不影响既有 4 个用例。
- `RoutingAdapter`：新增「敏感内容走 secondary 且 target 被重映射为
  `secondary_target`」与「非敏感仍走 primary 且 target 原样透传」两个用例；
  现有 3 个用例保持通过（验证 `new()` 签名未破坏兼容）。

Go（`go/installer/main_test.go`）：

- 选择器：`tg + simplex` 合法；`matrix + simplex` 拒绝；三方拒绝；
  勾选 TG 不再清空 SimpleX；勾选 SimpleX 清空 Matrix；`simplex` 单独合法。
- `parsePlatformChoice("telegram+simplex")` 与带空格变体。
- `platformSetupForChoice("6")`。
- `servicePlatformForSetup(tg=true, simplex=true)` → `"tg-simplex"`；
  `(simplex=true)` → `"simplex"`。
- `platformFromService` 对 `--tg-simplex` 单元内容 → `"tg-simplex"`。
- `platformFlagFor("tg-simplex")` → `"--tg-simplex"`。
- 非交互两条推导：`token + simplex_port` → `"tg-simplex"`；三方齐备 → 报错。

## 10. 文档更新

- 本文件（设计）。
- `docs/2026-09-16-simplex-platform.md`：§「平台互斥语义」表格与要点已过时
  （现文写「启用 SimpleX 会强制关闭 Telegram」），必须更新为新的组合矩阵。
- `README.md:47`（敏感转发说明）与 `:148`（平台与 flag 对照表）：补 `--tg-simplex`。

## 11. 验收清单

在真实主机上以 `--tg-simplex` 部署后逐项确认：

- [ ] Telegram 侧 `/menu` 可用，菜单正常渲染
- [ ] SimpleX 侧 `/menu` 有响应，回包落在 SimpleX
- [ ] 触发一次含 `vless://` 的导出（敏感）→ 只在 SimpleX 出现，TG 侧无
- [ ] 普通状态消息（如启动通知）→ 只在 TG 出现，SimpleX 侧无
- [ ] `journalctl -u wwps-aegis` 中 scheduler 初始化只出现一次，定时通知不翻倍
- [ ] `systemctl status wwps-simplex` 为 active，且 `ss -ltnp` 仅见 `127.0.0.1:<port>`
- [ ] 重跑安装器：平台从既有单元回读为 `tg-simplex`，不降级为 tg 或 simplex
- [ ] 非交互 stdin / keyval 携带 `token` + `simplex_port` 时单元为 `--tg-simplex`
