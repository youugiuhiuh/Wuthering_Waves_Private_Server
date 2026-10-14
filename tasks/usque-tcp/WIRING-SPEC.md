# Spec 2 接线层补全（Wiring）

> 状态：**待批准**。批准前不进入 BUILD。
> 起因：线上部署后发现 `usque::installer::install()` **无任何调用点**，
> usque 从未被安装。本文件覆盖补全接线层的设计。

---

## 问题：代码存在但未接线

线上复现（racknerd-534f8f8，2026-10-14 21:25）：

```
新 aegis 已部署（含 usque 代码）         ✅
TG 点「安装 WARP」                       ✅ 执行了
  → 但走的是 WarpInstaller::install()    ← Spec 1 的 WireGuard 路径
  → warp_account.json 重新生成           ✅
  → 10_warp_routing.json 仍是 wireguard  ❌ 永远不会通（UDP 被封）
/etc/wwps/usque/                         ❌ 不存在
wwps-usque.service                       ❌ 不存在
```

**根因**：
```bash
$ grep -rn 'usque::installer::install' src/
  ❌ 无任何调用点
```

`install()` 是**孤儿代码** —— 只有单测在调用。我先前只验证了「函数存在」，
没验证「是否被调用」。这是该做而没做的检查。

### 缺失面（比我先前报告的大）

| 环节 | 状态 |
|---|---|
| usque 安装代码 | ⚠️ 写了，但**无调用点** |
| TG 安装按钮接入 usque | ❌ 未接 |
| 出口引擎写入探测缓存 | ❌ **无任何地方写入** |
| 定时探测 | ❌ 未实现 |
| 健康检查调度 + 强杀 | ❌ 未实现 |
| 健康状态接入 `a_warp_status` | ⚠️ 纯函数在，但 `healthy` 参数永远传 `None` |

**连锁后果**：缓存永远为空 → `current_egress()` 保守回退 `Direct` →
即使装了 usque 也不会启用 → WARP 永远不生效。

---

## 决策（已确认）

| # | 决策点 | 选择 |
|---|---|---|
| W1 | 补全范围 | **安装接线 + 健康检查调度**（**不做**定时重探） |
| W2 | TG 安装按钮语义 | **按探测结果只装需要的** |
| W3 | 探测频率 | **只在安装时探一次**，写入缓存后不再重探 |
| W4 | 交付方式 | **接线层另开 PR**（#382 先合） |

### W2 的具体语义

```
点「安装 WARP」
      ↓
跑一次探测（实时，非缓存）
      ↓
┌─────────────────────┬──────────────────────────────┐
│ WgUsable            │ 只装 WireGuard               │
│ UdpOnly             │ 只装 WireGuard               │
│ TcpOnly             │ 只装 usque                   │
│ FullyBlocked        │ **拒绝安装** + 明确错误提示   │
└─────────────────────┴──────────────────────────────┘
      ↓
写入探测缓存（供 current_egress 使用）
```

**`FullyBlocked` 必须拒绝**：两条通路都不可用时安装毫无意义，
而且会让管理员误以为「装好了」。宁可明确报错。

**探测失败（`ProbeFailed`）也要拒绝**：本地发包都失败说明环境有问题，
不应基于未知状态安装。

---

## 设计

### 1. 安装按钮：按探测结果分派

```
a_inst_warp  (handlers/warp.rs)
      ↓
WarpInstaller::install_smart()          ← 新增
      ↓
run_probe()  (实时探测，不用缓存)
      ↓
select_egress(conclusion) → Egress
      ↓
match egress {
    WireGuard => install_wireguard_only(),
    Masque    => install_usque_only(),
    Direct    => bail!("出口完全受限，拒绝安装")
}
      ↓
global_probe_cache().store(report, now)  ← 写缓存
```

**要点**：
- 用**实时探测**而非缓存 —— 安装是一次性决策，不该基于过期数据
- 安装成功后**写缓存** —— 让 `current_egress()` 立刻可用
- 复用 Spec 1 的 `run_probe()`，不新增探测逻辑

### 2. 探测缓存：只在安装时写入一次（决策 W3）

**不做定时重探。** 探测只在点「安装 WARP」时执行一次，
结果写入 `global_probe_cache()` 后不再刷新。

**为什么可以不复探**（推理链）：

1. 出口条件（机房是否封 UDP）是**机房级策略**，不随时间漂移。
   本部署实测：UDP 2408 被丢、TCP 443 通 —— 这是固定的。
2. 缓存 TTL 10 分钟到期后 `current_egress()` 会回退 `Direct`。
   但这**只在重新生成 Xray 配置时**才被读到（增删域名规则、切模式等）。
   回退 `Direct` 的后果是「那次配置变更不启用 WARP」——**不会黑洞**，
   因为 `build_routing_rules` 在 `enforce=false` 时根本不生成 catch-all。
3. 想要重新启用，管理员**再点一次「安装 WARP」**即可（幂等：已装的会跳过）。

**代价（明确记录）**：若机房策略后来变化（例如解封了 UDP），
系统不会自动切回 WireGuard，需要管理员手动重触发一次安装。
这是「少一个常驻循环」换来的确定性。

**替代方案（未采用）**：定时重探（每 5 min）。
收益是能自动跟随网络变化；代价是多一个常驻任务、
多一份「探测失败时是否该降级」的判断面。

### 3. 健康检查调度 + 强杀

```rust
// 仅在 MASQUE 出口启用时才有意义
loop {
    sleep(HEALTH_INTERVAL).await;   // 60s
    if !usque_installed() { continue; }

    let ok = probe_tunnel_health().await;   // 走 SOCKS 请求 cdn-cgi/trace
    tracker.record(ok);

    if tracker.should_restart() {
        log::warn!("usque 连续 {} 次不健康，强杀重启", tracker.consecutive_failures());
        control_service(SERVICE_NAME, Restart).await;
        tracker.reset();
    }
}
```

**要点**：
- 用 `Mutex<FailureTracker>` 跨循环持有（`FailureTracker` 是值语义）
- 重启后**必须 reset**，否则会立刻再次触发
- 无 usque 时直接跳过，不做无谓的网络请求

### 4. `a_warp_status` 接入真实健康

当前 `usque_status_line(installed, healthy)` 的 `healthy` 永远传 `None`。
改为把健康结果缓存在进程内（与探测缓存同类），状态页读它。

**不**在状态查询时实时探测 —— 那会让菜单渲染阻塞最多 20s
（`HEALTH_TIMEOUT`）。健康数据由调度循环维护，状态页只读。

---

## 影响文件

| 文件 | 改动 |
|---|---|
| `core/usque/installer.rs` | 新增 `install_smart(conclusion)`；拆出 `install_wireguard_only` / `install_usque_only` |
| `core/usque/health.rs` | 新增健康检查循环函数 + 进程级健康状态缓存 |
| `core/usque/engine.rs` | `now_monotonic()` 改为公开（供安装路径写缓存） |
| `core/usque/mod.rs` | 新增 `spawn_background_tasks()` 入口 |
| `shared/handlers/warp.rs` | `a_inst_warp` 改调 `install_smart`；`a_warp_status` 读真实健康 |
| `main/runtime.rs` | 在调度器启动点旁 spawn **健康检查循环**（3 处启动点） |
| `resources/i18n/{zh,en,ja}.yml` | 新增「拒绝安装」「按探测结果安装」等文案 |

---

## 验证方式

**单元测试**
- `install_smart` 的 4 态分派（穷举）——`FullyBlocked` 必须拒绝
- `ProbeFailed` 也必须拒绝
- 安装成功后缓存**确实被写入**（断言 `current_egress()` 随之变化）
- 健康循环：无 usque 时不发请求；达阈值触发重启；重启后 reset
- **安装会写缓存**：断言 `install_smart` 后 `current_egress()` 立即反映探测结论
- **无定时探测任务**：断言后台只 spawn 了健康检查循环（防后人"顺手"加回定时重探）

**回归**
- Spec 1 + Spec 2 全部既有测试
- `cargo fmt` / `clippy -D warnings` / `nextest` / `doc`

**真机验证（你决定）**
- 部署后点安装 → 应装 usque（因这台机器实测是 `TcpOnly`）
- `ss -tlnp | grep 1080` 应看到监听
- `10_warp_routing.json` 的 outbound 应变为 `socks`
- 走隧道请求 `cdn-cgi/trace` 应返回 `warp=on`

---

## 风险

| 风险 | 缓解 |
|---|---|
| 后台循环 panic 导致静默失效 | 循环体内部 catch 错误，不 `?` 上抛 |
| 健康检查误杀健康隧道 | 连续 3 次阈值 + 成功清零（Spec 2 已实现） |
| 三处 spawn 点漏掉一处 | 统一封成 `spawn_background_tasks()`，三处都调 |
| 安装拒绝时管理员困惑 | i18n 明确说明原因与下一步 |
