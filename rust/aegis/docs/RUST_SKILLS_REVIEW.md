# Aegis Rust Skills 代码审查报告

> 基于 [rust-skills](https://github.com/obra/rust-skills) 的 14 个类别、179 条规则对 `rust/aegis` 进行系统性分析。
> 审查日期: 2026-06-30 | Aegis v3.1.6

**状态标记：**
- ✅ 通过 — 代码符合规则
- ❌ 违反 — 代码违反了规则，需要改进
- ➖ 不适用 — 该规则在当前代码中无应用场景
- ⚠️ 待改进 — 部分符合但可优化

---

## 汇总

| # | 类别 | 优先级 | 规则数 | ✅ 通过 | ❌ 违反 | ➖ 不适用 | ⚠️ 待改进 |
|---|------|--------|--------|---------|---------|----------|-----------|
| 1 | Ownership & Borrowing | CRITICAL | 12 | 9 | 0 | 1 | 2 |
| 2 | Error Handling | CRITICAL | 12 | 8 | 3 | 0 | 1 |
| 3 | Memory Optimization | CRITICAL | 15 | 4 | 0 | 6 | 5 |
| 4 | API Design | HIGH | 15 | 3 | 4 | 4 | 4 |
| 5 | Async/Await | HIGH | 15 | 6 | 3 | 3 | 3 |
| 6 | Compiler Optimization | HIGH | 12 | 3 | 1 | 6 | 2 |
| 7 | Naming Conventions | MEDIUM | 16 | 10 | 0 | 4 | 2 |
| 8 | Type Safety | MEDIUM | 10 | 5 | 1 | 3 | 1 |
| 9 | Testing | MEDIUM | 13 | 8 | 2 | 2 | 1 |
| 10 | Documentation | MEDIUM | 11 | 0 | 9 | 1 | 1 |
| 11 | Performance Patterns | MEDIUM | 11 | 5 | 0 | 4 | 2 |
| 12 | Project Structure | LOW | 11 | 6 | 2 | 3 | 0 |
| 13 | Clippy & Linting | LOW | 11 | 0 | 9 | 1 | 1 |
| 14 | Anti-patterns | REFERENCE | 15 | 10 | 2 | 1 | 2 |
| | **合计** | | **179** | **77** | **36** | **39** | **27** |

---

## 1. Ownership & Borrowing（CRITICAL）

### own-borrow-over-clone 克隆优先于借用
- **评估**: ⚠️ 待改进
- **证据**: `src/adapters/telegram/handlers/ops/upgrade.rs:14`, `src/main/runtime.rs:65-75`, `src/bootstrap.rs:249-253`
- **说明**: 代码库中使用 `.clone()` 超过 100 处。大部分是合理的（`Arc` 克隆、异步任务捕获），但 `bootstrap.rs:249-253` 对 `Option` 值同时使用 `.clone().unwrap()`，可改用 `take()` 或引用。部分 handler 中（如 `upgrade.rs:14`）克隆 `Arc<dyn BotAdapter>` 是必要的。整体模式可接受但需审查克隆热点。

### own-slice-over-vec 切片优于 Vec/String 引用
- **评估**: ✅ 通过
- **证据**: `src/core/utils.rs:42-56`, `src/adapters/common/trait.rs:37-44`
- **说明**: 函数签名广泛使用 `&str`（如 `parse_ip_version(s: &str)`, `generate_timestamp_filename(prefix: &str, extension: &str)`）。`BotAdapter` trait 方法用 `&TargetId`、`&MessageId`。未发现 `&String` 或 `&Vec<T>` 参数。构造函数取 `Vec<T>` 是合理的所有权转移模式。

### own-cow-conditional 条件性所有权使用 Cow
- **评估**: ✅ 通过
- **证据**: `src/adapters/telegram/handlers/xray/batch.rs:121,289,936,1018`
- **说明**: 在 batch 处理逻辑中正确使用 `std::borrow::Cow<'_, str>` 避免条件性字符串分配，根据 IP 版本返回字面量或拼接结果而不克隆。

### own-arc-shared 线程安全共享使用 Arc
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:2`, `src/core/security/self_destruct.rs:3`
- **说明**: 20 个文件使用 `Arc`。`AppState` 使用 `Arc<dyn BotAdapter>` 和 `Arc<dyn SelfDestructExecutor>`。所有跨任务共享都使用 `Arc`，符合异步多线程环境要求。

### own-rc-single-thread 单线程使用 Rc
- **评估**: ✅ 通过
- **证据**: 全库搜索无 `use std::rc::Rc`
- **说明**: 未使用 `Rc`。代码基于 tokio 异步运行时，所有引用计数共享都使用 `Arc`，正确。

### own-refcell-interior 内部可变性使用 RefCell
- **评估**: ✅ 通过
- **证据**: 全库搜索无 `use std::cell::RefCell`
- **说明**: 未使用 `RefCell`。所有内部可变性都使用 `Mutex`，在异步上下文中这是正确的选择。

### own-mutex-interior 内部可变性使用 Mutex
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:5`, `src/core/xray/installer.rs:11`
- **说明**: 4 个文件全部使用 `tokio::sync::Mutex`。没有使用 `std::sync::Mutex`（会阻塞异步任务）。`AppState` 使用多个 `Mutex` 分别保护不同字段，减小锁粒度。

### own-rwlock-readers RwLock 用于读多写少
- **评估**: ➖ 不适用
- **证据**: 全库搜索无 `RwLock`
- **说明**: 未使用 `RwLock`。`AppState` 字段读多写少的场景（如 `Lang`、`session_timeout_secs`）可用 `tokio::sync::RwLock` 优化读并发，但当前 `Mutex` 在低竞争下性能可接受。

### own-copy-small 小类型实现 Copy
- **评估**: ✅ 通过
- **证据**: `src/core/types.rs:26`, `src/app/state.rs:15-37`, `src/adapters/common/trait.rs:27`
- **说明**: 15 个类型正确实现 `Copy`，均为小枚举或包装类型：`IpVersion`、`Platform`、`DestructStep`、`ScheduleFrequency`、`AuthFailureOutcome`、`TimeoutStatus`、`Lang`、`CpuArch` 等。均为零开销复制。

### own-clone-explicit 显式克隆
- **评估**: ✅ 通过
- **证据**: 全库 `.clone()` 调用
- **说明**: Rust 语言特性保证克隆始终是显式的。代码库中所有 `.clone()` 调用均明确可见，无误触风险。

### own-move-large 移动大数据而非克隆
- **评估**: ⚠️ 待改进
- **证据**: `src/app/state.rs:278`, `src/app/state.rs:423-428`
- **说明**: `destruct_snapshot()` 和 `schedule_input_snapshot()` 使用 `.cloned()` 返回整个 `DestructState`/`ScheduleInputState`，包含 `String` 字段。可考虑返回引用（需生命周期调整）或使用 `Arc` 包装内部数据。`BatchCreationResult` 包含 `Vec<String>` 但只用于构造，影响有限。

### own-lifetime-elision 生命周期省略
- **评估**: ✅ 通过
- **证据**: `src/core/utils.rs:42-56`, `src/core/security/crypto.rs:51-69`
- **说明**: 函数签名正确使用生命周期省略。`generate_timestamp_filename` 返回 `String`（所有权转移），`parse_ip_version` 返回 `Option<IpVersion>`（Copy 类型），无需显式生命周期。`SecurityManager` 方法使用 `&self` + `&[u8]`，省略规则自动推断。

---

## 2. Error Handling（CRITICAL）

### err-thiserror-lib 错误类型使用 thiserror
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:3`
- **说明**: `AppError` 使用 `#[derive(Error, Debug)]` 派生宏，正确应用了 thiserror 库。所有变体均有 `#[error("...")]` 属性定义用户友好消息。

### err-anyhow-app 应用层使用 anyhow
- **评估**: ✅ 通过
- **证据**: `src/core/security/crypto.rs:5`, `src/core/cmd_async.rs:1`, `src/app/auth.rs:5`
- **说明**: 48 个生产文件使用 `anyhow::Result`（包括 `use anyhow::{Context, Result}` 或 `use anyhow::Result`）。应用层统一使用 anyhow，库层定义 `AppError`，模式符合 Rust 实践。

### err-result-over-panic Result 优于 Panic
- **评估**: ❌ 违反
- **证据**: `src/bootstrap.rs:249-253`, `src/core/security/firewalld.rs:95,138,210,258,312`, `src/core/sni/selector.rs:146`, `src/adapters/telegram/handlers/xray/batch.rs:496,629,634`
- **说明**: 发现多处生产环境非测试代码中的 `.unwrap()` 调用。最严重的是 `bootstrap.rs:249-253` 中 5 处 `Option.unwrap()`（matrix 配置项），直接 panic。`firewalld.rs` 中多处 zbus 调用使用 `.unwrap()` 而非错误传播。`sni/selector.rs:146` 中 `pop().unwrap()` 在空向量上会 panic。

### err-context-chain 上下文链式追加
- **评估**: ✅ 通过
- **证据**: `src/main/config.rs:37-56`, `src/core/system/operations.rs:169-241`, `src/core/security/ufw.rs:65-100`
- **说明**: 广泛使用 `.context()`（100+ 处）和 `.with_context()`（27 处）添加中文错误上下文，如 `fs::read(&config_path).context("Config file miss")?`。部分 `with_context` 使用懒初始化闭包。错误消息清晰可读。

### err-no-unwrap-prod 生产代码无 unwrap
- **评估**: ❌ 违反
- **证据**: 同上 `err-result-over-panic`
- **说明**: 生产代码中存在多处 `.unwrap()`（参见规则 3）。`bootstrap.rs:249-253` 的矩阵配置应使用 `Option::ok_or_else()` + `?`。`firewalld.rs` 中的 zbus 调用应传播错误。建议使用 clippy 的 `unwrap_used` lint 静态检测。

### err-expect-bugs-only expect 仅用于编程错误
- **评估**: ✅ 通过
- **证据**: `src/core/xray/config.rs:767-771`, `src/core/xray/reality.rs:240-249`, `src/core/network/release_api.rs:9`
- **说明**: 12 处 `.expect()` 调用均有中文语义消息："应存在 extra 参数"、"应成功转换"、"valid sha256 regex"。`release_api.rs:9` 的静态 Regex 编译 `expect` 合理。测试代码中的 `expect` 均为临时目录/文件创建。

### err-question-mark 问号操作符传播错误
- **评估**: ✅ 通过
- **证据**: `src/app/auth.rs:30-48`, `src/core/security/crypto.rs:52-66`
- **说明**: 所有生产函数均使用 `?` 进行错误传播。`process_auth_code` 中每个可能失败的调用均用 `?`。`SecurityManager::encrypt/decrypt` 中的 anyhow 错误统一用 `?` 传播。

### err-from-impl 自动转换 via #[from]
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:17,20`
- **说明**: `AppError` 正确为 `std::io::Error` 和 `serde_json::Error` 添加 `#[from]` 属性。测试验证 `io_err.into()` 和 `json_err.into()` 自动转换为 `AppError`。

### err-source-chain 错误源链
- **评估**: ⚠️ 待改进
- **证据**: `src/core/error.rs:6-36`
- **说明**: `AppError` 中没有使用 `#[source]` 属性标注源错误。`Io` 和 `Json` 变体由 `#[from]` 隐式添加 `source()`，但其他包装错误（如 `Config(String)`）不保留源错误信息。可考虑为需要链式追溯的变体添加 `#[source]`。

### err-lowercase-msg 错误消息小写风格
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:7-35`, `src/main/config.rs:37`
- **说明**: 错误消息使用中文和英文混合，起始字符未大写。中文消息："配置错误: {0}"、"服务错误: {0}"等遵循 Rust 小写惯例。英文消息如 "Config file miss" 也未大写首字母。风格统一。

### err-doc-errors 文档标注 # Errors
- **评估**: ❌ 违反
- **证据**: 全库搜索无 `# Errors`
- **说明**: 没有任何函数文档包含 `# Errors` 标签说明可能的错误情况。公开函数如 `SecurityManager::new`、`BotAdapter::send_message` 等缺少错误文档。建议在公开 API 上添加 `# Errors` 文档。

### err-custom-type 自定义错误类型
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:6-36`
- **说明**: 使用 `AppError` 自定义枚举错误类型，而非 `Box<dyn Error>`。包含 9 个枚举变体覆盖所有错误类别（配置、服务、网络、IO、JSON 解析等）。类型是 `Send + Sync`，支持异步传播。附带 `pub type Result<T> = std::result::Result<T, AppError>` 类型别名。

---

## 3. Memory Optimization（CRITICAL）

### mem-with-capacity 使用 with_capacity 预分配
- **评估**: ⚠️ 待改进
- **证据**: `src/core/security/crypto.rs:63`（唯一用 `with_capacity` 处），全库 57 处 `Vec::new()` + 25 处 `String::new()`
- **说明**: `crypto.rs:63` 在 `encrypt()` 中正确使用 `Vec::with_capacity(12 + ciphertext.len())` 预分配精确容量，避免 reallocation。但 handler 层大量 `Vec::new()` / `String::new()` 在构建消息时未考虑初始容量（如 `batch.rs:152,320,1045` 的 `message_ids: Vec<String>` 和 `combined_links: String`），可添加 `with_capacity` 预估。batch handler 的按钮构造（`batch.rs:423,452,577,697`）尤为显著。

### mem-smallvec SmallVec 用于小集合
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 和全库搜索无 `smallvec` 依赖或引用
- **说明**: 代码库未引入 `smallvec` crate。多数 Vec 存储动态大小的数据（域名列表、配置行等），SmallVec 收益有限。若在 InlineKeyboardButton 构造等场景（通常 1-5 元素）引入，可减少堆分配，但非当前优先事项。

### mem-arrayvec ArrayVec 用于定界集合
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 和全库搜索无 `arrayvec` 依赖或引用
- **说明**: 代码库未使用 `arrayvec`。无固定容量的集合场景。若固定大小的配置字段（如 32 字节 key 数组 `[u8; 32]`）使用数组而非 Vec，但当前已正确使用定长数组。

### mem-box-large-variant 大枚举变体使用 Box
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:6-36`, `src/core/events.rs:29-43`, 全库搜索无 `Box<` 在枚举内
- **说明**: `AppError` 枚举（9 变体，多数含 `String`，~24-32 字节）和 `CoreEvent` 枚举（3 变体，含 `String` 字段）大小在合理范围内（均 < 128 字节）。Rust 枚举大小由最大变体决定，当前未超栈上存储阈值。无需 `Box` 化。

### mem-boxed-slice Box<[T]> 替代 Vec<T>
- **评估**: ➖ 不适用
- **证据**: `src/core/sni/selector.rs:55`（`Vec<String>`），全库无 `Box<[` 用法
- **说明**: 代码库未使用 `Box<[T]>` 模式。所有 Vec 在运行时大小动态变化（配置列表、域名选择器等），不适合固定大小的 `Box<[T]>`。SNI 域名列表创建后不再增删，但生命周期依赖于引用和状态持久化，改为 `Box<[String]>` 能节省 8 字节的容量指针，影响微乎其微。

### mem-thinvec ThinVec 使用
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 和全库搜索无 `thin_vec` 依赖
- **说明**: 代码库未使用 `ThinVec`。多数 Vec 分布在 handler 等非热点路径，ThinVec 收益有限。`thin_vec` 非主流生态 crate，引入会增加维护成本。

### mem-clone-from 使用 clone_from 避免多余分配
- **评估**: ⚠️ 待改进
- **证据**: 全库搜索无 `.clone_from()` 调用
- **说明**: 代码库完全未使用 `.clone_from()`。在多处构造新值后赋值给已有变量时（如 `state.rs:183` `self.self_destruct_key_hash.lock().await = hash` 直接设值而非克隆），模式正确。但若存在 `self.x = other.clone()` 的热路径，改用 `self.x.clone_from(&other)` 可复用已有分配。当前无显著热路径，但可审查 snapshot 返回模式（`state.rs:278`）。

### mem-reuse-collections 循环中复用集合
- **评估**: ⚠️ 待改进
- **证据**: `src/core/sni/state.rs:52`（唯一 `.clear()` 调用），全库 353 个 `for` 循环
- **说明**: `SNIState::reset()` 正确使用 `clear()` 复用 `shuffled_indices` 集合。但 handler 层每个请求都新建 Vec（如 `batch.rs` 中多次 `Vec::new()` 构造按钮和消息列表），未在循环/回调中复用。建议在热点 handler 中复用按钮集合以降低分配频率。

### mem-avoid-format 避免不必要的 format!
- **评估**: ⚠️ 待改进
- **证据**: `src/core/security/crypto.rs:40`, 全库 401 处 `format!()` 调用
- **说明**: 401 处 `format!()` 中绝大多数用于构造用户消息和回调数据（需动态插值），合理。但在 `crypto.rs:40` 中 `obfstr!("Invalid key length").to_string()` 可改为直接返回 `obfstr!("Invalid key length")` 而非调用 `to_string()`。此外 `maintenance.rs:738`、`operations.rs:306` 等处的 `String::new()` + 增量拼接可改用 `write!` 到预分配 buffer。

### mem-write-over-format 写入 buffer 时用 write! 而非 format!
- **评估**: ✅ 通过
- **证据**: `src/core/singbox/error.rs:16-21`（6 处 `write!`）
- **说明**: `SingBoxError` 的 `Display` 实现正确使用 `write!(f, ...)` 而非 `format!`，避免了中间 `String` 分配。核心库层模式正确。handler 层的 `menu.rs`、`schedule/handle.rs` 等使用 `format!` 构造消息内容是合理的（直接分配最终目标字符串），无需改为 `write!`。

### mem-arena-allocator 使用 Arena 分配器
- **评估**: ➖ 不适用
- **证据**: 全库搜索无 arena 分配器使用
- **说明**: 代码库无高频率短寿命对象的场景（典型 Arena 使用场景）。SNI 域名列表、Xray 配置等对象生命周期长且不稳定，不适合 Arena。

### mem-zero-copy 零拷贝模式
- **评估**: ✅ 通过
- **证据**: `src/core/security/crypto.rs:79-80`（切片），`src/core/sni/selector.rs:186-188`（嵌入式资源引用）
- **说明**: `crypto.rs` 中 `decrypt` 方法使用 `&encrypted_data[..12]` 和 `&encrypted_data[12..]` 切片而非拷贝。`selector.rs` 的 `load_embedded` 返回 `Vec<String>`（因 protobuf 解码需要），但 `SniAssets::get` 返回的内存是静态嵌入无需拷贝。网络层使用 reqwest，内部已支持零拷贝。无 `bytes` crate 但场景不需。

### mem-compact-string CompactString 使用
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 和全库搜索无 `compact_str` 依赖
- **说明**: 代码库未使用 CompactString。多数字符串动态长度 > 24 字节（URL、配置路径、消息内容），CompactString 收益有限。TOTP code 等 6 字符短字符串可用但数量少，优化意义不大。

### mem-smaller-integers 使用更小整数类型
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:75`（`admin_id: i64` 正确），`src/core/error.rs:26`（`PortUnavailable(u16)` 正确），`src/app/state.rs:67-68`（`hour: Option<u8>`, `minute: Option<u8>` 正确）
- **说明**: 整数类型选择合理：Telegram ID 用 `i64`（API 要求），端口用 `u16`（范围 0-65535），尝试次数用 `u32`，超时秒数用 `u64`，时间字段用 `u8`（0-23, 0-59），锁级别用 `usize`。`xray/config.rs:291` 中端口范围用 `i32` 因 `gen_range` 返回 `i32`（`rng.gen_range` 需要的类型），转换开销可忽略。无 `u128` 或 `i128` 等过度声明。

### mem-assert-type-size 断言类型大小
- **评估**: ⚠️ 待改进
- **证据**: `src/core/security/firewalld.rs:374-377`（4 处 `size_of` 断言），`src/core/security/ufw.rs:200`，`src/core/system/operations.rs:458,463`
- **说明**: 7 处 `size_of::<T>()` 断言全部在测试中且仅检查 `> 0`，这些是对 zbus proxy 类型的编译期合理性检查，有效但基础。未发现确保枚举大小（如 `size_of::<AppError>()`）或结构体对齐的优化断言。可在核心类型的关键边界处添加 `size_of` 断言以防范意外增长（如 `DestructState`、`ScheduleInputState` 等高频数据）。

---

## 4. API Design（HIGH）

### api-builder-pattern Builder 模式用于复杂构造
- **评估**: ⚠️ 待改进
- **证据**: `src/core/singbox/hysteria2.rs:15-47`, `src/core/xray/config.rs:169-254`
- **说明**: `Hysteria2Config` 提供 `new()` + `with_pin_sha256()` 链式方法（非正式 Builder）。`ConfigManager::build_reality_vless_inbound()` 接收 12+ 参数，应改为 Builder 模式提升可读性。无独立的 Builder 结构体。

### api-builder-must-use Builder 类型使用 #[must_use]
- **评估**: ❌ 违反
- **证据**: 全库搜索无 `#[must_use]`
- **说明**: 代码库中没有任何 `#[must_use]` 属性。Builder 类型缺失会导致调用者无意丢弃中间值而不报错。建议在可能创建 Builder 的位置添加该属性。

### api-newtype-safety 使用 newtype 增强类型安全
- **评估**: ⚠️ 待改进
- **证据**: `src/adapters/common/trait.rs:5,8`
- **说明**: 存在 `TargetId(pub String)` 和 `MessageId(pub String)` 两个 newtype，但字段标记为 `pub` 使得外部可直接修改内部值，削弱了类型安全保障。建议改为 `pub struct TargetId(String)` + 公开方法。

### api-typestate Typestate 模式用于编译期状态机
- **评估**: ➖ 不适用
- **证据**: 全库无 Typestate 模式
- **说明**: `DestructStep` 是运行时枚举状态机而非编译期 Typestate。当前项目为 bot 应用，Typestate 模式可能过度设计。

### api-sealed-trait 使用 Sealed Trait 防止外部实现
- **评估**: ➖ 不适用
- **证据**: 全库无 Sealed Trait 模式
- **说明**: `BotAdapter` trait 是公开 trait 但未 sealed。考虑到 adapter 的扩展性需求（Telegram/Discord/Matrix），开放实现是合理选择。

### api-extension-trait 使用 Extension Trait 扩展外部类型
- **评估**: ➖ 不适用
- **证据**: 全库无 Extension Trait 模式
- **说明**: 未发现对 `str`、`Path` 等外部类型的扩展 trait。当前代码对此模式无强烈需求。

### api-parse-dont-validate 在边界解析为验证类型
- **评估**: ⚠️ 待改进
- **证据**: `src/bootstrap.rs:302-379`
- **说明**: `ConfigValidator` 通过 `validate_token()`、`validate_admin_id()` 等方法进行运行时验证，但返回 `Result<(), String>`，未解析为带语义的类型。Token 始终是 `&str`，未封装为 `BotToken` 等类型。部分验证位于应用层而非边界层。

### api-impl-into 接受 impl Into<T> 提高灵活性
- **评估**: ✅ 通过
- **证据**: `src/adapters/telegram/handlers/ops/deploy.rs:256`, `src/core/system/core_upgrade.rs:107-109`, `src/core/system/upgrade.rs:57`
- **说明**: 多处正确使用 `impl Into<String>` 参数，允许调用者传入 `&str` 或 `String`。`ReleaseFetcher::new` 等函数签名灵活。

### api-impl-asref 接受 impl AsRef<T> 用于借用
- **评估**: ➖ 不适用
- **证据**: 全库无 `impl AsRef<T>` 参数
- **说明**: 代码偏好明确的 `&str` 引用而非 `impl AsRef<str>`。当前使用方式与 Rust 惯例一致，未发现需要 `AsRef` 的场景。

### api-must-use 对返回 Result 的函数使用 #[must_use]
- **评估**: ❌ 违反
- **证据**: 全库搜索无 `#[must_use]`
- **说明**: 所有 `Result` 返回函数均未标注 `#[must_use]`。Rust 编译器默认对 `Result` 有 `must_use` 警告（通过 `#[must_use]` 在标准库中的定义），因此编译器仍会警告未使用的结果。但自定义类型无法享受此保护。

### api-non-exhaustive 使用 #[non_exhaustive] 保证枚举向前兼容
- **评估**: ❌ 违反
- **证据**: 全库搜索无 `#[non_exhaustive]`
- **说明**: `Platform`、`Proto`、`IpVersion`、`DestructStep`、`AuthFailureOutcome` 等公开枚举均未添加 `#[non_exhaustive]`。新增变体将导致下游 match 编译错误。建议为 `Platform` 和 `DestructStep` 等公开枚举添加该属性。

### api-from-not-into 实现 From 而非 Into
- **评估**: ✅ 通过
- **证据**: `src/core/network/geoip.rs:35`
- **说明**: `impl From<IpSbLocation> for GeoIPLocation` 正确实现 `From` 而非 `Into`，利用标准库自动提供反向 `Into` 实现。`AppError` 中 `#[from]` 属性自动生成 `From` 实现。

### api-default-impl 为类型实现 Default
- **评估**: ✅ 通过
- **证据**: `src/core/types.rs:6,26`, `src/core/system/scheduler/mod.rs:20`, `src/core/network/geoip.rs:52`
- **说明**: `IpVersion`、`BatchCreationResult`、`SchedulerState`、`SchedulerValidator`、`GeoIPService` 等类型正确实现了 `Default`。但 `Hysteria2Config`、`RoutingAdapter` 未实现（因含必填参数，不适用 default）。

### api-common-traits 实现 Debug、Clone、PartialEq 等通用 Trait
- **评估**: ⚠️ 待改进
- **证据**: `src/adapters/common/trait.rs:4-25`, `src/app/state.rs:72`, `src/core/singbox/hysteria2.rs:6`
- **说明**: `Platform`、`Proto`、`IpVersion`、`ScheduleFrequency` 等小枚举正确派生了全部 trait。但 `AppState` 无任何 derive（因含 `Mutex`，`Clone` 不可行但至少应有 `Debug`）。`Hysteria2Config` 无 derive。`RoutingAdapter` 因 `Arc` 字段未实现 `Clone`、`Debug`。一致性差。

### api-serde-optional 将 serde 置于 feature flag 后
- **评估**: ❌ 违反
- **证据**: `rust/aegis/Cargo.toml:18`, 多处 `#[derive(serde::Serialize, serde::Deserialize)]`
- **说明**: serde 依赖在 `Cargo.toml` 主 `[dependencies]` 中，未置于 feature flag 后。`serde` 编译时间开销始终存在。建议 `serde = { version = "1.0", features = ["derive"], optional = true }` + cfg gate。

---

## 5. Async/Await（HIGH）

### async-tokio-runtime Tokio 运行时配置
- **评估**: ✅ 通过
- **证据**: `rust/aegis/Cargo.toml:8`
- **说明**: `tokio = { version = "1", features = ["full"] }` 启用全部 Tokio 特性（rt-multi-thread、net、io-util、sync、time、process、signal 等）。使用 `#[tokio::main]` 多线程运行时。

### async-no-lock-await 跨 await 持有锁（CRITICAL）
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:134-138`, `src/core/system/scheduler/mod.rs:139-181`
- **说明**: 全部使用 `tokio::sync::Mutex`（非 `std::sync::Mutex`），`.lock().await` 语义正确。所有锁的作用域均在单个方法内，没有跨 `.await` 持有锁的情况。`scheduler/mod.rs` 中锁持有期间无 `.await` 调用，符合最佳实践。

### async-spawn-blocking 使用 spawn_blocking 处理 CPU 密集型任务
- **评估**: ⚠️ 待改进
- **证据**: `src/core/system/scheduler/mod.rs:110`, `src/core/sni/selector.rs:48-53`
- **说明**: `SchedulerManager::new` 正确使用 `spawn_blocking` 进行文件 I/O。但 `SNISelector` 的 `load_protobuf` 在异步上下文中同步执行 protobuf 解码（CPU 密集型），未使用 `spawn_blocking`。部分 `std::fs` 调用在非关键路径上未包装。

### async-tokio-fs 使用 tokio::fs 替代 std::fs
- **评估**: ⚠️ 待改进
- **证据**: `src/core/xray/config.rs:7`, `src/core/singbox/config.rs:11`, `src/bootstrap.rs:263-276`
- **说明**: 核心功能正确使用 `tokio::fs`（read、write、create_dir_all 等）。但 `bootstrap.rs:263-276` 中 `verify_integrity` 使用 `std::fs::read` 在异步函数中阻塞调用。`scheduler/mod.rs:42` 的 `save_to_file` 使用 `std::fs`。建议统一为 `tokio::fs` + `spawn_blocking`。

### async-cancellation-token 使用 CancellationToken 实现优雅关闭
- **评估**: ❌ 违反
- **证据**: 全库无 `CancellationToken` 使用
- **说明**: 无任何优雅关闭机制。`runtime.rs:164` 使用 `std::future::pending().await` 永久挂起。`Dispatcher` 的 `enable_ctrlc_handler()` 通过外部信号处理关闭，但无内部信号传播。建议添加 `CancellationToken` 在 Matrix-only 模式中实现安全关闭。

### async-join-parallel 使用 tokio::join! 并行执行无关操作
- **评估**: ❌ 违反
- **证据**: 全库无 `join!` 使用
- **说明**: 多处可并行操作（如 `bootstrap.rs` 中同时检查多个文件存在性、`reality.rs` 中并行获取 IPv4/IPv6）被顺序执行。建议在无依赖的 async 调用中使用 `tokio::join!`。

### async-try-join 使用 tokio::try_join! 并行执行可失败操作
- **评估**: ❌ 违反
- **证据**: 全库无 `try_join!` 使用
- **说明**: 无并行可失败操作的场景。涉及多个独立网络请求的操作（如批量 SNI 探测、多地址解析）可考虑使用 `try_join!` 或 `futures::future::join_all` 优化。

### async-select-racing 使用 tokio::select! 实现竞速/超时
- **评估**: ✅ 通过
- **证据**: `src/core/cmd_async.rs:81-100`, `src/app/state.rs:133-138`, `src/core/system/scheduler/mod.rs:150`
- **说明**: `cmd_async.rs` 正确使用 `tokio::select!` 在 stdout/stderr 间竞速读取。`timeout()` 在命令执行、调度器关闭等处广泛用于超时控制。模式正确且合理。

### async-bounded-channel 有界通道提供背压
- **评估**: ⚠️ 待改进
- **证据**: `src/core/events.rs:52`, `src/adapters/telegram/handlers/ops/mod.rs:5`
- **说明**: `EventBus` 使用 `broadcast::channel(capacity)` 有界通道。但 `ops/mod.rs` 和 `deploy.rs` 中使用 `UnboundedSender`/`unbounded_channel`，在高负载下可能导致内存膨胀。建议评估是否可限界。

### async-mpsc-queue 使用 mpsc 实现工作队列
- **评估**: ✅ 通过
- **证据**: `src/adapters/telegram/handlers/ops/mod.rs:5`, `src/adapters/telegram/handlers/ops/deploy.rs:253`
- **说明**: 正确使用 `tokio::sync::mpsc::UnboundedSender` 在部署等长时间操作中传递日志行回 UI。无其他多生产者-单消费者场景。

### async-broadcast-pubsub 使用 broadcast 实现发布/订阅
- **评估**: ✅ 通过
- **证据**: `src/core/events.rs:1-63`
- **说明**: `EventBus` 结构体封装 `broadcast::Sender<CoreEvent>`，提供 `subscribe()` 和 `emit()` 方法。支持 `Severity`、`Status`、`Component` 等事件类型。设计干净，有界容量防止 OOM。

### async-watch-latest 使用 watch 获取最新值
- **评估**: ➖ 不适用
- **证据**: 全库无 `watch` 使用
- **说明**: 当前代码无"获取最新值"场景。`i18n::Lang` 等配置通过 `Mutex` 访问，如需优化读性能可考虑 `watch` 通道。

### async-oneshot-response 使用 oneshot 实现请求/响应
- **评估**: ➖ 不适用
- **证据**: 全库无 `oneshot` 使用
- **说明**: 当前代码无跨任务请求/响应模式。`tokio::spawn` 任务间通信通过共享 `AppState` 实现，无需 oneshot。

### async-joinset-structured 使用 JoinSet 管理动态任务
- **评估**: ➖ 不适用
- **证据**: 全库无 `JoinSet` 使用
- **说明**: 代码使用 `tokio::spawn` 启动后台任务（通知、调度器初始化等），任务数量固定且无需逐个取消，`JoinSet` 增加复杂度但无收益。

### async-clone-before-await 在 await 前克隆值并释放所有权
- **评估**: ✅ 通过
- **证据**: `src/core/system/scheduler/mod.rs:337-339`, `src/main/runtime.rs:65-76,121-134`
- **说明**: 所有 `tokio::spawn(async move { ... })` 均在外层正确 `clone()` `Arc` 值，闭包通过 `move` 获取所有权。`scheduler/mod.rs` 中 `build_job` 在 `Box::pin` 前克隆 adapter/target 和 task_type。模式一致且正确。

---

## 6. Compiler Optimization（HIGH）

### opt-inline-small 小热函数用 #[inline]
- **评估**: ➖ 不适用
- **证据**: 全库搜索无 `#[inline]` 注解
- **说明**: 单 crate 应用（非库），编译器在 crate 内自动决定内联策略。无非公开的小型热函数需跨 crate 内联。若将来提取为独立 crate，需为 utils 等小型函数添加 `#[inline]`。

### opt-inline-always-rare #[inline(always)] 谨慎使用
- **评估**: ✅ 通过
- **证据**: 全库搜索无 `#[inline(always)]`
- **说明**: 代码库未使用强制内联注解。无经基准测试证明的热点路径，编译器内联决策优于人工猜测。

### opt-inline-never-cold 冷路径使用 #[inline(never)]
- **评估**: ➖ 不适用
- **证据**: 全库搜索无 `#[inline(never)]`
- **说明**: 未对冷路径进行显式内联控制。代码中错误处理路径与热路径混合在同一个函数中，未提取为 `#[inline(never)]` 辅助函数。但当前无基准测试证明冷路径代码影响热路径性能。

### opt-cold-unlikely 错误/罕见路径用 #[cold]
- **评估**: ❌ 违反
- **证据**: `src/core/security/firewalld.rs:95,138,210,258,312` 多处 `.unwrap()` 内联；`src/bootstrap.rs:249-253` 内联 panic；全库无 `#[cold]` 注解
- **说明**: 错误/panic 路径完全内联在函数体中。`firewalld.rs` 中 zbus 调用错误处理与正常逻辑交织；`bootstrap.rs` 中矩阵配置项 `unwrap()` 无冷路径标记。建议将错误构造提取为 `#[cold]` 辅助函数，使热路径更紧凑、指令缓存效率更高。

### opt-likely-hint likely()/unlikely() 分支预测提示
- **评估**: ➖ 不适用
- **证据**: 全库搜索无 `likely`/`unlikely` 调用
- **说明**: `likely()/unlikely()` 为 nightly API，项目使用 stable Rust（edition 2024）。代码通过早期 return 模式隐式提示分支可能性（如 match guard、Option 早期检查），但未使用 `likely_stable` crate。

### opt-lto-release 发布构建启用 LTO
- **评估**: ⚠️ 待改进
- **证据**: `rust/aegis/Cargo.toml:86` `lto = "thin"`
- **说明**: 使用 Thin LTO 而非 Fat LTO。Thin LTO 提供约 80% 的 LTO 收益但编译更快。生产发布可考虑 `lto = "fat"` 获取最大优化（+10-20% 性能），但需权衡 CI 编译时间。`opt-level = "z"`（优化大小而非速度）与 LTO 各有所长，当前以减小二进制体积为目标。

### opt-codegen-units 单代码生成单元
- **评估**: ✅ 通过
- **证据**: `rust/aegis/Cargo.toml:83` `codegen-units = 1`
- **说明**: 正确设置为 1，允许 LLVM 在整个 crate 级别进行跨模块优化、常量传播和死代码消除。`build-override` 部分使用 `codegen-units = 256` 加速依赖编译，合理。

### opt-pgo-profile 用 PGO 优化生产构建
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 和构建脚本中无 PGO 配置
- **说明**: PGO 需要 instrument → profile → optimize 三阶段构建流程，适合性能关键型应用。当前 bot 应用无此需求。若将来有 CPU 密集型操作（如连接池管理），可考虑引入。

### opt-target-cpu 设置 target-cpu
- **评估**: ⚠️ 待改进
- **证据**: `.cargo/config.toml` 和 `Cargo.toml` 中无 `target-cpu` 设置
- **说明**: 编译为通用 x86-64 baseline（约 Sandy Bridge 级别），未启用现代 CPU 特性（AVX2、BMI 等）。建议在已知部署硬件时通过 `.cargo/config.toml` 或 `RUSTFLAGS` 设置 `target-cpu=native`，可提升自动向量化效果。

### opt-bounds-check 迭代器替代索引避免越界检查
- **评估**: ✅ 通过
- **证据**: `src/core/` 大量 `.iter()` 使用（20+ 处），`for i in 0..count` 仅出现在配置生成等非热点（`reality.rs:169`、`xhttp.rs:31`、`kcp.rs:114` 等）
- **说明**: 核心数据处理路径使用迭代器模式（`.iter().map()`、`.iter().filter()`、`.iter().find()`）。索引循环仅用于构造/生成重复配置，无需频繁越界检查。无直接 `data[i]` 索引访问热路径模式。

### opt-simd-portable 可移植 SIMD
- **评估**: ➖ 不适用
- **证据**: 全库无 SIMD 使用
- **说明**: 代码库为 bot 应用，无数据并行计算场景（无矩阵运算、图像处理、大规模数值计算）。无需引入 SIMD 复杂性。

### opt-cache-friendly 缓存友好数据布局（SoA）
- **评估**: ➖ 不适用
- **证据**: 全库无 SoA（Struct of Arrays）模式
- **说明**: 代码处理配置管理、消息路由等控制流操作，非数据密集型。所有集合使用 AoS 模式（`Vec<String>`、`Vec<Value>`、`Vec<(PathBuf, SystemTime)>`）。无高频率遍历场景，SoA 优化收益为零。

---

## 7. Naming Conventions（MEDIUM）

### name-types-camel 类型命名用 UpperCamelCase
- **评估**: ✅ 通过
- **证据**: `src/core/types.rs`、`src/core/error.rs`、`src/adapters/common/trait.rs` 等
- **说明**: 所有类型/结构体/枚举名称正确使用 UpperCamelCase：`TargetId`、`MessageId`、`Platform`、`IpVersion`、`AppError`、`EventBus`、`SecurityManager`、`DestructStep`、`FirewallBackend` 等。无编译器警告。

### name-variants-camel 枚举变体用 UpperCamelCase
- **评估**: ✅ 通过
- **证据**: `src/adapters/common/trait.rs:28-33`（`Platform` 变体），`src/app/state.rs:17-23`（`DestructStep` 变体），`src/core/types.rs:29-30`（`IpVersion` 变体）
- **说明**: 所有枚举变体使用 UpperCamelCase：`Platform::Telegram`、`Platform::Discord`、`Platform::Matrix`；`DestructStep::Idle`、`DestructStep::PreDestruct`；`TaskType::Reality`、`TaskType::XrayUpgrade` 等。`IpVersion::IPv4`/`IPv6` 的 IPv 前缀遵循常见网络约定，编译器无警告。

### name-funcs-snake 函数/方法用 snake_case
- **评估**: ✅ 通过
- **证据**: `src/core/network/geoip.rs:99`（`get_country_code`），`src/core/system/monitor.rs:12`（`get_status_report`），`src/adapters/common/trait.rs:37`（`send_message`、`delete_message`）
- **说明**: 所有函数和方法使用 `snake_case`。无 camelCase 或 PascalCase 函数名。模块名也是 `snake_case`（如 `core/system/scheduler/mod.rs`）。

### name-consts-screaming 常量用 SCREAMING_SNAKE_CASE
- **评估**: ✅ 通过
- **证据**: `src/core/paths.rs:5-35`（`WWPS_BASE_DIR`、`SINGBOX_DIR`、`AEGIS_DIR`、`XRAY_DIR`、`TLS_CERT`、`TLS_KEY` 等），`src/core/network/release_api.rs:11`（`USER_AGENT_VALUE`）
- **说明**: 所有 `const` 和 `static` 使用正确的大写下划线命名。路径常量、API 常量风格统一且符合约定。

### name-lifetime-short 短生命周期名称
- **评估**: ✅ 通过
- **证据**: `src/core/security/self_destruct.rs:9`（`BoxFuture<'static>`），`src/core/system/maintenance.rs:29-874`（`'static`），`src/core/system/operations.rs:36`（`&'static str`）
- **说明**: 使用标准短生命周期名称 `'a`、`'static`。无冗长生命周期名称。借助生命周期省略避免不必要的显式标注。

### name-type-param-single 单大写字母类型参数
- **评估**: ✅ 通过
- **证据**: `src/core/system/maintenance.rs:144,154,236,274,360` 使用 `F` 作为闭包类型参数
- **说明**: 有限使用泛型，但 `F` 作为函数类型参数遵循标准约定。无冗长或非标准类型参数名称。

### name-as-free as_ 前缀表示免费引用转换
- **评估**: ✅ 通过
- **证据**: `src/core/system/operations.rs:36` `DistroFamily::as_str()` 返回 `&'static str`
- **说明**: `as_str()` 正确用于免费引用转换。无 `as_` 前缀但实际分配的错误用法。

### name-to-expensive to_ 前缀表示昂贵转换
- **评估**: ✅ 通过
- **证据**: `src/core/singbox/tuic.rs:34,57`（`to_inbound_json`、`to_client_link`），`src/core/singbox/hysteria2.rs:49,90,133,185`（`to_inbound_json`、`to_client_link` 方法）
- **说明**: `to_` 方法正确用于分配新 `String`/`Value` 的昂贵转换。`to_inbound_json()` 分配 `serde_json::Value`，`to_client_link()` 分配 `String`。语义准确。

### name-into-ownership into_ 前缀表示所有权转移
- **评估**: ➖ 不适用
- **证据**: `src/core/` 中无 `into_` 前缀方法
- **说明**: 当前类型设计未提供所有权转移方法。多数类型使用引用或 clone 而非消耗 self 的转换。

### name-no-get-prefix 简单 getter 不含 get_ 前缀
- **评估**: ⚠️ 待改进
- **证据**: `src/core/sni/selector.rs:62,136`（`get_for_country`、`get_next`），`src/core/system/scheduler/mod.rs:33,68,245`（`get_default`、`get_tasks_summary`、`get_summary`），`src/core/system/monitor.rs:12-248`（9 个 `get_` 方法），`src/core/singbox/config.rs:224`（`get_config_count`）
- **说明**: 大量使用 `get_` 前缀（25+ 处），虽多数涉及计算/lookup（非简单字段访问），但 `get_default()`、`get_display_name()`、`get_summary()` 等可简化为 `default()`、`display_name()`、`summary()`。Rust 约定简单 getter 省略 `get_`。

### name-is-has-bool is_/has_/can_ 前缀用于布尔方法
- **评估**: ✅ 通过
- **证据**: `src/adapters/common/routing.rs:17`（`is_sensitive`），`src/app/state.rs:120,128`（`is_admin_user`、`is_authorized`）
- **说明**: 所有布尔方法正确使用 `is_` 前缀。代码阅读自然：`if user.is_admin_user()`、`if routing.is_sensitive(text)`。无不带前缀的布尔方法。

### name-iter-convention iter/iter_mut/into_iter 迭代器命名
- **评估**: ➖ 不适用
- **证据**: 全库无自定义集合类型的迭代器方法
- **说明**: 代码通过 `Vec<T>`、`HashMap` 等标准集合使用迭代器（`.iter().map()`、`.iter().filter()` 等），无需要实现 `IntoIterator` 的自定义集合。

### name-iter-method 统一迭代器方法名
- **评估**: ➖ 不适用
- **证据**: 同上
- **说明**: 无自定义迭代器方法实现。标准库提供的迭代器方法命名、语义正确。

### name-iter-type-match 迭代器类型与方法名匹配
- **评估**: ➖ 不适用
- **证据**: 同上
- **说明**: 无自定义迭代器类型。使用标准 `std::slice::Iter`、`std::vec::IntoIter` 等，类型名与上下文匹配。

### name-acronym-word 缩写视为单词：Uuid 非 UUID
- **评估**: ⚠️ 待改进
- **证据**: `src/core/types.rs:29-30` `IpVersion::IPv4`/`IPv6` 变体名
- **说明**: `IPv4` 和 `IPv6` 变体使用 `IP` 全大写而非 `Ip`。Rust 标准库使用 `Ipv4Addr`/`Ipv6Addr` 风格。其他缩写使用正确：`TargetId`、`MessageId`、`ChatId`（`Id` 而非 `ID`）、`Http`（`Http` 而非 `HTTP`）。建议将 `IPv4` → `Ipv4`、`IPv6` → `Ipv6` 以与 std 一致。

### name-crate-no-rs crate 名无 -rs 后缀
- **评估**: ✅ 通过
- **证据**: `rust/aegis/Cargo.toml:2` `name = "aegis"`
- **说明**: crate 名 `aegis`，无 `-rs` 或 `-rust` 后缀。简洁且符合 Rust 生态命名习惯。

---

## 8. Type Safety（MEDIUM）

### type-newtype-ids 使用 newtype 包装 ID 类型
- **评估**: ⚠️ 待改进
- **证据**: `src/adapters/common/trait.rs:5,8`
- **说明**: `TargetId(pub String)` 和 `MessageId(pub String)` 两个 newtype 已存在，但字段标记为 `pub` 削弱了封装性。此外缺少 `AdminId`、`UserId`、`TotpCode`、`BotToken` 等业务 ID 的 newtype 包装，ID 在参数间以裸 `&str` / `String` 传递。

### type-newtype-validated 使用 newtype 封装验证后数据
- **评估**: ❌ 违反
- **证据**: `src/bootstrap.rs:302-379`, 全库无验证后 newtype
- **说明**: 配置验证器 (`ConfigValidator`) 通过 `validate_token()`、`validate_admin_id()` 等方法进行运行时验证，但返回 `Result<(), String>`，未解析为带语义的类型。BotToken、URL、配置路径等经过验证的数据仍以 `&str` / `String` 传递，丢失类型安全。

### type-enum-states 枚举表示互斥状态
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:17-42`, `src/adapters/common/trait.rs:28`, `src/core/xray/config.rs:16`, `src/core/events.rs:4-26`
- **说明**: 30 个枚举类型正确表示互斥状态和选项：`DestructStep`（5 步清除流程）、`ScheduleFrequency`（Daily/Weekly）、`AuthFailureOutcome`（含 attempts 数据）、`TimeoutStatus`、`Platform`、`Proto`、`IpVersion`、`WarpMode`、`Severity`、`Component`、`Status` 等。嵌套数据使用结构体变体，无布尔参数伪装状态。

### type-option-nullable 使用 Option 表示可空值
- **评估**: ✅ 通过
- **证据**: `src/core/types.rs:9-10`, `src/adapters/common/trait.rs:13`
- **说明**: `MessageContent.markup: Option<Markup>`、`BatchCreationResult.config_file: Option<String>` 等正确使用 `Option`。未使用哨兵值（-1、空字符串、`null`）表示缺失。

### type-result-fallible 使用 Result 表示可能失败的操作
- **评估**: ✅ 通过
- **证据**: `src/core/error.rs:38`, `src/adapters/common/trait.rs:1`
- **说明**: 统一使用 `anyhow::Result`（48 文件 + 应用层）和 `AppError::Result`（核心层）。BotAdapter 所有方法返回 `anyhow::Result`，符合语义。

### type-phantom-marker 使用 PhantomData 实现类型级标记
- **评估**: ➖ 不适用
- **证据**: 全库无 `PhantomData`
- **说明**: 项目中无需要类型级标记参数的模式。`Fail2BanManager;` 等单元结构体不携带类型参数，无 PhantomData 需求。

### type-never-diverge 使用 `!` 类型表示永不返回函数
- **评估**: ➖ 不适用
- **证据**: 全库无 `-> !`
- **说明**: Bot 应用无自定义永不返回函数。`runtime.rs:164` 使用 `std::future::pending().await` 实现挂起，无需 `!` 类型。

### type-generic-bounds 约束仅出现在需要处
- **评估**: ✅ 通过
- **证据**: `src/adapters/common/trait.rs:34`, `src/adapters/telegram/handlers/ops/deploy.rs:256`
- **说明**: `BotAdapter` 仅约束 `Send + Sync`，未过度约束。`impl Into<String>` 在 deploy 和 release_fetcher 中正确使用。`ScheduledTask::execute` 正确使用 `&dyn BotAdapter` 而非过度泛型化。

### type-no-stringly 避免 stringly-typed API
- **评估**: ✅ 通过
- **证据**: 全库参数类型
- **说明**: 无 `&str` 表示 type/kind/mode 参数——全部使用枚举（`TaskType`、`Platform`、`WarpMode`、`Proto` 等）。`IpVersion::label()` 返回 `&'static str` 是显示文本而非类型标识。回调数据、cron 表达式是真实的文本数据。

### type-repr-transparent 为 FFI newtype 使用 repr(transparent)
- **评估**: ➖ 不适用
- **证据**: 全库无 `#[repr(transparent)]`
- **说明**: `libc` FFI 调用（`mlock`、`munlock`、`setrlimit`、`prctl`）直接操作裸指针或整数，没有 wrapper newtype 需要 repr(transparent)。

---

## 9. Testing（MEDIUM）

### test-cfg-test-module 使用 #[cfg(test)] mod tests
- **评估**: ✅ 通过
- **证据**: 20+ 文件含 `#[cfg(test)]` 块（`types.rs`、`error.rs`、`paths.rs`、`state.rs`、`crypto.rs`、`routing.rs`、`task_types.rs`、`firewall.rs`、`ufw.rs` 等）
- **说明**: 所有内部测试模块统一使用 `#[cfg(test)] mod tests { ... }` 包裹，条件编译正确。

### test-use-super 测试模块使用 use super::*
- **评估**: ✅ 通过
- **证据**: 20+ 测试模块中的 `use super::*;`
- **说明**: 所有 `#[cfg(test)] mod` 块均在模块开头使用 `use super::*;`，可访问父模块的私有项。

### test-integration-dir 集成测试位于 tests/ 目录
- **评估**: ✅ 通过
- **证据**: `rust/aegis/tests/` 目录下 10 个集成测试文件
- **说明**: `tests/integration_security.rs`（SecurityManager 加解密）、`tests/integration_totp_trim.rs`（TOTP 空值修剪）、`tests/integration_setup_roundtrip.rs`（setup 往返）、`tests/test_self_destruct.rs`（自毁逻辑）、`tests/test_self_destruct_e2e.rs`（E2E 自毁）、5 个 CLI 测试。

### test-descriptive-names 测试函数命名描述性
- **评估**: ✅ 通过
- **证据**: `tests/test_self_destruct.rs:48-405`, `tests/integration_security.rs:10,27`
- **说明**: 测试命名详细描述测试场景：`security_manager_creates_key_and_encrypt_decrypt_roundtrip`、`test_destruct_state_step_transitions`、`test_secure_wipe_overwrites_content`、`test_timeout_resets_on_action` 等。

### test-arrange-act-assert 测试遵循 AAA 结构
- **评估**: ✅ 通过
- **证据**: `tests/integration_security.rs:10-24`, `tests/test_self_destruct.rs:48-54`
- **说明**: 测试函数自然遵循 Arrange-Act-Assert 模式：创建临时目录和对象（Arrange）、执行操作（Act）、断言结果（Assert），各阶段以空行分隔。

### test-proptest-properties 使用 proptest 进行属性测试
- **评估**: ❌ 违反
- **证据**: `Cargo.toml` 无 proptest，全库无 `proptest!`
- **说明**: 完全未使用属性测试。加密/解密往返、TOTP 生成和验证、自毁状态机等场景的属性性质未通过 proptest 验证。建议为 `SecurityManager`（加密→解密→恒等）、`TotpManager`（同 code→验证通过）等核心逻辑添加属性测试。

### test-mockall-mocking 使用 mockall 进行模拟
- **评估**: ⚠️ 待改进
- **证据**: `Cargo.toml:70`（`mockall = "0.14"`），全库无 `mock!` 使用
- **说明**: `mockall` 已声明在 `[dev-dependencies]` 但完全未被使用。测试中手写 `MockAdapter`（`state.rs:460`）和 `NoopExecutor`（`state.rs:452`）。建议将手写 mock 替换为 mockall 生成，利用 `#[automock]` 自动提供 `expect_*` 方法，减少样板代码。

### test-mock-traits 使用 trait 实现依赖模拟
- **评估**: ✅ 通过
- **证据**: `src/adapters/common/trait.rs:34`, `src/app/state.rs:452,460`
- **说明**: `BotAdapter` 和 `SelfDestructExecutor` trait 提供清晰的依赖抽象接口。测试中 `MockAdapter` 和 `NoopExecutor` 正确实现了 trait 用于替换真实依赖。

### test-fixture-raii 使用 RAII 进行测试清理
- **评估**: ✅ 通过
- **证据**: `tests/integration_security.rs:11`, `tests/test_self_destruct_e2e.rs:40-41`
- **说明**: 所有集成测试正确使用 `tempfile::tempdir()`（`tempfile = "3.8"`），临时目录在 `Drop` 时自动删除。`DestructState` 等无外部资源需要清理。

### test-tokio-async 异步测试使用 #[tokio::test]
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:501,508,529` 等 10+ 处 `#[tokio::test]`
- **说明**: 异步函数测试正确使用 `#[tokio::test]`。同步测试使用 `#[test]`。`SecurityManager`（同步）使用常规 `#[test]`，`AppState` 异步操作使用 `#[tokio::test]`。

### test-should-panic 使用 #[should_panic] 测试预期 panic
- **评估**: ➖ 不适用
- **证据**: 全库无 `#[should_panic]`
- **说明**: 代码库通过 `Result` 类型处理所有错误情况，无预期 panic 的测试场景。核心函数返回 `Result` 而非直接 panic，所以 `should_panic` 无用武之地。

### test-criterion-bench 使用 Criterion 基准测试
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 无 criterion，无 `benches/` 目录
- **说明**: 项目为 CLI bot 应用，无高性能库级别的热点代码。加密操作受 I/O 和网络延迟主导，单元基准测试意义有限。

### test-doctest-examples 将 doc 测试作为示例
- **评估**: ❌ 违反
- **证据**: 全库无 `/// \`\`\`` rust 代码块
- **说明**: 没有任何文档示例。`IpVersion`、`DestructStep`、`AppError`、`SecurityManager` 等核心类型的 doc 注释均为纯文本说明，无可运行示例。建议在核心类型和方法上添加 doc test 示例。

---

## 10. Documentation（MEDIUM）

### doc-all-public 所有公开项标注 /// 文档
- **评估**: ❌ 违反
- **证据**: 77 个 `///` doc 注释 vs 269 公开项（含 fn/struct/enum/trait/type/mod/const/use），覆盖率约 29%
- **说明**: 公开项文档覆盖率低。`IpVersion`、`DestructStep`、`AppError`、`EventBus` 等核心类型有 doc 注释，但多数方法（`SecurityManager::encrypt/decrypt`、`BotAdapter` 方法、`DestructState` 字段、`AppState` 字段、handler 函数、`RoutingAdapter` 等）缺少文档。建议优先为所有公开 API 项添加文档。

### doc-module-inner 模块级文档使用 //!
- **评估**: ⚠️ 待改进
- **证据**: `src/core/paths.rs:1-3`, `src/core/error.rs:1`, `src/core/types.rs:1`, `src/core/mod.rs:1-3`, `tests/integration_security.rs:1-3`
- **说明**: `core` 模块及其子模块有 `//!` 文档。但 `adapters/`（含 `common/`、`telegram/`、`matrix/`、`discord/`）、`app/`、`main/` 等顶层模块缺少模块级文档。集成测试文件有 `//!` 文档。

### doc-examples-section 文档包含 # Examples 部分
- **评估**: ❌ 违反
- **证据**: 全库无 `# Examples` 标签
- **说明**: 没有任何公开 API 的文档包含 `# Examples` 部分。`SecurityManager::new`、`IpVersion::label`、`EventBus::emit` 等核心方法缺少使用示例。建议从核心类型开始添加 `# Examples`。

### doc-errors-section 文档包含 # Errors 部分
- **评估**: ❌ 违反
- **证据**: 全库无 `# Errors` 标签
- **说明**: 没有任何函数文档说明可能返回的错误。`SecurityManager::new`、`BotAdapter::send_message`、`EventBus::emit`、`TotpManager::generate` 等公开方法未标注错误条件。同上第 2 节（`err-doc-errors`）已指出此问题。

### doc-panics-section 文档包含 # Panics 部分
- **评估**: ❌ 违反
- **证据**: 全库无 `# Panics` 标签
- **说明**: 没有任何函数文档标注可能的 panic 条件。`selector.rs:146` 的 `pop().unwrap()`、`bootstrap.rs:249-253` 的 `Option.unwrap()` 等含 panic 风险的代码段缺少 panic 文档。建议为可能 panic 的函数添加 `# Panics` 标注。

### doc-safety-section unsafe 函数必须标注 # Safety
- **评估**: ❌ 违反
- **证据**: 13 处 `unsafe` 块（`crypto.rs:91-124`, `core_upgrade.rs:751-795`, `bootstrap.rs:290-295`），全库无 `# Safety`
- **说明**: 生产代码中包含 13 处 `unsafe` 调用（`mlock/munlock`、`setrlimit`、`prctl`、`env::set_var/remove_var`），但没有任何 `# Safety` 文档说明前置条件。`crypto.rs` 中 `mlock` 调用应标注指针有效性和长度约束；`core_upgrade.rs` 的 `env::set_var` 应说明线程安全问题。

### doc-question-mark 示例中使用 `?` 传播错误
- **评估**: ❌ 违反
- **证据**: 无 doc 示例
- **说明**: 无任何 doc 示例，因此也无 `?` 使用。添加 doc 示例时应使用 `?` 展示错误传播（配合 `fn main() -> Result<()>` 或测试函数）。

### doc-hidden-setup 使用 `# ` 隐藏示例设置代码
- **评估**: ➖ 不适用
- **证据**: 无 doc 示例
- **说明**: 当前无 doc 示例，因此无需隐藏设置代码。添加 doc 示例时应在初始化代码前加 `# ` 以保持示例可读性。

### doc-intra-links 使用 [`Vec`] 等内部链接
- **评估**: ❌ 违反
- **证据**: 全库无 `` [` `` 或 `[` 链接语法
- **说明**: 没有任何 intra-doc 链接。`AppError` 文档应链接到 `DestructStep` 等关联类型；`EventBus` 应链接到 `CoreEvent` 和 `Severity`。推荐使用 markdown 链接语法 `` [`AppError`] ``。

### doc-link-types 文档中链接关联类型
- **评估**: ❌ 违反
- **证据**: 全库无关联类型链接
- **说明**: 同 `doc-intra-links`，无任何类型链接。`EventBus` 的文档中未链接 `CoreEvent` 和 `broadcast::Sender`；`BotAdapter` trait 的文档中未链接 `Platform`、`TargetId`、`MessageId`。

### doc-cargo-metadata Cargo.toml 包含完整元数据
- **评估**: ❌ 违反
- **证据**: `Cargo.toml:1-4`
- **说明**: `Cargo.toml` 仅含 `name`、`version`、`edition` 字段。缺少 `description`、`authors`、`license`、`repository`、`homepage`、`readme`、`keywords`、`categories` 等元数据。对于公开项目，完整元数据影响 crates.io 可发现性。

---

## 11. Performance Patterns（MEDIUM）

### perf-iter-over-index 迭代器优先于索引
- **评估**: ✅ 通过
- **证据**: 143 个 `for` 循环 + 40+ 处 `.iter()` 调用；13 处 `0..n` 索引循环（5 重试、1 防调试、1 工具函数、6 批量配置生成需要索引赋值）
- **说明**: 数据处理路径统一使用迭代器。`for i in 0..count` 模式出现在配置生成（`reality.rs:169`, `xhttp.rs:31`, `kcp.rs:114`）和批量构造（`hy2_batch.rs:45`, `tuic_batch.rs:41`）中——这些场景需要索引序号作为输出 ID。`for _ in 0..N` 的 retry 循环也正确。无 `data[i]` 越界敏感的热路径模式。

### perf-iter-lazy 保持迭代器惰性
- **评估**: ⚠️ 待改进
- **证据**: 40+ 处 `.collect()` 调用；`batch.rs:566,677,774` 中 `split(',').collect()` 后再次迭代
- **说明**: 多数 `.collect()` 位于 terminal 位置（构造最终消息/配置字符串）。但 batch handler 中（如 `batch.rs:566`）对逗号分隔的字符串 `split().collect()` 为 `Vec<&str>` 后再次迭代处理，可链式组合避免中间 Vec 分配。handler 层收集为 Vec 再通过 `format!` 拼接的模式也产生小幅中间开销。建议在非必需中间结果的链路上保持惰性。

### perf-collect-once 避免收集中间迭代器
- **评估**: ✅ 通过
- **证据**: 全库无 `.collect().iter()` 或 `collect()` 后立即再开迭代器的链式模式
- **说明**: 没有发现重复收集模式。所有 `.collect()` 均在 iterator chain 的终端位置。adapter 层正确链式组合 `.map().collect()` 后直接返回，无中间集合开销。

### perf-entry-api HashMap entry() API
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:205` `fails.entry(user_id).or_insert(FailedRecord { ... })`
- **说明**: 在 `record_auth_failure` 中正确使用 HashMap entry API 实现「插入或更新」失败记录计数。无 `contains_key() + insert()` 两步式旧模式。

### perf-drain-reuse drain() 复用分配
- **评估**: ➖ 不适用
- **证据**: 全库无 `.drain()` 调用
- **说明**: 无需要 drain 后复用的长期集合。唯一集合复用是 `SNIState::reset()` 的 `.clear()`。若将来有需要重建集合的热路径，可引入 drain 复用底层缓冲区减少分配。

### perf-extend-batch extend() 批量插入
- **评估**: ✅ 通过
- **证据**: 12 处 `.extend()`（`deploy.rs:95-182` 3 处、`firewall_scanner.rs:38-180` 5 处、`maintenance.rs:476-477` 2 处、`port_allocator.rs:88-102` 2 处）
- **说明**: 所有 extend 使用在正确场景：`deploy.rs` 中批量追加多个批次结果到 `all_links`，`firewall_scanner.rs` 和 `port_allocator.rs` 中合并扫描结果到端口集合。均为 patch/merge 更新模式，非逐个 `push()`。

### perf-chain-avoid 避免 hot loop 中 chain()
- **评估**: ➖ 不适用
- **证据**: 全库无 `.chain()` 调用
- **说明**: 未使用 `chain()`。需合并多个集合时使用 `extend()` 替代（直接追加到目标集合），避免创建双迭代器。

### perf-collect-into collect_into() 复用容器
- **评估**: ➖ 不适用
- **证据**: 全库无 `collect_into` 调用
- **说明**: `collect_into` 需要 `itertools` crate 或 nightly Rust。当前无热点路径需要复用现有容器。若将来引入需权衡依赖成本。

### perf-black-box-bench black_box() 在基准测试中
- **评估**: ➖ 不适用
- **证据**: `anti_debug.rs:22` 使用 `std::hint::black_box`（防调试用途）；无 Criterion 或 `benches/` 目录
- **说明**: `black_box` 仅出现在 anti-debug 模块中，用于阻止编译器优化掉运算循环——这是安全特性，非基准测试用途。项目未引入 Criterion，作为 I/O 密集型 bot 应用，单元基准测试收益有限。

### perf-release-profile 发布配置文件
- **评估**: ⚠️ 待改进
- **证据**: `Cargo.toml:81-86` `opt-level = "z"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`, `lto = "thin"`
- **说明**: 发布配置以体积优化为目标（`opt-level = "z"` 而非 `"3"`）。`codegen-units = 1` 和 `lto = "thin"` 提供良好的跨模块内联优化。可从两方面改进：(1) `lto = "thin"` → `"fat"` 换取 ~10% 性能提升（体积增加有限）；(2) 添加 `debug = 1` 保留函数名符号用于生产 profiling。当前 `lto = "thin"` + `panic = "abort"` + `strip = true` 组合对体积优先场景合理。

### perf-profile-first 先分析再优化
- **评估**: ✅ 通过
- **证据**: 无性能分析数据、无基准测试数据、无证据早优化痕迹
- **说明**: 项目无 CPU 密集型热点。所有「优化」（entry API、extend 批量插入、迭代器模式）是标准 Rust 写法而非微优化。没有 `#[inline]` 滥用或手动循环展开。实践中自然遵循了先分析再优化的原则。

---

## 12. Project Structure（LOW）

### proj-lib-main-split lib.rs 最小化
- **评估**: ✅ 通过
- **证据**: `lib.rs` 4 行（2 声明 + 1 宏），`main.rs` 489 行（入口），`bootstrap.rs` 522 行（初始化）
- **说明**: `lib.rs` 仅声明 `pub mod adapters` 和 `pub mod core` 两个模块 + i18n 宏。二进制入口逻辑在 `main.rs`。启动初始化逻辑独占 `bootstrap.rs`（522 行）。lib、main、bootstrap 职责分明。

### proj-mod-by-feature 按功能组织模块
- **评估**: ✅ 通过
- **证据**: 22 个子目录，按功能分组：`adapters/`、`core/`、`app/`、`main/`、`utils/`
- **说明**: 模块结构按功能而非类型组织。避免「按类型」结构（`types/`, `utils/`, `handlers/` 顶层）。`core/` 内部分为 `security/`、`network/`、`system/`、`xray/`、`singbox/`、`sni/`。`adapters/` 按平台分 `telegram/`、`matrix/`、`discord/`。

### proj-flat-small 小项目保持扁平
- **评估**: ✅ 通过
- **证据**: 22 目录、约 50 源文件、最多 5 层嵌套（`adapters/telegram/handlers/xray/`）
- **说明**: 深度适中。`handlers/` 内 `schedule/` 和 `xray/` 子模块因 handler 数量多（`batch.rs` 1200 行、`handle.rs` 700 行）需要细分，合理。整体结构足够扁平。

### proj-mod-rs-dir 多文件模块用 mod.rs
- **评估**: ✅ 通过
- **证据**: 14 个 `mod.rs`：`core/`、`adapters/`、`app/`、`main/`、`utils/`、`core/{network,security,singbox,sni,system,xray}/`、`adapters/{common,discord,matrix,telegram}/`
- **说明**: 所有多文件模块统一使用 `mod.rs`。无 `module_name/` 下缺少 `mod.rs` 的情况。`adapters/telegram/handlers/mod.rs`（2263 行）是 handler 路由中心，行数多因 handler 数量庞大。

### proj-pub-crate-internal pub(crate) 用于内部 API
- **评估**: ✅ 通过
- **证据**: 20+ 处 `pub(crate)`：`routing.rs:17`、`keyboard.rs:7-167`（10 项）、`schedule/mod.rs:41-71`（3 项）、`singbox/config.rs:285-475`（5 项）、`operations.rs:140`
- **说明**: crate 内部 API 标记 `pub(crate)` 使用恰当。`routing.rs` 的 `is_sensitive` 仅用于 adapter 内部。keyboard 构建函数和 schedule 辅助函数仅限 telegram handler 内部使用。

### proj-pub-super-parent pub(super) 用于父模块
- **评估**: ✅ 通过
- **证据**: 60+ 处 `pub(super)`：`handle.rs:16-756`（20+ handler）、`batch.rs:14-1216`（16+ handler）、`delete.rs:9-120`、`mgmt.rs:7-111` 等
- **说明**: handler 子模块广泛使用 `pub(super)`，确保函数仅对父模块 `handlers/` 可见。`schedule/handle.rs` 的 20 个 handler 函数和 `xray/batch.rs` 的 16 个 handler 均正确使用 `pub(super)`，边界清晰且规范一致。

### proj-pub-use-reexport pub use 重新导出
- **评估**: ❌ 违反
- **证据**: `lib.rs` 无任何 `pub use`；唯一一处是 `schedule/mod.rs:11` 的 `pub(super) use keyboard::...`
- **说明**: `lib.rs` 声明 `pub mod adapters` + `pub mod core` 但不重新导出常用类型。消费者（`main.rs` 第 23-30 行）使用 `use aegis::adapters::common::BotAdapter`、`use aegis::core::i18n`、`use aegis::core::security::SecurityManager` 等深度路径。建议在 `lib.rs` 添加 `pub use` 重新导出（例如 `pub use adapters::common::BotAdapter`）简化外部使用路径。

### proj-prelude-module prelude 模块
- **评估**: ❌ 违反
- **证据**: 全库无 `prelude` 模块或 `prelude.rs`
- **说明**: 20+ 文件重复导入 `use aegis::adapters::common::{BotAdapter, MessageContent, TargetId}`、`use aegis::core::i18n`、`use aegis::core::types::IpVersion` 等。缺少 prelude 模块聚合常用导入。建议在 `lib.rs` 或 `core/mod.rs` 中添加 `pub mod prelude` 重导出核心类型。

### proj-bin-dir 多二进制在 src/bin/
- **评估**: ➖ 不适用
- **证据**: 无 `src/bin/` 目录；单二进制 crate
- **说明**: 项目为单一二进制应用（Telegram/Discord/Matrix bot），无需多入口点。如将来需拆分 CLI 模式与守护进程为独立二进制，可引入 `src/bin/`。

### proj-workspace-large 大型项目用 Workspace
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml` 无 `[workspace]` 段
- **说明**: 单 crate 项目，代码约 1 万行。无明显的分离边界。若将来将 `core` 提取为独立库 crate 可引入 workspace，但当前单 crate 结构对项目规模合适。

### proj-workspace-deps Workspace 依赖继承
- **评估**: ➖ 不适用
- **证据**: 无 workspace 配置
- **说明**: 无 workspace 因此无依赖继承。若引入 workspace，`serde`、`tokio`、`anyhow`、`thiserror` 等共享依赖可通过 `[workspace.dependencies]` 统一版本管理。

---

## 13. Clippy & Linting（LOW）

### lint-deny-correctness #![deny(clippy::correctness)]
- **评估**: ❌ 违反
- **证据**: `src/main.rs:2-3`（`#![recursion_limit = "256"]`, `#![allow(clippy::vec_init_then_push)]`），全库无 `#![deny(clippy::correctness)]`
- **说明**: 没有任何 `deny` 级别的 clippy lint 配置。`lib.rs`（4 行）和 `main.rs` 均未设置 `#![deny(clippy::correctness)]`。这表示正确性相关的 lint（如 `unit_arg`、`int_plus_one`）仅在 `warn` 级别（默认），编译器不会阻止 CI 通过。建议在 `lib.rs` 或 `main.rs` 开头添加 `#![deny(clippy::correctness)]` 强制阻断正确性问题。

### lint-warn-suspicious #![warn(clippy::suspicious)]
- **评估**: ❌ 违反
- **证据**: 全库无 `#![warn(clippy::suspicious)]`
- **说明**: 没有启用 suspicious 类别 lint。该类别包含 `manual_swap`、`manual_range_contains`、`map_entry`、`eq_op` 等检测逻辑疑点（如 `a == a`、不必要的 `clone` 后 `swap`），有助于在 code review 前发现逻辑不一致。建议添加。

### lint-warn-style #![warn(clippy::style)]
- **评估**: ❌ 违反
- **证据**: 全库无 `#![warn(clippy::style)]`
- **说明**: 没有启用 style lint 类别。该类别包含 `enum_variant_names`、`if_same_then_else`、`needless_pass_by_value`、`single_match` 等风格优化。当前 main.rs 包含针对 `clippy::vec_init_then_push` 的 `#![allow]`——如果有 style 警告，此处可直接修复。建议启用以便及早发现风格问题。

### lint-warn-complexity #![warn(clippy::complexity)]
- **评估**: ❌ 违反
- **证据**: 全库无 `#![warn(clippy::complexity)]`
- **说明**: 没有启用 complexity 类别 lint。该类别检测过度复杂的代码模式：`borrowed_box`、`needless_collect`、`needless_return`、`useless_conversion`、`question_mark` 等。对 batch handler 中多处 `collect()` 后立即迭代的模式，complexity lint 可自动识别。建议添加。

### lint-warn-perf #![warn(clippy::perf)]
- **评估**: ❌ 违反
- **证据**: 全库无 `#![warn(clippy::perf)]`
- **说明**: 没有启用 perf 类别 lint。该类别包含 `clone_on_copy`、`large_enum_variant`、`needless_collect`、`unnecessary_to_owned` 等性能模式。对代码库中 57 处 `Vec::new()`、25 处 `String::new()` 和 `.clone()` 热点的审查，自动化 lint 可在编译时发现可优化场景。

### lint-pedantic-selective 选择性启用 clippy::pedantic
- **评估**: ⚠️ 待改进
- **证据**: `src/main.rs:3` `#![allow(clippy::vec_init_then_push)]`；`src/app/auth.rs:10` `#[allow(clippy::too_many_arguments)]`；`src/core/xray/config.rs:168,361` `#[allow(clippy::too_many_arguments)]`
- **说明**: 代码使用 `#[allow]` 压制了若干 clippy 警告，但这是被动的"压制已有警告"模式，而非主动的"选择性启用 pedantic"。可添加 `#![warn(clippy::pedantic)]` 后逐一压制 `#[allow]` 不适用项，而非大量跳过 pedantic。当前使用的 `#[allow]` 项（`vec_init_then_push`、`too_many_arguments`、`dead_code`、`enum_variant_names`）均合理，体现了选择性但缺少主动启用。

### lint-missing-docs #![warn(missing_docs)]
- **评估**: ❌ 违反
- **证据**: 全库无 `#![warn(missing_docs)]`；Section 10 指出文档覆盖率仅 ~29%
- **说明**: 没有启用 `missing_docs` lint。公开项文档覆盖率低（77 `///` doc 注释 vs 269 公开项）。`TargetId`、`MessageId`、`AppState` 的公开方法、handler 函数等大量项无文档。对于非库应用，可容忍一定缺失，但核心类型（`AppError`、`DestructStep`、`EventBus`）应有完整文档。

### lint-unsafe-doc #![warn(clippy::undocumented_unsafe_blocks)]
- **评估**: ❌ 违反
- **证据**: 13 处 `unsafe` 块（`crypto.rs:91-124`、`core_upgrade.rs:751-795`、`bootstrap.rs:290-295`），全库无 `#![warn(clippy::undocumented_unsafe_blocks)]` 且无 `# Safety` 文档
- **说明**: 没有启用 `undocumented_unsafe_blocks` lint。生产代码中使用 13 处 `unsafe` 调用（`mlock/munlock`、`setrlimit`、`prctl`、`env::set_var/remove_var`），全部缺少 `// SAFETY:` 注释说明前置条件。启用此 lint 可强制每个 unsafe 块提供安全论证。

### lint-cargo-metadata 为发布 crate 启用 clippy::cargo
- **评估**: ➖ 不适用
- **证据**: `Cargo.toml:1-4` absence of metadata fields（无 `description`、`authors`、`license`、`repository`）
- **说明**: `clippy::cargo` 类别主要检查 Cargo.toml 元数据完整性（missing description, multiple versions 等），适用于 crates.io 发布。aegis 为 bot 应用，非发布 crate。`Cargo.toml` 元数据确实不完整，但不由 clippy 约束。若未来发布为库，应同时添加 metadata 和 `#![warn(clippy::cargo)]`。

### lint-rustfmt-check CI 中运行 cargo fmt --check
- **评估**: ❌ 违反
- **证据**: `.github/workflows/` 无 CI 配置；全库无 rustfmt 检查脚本
- **说明**: 没有 CI pipeline，无 `cargo fmt --check` 或 `cargo clippy` 自动化步骤。虽然没有 CI 基础设施，但建议在 `.github/` 中配置基础的 fmt + clippy CI workflow。作为最低要求，可在 `Makefile` 或 `justfile` 中添加 fmt check target 用于手动运行。

### lint-workspace-lints 工作区级别 lint 配置
- **评估**: ❌ 违反
- **证据**: `Cargo.toml` 无 `[lints]` 部分；项目为单 crate，无 `[workspace]` 配置
- **说明**: Rust 1.74+ 支持在 `Cargo.toml` 中通过 `[lints]` 统一配置 lint 级别。项目未使用此机制。若要添加 lint 配置，建议在 `Cargo.toml` 中添加 `[lints]` 集中管理所有 clippy 配置，而非分散在 lib.rs/main.rs 的 `#![]` 属性中。

---

## 14. Anti-patterns（REFERENCE）

### anti-unwrap-abuse 生产代码中避免 unwrap
- **评估**: ❌ 违反
- **证据**: `src/core/security/firewalld.rs:95,138,210,258,312`（5 处 zbus 调用的 `.unwrap()`），`src/core/network/warp_api.rs:134,150`，`src/core/network/geoip.rs:114,122`，`src/core/security/firewall_scanner.rs:280,288,300`，`src/bootstrap.rs:249-253`（5 处 `Option.unwrap()`），`src/adapters/telegram/handlers/xray/batch.rs:496,629,634`
- **说明**: 生产代码中存在 20+ 处 `.unwrap()` 调用。最严重的是 `firewalld.rs` 中 zbus 远程调用（网络 I/O，可能因服务未运行而失败）直接 unwrap。`bootstrap.rs:249-253` 的矩阵配置项 unwrap 会导致配置错误时直接 panic。`geoip.rs` 和 `warp_api.rs` 中 `serde_json::from_str` 的 unwrap 应传播为 `?`。建议所有 `from_str` 调用和远程 I/O 使用 `?` 或 `.context()`。

### anti-expect-lazy 使用有意义的 expect 消息
- **评估**: ✅ 通过
- **证据**: `src/core/xray/config.rs:767` `expect("应存在 extra 参数")`，`src/core/xray/config.rs:770` `expect("extra 应可解码")`，`src/core/xray/reality.rs:240,249` `expect("应成功转换")`，`src/core/network/release_api.rs:9` `expect("valid sha256 regex")`
- **说明**: 所有 12 处 `.expect()` 调用均包含有意义的语义消息（中文或英文），明确标注了失败原因。`release_api.rs:9` 的静态正则编译 `expect` 是标准用法。没有 `expect("")`、`expect("failed")` 等无意义消息。测试代码中的 `expect` 也提供了变体上下文。

### anti-clone-excessive 避免不必要的克隆
- **评估**: ⚠️ 待改进
- **证据**: `src/core/security/tls_probe.rs:93,112,116,155`（多次克隆 TLS 探测结果），`src/core/singbox/hy2_batch.rs:70-78`（批量克隆 password/sni），`src/core/sni/selector.rs:150,166-167`（克隆 domains/suffled_indices），`src/core/sni/state.rs:114`（克隆 secret 内容）
- **说明**: 批量构造（`hy2_batch.rs`、`tuic_batch.rs`）对同一字段在每个循环迭代中 `.clone()`，可考虑循环外克隆后引用传递。`tls_probe.rs` 对探测结果的多次克隆部分可通过调整引用逻辑减少。`sni/selector.rs` 中 `get_next()` 返回克隆的是必需的所有权转移。多数克隆在非热点路径上，但可减少 15-20% 的 `clone()` 调用。

### anti-lock-across-await 避免锁跨 await
- **评估**: ✅ 通过
- **证据**: `src/app/state.rs:134-258` 中 20+ 处 `.lock().await`，`src/core/system/scheduler/mod.rs:139-181` 中 `.lock().await`
- **说明**: 所有 `Mutex` 均来自 `tokio::sync::Mutex`，使用 `.lock().await` 非阻塞获取。锁持有期间无 `.await` 调用，所有锁作用域均在同一方法内，在 `}` 处自动释放。无 `std::sync::Mutex` 使用。符合规则要求。

### anti-string-for-str 使用 &str 而非 &String
- **评估**: ✅ 通过
- **证据**: `src/core/utils.rs:42-56` `parse_ip_version(s: &str)`、`generate_timestamp_filename(prefix: &str, extension: &str)`、`BotAdapter` trait 方法参数
- **说明**: 函数签名广泛使用 `&str`。未发现 `&String` 参数。`MessageContent::markup` 等构造使用 `impl Into<String>` 支持灵活传参。`TargetId(pub String)` 是所有权持有类型而非函数参数。

### anti-vec-for-slice 使用 &[T] 而非 &Vec<T>
- **评估**: ✅ 通过
- **证据**: 全库搜索无 `&Vec<T>` 参数
- **说明**: 没有函数使用 `&Vec<T>` 作为参数类型。所有集合引用通过切片 `&[T]` 或直接所有权传递。配置构造器接收 `Vec<T>` 是合理所有权转移。

### anti-index-over-iter 使用迭代器而非索引
- **评估**: ✅ 通过
- **证据**: `src/core/security/crypto.rs:79-80`（切片 `&encrypted_data[..12]` 是标准 AES-GCM nonce 提取），全库无 `data[i]` 索引访问
- **说明**: 代码中无通过索引直接访问集合元素的模式。数据处理路径使用 `.iter()`、`.map()`、`.filter()`、`.find()`。唯一"索引"操作为 `crypto.rs:79-80` 的范围切片（`[..12]`、`[12..]`），这是 AES-GCM nonce 分离的标准模式，编译器直接生成指针运算而非边界检查。

### anti-panic-expected 勿对预期错误 panic
- **评估**: ❌ 违反
- **证据**: 同上 `anti-unwrap-abuse`：`firewalld.rs:95,138,210,258,312`（zbus 网络调用）、`warp_api.rs:134,150`（JSON 解析）、`geoip.rs:114,122`（JSON 解析），`bootstrap.rs:249-253`（配置缺失）
- **说明**: 所有 `unwrap()` 位置本质上是将预期错误（JSON 解析失败、网络连接失败、配置缺失）转化为 panic。对于 bot 应用，JSON 解析失败可能是文件损坏或 API 格式变更，正确的做法是传播错误并让用户反馈"配置格式错误"。`bootstrap.rs` 中矩阵配置缺失应在日志中输出明确说明并建议解决而非直接崩溃。

### anti-empty-catch 避免空的 if let Err 块
- **评估**: ✅ 通过
- **证据**: 所有 `if let Err(e) = ...` 块均包含处理逻辑（`log::warn!`、`ctx.bot.send_message(...)`、`log::error!`）
- **说明**: 10 处 `if let Err` 模式全部包含非空处理体：记录日志并通常向用户发送错误消息。无 `if let Err(_) = ... { }` 或 `if let Err(_) = ... {}` 的静默忽略模式。

### anti-over-abstraction 避免过度抽象
- **评估**: ✅ 通过
- **证据**: `src/adapters/common/trait.rs:34`（`BotAdapter` trait 仅 `Send + Sync`），`src/adapters/telegram/handlers/ops/deploy.rs:256`（单处 `impl Into<String>`），`src/core/system/maintenance.rs:144`（单处 `F` 泛型闭包参数）
- **说明**: 泛型使用克制。`BotAdapter` 没有过度泛型化（使用 `&dyn BotAdapter` 而非 `<A: BotAdapter + ?Sized>`）。`ConfigBuilder` 使用具体类型。无多层泛型嵌套或无参数的结构体泛型。

### anti-premature-optimize 避免先优化后分析
- **评估**: ➖ 不适用
- **证据**: 无性能分析数据、无基准测试、无微优化痕迹
- **说明**: 项目为 I/O 密集型 bot 应用，无 CPU 密集型热点。所有优化选择（entry API、extend 批量插入、iterators）是标准 Rust 实践而非微优化。无 `#[inline]` 滥用、手动循环展开、unsafe 内联汇编等。实践中自然避免了过早优化。此规则在代码审查中无法正面验证。

### anti-type-erasure 使用 impl Trait 而非 Box<dyn Trait>
- **评估**: ✅ 通过
- **证据**: 全库无 `Box<dyn ` 在返回值或参数位置（除必要的动态分发：`Arc<dyn BotAdapter>`、`BoxFuture<'static>`）
- **说明**: 代码中的 `dyn` 使用均为必要的动态分发场景：`Arc<dyn BotAdapter>`（运行时选择 Telegram/Discord/Matrix adapter）、`BoxFuture<'static>`（异步 trait 方法需要）、`Arc<dyn SelfDestructExecutor>`（运行时选择 production/test executor）。没有不必要的 `Box<dyn Trait>` 替代 `impl Trait` 的情况。

### anti-format-hot-path 避免在热路径中使用 format!
- **评估**: ✅ 通过
- **证据**: `src/core/network/release_api.rs:52`（URL 构造，每次请求一次），`src/core/security/fail2ban.rs:62,150,181,188`（Fail2Ban 配置生成，低频），`src/core/security/tls_probe.rs:43,48,56,158`（TLS 探测日志/错误消息），`src/core/security/ufw.rs:44`（错误消息拼接）
- **说明**: 所有 `format!()` 调用位于网络请求构造、配置生成、错误消息格式化等路径上——均为低频 I/O 操作而非 CPU 热路径。无 `format!()` 在循环内部（batch handler 的循环中构造消息需 `format!` 属于必要的输出构造）。热路径定义（毫秒级重复调用）不成立。

### anti-collect-intermediate 避免收集中间迭代器
- **评估**: ⚠️ 待改进
- **证据**: `src/core/security/anti_debug.rs:39,72,88`（`split_whitespace().collect::<Vec<&str>>()` 后遍历），`src/core/security/firewall_scanner.rs:96,106,128,138`（类似 pattern），`src/core/security/ufw.rs:170`（同上），`src/core/sni/selector.rs:154`（`(0..len).collect::<Vec<usize>>()` 后 shuffle 再遍历）
- **说明**: 多处 `split()` 后将片段收集为 `Vec<&str>` 再 `for` 迭代，可直接使用 `split().filter_map()` 链式处理避免中间分配。`anti_debug.rs` 和 `firewall_scanner.rs` 中各行解析的 split→collect→iterate 模式可直接链式迭代。`selector.rs:154` 的索引收集是 shuffle 语义所必需的（需要确切元素才能 `shuffle`），无法直接链式。建议在仅遍历的场景消除中间 Vec。

### anti-stringly-typed 避免使用字符串表示结构化数据
- **评估**: ✅ 通过
- **证据**: `src/core/types.rs`（全枚举类型），`src/adapters/common/trait.rs:28`（`Platform::Telegram/Discord/Matrix`），`src/app/state.rs:17`（`DestructStep` 枚举），`src/core/events.rs:4-26`（`Severity`/`Component`/`Status` 枚举）
- **说明**: 无 stringly-typed 模式。所有业务概念（平台、协议、步骤、频率、任务类型）使用枚举表示。回调数据字符串是真实的序列化/反序列化用途，而非替代类型系统。配置键值对使用 `serde_json::Value` 是合理的半结构化数据。

---

## 改进建议（按优先级排序）

整体评估：**179 条规则中 77 条通过 (43%)、36 条违反 (20%)、39 条不适用 (22%)、27 条待改进 (15%)**。违反率最高的类别为 Documentation（82% 违反）和 Clippy & Linting（82% 违反），需优先投入。

### Top 5 严重问题（立即处理）

1. **文档严重缺失（Section 10: 9 ❌ / 11）** — 公开项文档覆盖率仅 29%。13 处 `unsafe` 块全部缺少 `# Safety` 文档。无 `# Errors`、`# Panics`、`# Examples` 标注。改进优先级最高，因为无文档的 unsafe 代码是审计和正确性隐患。**建议**：优先为所有 `unsafe` 块添加 `// SAFETY:` 注释；然后为核心类型（`AppError`、`DestructStep`、`EventBus`）补充 `///` 文档。

2. **生产代码中 20+ 处 `.unwrap()` 导致潜在 panic（Section 2/14）** — `firewalld.rs` 中 5 处 zbus 网络调用、`bootstrap.rs` 中 5 处配置项、`geoip.rs`/`warp_api.rs` 中 JSON 解析均使用 `.unwrap()`。任何网络故障或配置损坏导致 bot 直接崩溃。**建议**：将所有远程 I/O 和解析调用的 `unwrap()` 替换为 `?` + `.context()`；配置项缺失使用 `Option::ok_or_else()` + 用户友好错误提示。

3. **Clippy lint 基础设施完全缺失（Section 13: 9 ❌ / 11）** — 无 `deny(correctness)`、无 `warn(suspicious/style/complexity/perf)`、无 `missing_docs` 和 `undocumented_unsafe_blocks`。缺少编译时质量门禁，正确性问题（如多余克隆、无文档 unsafe 块）无法自动捕获。**建议**：在 `Cargo.toml` 的 `[lints]` 部分集中配置 clippy lint 级别，至少启用 correctness/suspicious/complexity/perf。

4. **API 设计模式缺失（Section 4: 4 ❌ / 4 ⚠️）** — 无 `#[must_use]`、无 `#[non_exhaustive]`、serde 未置于 feature flag 后、`ConfigManager::build_reality_vless_inbound()` 接收 12+ 参数却无 Builder 模式。**建议**：为公开枚举添加 `#[non_exhaustive]`，为 `Platform`、`DestructStep` 等确保前向兼容；serde 设为 `optional = true`。

5. **异步并行缺失（Section 5: 3 ❌ / 3 ⚠️）** — 无 `tokio::join!`、无 `try_join!`、无 `CancellationToken` 优雅关闭。多个独立异步操作（如 `bootstrap.rs` 的文件检查、`reality.rs` 的 IPv4/IPv6 探测）被顺序执行。**建议**：在无依赖的异步调用处使用 `tokio::try_join!`；添加 `CancellationToken` 支持 Matrix-only 模式的优雅关闭。

### Top 5 中等问题（近期处理）

6. **AppError 缺少 #[source] 链式追溯（Section 2）** — `#[from]` 隐式添加了 `source()` 但 `Config(String)` 等变体不保留源错误。错误消息丢失根因。**建议**：为需要追溯的变体显式标注 `#[source]`，或将 `Config` 改为包装 `anyhow::Error`。

7. **内存优化可改进点较多（Section 3: 5 ⚠️ / 15）** — 57 处 `Vec::new()` 和 25 处 `String::new()` 未使用 `with_capacity` 预分配；无 `.clone_from()` 调用；353 个 `for` 循环中仅 1 处复用集合。handler 层每个请求新建集合。**建议**：为已知大小的集合添加 `with_capacity()`；在热点路径复用缓冲区。

8. **测试覆盖率不足（Section 9: 2 ❌ / 1 ⚠️）** — 无属性测试（proptest）、无 doc test 示例。`mockall` 已声明为 dev-dependency 但完全未使用。**建议**：为 `SecurityManager` 的加密/解密往返添加 proptest；为 `TotpManager` 添加属性测试；在核心类型上添加 doc test。

9. **get_ 前缀违规（Section 7: 25+ 处）** — `get_default()`、`get_display_name()`、`get_summary()`、`get_status_report()` 等大量方法使用 `get_` 前缀。Rust 约定 getter 不加前缀。**建议**：`get_default()` → `default()`、`get_summary()` → `summary()`、`get_status_report()` → `status_report()`。

10. **文档 infra 全面缺失（Section 10: doc-intra-links/doc-link-types）** — 无任何 intra-doc 链接，关联类型间无交叉引用。**建议**：在核心类型文档中添加 `[`AppError`]`、[`EventBus`]`` 等链接。

### Quick Wins（可立即执行）

- **添加 lint config**：在 `lib.rs` 添加 `#![deny(clippy::correctness)]` + `#![warn(clippy::perf, clippy::complexity, clippy::suspicious, clippy::style)]` — 零代码改动，立即获得编译时质量检查。
- **修复 firewalld.rs unwrap**：`firewalld.rs:95,138,210,258,312` 的 5 处 zbus `.unwrap()` 改为 `?`，约 10 分钟工作量。
- **添加 #[must_use]**：在 `Cargo.toml` `[lints]` 或函数上添加 `#[must_use]`，防止返回的 `Result` 被丢弃。
- **运行 cargo fmt**：`cargo fmt --check` 确保代码格式化一致，可添加为 `Makefile` target 或 git pre-commit hook。
- **添加 CI workflow**：创建简单的 `.github/workflows/rust.yml` 运行 `cargo build` + `cargo test`。即使只有手动触发也优于零 CI 状态。
