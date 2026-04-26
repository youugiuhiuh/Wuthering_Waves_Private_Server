# tgbot 分层架构重构设计文档

**日期**: 2025-04-26  
**版本**: 1.0  
**作者**: OpenCode AI

---

## 1. 项目背景

当前 `tgbot` 是一个用于管理 VPS 的 Telegram Bot，主要功能包括：
- 用户管理（Xray-core、Sing-box 配置生成）
- 系统运维（WARP、BBR3、防火墙等）
- 定时任务调度
- 安全认证（TOTP）

### 1.1 当前问题

| 问题 | 影响 | 优先级 |
|------|------|--------|
| main.rs 超过 4500 行 | 难以维护、查找困难 | 高 |
| UI 逻辑与业务逻辑混杂 | 修改 UI 可能破坏业务 | 高 |
| Hysteria2/TUIC 批处理代码重复 | 维护成本高 | 中 |
| 大量 clippy 警告 | 代码质量下降 | 中 |
| 测试覆盖率极低 | 缺乏信心 | 低 |

### 1.2 使用场景

本项目主要用于**个人 VPS 管理**，变更频率较低，但追求长期可维护性。

---

## 2. 设计目标

### 2.1 主要目标

1. **将 main.rs 缩减到 <200 行** - 仅保留 bot 初始化和依赖注入
2. **建立清晰的分层架构** - UI、业务、实现三层分离
3. **提高代码可维护性** - 每个文件 <300 行，职责单一
4. **修复所有 clippy 警告** - 代码质量达到生产标准

### 2.2 非目标

- ❌ 不修改现有 `logic/` 模块的实现（仅重新组织调用）
- ❌ 不添加新功能
- ❌ 不修改数据存储格式
- ❌ 不改变用户界面和交互流程

---

## 3. 架构设计

### 3.1 分层架构

```
┌─────────────────────────────────────┐
│          Telegram Bot API           │
└─────────────────┬───────────────────┘
                  ↓
┌─────────────────────────────────────┐
│  handlers/ (UI Layer)               │
│  - 接收用户输入                     │
│  - 输入验证                         │
│  - 调用 Service 层                  │
│  - 格式化响应                       │
└─────────────────┬───────────────────┘
                  ↓
┌─────────────────────────────────────┐
│  services/ (Business Layer)         │
│  - 业务逻辑编排                     │
│  - 事务管理                         │
│  - 错误转换                         │
│  - 调用 logic/ 实现                 │
└─────────────────┬───────────────────┘
                  ↓
┌─────────────────────────────────────┐
│  logic/ (Implementation Layer)      │
│  - 现有模块保持不变                 │
│  - 具体技术实现                     │
│  - 文件操作、命令执行               │
└─────────────────────────────────────┘
```

### 3.2 设计原则

1. **依赖方向**: handlers → services → logic
2. **上层不依赖下层细节**: Service 只知道 logic 的接口
3. **错误转换**: 每层将下层错误转换为本层错误类型
4. **可测试性**: Service 层通过 trait 抽象依赖，便于 mock

---

## 4. 目录结构

### 4.1 新目录结构

```
rust/tgbot/src/
├── main.rs                      # ~100行：bot初始化、DI设置
├── bootstrap.rs                 # 启动配置（不变）
├── core/                        # 核心共享定义（不变）
│   ├── mod.rs
│   ├── paths.rs
│   ├── types.rs
│   ├── error.rs
│   └── utils.rs
├── handlers/                    # 🔥 新增：UI层
│   ├── mod.rs                   # Handler注册和分发
│   ├── commands.rs              # /start, /auth, /menu等命令
│   ├── callbacks/               # 回调按钮处理
│   │   ├── mod.rs               # 回调路由中心
│   │   ├── main_menu.rs         # m_main, m_mon, m_usr...
│   │   ├── user_mgmt.rs         # 用户管理相关回调
│   │   ├── ops_center.rs        # 运维中心回调
│   │   ├── settings.rs          # 系统设置回调
│   │   └── destruct.rs          # 自毁流程（保持现有逻辑）
│   └── messages.rs              # 普通消息处理
├── services/                    # 🔥 新增：业务逻辑层
│   ├── mod.rs
│   ├── user_service.rs          # 用户管理服务
│   ├── config_service.rs        # 配置生成服务
│   ├── system_service.rs        # 系统操作服务
│   └── security_service.rs      # 安全服务
├── app/                         # 应用状态（已存在）
│   ├── mod.rs
│   ├── state.rs
│   ├── auth.rs
│   └── destruct_flow.rs
└── logic/                       # 实现层（已存在，不变）
    ├── mod.rs
    ├── config.rs
    ├── installer.rs
    ├── upgrade.rs
    ├── scheduler/
    ├── singbox/
    └── ... (25+ 模块)
```

### 4.2 文件大小目标

| 文件 | 当前行数 | 目标行数 |
|------|----------|----------|
| main.rs | ~4551 | <150 |
| handlers/commands.rs | 0 | ~80 |
| handlers/callbacks/main_menu.rs | 0 | ~150 |
| handlers/callbacks/user_mgmt.rs | 0 | ~200 |
| handlers/callbacks/ops_center.rs | 0 | ~250 |
| handlers/callbacks/settings.rs | 0 | ~200 |
| services/user_service.rs | 0 | ~150 |
| services/config_service.rs | 0 | ~200 |
| services/system_service.rs | 0 | ~100 |

---

## 5. 模块职责

### 5.1 handlers/

#### commands.rs
- 处理所有 Telegram 命令（/start, /auth, /menu, /setsecurityfile）
- 验证用户权限
- 调用相应 Service

#### callbacks/
每个文件处理一个菜单领域的回调：
- `main_menu.rs`: m_main, m_mon 等主菜单
- `user_mgmt.rs`: m_usr, m_xray_mgmt, m_singbox_mgmt 等
- `ops_center.rs`: m_ops_center, m_net_opt, m_security 等
- `settings.rs`: m_settings, a_geo_menu, a_upgrade 等

#### messages.rs
- 处理普通文本消息
- TOTP 验证码验证
- WARP 规则输入
- 调度任务输入

### 5.2 services/

#### user_service.rs
```rust
pub struct UserService;

impl UserService {
    pub async fn list_users(&self) -> Result<Vec<User>>;
    pub async fn create_batch_config(&self, proto: RealityProto, count: usize) -> Result<BatchResult>;
    pub async fn delete_user(&self, user_id: &str) -> Result<()>;
}
```

#### config_service.rs
```rust
pub struct ConfigService;

impl ConfigService {
    pub async fn generate_reality_config(&self, ip_version: IpVersion, count: usize) -> Result<ConfigResult>;
    pub async fn generate_hysteria2_config(&self, ip_version: IpVersion, count: usize, obfs: bool) -> Result<ConfigResult>;
    pub async fn backup_configs(&self) -> Result<String>; // 返回备份文件路径
}
```

#### system_service.rs
```rust
pub struct SystemService;

impl SystemService {
    pub async fn perform_maintenance(&self) -> Result<String>; // 返回日志
    pub async fn reboot_system(&self) -> Result<()>;
    pub async fn get_system_status(&self) -> Result<StatusReport>;
    pub async fn install_bbr3<F>(&self, progress_cb: F) -> Result<InstallResult> where F: Fn(u8, &str);
}
```

---

## 6. 数据流示例

### 场景：用户点击"批量创建 Reality 配置"

```
用户点击 "u_batch_init" 回调
    ↓
handlers/callbacks/user_mgmt::handle_callback()
    ↓ 提取参数，验证权限
调用 services::config_service::create_batch_reality(ip_version, count)
    ↓
协调：
  - logic::installer::RealityInstaller::ensure_ready()
  - logic::config::generate_secure_batch_filename()
  - logic::sni_selector::get_unique_sni()
  - logic::config::build_reality_config()
    ↓
返回结果给 handler
    ↓
handler 发送 Telegram 消息给用户
```

---

## 7. 错误处理策略

### 7.1 错误类型分层

```rust
// services/error.rs
#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("配置生成失败: {0}")]
    ConfigGeneration(String),
    
    #[error("系统操作失败: {0}")]
    SystemOperation(String),
    
    #[error("验证失败: {0}")]
    Validation(String),
    
    #[error("资源未找到: {0}")]
    NotFound(String),
    
    #[error("权限不足")]
    Unauthorized,
}

// handlers 统一处理
match result {
    Ok(data) => send_success(bot, chat_id, data),
    Err(ServiceError::Validation(msg)) => send_validation_error(bot, chat_id, msg),
    Err(ServiceError::NotFound(res)) => send_not_found(bot, chat_id, res),
    Err(e) => send_generic_error(bot, chat_id, e),
}
```

### 7.2 错误转换规则

- `logic::AppError` → `ServiceError` (在 Service 层转换)
- `ServiceError` → `anyhow::Error` (在 Handler 层转换，或者直接处理)
- 保持现有 `logic/` 的 `AppError` 不变

---

## 8. 重构阶段规划

### Phase 1: 基础设施 (预计 1-2 小时)

1. **创建目录结构**
   ```bash
   mkdir -p src/handlers/callbacks
   mkdir -p src/services
   ```

2. **提取 commands.rs**
   - 从 main.rs 提取 `handle_command` 函数
   - 提取 `Command` 枚举
   - 保持现有逻辑，仅移动位置

3. **提取 messages.rs**
   - 从 main.rs 提取 `handle_message` 函数
   - 保持现有逻辑

4. **验证**
   - `cargo build` 通过
   - `cargo test` 通过
   - main.rs 行数 < 4000

### Phase 2: 回调处理 (预计 3-4 小时)

1. **创建回调路由系统**
   - 设计回调路由表（callback → handler）
   - 实现 `handlers/callbacks/mod.rs`

2. **提取 main_menu.rs**
   - 移动 m_main, m_mon, m_usr 等回调
   - 验证功能正常

3. **提取 user_mgmt.rs**
   - 移动所有用户管理相关回调
   - 包括：m_xray_mgmt, m_singbox_mgmt, sb_install, sb_h2_init 等

4. **提取 ops_center.rs**
   - 移动运维中心相关回调
   - 包括：m_ops_center, m_net_opt, m_security, m_sys_cmd 等

5. **提取 settings.rs**
   - 移动系统设置相关回调
   - 包括：m_settings, a_geo_menu, a_upgrade, m_sched 等

6. **验证**
   - 所有菜单功能正常
   - main.rs 行数 < 1000

### Phase 3: Service 层 (预计 2-3 小时)

1. **创建 services/mod.rs**
   - 定义 Service 结构体和错误类型

2. **提取 config_service.rs**
   - 从 main.rs 和 logic/ 提取配置生成逻辑
   - 消除 Hysteria2/TUIC 重复代码

3. **提取 user_service.rs**
   - 用户管理业务逻辑

4. **提取 system_service.rs**
   - 系统操作业务逻辑

5. **重构 main.rs**
   - 清理提取后的代码
   - 仅保留 bot 初始化和依赖注入

6. **验证**
   - main.rs 行数 < 200
   - 所有功能正常

### Phase 4: 代码优化 (预计 1-2 小时)

1. **修复 clippy 警告**
   - 修复所有 `cargo clippy` 警告
   - 重点：可折叠的 if 语句、函数参数过多

2. **添加文档注释**
   - 为所有 public API 添加 rustdoc 注释

3. **最终验证**
   - `cargo build --release` 成功
   - `cargo test` 全部通过
   - `cargo clippy` 无警告

---

## 9. 风险与缓解

| 风险 | 可能性 | 影响 | 缓解措施 |
|------|--------|------|----------|
| 功能回归 | 中 | 高 | 每阶段完成后全面测试；保持 git commit 历史；准备回滚方案 |
| 编译错误 | 高 | 低 | 使用 IDE 实时检查；小步提交；及时修复 |
| 性能下降 | 低 | 中 | 保持现有 logic/ 实现不变；仅重新组织调用 |
| 代码冲突 | 低 | 中 | 重构期间避免其他修改；必要时 rebase |

---

## 10. 成功标准

### 10.1 量化指标

- [ ] main.rs 行数 < 200
- [ ] `cargo clippy` 0 警告
- [ ] 代码重复率降低 50%+
- [ ] 所有现有功能正常工作

### 10.2 质量指标

- [ ] 每个文件职责单一
- [ ] 模块间依赖清晰
- [ ] 添加基本单元测试框架（可选）

---

## 11. 附录

### 11.1 参考资料

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Clean Architecture](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- 项目现有代码：`main.rs`, `logic/`, `core/`

### 11.2 决策记录

| 决策 | 原因 |
|------|------|
| 保留现有 logic/ 模块 | 风险最小，专注于架构重组而非重写 |
| 使用三层架构 | 清晰的职责分离，便于个人长期维护 |
| 不添加新功能 | 避免在重构中引入新 bug |

---

**状态**: 已批准  
**下一步**: 调用 writing-plans skill 创建详细实施计划
