# P1 实施计划：SimpleX 管理员自愈 + 全局限流

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 SimpleX 管理员在 contactId 变化后（删联系人重连、重装客户端、换设备）**只需发一次 6 位 TOTP 码**即可恢复权限，无需再读 journal 抄 contactId、无需改配置。

**Architecture:** 在纯 `--simplex` 部署下把「管理员身份」的依据从易变的 `contactId` 转移到稳定的 TOTP 密钥：TOTP 验证成功即就地重钉 `simplex_admin_id`（落盘 + 内存态）。为此必须放开**两道**门禁让 6 位码能到达验证逻辑，并新增**全局**失败计数（因为白名单放开后「每身份一份尝试预算」会让暴力破解面随联系人数量放大）。

**Tech Stack:** Rust 2024；`log` 0.4；`tokio::sync::Mutex`；无新增依赖。

**Spec:** `docs/superpowers/specs/2026-09-18-simplex-approval-selfheal-design.md`（本计划实现其中的 P1 阶段）

## Global Constraints

- **工作树**：`/home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected`，分支 `fix/simplex-contact-connected`
- **Rust 目标目录必须复用**（否则重建 matrix-sdk 全套，10+ 分钟）：每条 cargo 命令前
  `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`
- **完整 Rust 质量门**（每个任务提交前全绿）：
  `cargo fmt --check` ｜ `cargo clippy --all-targets --all-features -- -D warnings` ｜
  `cargo nextest run --cargo-profile fast-test` ｜ `cargo test --doc`
- **回归基线**：Rust **928 passed, 1 skipped**（`72f644d` 实测）。本计划累计 +17 → 预计 **945**
  （链路：928 → T1 931 → T2 基础 934 → T2 修复轮 935 → T3 938 → T4 942 → T5 945 → T6 945）
- **不新增依赖**（Rust 无）
- **不许改动**（超出本计划范围）：`--tg-simplex` 的硬校验；`--discord` 拒绝路径；`BotAdapter` trait；
  `MessageEvent` / `CommandEvent` 的字段（各有 12 / 多处构造点，加字段会散开）；`AppState::new` 的签名
  （有 9 个调用点，8 个在测试）
- **git pathspec 相对当时 cwd 解析**：在 `rust/aegis` 下用 `git add src/...`
- **`bootstrap.rs` 同时编进 lib 与 bin**（`src/lib.rs:6` `pub mod bootstrap;` + `src/main.rs:6` `mod bootstrap;`），
  因此放在那里的测试**会各跑两遍**（+N 变 +2N），这是预期行为，不要「修」
- **安全语义边界（不得越界）**：
  - 自愈**只允许在纯 `--simplex` 下启用**。`is_admin_user` 的 `user_id` 命名空间是 TG 与 SimpleX **共用**的，
    tg-simplex 下无差别重钉会把 `simplex_admin_id` 覆写成 Telegram 的 chat id
  - 全局封锁**必须封顶 900 秒**。`record_auth_failure` 取 `lockout_durations.last()` 作封顶，而最后一级
    是 172800 秒 = 48 小时；若全局复用它，攻击者能靠烧次数把真管理员锁在门外 48 小时（自 DoS）
- **禁写占位符**：本计划每一步都含可直接粘贴的完整代码

---

### Task 1: `AppState` —— `simplex_admin_id` 改为运行时可改 + 自愈开关

**Files:**
- Modify: `rust/aegis/src/app/state.rs`（字段 75、`new()` 内初始化、accessor 128-130、`is_admin_user` 133，以及测试模块）

**Interfaces:**
- Consumes: 无
- Produces:
  - `AppState::with_simplex_repin(self) -> Self`（`#[must_use]`，链式）
  - `AppState::simplex_repin_enabled(&self) -> bool`
  - `AppState::set_simplex_admin_id(&self, admin_id: i64)`（只改内存态，不落盘）
  - `AppState::simplex_admin_id(&self) -> Option<i64>`（签名不变，内部改读原子量）

**背景**：`simplex_admin_id` 现在是 `Option<i64>` 普通字段，运行时改不了。改成 `AtomicI64`
（0 表示无）可保持 accessor 的**同步**签名 —— 现有的调用点 `runtime.rs:237` 与 `state.rs:133`
都处在同步上下文，换成 `Mutex` 会破坏它们。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/app/state.rs` 的 `#[cfg(test)] mod tests` 里追加：

```rust
    #[test]
    fn simplex_admin_id_can_be_repinned_at_runtime() {
        let state = AppState::new(
            None,
            Some(3),
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        );
        assert_eq!(state.simplex_admin_id(), Some(3));
        assert!(state.is_admin_user(3));
        assert!(!state.is_admin_user(4));

        // 就地重钉：内存态应立即反映，且 is_admin_user 跟随
        state.set_simplex_admin_id(4);
        assert_eq!(state.simplex_admin_id(), Some(4));
        assert!(state.is_admin_user(4), "重钉后新身份必须被认作管理员");
        assert!(!state.is_admin_user(3), "重钉后旧身份必须失去管理员身份");
    }

    #[test]
    fn simplex_admin_id_none_round_trips_as_zero() {
        let state = AppState::new(
            None,
            None,
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        );
        assert_eq!(state.simplex_admin_id(), None);
    }

    #[test]
    fn simple_repin_is_off_by_default_and_opt_in() {
        let state = AppState::new(
            None,
            None,
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        );
        assert!(
            !state.simplex_repin_enabled(),
            "默认必须关闭：tg-simplex 下重钉会把 simplex_admin_id 覆写成 TG chat id"
        );
        assert!(state.with_simplex_repin().simplex_repin_enabled());
    }
```

**⚠️ 不要断言 `!is_admin_user(0)`。** `is_admin_user` 的实现是
`user_id == self.admin_id.unwrap_or(0) || …`，而本任务要求**保留**该语义；当 `admin_id` 为 `None` 时
`is_admin_user(0)` 就是 `true` —— 这是**既有行为**，不是本任务引入的。
它也不可利用：Telegram chat id 永不为 0；SimpleX contactId 为正整数（`set_simplex_admin_id` 对 `<= 0` 直接 bail），
且门禁① 要求 `simplex_admin == Some(contact_id)` 精确相等。
在 `is_admin_user` 里加哨兵会改变安全敏感路径上 Telegram 的既有行为，**不属于本任务**。

同一个 atomic 哨兵约定由 `simplex_admin_id() == None` 这一行已充分覆盖。

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex_admin_id
```
Expected: **编译失败**（`set_simplex_admin_id`、`simplex_repin_enabled`、`with_simplex_repin` 不存在）。
编译失败即本步的 RED。

- [ ] **Step 3: 实现**

在 `rust/aegis/src/app/state.rs` 顶部的 use 区追加（现在只有 `use std::sync::Arc;`）：

```rust
use std::sync::atomic::{AtomicI64, Ordering};
```

把字段声明（当前 `simplex_admin_id: Option<i64>,`）改为：

```rust
    /// 运行时可变：TOTP 验证成功后可就地重钉（见 `with_simplex_repin`）。
    /// `0` 表示「未配置」—— 这与 `is_admin_user` 里 `admin_id.unwrap_or(0)` 的既有约定一致。
    simplex_admin_id: AtomicI64,
    /// 是否允许 TOTP 成功后重钉 `simplex_admin_id`。仅纯 `--simplex` 部署开启。
    simplex_repin_enabled: bool,
```

在 `new()` 的构造体里，把 `simplex_admin_id,` 那行改为（其余字段不动）：

```rust
            simplex_admin_id: AtomicI64::new(simplex_admin_id.unwrap_or(0)),
            simplex_repin_enabled: false,
```

把 accessor 与 `is_admin_user`（当前 128-133 行）改为：

```rust
    pub fn simplex_admin_id(&self) -> Option<i64> {
        match self.simplex_admin_id.load(Ordering::Relaxed) {
            0 => None,
            id => Some(id),
        }
    }

    /// 就地重钉管理员身份（**只改内存态**；落盘由调用方负责）。
    pub fn set_simplex_admin_id(&self, admin_id: i64) {
        self.simplex_admin_id.store(admin_id, Ordering::Relaxed);
    }

    /// 允许 TOTP 验证成功后重钉 `simplex_admin_id`。
    ///
    /// **只允许在纯 `--simplex` 部署下开启。** `is_admin_user` 的 `user_id` 命名空间是
    /// Telegram 与 SimpleX **共用**的；`--tg-simplex` 下两个平台同时在线，无差别重钉会把
    /// `simplex_admin_id` 覆写成 Telegram 的 chat id，从而破坏「敏感内容落点」这个发送目标。
    #[must_use]
    pub fn with_simplex_repin(mut self) -> Self {
        self.simplex_repin_enabled = true;
        self
    }

    pub fn simplex_repin_enabled(&self) -> bool {
        self.simplex_repin_enabled
    }
```

`is_admin_user` 的实现改为（只把 `self.simplex_admin_id == Some(user_id)` 换成 accessor）：

```rust
    pub fn is_admin_user(&self, user_id: i64) -> bool {
        user_id == self.admin_id.unwrap_or(0) || self.simplex_admin_id() == Some(user_id)
    }
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex
```
Expected: 全绿，含既有 `simplex_admin_id_is_recognized_as_admin` 与 3 条新测试

- [ ] **Step 5: 跑完整 Rust 质量门**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings \
  && cargo nextest run --cargo-profile fast-test && cargo test --doc
```
Expected: clippy exit 0 且零 `clippy::` 诊断；nextest **931 passed, 1 skipped**（928 + 3）；doctest ok

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/app/state.rs
git commit -m "feat(aegis): simplex_admin_id 支持运行时重钉 + 自愈开关（默认关）"
```

---

### Task 2: 全局限流 + 900 秒封顶

**Files:**
- Modify: `rust/aegis/src/app/state.rs`（字段区、`new()` 构造体、`record_auth_failure` 216-263、`auth_cooldown_remaining` 265-、测试模块）

**Interfaces:**
- Consumes: Task 1 的 `simplex_repin_enabled`（不相关，但同文件顺序修改）
- Produces:
  - `AppState::GLOBAL_LOCKOUT_CAP: Duration`（= 900 秒，public 常量）
  - 全局失败记录参与 `record_auth_failure` 与 `auth_cooldown_remaining`

**背景（这是本计划最危险的一处）**：`failed_attempts: Mutex<HashMap<i64, FailedRecord>>`
按 `user_id` **独立**计数（5 次 / 600 秒）。白名单存在时攻击者只有 1 个身份，per-user 等价于全局；
门禁一放开（Task 3），攻击者可以用公开地址造 N 个联系人拿到 N 份独立预算，6 位码的
3/10⁶ 单次命中率就被线性乘上去（1000 个联系人 ≈ 11 小时可暴力破解）。

必须加**全局**计数；而全局计数会带来**自我 DoS**：攻击者烧够次数就能把真管理员锁在门外。
`record_auth_failure` 用 `lockout_durations.last()` 作封顶，而最后一级是 **172800 秒 = 48 小时** ——
所以全局记录**必须**用自己的 900 秒封顶，不能复用那条梯度。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/app/state.rs` 的测试模块里追加：

```rust
    /// 全局计数必须与身份无关：换一个 user_id 不能重置预算。
    #[tokio::test]
    async fn global_failures_are_not_reset_by_switching_identity() {
        let state = make_state();
        let now = Instant::now();
        let max = 5u32;
        let window = Duration::from_secs(600);
        let ladder = [Duration::from_secs(900)];

        // 每个身份各失败 1 次（per-user 计数永远到不了 5）
        for uid in 1..=5 {
            let out = state
                .record_auth_failure(uid, now, max, window, &ladder)
                .await;
            assert!(
                matches!(out, AuthFailureOutcome::Invalid { .. }),
                "uid={uid} 单身份不应触发封锁"
            );
        }

        // 第 6 个身份：全局已达 5 次，必须被封锁
        let out = state
            .record_auth_failure(6, now, max, window, &ladder)
            .await;
        assert!(
            matches!(out, AuthFailureOutcome::Locked { .. }),
            "全局计数达上限必须对任何身份封锁，否则造联系人即可无限刷码"
        );
    }

    /// 全局封锁必须封顶 900 秒 —— 梯度最后一级是 48 小时，复用会变成自我 DoS。
    #[tokio::test]
    async fn global_lockout_is_capped_at_900_seconds() {
        let state = make_state();
        let now = Instant::now();
        // 故意传入一条以 48 小时结尾的梯度（与 dispatch.rs 实际传入的一致）
        let ladder = [
            Duration::from_secs(900),
            Duration::from_secs(3600),
            Duration::from_secs(86400),
            Duration::from_secs(172_800),
        ];
        let max = 5u32;
        let window = Duration::from_secs(600);

        let mut last = None;
        // 反复触发封锁，逼出梯度最高级
        for round in 0..8 {
            for i in 0..max {
                last = Some(
                    state
                        .record_auth_failure(100 + round * 10 + i, now, max, window, &ladder)
                        .await,
                );
            }
        }
        match last.expect("至少触发一次封锁") {
            AuthFailureOutcome::Locked { duration } => assert!(
                duration <= AppState::GLOBAL_LOCKOUT_CAP,
                "全局封锁必须封顶 900s，实际 {duration:?}；\
                 复用梯度的 48h 会让攻击者把真管理员永久锁在门外"
            ),
            other => panic!("期望 Locked，得到 {other:?}"),
        }
    }

    /// 全局冷却必须能被任何身份观察到（真的挡住了）。
    #[tokio::test]
    async fn global_cooldown_blocks_all_identities() {
        let state = make_state();
        let now = Instant::now();
        let max = 5u32;
        let window = Duration::from_secs(600);
        let ladder = [Duration::from_secs(900)];

        for uid in 1..=5 {
            let _ = state.record_auth_failure(uid, now, max, window, &ladder).await;
        }

        assert!(
            state.auth_cooldown_remaining(999, now).await.is_some(),
            "全局封锁期间，任何身份（含从未失败过的新身份）都必须被挡"
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test global_
```
Expected: **编译失败**（`AppState::GLOBAL_LOCKOUT_CAP` 不存在）即 RED；
若已能编译则 3 条测试必红（全局计数尚未实现）。

- [ ] **Step 3: 实现**

在 `rust/aegis/src/app/state.rs` 的字段声明区（`failed_attempts` 之后）追加：

```rust
    /// 全局失败记录，与 per-user 记录**并存**。
    ///
    /// per-user 记录按身份限额；但门禁放开后攻击者可用公开地址造 N 个联系人拿到 N 份
    /// 独立预算，因此必须有一条与身份无关的总计数把总尝试次数压住。
    global_failed_attempts: Mutex<FailedRecord>,
```

在 `new()` 构造体里（`failed_attempts: Mutex::new(HashMap::new()),` 之后）追加：

```rust
            global_failed_attempts: Mutex::new(FailedRecord {
                count: 0,
                first_fail: Instant::now(),
                cooldown_until: None,
                lock_level: 0,
            }),
```

在 `impl AppState` 内（`RECENT_AUTH_WINDOW_SECS` 是文件级常量，放在 impl 里的关联常量）：

```rust
    /// 全局封锁的时长上限。
    ///
    /// **不能复用 `lockout_durations`**：`record_auth_failure` 取 `.last()` 作封顶，
    /// 而 `dispatch.rs` 传入的最后一级是 172800 秒（48 小时）。若全局复用它，
    /// 攻击者烧够次数就能把真管理员锁在门外 48 小时（自我 DoS）。
    ///
    /// 900 秒可接受，因为 `failed_attempts` 系列是**纯内存态**（本文件内 `Mutex`，
    /// 全仓无持久化引用）：`systemctl restart wwps-aegis` 立即清零，
    /// 而服务器管理员有 root ——「等 ≤15 分钟 / 重启」两条路都在自己手上。
    pub const GLOBAL_LOCKOUT_CAP: Duration = Duration::from_secs(900);
```

在 `record_auth_failure` 的**开头**（拿到 `self.failed_attempts` 锁之前）插入全局记账调用：

```rust
        self.record_global_failure(now, max_attempts, failure_window)
            .await;
```

并在 `impl AppState` 内新增这个私有方法：

```rust
    /// 记录一次全局失败。达 `max_attempts` 则封锁 `GLOBAL_LOCKOUT_CAP`，不升级、不累积等级。
    async fn record_global_failure(
        &self,
        now: Instant,
        max_attempts: u32,
        failure_window: Duration,
    ) {
        let mut rec = self.global_failed_attempts.lock().await;
        if now.duration_since(rec.first_fail) > failure_window {
            rec.count = 0;
            rec.first_fail = now;
            rec.cooldown_until = None;
        }
        rec.count += 1;
        rec.first_fail = rec.first_fail.min(now);
        if rec.count >= max_attempts {
            rec.cooldown_until = Some(now + Self::GLOBAL_LOCKOUT_CAP);
            rec.count = 0;
            rec.first_fail = now;
        }
    }
```

把 `auth_cooldown_remaining` 改为同时考虑全局记录（保留原 per-user 行为）：

```rust
    pub async fn auth_cooldown_remaining(&self, user_id: i64, now: Instant) -> Option<Duration> {
        // 全局优先：封锁期间任何身份都被挡，包括从未失败过的新身份。
        {
            let mut g = self.global_failed_attempts.lock().await;
            if let Some(until) = g.cooldown_until {
                if until > now {
                    return Some(until - now);
                }
                g.cooldown_until = None;
            }
        }

        let mut fails = self.failed_attempts.lock().await;
        let rec = fails.get_mut(&user_id)?;
        let until = rec.cooldown_until?;
        if until > now {
            Some(until - now)
        } else {
            rec.cooldown_until = None;
            None
        }
    }
```

同时把 `record_auth_success` 改为成功后也清全局计数（一次成功应重置总预算，否则正常使用会把自己拖到封锁）：

```rust
    pub async fn record_auth_success(&self, user_id: i64, now: Instant) -> u64 {
        self.sessions.lock().await.insert(user_id, now);
        self.failed_attempts.lock().await.remove(&user_id);
        {
            let mut g = self.global_failed_attempts.lock().await;
            g.count = 0;
            g.cooldown_until = None;
            g.first_fail = now;
        }
        self.session_timeout_secs().await
    }
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test global_
```
Expected: 3 passed

- [ ] **Step 5: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **935 passed, 1 skipped**（931 + 3 + Task 2 修复轮 1）；clippy 零诊断

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/app/state.rs
git commit -m "feat(aegis): 新增全局限流，封锁封顶 900s（防造联系人放大尝试预算）"
```

---

### Task 3: 门禁② —— `check_auth` 放行 6 位码

**Files:**
- Modify: `rust/aegis/src/shared/dispatch.rs`（`check_auth` 75-101）

**Interfaces:**
- Consumes: 无（`is_totp_code` 已是同文件私有函数，105 行左右）
- Produces: 6 位纯数字消息对**任何**身份都能通过 `check_auth`，从而到达 `auth::process_auth_code`

**背景**：`check_auth` 第一句就是 `if !state.is_admin_user(user_id) { return false; }`，
而底下那段「TOTP codes allowed when not authorized」（89-96）**只对已通过白名单的人生效** ——
对陌生人是不可达的。必须让 6 位码先通过，否则 Task 5 的重钉逻辑永远触发不了。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/shared/dispatch.rs` 的 `#[cfg(test)] mod tests` 里追加。

**先加一个带显式身份的测试辅助**（就放在既有 `message_event` 旁边）：

```rust
    /// 与 `message_event` 相同，但身份显式 —— 既有 `message_event` 把 `user_id` 硬编码为 42，
    /// 而那正是 `make_state()` 的管理员，用它无法测到「非管理员」路径。
    fn message_event_from(
        adapter: Arc<MockAdapter>,
        user_id: i64,
        text: Option<String>,
    ) -> BotEvent {
        BotEvent::Message(MessageEvent {
            adapter,
            target: TargetId(user_id.to_string()),
            user_id,
            text,
            file_id: None,
            file_name: None,
            reply_to_text: None,
            thread_root: None,
        })
    }
```

**⚠️ 三个测试必须真红。** `make_state()` 的管理员是 `Some(42)`（`dispatch.rs:432`），而既有
`message_event` 把 `user_id` 钉死在 42（`:467`）——**所以用 `message_event` 写的测试永远不经过门禁，
改前改后都绿，是空洞测试**。必须用上面这个 `message_event_from` 传一个**不是 42** 的身份。

```rust
    /// 未授权身份发 6 位码必须能通过 check_auth（登录尝试），否则重钉路径不可达。
    #[tokio::test]
    async fn totp_code_from_non_admin_passes_check_auth() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99, // 不是 make_state() 配的管理员（42）
            Some("123456".to_string()),
        );
        assert!(
            check_auth(&event, &state).await,
            "6 位码是登录尝试，必须放行到 process_auth_code；否则新 contactId 永远无法自愈"
        );
    }

    /// 非 6 位码的普通消息来自非管理员时必须仍然被挡。
    #[tokio::test]
    async fn non_code_message_from_non_admin_is_still_rejected() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99,
            Some("hello".to_string()),
        );
        assert!(!check_auth(&event, &state).await);
    }

    /// 6 位但非全数字（如 "12345a"）不算码，仍须被挡。
    #[tokio::test]
    async fn non_numeric_six_chars_from_non_admin_is_rejected() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99,
            Some("12345a".to_string()),
        );
        assert!(!check_auth(&event, &state).await);
    }
```

**已核实的既有辅助（直接复用，禁止新写替身）**：
`TestAdapter`（本模块 271 行）、`NoopExecutor`（424 行）、`make_state()`（432 行，已配管理员 `Some(42)`）、
`message_event(adapter, target, text)`（467 行，**身份硬编码为 42，故不适用于本任务的测试**）。
注意 `MockAdapter` 是带字段的结构体（361 行），必须用 **`MockAdapter::default()`**。

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test passes_check_auth
```
Expected: FAIL（当前 `is_admin_user` 先返回 false）

- [ ] **Step 3: 实现**

把 `check_auth` 开头三行（`let user_id = ...` 与那个 `if !state.is_admin_user(...) { return false; }`）
替换为：

```rust
    let user_id = event.user_id();

    // 6 位纯数字消息是**登录尝试**，必须对任何身份放行到 `auth::process_auth_code`。
    // 这是 SimpleX 管理员自愈的前提：新 contactId（删联系人重连 / 换设备）在重钉之前
    // 一定不是管理员，若在这里就挡掉，它就永远发不出那个能证明自己的码。
    // 安全性由「全局限流 + TOTP 本身」承担，而不是由这道白名单承担。
    let is_login_attempt = matches!(
        event,
        BotEvent::Message(msg) if msg.text.as_deref().is_some_and(is_totp_code)
    ) && !state.is_authorized(user_id).await;

    if !state.is_admin_user(user_id) && !is_login_attempt {
        return false;
    }
```

其余分支保持不变。

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test check_auth
```
Expected: 全绿（含既有 `totp_code_message_when_unauthorized_triggers_auth`）

- [ ] **Step 5: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **938 passed, 1 skipped**（935 + 3）

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/shared/dispatch.rs
git commit -m "feat(aegis): check_auth 放行 6 位码（登录尝试不受管理员白名单约束）"
```

---

### Task 4: 门禁① —— SimpleX 事件循环放行 6 位码

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs`（`for msg in mapped` 里的 contactId 检查，约 317 行；文件底部测试模块）

**Interfaces:**
- Consumes: 无
- Produces: `fn should_forward_simplex_msg(is_admin: bool, text: Option<&str>) -> bool`（同文件私有，供单测）

**背景**：`runtime.rs:317` 的 `if simplex_admin != Some(msg.contact_id) { warn; continue }`
把非管理员的**全部**消息丢掉，包括那个能证明身份的 6 位码 —— 消息根本进不了 dispatch。
这里必须放行 6 位码，否则 Task 3 的改动形同虚设。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/main/runtime.rs` 现有的 `#[cfg(test)] mod tests`（Task 见 #339 已建立）里追加：

```rust
    #[test]
    fn simplex_forwards_totp_code_from_unknown_contact() {
        assert!(
            should_forward_simplex_msg(false, Some("123456")),
            "非管理员发来的 6 位码必须放行，否则新 contactId 无法自愈"
        );
    }

    #[test]
    fn simplex_drops_ordinary_text_from_unknown_contact() {
        assert!(!should_forward_simplex_msg(false, Some("/menu")));
        assert!(!should_forward_simplex_msg(false, Some("hello")));
        assert!(!should_forward_simplex_msg(false, None));
    }

    #[test]
    fn simplex_forwards_everything_from_admin() {
        assert!(should_forward_simplex_msg(true, Some("/menu")));
        assert!(should_forward_simplex_msg(true, Some("hello")));
        assert!(should_forward_simplex_msg(true, None));
    }

    #[test]
    fn simplex_does_not_treat_near_miss_codes_as_login() {
        assert!(!should_forward_simplex_msg(false, Some("12345")));
        assert!(!should_forward_simplex_msg(false, Some("1234567")));
        assert!(!should_forward_simplex_msg(false, Some("12345a")));
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex_forwards
```
Expected: **编译失败**（`should_forward_simplex_msg` 不存在）即 RED

- [ ] **Step 3: 实现**

在 `rust/aegis/src/main/runtime.rs` 里 `contact_connected_log_line` 附近新增私有函数：

```rust
/// 是否把这条 SimpleX 入站消息交给 dispatch。
///
/// 管理员的消息全放行；非管理员**只有 6 位纯数字码**放行 —— 那是登录尝试，
/// 也是新 contactId 证明自己的唯一途径（自愈的前提）。
/// 其余非管理员消息按原样丢弃并记日志。
///
/// 与 `shared::dispatch::is_totp_code` 保持同样的判据（6 位 ASCII 数字）。
/// 此处无法复用那个私有函数（跨 crate 边界：runtime.rs 在 bin，dispatch 在 lib），
/// 因此判据写在这里；两处不一致会让码在门口被丢，是本模块最该盯的回归点。
fn should_forward_simplex_msg(is_admin: bool, text: Option<&str>) -> bool {
    if is_admin {
        return true;
    }
    text.is_some_and(|t| t.len() == 6 && t.chars().all(|c| c.is_ascii_digit()))
}
```

把 `for msg in mapped` 循环里那段（当前 316-323 行）改为：

```rust
                for msg in mapped {
                    let is_admin = simplex_admin == Some(msg.contact_id);
                    if !should_forward_simplex_msg(is_admin, msg.text.as_deref()) {
                        log::warn!(
                            "SimpleX 未授权联系人 contactId={} 尝试发消息，已忽略",
                            msg.contact_id
                        );
                        continue;
                    }
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test simplex_
```
Expected: 全绿（含 Task 1 的 simplex 测试与 contact_connected 测试）

- [ ] **Step 5: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **942 passed, 1 skipped**（938 + 4）

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/main/runtime.rs
git commit -m "feat(simplex): 事件循环放行非管理员的 6 位码（自愈入口）"
```

---

### Task 5: TOTP 成功后重钉管理员

**Files:**
- Modify: `rust/aegis/src/app/auth.rs`（`process_auth_code` 开头的 `is_admin_user` 拒绝，20-31 行；以及成功分支）

**Interfaces:**
- Consumes: Task 1 的 `AppState::simplex_repin_enabled` / `set_simplex_admin_id`；
  既有 `crate::bootstrap::{config_dir, set_simplex_admin_id}`
- Produces: 行为 —— 纯 `--simplex` 部署下，任意身份的 6 位码验证成功即把 `simplex_admin_id` 重钉为该身份
  （落盘 + 内存态）；未开启重钉时行为与今天完全一致

**背景**：`process_auth_code` 第一段就是「非管理员 → 回 `auth.no_permission` 并返回」。
自愈需要它在**未验证之前**不能拒绝（身份尚未成立），改为先验码、成功后再决定权限语义。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/app/auth.rs` 底部新增测试模块（该文件当前没有测试模块）。

**设计要点**：不造 `config.enc` 夹具。`set_simplex_admin_id` 的落盘逻辑**已由 `bootstrap.rs`
自己的测试覆盖**（含「新值必须被写入」与 0600 权限断言），在此重测只会脆且重复。
本任务要验的是 **auth 层的行为**：验证成功后内存态是否重钉、失败时是否不丢。
因此把 `AEGIS_CONFIG_DIR` 指向一个**空的**临时目录 —— `set_simplex_admin_id` 会因读不到
`config.enc` 而失败，正好覆盖「落盘失败但本次运行已可用」这条分支。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use aegis::core::totp::TotpManager;
    use secrecy::SecretString;
    use std::sync::Arc;

    struct NoopExecutor;
    #[async_trait::async_trait]
    impl SelfDestructExecutor for NoopExecutor {
        async fn execute(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct RecordingAdapter;
    #[async_trait::async_trait]
    impl BotAdapter for RecordingAdapter {
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn download_file(&self, _file_id: &str) -> anyhow::Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn parse_chat_id(&self, _target: &TargetId) -> Option<i64> {
            None
        }
    }

    /// 空 config 目录：让 `set_simplex_admin_id` 读不到 config.enc 而失败，
    /// 从而验证「落盘失败不丢内存态」。
    fn state_with_repin(repin: bool) -> (AppState, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        // 安全性：测试进程内串行修改环境变量。本模块测试均在同一进程且不并发读写
        // AEGIS_CONFIG_DIR（nextest 默认每个测试独立进程）。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", dir.path()) };
        let state = AppState::new(
            None,
            Some(3),
            Some(
                TotpManager::new(&SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(RecordingAdapter),
        );
        let state = if repin { state.with_simplex_repin() } else { state };
        (state, dir)
    }

    async fn run_code(state: &AppState, user_id: i64, code: &str) -> bool {
        process_auth_code(
            &RecordingAdapter,
            &TargetId(user_id.to_string()),
            user_id,
            code,
            state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap_or(false)
    }

    #[tokio::test]
    async fn successful_code_repins_simplex_admin_when_enabled() {
        let (state, _dir) = state_with_repin(true);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        assert!(run_code(&state, 7, &code).await);
        assert_eq!(
            state.simplex_admin_id(),
            Some(7),
            "验证成功后必须把管理员重钉到本身份；即使落盘失败也要本次可用"
        );
        assert!(state.is_admin_user(7));
    }

    #[tokio::test]
    async fn successful_code_does_not_repin_when_disabled() {
        let (state, _dir) = state_with_repin(false);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        assert!(run_code(&state, 7, &code).await);
        assert_eq!(
            state.simplex_admin_id(),
            Some(3),
            "未开启重钉时（tg-simplex）绝不能被改动"
        );
    }

    #[tokio::test]
    async fn wrong_code_does_not_repin() {
        let (state, _dir) = state_with_repin(true);
        assert!(!run_code(&state, 7, "000000").await);
        assert_eq!(state.simplex_admin_id(), Some(3), "错码不得改动管理员");
    }
}
```

（`state.generate_current_totp()` 已存在于 `state.rs:144`；`tempfile` 与 `async-trait` 已在
`[dependencies]`。**禁止为测试新增依赖。**）

- [ ] **Step 2: 运行测试确认失败**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test repin
```
Expected: `successful_code_repins_simplex_admin_when_enabled` FAIL（当前非管理员在验码前就被拒，
`process_auth_code` 返回 false 且不会重钉）

- [ ] **Step 3: 实现**

在 `rust/aegis/src/app/auth.rs`：**删除**开头的 `is_admin_user` 拒绝块（当前 20-31 行，含那句
`t!("auth.no_permission")` 发送与 `return Ok(false);`）。

然后在 `if state.verify_totp(code) {` 成功分支内、`let timeout = state.record_auth_success(...)`
**之后**插入重钉：

```rust
        // 自愈：TOTP 是稳定凭据，contactId 是易变标识（删联系人重连/换设备即变）。
        // 验证成功即把管理员重钉到**当前身份**，使重连不再需要读 journal 抄 ID。
        //
        // 仅在纯 --simplex 下由 `with_simplex_repin()` 开启；`--tg-simplex` 下
        // user_id 命名空间与 Telegram 共用，无差别重钉会把 simplex_admin_id
        // 覆写成 TG chat id（见 AppState::with_simplex_repin 的说明）。
        if state.simplex_repin_enabled() {
            let previous = state.simplex_admin_id();
            // 先更新内存态：即使落盘失败，本次运行也已可用，避免「验过码却还是没权限」。
            state.set_simplex_admin_id(user_id);
            match crate::bootstrap::set_simplex_admin_id(
                &crate::bootstrap::config_dir(),
                user_id,
            ) {
                Ok(()) => log::warn!(
                    "SimpleX 管理员已重钉: contactId={user_id}（原 {previous:?}），已落盘"
                ),
                Err(e) => log::error!(
                    "SimpleX 管理员已重钉为 contactId={user_id}（仅内存态）；落盘失败: {e}"
                ),
            }
        }
```

**同时**：删掉开头拒绝块之后，非管理员第二次发码时会走到 `record_auth_failure`；
这一条路径今天对非管理员是「回一句无权限」，改成「计入失败」后**行为变化**：
非管理员发普通消息不走这里（`check_auth` 仍挡），只有发码才走 —— 符合设计。

- [ ] **Step 4: 运行测试确认通过**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test repin
```
Expected: 3 passed

- [ ] **Step 5: 跑完整 Rust 质量门**

同 Task 1 Step 5。Expected: nextest **945 passed, 1 skipped**（942 + 3）

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/app/auth.rs
git commit -m "feat(aegis): TOTP 验证成功后重钉 SimpleX 管理员（仅纯 simplex 开启时）"
```

---

### Task 6: 接线 + 全量门禁

**Files:**
- Modify: `rust/aegis/src/main.rs`（`AppState::new` 调用点 124-136）

**Interfaces:**
- Consumes: Task 1 的 `with_simplex_repin`；既有 `selection.simplex` / `selection.telegram`
- Produces: 生产路径启用自愈（仅纯 simplex）

- [ ] **Step 1: 实现接线**

把 `rust/aegis/src/main.rs` 里的：

```rust
    let state = Arc::new(AppState::new(
        app_config.decrypted.admin_id,
        app_config.decrypted.simplex_admin_id,
        app_config.totp_manager,
        production_executor(),
        app_config
            .decrypted
            .encrypted_config
            .self_destruct_key_hash
            .clone(),
        app_config.bot_settings.session_timeout_secs,
        adapter,
    ));
```

改为：

```rust
    let state = AppState::new(
        app_config.decrypted.admin_id,
        app_config.decrypted.simplex_admin_id,
        app_config.totp_manager,
        production_executor(),
        app_config
            .decrypted
            .encrypted_config
            .self_destruct_key_hash
            .clone(),
        app_config.bot_settings.session_timeout_secs,
        adapter,
    );
    // 自愈只在纯 --simplex 下开启：`is_admin_user` 的 user_id 命名空间与 Telegram 共用，
    // `--tg-simplex` 下无差别重钉会把 simplex_admin_id 覆写成 Telegram 的 chat id，
    // 破坏「敏感内容落点」这个发送目标。tg-simplex 的重钉留待 P2（那时才引入平台来源）。
    let state = if selection.simplex && !selection.telegram {
        state.with_simplex_repin()
    } else {
        state
    };
    let state = Arc::new(state);
```

- [ ] **Step 2: 确认编译与既有测试**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test
```
Expected: **945 passed, 1 skipped**（无新增测试，只接线）

- [ ] **Step 3: 全量门禁**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected/rust/aegis
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings \
  && cargo nextest run --cargo-profile fast-test && cargo test --doc
```
Expected: 全绿；clippy exit 0 且零 `clippy::` 诊断

- [ ] **Step 4: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-contact-connected
git add rust/aegis/src/main.rs
git commit -m "feat(aegis): 纯 simplex 部署启用管理员自愈"
```

---

## 真实主机验收（控制器执行，不派子代理）

部署本分支构建到测试机后：

```bash
# 1) 自愈：删联系人重连 → 发 6 位码 → 无任何手工步骤
journalctl -u wwps-aegis | grep '管理员已重钉'      # 必须出现
/etc/wwps/aegis/aegis --set-simplex-admin <旧ID>   # 不需要再执行这类命令

# 2) 限流：连发 6 次错码
journalctl -u wwps-aegis | tail -20                 # 应出现被拒/等待提示

# 3) 重启清零（自 DoS 的逃生通道）
systemctl restart wwps-aegis                        # 之后应立即恢复可试
```

## 已知边界（写进 PR，不得隐去）

1. **自愈只覆盖纯 `--simplex`**。`--tg-simplex` 仍须手工 `--set-simplex-admin`（控制面在 TG 不受影响，
   受影响的是「敏感内容落点」）。原因：`user_id` 命名空间跨平台共用，逐消息区分平台要动 13 个调用点。
2. **门禁① 的 6 位码判据与 `dispatch::is_totp_code` 是两份实现**（bin/lib 边界无法复用）。
   两处不一致会让码在门口被丢 —— 这是本计划最该盯的回归点，已写在代码注释里。
3. **TOTP 成为凭据**：泄露即失守，且它安装时被打印到终端。
4. 重钉是**覆盖式**：同时存在多个旧客户端时，只有最后验证成功的那个是管理员。
