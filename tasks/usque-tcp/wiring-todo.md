# Tasks: usque-wiring

> 属于 `feat/usque-wiring` 分支。
> ⚠️ `tasks/plan.md` / `tasks/todo.md` 属于 `custom-allowlist`，不要覆盖。

**执行方式**: TDD（RED → GREEN → REFACTOR），每任务一提交
**测试命令（权威）**: `cargo nextest run --cargo-profile fast-test`
**工作目录**: `rust/aegis/`
**基线**: 1223 passed, 1 skipped

## 进度

| 任务 | 状态 |
|---|---|
| T1–T10 | ⬜ 待做 |

---

## T1: 按探测结论分派安装（纯函数）

**Description**: 新增 `install_plan(conclusion, probe_failed) -> Result<InstallPlan>`，
把「装什么」的判定抽成纯函数。

**验收标准:**
- [ ] `pub enum InstallPlan { WireGuard, Usque }`
- [ ] `WgUsable` → `WireGuard`；`UdpOnly` → `WireGuard`；`TcpOnly` → `Usque`
- [ ] `FullyBlocked` → `Err`（拒绝安装）
- [ ] `ProbeFailed` 也要拒绝（环境异常时不应基于未知状态安装）
- [ ] 穷举 4 态 + `ProbeFailed` 的测试（**不是**抽 2 个）

**验证:** `cargo nextest run usque::installer`

**依赖**: 无
**文件:** `src/core/usque/installer.rs`

**RED:**
```rust
#[test]
fn test_install_plan_refuses_when_fully_blocked() {
    assert!(install_plan(ProbeConclusion::FullyBlocked, false).is_err(),
        "两条通路都不通时装了没意义，必须拒绝");
}
```

---

## T2: `install_smart` 实时探测 + 写缓存

**Description**: 组合：实时 `run_probe()` → `install_plan` → 执行 → 写缓存。

**验收标准:**
- [ ] 用**实时探测**（不读缓存）——安装是一次性决策
- [ ] 成功后 `global_probe_cache().store(report, now_monotonic())`
- [ ] 返回装了哪套（供 bot 文案）
- [ ] 拒绝时错误信息含**具体原因**（哪条路不通）
- [ ] 测试：mock 探测结论 → 断言分派与缓存写入

**验证:** `cargo nextest run usque::installer`

**依赖**: T1
**文件:** `src/core/usque/installer.rs`

---

## T3: 拆分 WireGuard / usque 安装（可独立调用）

**Description**: 把现有 `WarpInstaller::install()` 拆成可独立调用的两半。

**验收标准:**
- [ ] `install_wireguard_only()`：现有逻辑（保留既有配置 + 注册账号 + 写路由）
- [ ] `install_usque_only(arch)`：转发到 `usque::installer::install(arch)`
- [ ] 两者都幂等
- [ ] `install()` 保留为兼容入口（等价于 WireGuard 那半 + 写缓存）
- [ ] 既有 Spec 1/2 测试全通过

**验证:** `cargo nextest run xray::installer`

**依赖**: 无
**文件:** `src/core/xray/installer.rs`

---

## T4: 健康检查循环

**Description**: `core/usque/health.rs` 加后台循环函数。

**验收标准:**
- [ ] `health_loop()`：每 `HEALTH_INTERVAL` 跑一次，**永不返回**（除非 panic）
- [ ] 未装 usque → 跳过，**不发网络请求**
- [ ] 走 SOCKS 请求 `cdn-cgi/trace`，用 `is_tunnel_healthy` 判定
- [ ] 达阈值 → `systemctl restart wwps-usque` + **reset tracker**
- [ ] 循环体内错误被 catch，**不上抛**（避免循环退出导致静默失效）
- [ ] 测试：用可注入的探测闭包断言「达阈值触发重启」「成功清零」「未装不发请求」

**验证:** `cargo nextest run usque::health`

**依赖**: 无
**文件:** `src/core/usque/health.rs`

**注意**: 时间注入 —— 用参数传入 sleep 时长或可注入的 `sleep` fn，便于快速测试。

---

## T5: 健康状态进程级缓存

**Description**: 状态页读真实健康，但**不**在渲染时探测（会阻塞最多 20s）。

**验收标准:**
- [ ] `global_health_state()`：`Mutex<Option<bool>>`
- [ ] 健康循环写它；`usque_status_line` 读它
- [ ] 未探测时仍为 `None`（显示「未探测」，不谎报）
- [ ] 测试：写入后读出；初始为 None

**验证:** `cargo nextest run usque::health`

**依赖**: T4
**文件:** `src/core/usque/health.rs`

---

## T6: `spawn_background_tasks()` 统一入口

**Description**: 封一个入口，避免 3 处 spawn 点漏掉其一。

**验收标准:**
- [ ] `pub fn spawn_background_tasks()` 只 spawn **健康检查循环**
- [ ] **不得** spawn 定时探测任务（决策 W3）——写注释说明原因
- [ ] 测试：断言源码中只 spawn 一件事（防后人"顺手"加回定时重探）

**验证:** `cargo nextest run usque::`

**依赖**: T4
**文件:** `src/core/usque/mod.rs`

---

## T7: 接入 `runtime.rs` 三处启动点

**Description**: 在 3 个 `start_scheduler` 旁调用 `spawn_background_tasks()`。

**验收标准:**
- [ ] 3 处启动点全部接入（TG / SimpleX / 组合）
- [ ] 接入方式一致（避免某部署形态下静默失效）
- [ ] 编译通过

**验证:** `cargo check --all-targets`

**依赖**: T6
**文件:** `src/main/runtime.rs`

---

## T8: handler 接线

**Description**: `a_inst_warp` 改调 `install_smart`；`a_warp_status` 读真实健康。

**验收标准:**
- [ ] `a_inst_warp` → `WarpInstaller::install_smart()`
- [ ] 成功文案由**实际装了哪套**决定（不是笼统"安装成功"）
- [ ] 拒绝时文案说明原因（哪条路不通）
- [ ] `a_warp_status` 的 `healthy` 参数改读 `global_health_state()`
- [ ] 卸载按钮的既有行为不变

**验证:** `cargo nextest run handlers::warp`

**依赖**: T2, T5
**文件:** `src/shared/handlers/warp.rs`

---

## T9: i18n 三语

**Description**: 补齐新文案。

**验收标准:**
- [ ] 新增：装 usque 成功 / 装 WireGuard 成功 / 拒绝安装（含原因）
- [ ] `zh`/`en`/`ja` key 集合**完全一致**（脚本校验）
- [ ] 无硬编码文案
- [ ] 现有键未被破坏

**验证:** `cargo nextest run` 全绿 + 脚本比对三语

**依赖**: T8
**文件:** `src/resources/i18n/{zh,en,ja}.yml`

---

## T10: 质量门禁 + 审查

**验收标准:**
- [ ] `cargo fmt` 无 diff
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 零警告
- [ ] `cargo nextest run --cargo-profile fast-test` ≥ 1223 通过
- [ ] `cargo test --doc` 通过
- [ ] 五轴审查无 Critical/High

**依赖**: T1–T9
