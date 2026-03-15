# rust/tgbot 容易堵塞点分析

本文档梳理 tgbot 中可能导致阻塞、卡顿或死锁的代码点，并给出改进建议。

---

## 1. 高优先级：在 async 上下文中使用阻塞锁（std::sync::RwLock）

### 位置

- **`src/logic/sni_selector.rs`**  
  - `SNI_CACHE` 使用 `std::sync::RwLock<HashMap<...>>`  
  - `get_for_country()` 内会调用 `SNI_CACHE.read().unwrap()` 和 `SNI_CACHE.write().unwrap()`  

### 调用路径

- `logic/config.rs` 中 `batch_create_reality_enhanced()` / `batch_create_xhttp_reality_enhanced()` 在 **async** 流程里调用  
  `SNISelector::get_for_country()`（约 227、297 行）。

### 问题

- `std::sync::RwLock` 的 `read()`/`write()` 会**阻塞当前线程**。  
- 在 tokio 的 async 任务里调用，会占住该 worker 线程，导致同一线程上其它任务无法推进，表现为“卡住”或整体延迟升高。  
- 若持锁期间发生 cache miss 并做嵌入资源解析（`load_embedded`），持锁时间更长，堵塞更明显。

### 建议

- **方案 A（推荐）**：把 `SNI_CACHE` 改为 `tokio::sync::RwLock`，在 `get_for_country` 中 `read().await` / `write().await`，使等待锁时让出 executor，不阻塞线程。  
- **方案 B**：在调用 `SNISelector::get_for_country()` 的外层用 `tokio::task::spawn_blocking()` 包一层，把“可能阻塞”的整段逻辑丢到专用线程池，避免阻塞 async worker。  
- 若保留 `std::sync::RwLock`，至少应保证持锁期间只做内存操作，且尽量短；当前在持写锁时还会做 `load_embedded` 和 clone，建议先在不持锁的情况下计算好 `domains`，再短暂持锁只做 `insert`。

---

## 2. 中优先级：全局 Mutex 与长时间持锁

### 2.1 Reality 安装进度状态 `PROGRESS_STATE`

- **位置**：`src/logic/installer.rs`  
  - `static PROGRESS_STATE: Lazy<Mutex<ProgressState>>`  
  - `execute_reality_install()`、`update_progress()` 等会 `.lock().await`。

- **现状**：  
  - 持锁区间已尽量缩短：更新完 `state.step/description` 后立刻 `drop(state)`，再在锁外 `edit_message_text(...).await`，这点是好的。  
  - 唯一需要注意的是：若同一进程内多处并发调用 `execute_reality_install()`，会串行在 `PROGRESS_STATE` 上（设计上 `state.running` 也会把后续请求直接返回 `InProgress`），一般不会死锁，但会排队。

- **建议**：  
  - 保持“锁内只改状态、锁外再 await”的写法；  
  - 若将来在持锁期间增加任何 `.await` 或重入，必须避免形成“锁 A → await → 再拿锁 B”的交叉依赖。

### 2.2 调度器全局 `SCHEDULER`

- **位置**：`src/logic/scheduler/mod.rs`  
  - `pub static SCHEDULER: Lazy<Arc<Mutex<Option<Arc<SchedulerManager>>>>>`  
  - `get_manager()`：短暂持锁并 clone `Option<Arc<SchedulerManager>>` 后返回，锁立即释放，设计合理。  
  - `start_scheduler()`：持锁期间只做 `*manager_guard = Some(manager)`，无 await，可接受。

- **main.rs 中的用法**（如 2125–2150 行）：  
  - 先 `let manager = get_manager().await`，再 `manager.state.lock().await`，没有“先拿 SCHEDULER 再拿 state”的长时间持锁，顺序一致，死锁风险低。

- **建议**：  
  - 继续保持“通过 `get_manager().await` 拿到 `Arc<SchedulerManager>` 后立刻释放 SCHEDULER 锁，再对 `manager.state` 加锁”的模式；  
  - 避免在任意路径上出现“先持 `state.lock()` 再调会获取 `SCHEDULER.lock()` 的逻辑”。

### 2.3 UFW 串行化 `UFW_MUTEX`

- **位置**：`src/logic/ufw.rs`  
  - `static UFW_MUTEX: Lazy<Mutex<()>>`，所有 ufw 操作前 `let _lock = UFW_MUTEX.lock().await`。

- **问题**：  
  - 持锁期间会 `run_cmd_output("ufw", args, Duration::from_secs(10)).await`，即**长时间持有一个全局锁并 await**。  
  - 所有其它需要 ufw 的请求都会排队等待，若 `ufw` 命令本身卡住或很慢，会拖死整条链路。

- **建议**：  
  - 若 ufw 命令本身已有 10s 超时，风险可控，但并发能力差；  
  - 可考虑缩小“锁只保护 ufw 命令的启动与结果”的粒度，或把“执行 ufw”放到 `spawn_blocking` 里，锁只用于保证同一时刻只有一个 ufw 在跑，且锁的持有时间尽量不包含长时间 await（例如用 channel 把“执行”和“加锁”拆开）。

---

## 3. 中优先级：AppState 多 Mutex 与锁顺序

- **位置**：`src/app/state.rs`  
  - 多个独立 `Mutex`：`sessions`、`failed_attempts`、`pending_destructs`、`self_destruct_key_hash`、`pending_warp_inputs`、`pending_schedule_inputs`、`session_timeout_secs`。

- **现状**：  
  - 各方法通常**只持有一个锁**，且没有在持有一个锁的情况下再去 await 获取另一个锁，未发现明显的 AB-BA 死锁顺序。  
  - `confirm_second_destruct_totp` 先 `destruct_snapshot(chat_id).await`（内部只拿 `pending_destructs`），再 `with_destruct(...)`（再拿 `pending_destructs`），属于同一把锁的重入/顺序获取，不会和别的锁交叉，安全。

- **建议**：  
  - 为新逻辑约定“锁顺序”（例如永远先 `sessions` 再 `pending_destructs` 等），并避免在持任何锁时 await 可能再去拿其它锁的调用。  
  - 保持“锁内只做快速内存操作”，不要在有锁的闭包里做网络或长时间计算。

---

## 4. 低优先级：channel 与无界队列

- **位置**：`src/main.rs` 约 2009 行  
  - `tokio::sync::mpsc::unbounded_channel::<String>()` 用于防火墙加固进度条。

- **现状**：  
  - 生产者：`harden_firewall` 的回调；消费者：`update_task` 里 `rx.recv().await`，且有 45s 总超时和 `drop(tx)` 关闭，不会无限等待。  
  - 无界 channel 若生产者过快、消费者过慢，内存会涨，但在当前“单次加固 + 进度文本”场景下风险有限。

- **建议**：  
  - 若将来复用该模式处理大量事件，可考虑有界 channel 或背压，避免无界堆积。

---

## 5. 已做好的部分（不易堵塞）

- **cmd_async**：  
  - `run_cmd_output` / `run_cmd_status` / `run_cmd_checked` 使用 `tokio::process::Command` + `timeout(...).await`，不会阻塞 executor，且有超时。

- **installer**：  
  - `run_command` 使用 `tokio::process::Command`，`check_installed` 同理，均为 async，无阻塞。

- **upgrade**：  
  - 下载/自替换等使用 `spawn_blocking` 或 async IO + timeout，符合“重活放 blocking 或带超时的 async”的做法。

- **system**：  
  - `get_status_report` 里 sysinfo 的 CPU 刷新放在 `spawn_blocking` 中，避免阻塞 async 运行时。

- **maintenance**：  
  - 自毁流程里 `std::process::Command::new("rm").spawn()` 仅 spawn 不 wait，不阻塞；脚本执行用 `tokio::process::Command`。

---

## 6. 小结表

| 位置 | 类型 | 风险 | 建议 |
|------|------|------|------|
| `sni_selector.rs` 的 `SNI_CACHE` | 在 async 中用 std RwLock | 高：阻塞 worker 线程 | 改为 tokio RwLock 或 spawn_blocking 包裹 |
| `ufw.rs` 的 `UFW_MUTEX` | 持锁期间长时间 await | 中：串行 + 长时间持锁 | 缩短持锁时间或拆成“锁 + spawn_blocking” |
| `installer.rs` 的 `PROGRESS_STATE` | 全局 Mutex | 低：已注意锁外 await | 保持现状，避免锁内 await |
| `scheduler` 的 `SCHEDULER` / `state` | 全局 + 每 manager 的 Mutex | 低：取 manager 后即放锁 | 保持“get_manager 再 state.lock”顺序 |
| `app/state.rs` 多 Mutex | 多锁 | 低：单锁使用、无交叉 | 约定锁顺序，锁内不 await 其它锁 |
| main 无界 channel | 无界 mpsc | 低 | 高吞吐时考虑有界或背压 |

优先处理 **sni_selector 的 std::sync::RwLock**，其次可优化 **ufw 的持锁时间**，其余以“保持当前良好实践 + 代码审查时注意锁与 await”即可。
