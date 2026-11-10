# SPEC: XDRIVE 个人版 OneDrive Template 原生集成与旧版 Azure App 废除规范

## 1. Objective (目标与背景)
在 `rust/aegis` 服务端体系中彻底废弃旧版需要手动在 Azure Entra ID 创建企业级 App 的机制，全面升级 `XdriveService::OneDrive`：
1. **底层引擎升级**：由于 Xray-core 原生并未实现 `service: "onedrive"`，序列化及生成 Xray 配置文件时，自动将 `OneDrive` 服务转换为经过完整实测验证的 `service: "template"` 对象模式（携带完整的 OAuth2、PUT、GET、DELETE、LIST 模板，强制注入 `"flatten": true`）；
2. **极简用户体验**：内置已验证的公开公共凭证：
   - Client ID: `b15665d9-eda6-4092-8539-0eec376afd59`
   - Client Secret: `qtyfaBBYA403=unZUP40~_#`
   用户只需通过 OAuth2 登录普通个人版 OneDrive 账号，粘贴获取的 Token JSON（如 `rclone authorize` 的输出），向导即自动绑定，无需再手动创建任何 Azure 应用程序；
3. **消除无效逻辑**：清理现有代码中对 Azure App Client ID / Secret 的手动向导提示与无用分支。

---

## 2. Commands (执行与验证命令)
```bash
# 格式化
cargo fmt --all --check

# 代码质量检查
cargo clippy --all-targets --all-features -- -D warnings

# 测试驱动（全量测试与特定单元测试）
cargo test --package aegis --test test_xdrive
cargo test --package aegis xdrive
```

---

## 3. Project Structure & Scope (涉及项目结构)
```
rust/aegis/
├── src/
│   ├── core/
│   │   └── xray/
│   │       ├── xdrive.rs          # XdriveConfig 结构定义、公共凭证常量、OneDrive 构造器与 Template 生成逻辑
│   │       └── config.rs          # Xray JSON 配置组装器、VLESS 链接构造与 XDRIVE 设置序列化
│   └── shared/
│       └── handlers/
│           └── message.rs         # 交互式向导解析器（extract_onedrive_tokens 升级）
├── tests/
│   └── xdrive_onedrive_template_test.rs  # 新增端到端构造与序列化验证测试
└── docs/design/
    └── 2026-11-10-xdrive-onedrive-template-spec.md  # 本设计规范
```

---

## 4. Architecture & Technical Design (架构与技术设计)

### 4.1 常量与默认凭据定义 (`rust/aegis/src/core/xray/xdrive.rs`)
```rust
/// rclone 默认公开 Client ID（适用于个人免费版 OneDrive）
pub const ONEDRIVE_DEFAULT_CLIENT_ID: &str = "b15665d9-eda6-4092-8539-0eec376afd59";
/// rclone 默认公开 Client Secret（解密值）
pub const ONEDRIVE_DEFAULT_CLIENT_SECRET: &str = "qtyfaBBYA403=unZUP40~_#";
```

### 4.2 OneDrive 构造函数签名与默认流控参数
```rust
impl XdriveConfig {
    /// 预设构建器：Microsoft OneDrive 个人版（自动装配公开凭证与经过验证的平衡流控参数）
    pub fn new_onedrive(
        refresh_token: &str,
        folder: &str,
    ) -> Self {
        Self::new_onedrive_custom(
            ONEDRIVE_DEFAULT_CLIENT_ID,
            ONEDRIVE_DEFAULT_CLIENT_SECRET,
            refresh_token,
            folder,
        )
    }

    /// 支持自定义凭证的高级构建器
    pub fn new_onedrive_custom(
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
        folder: &str,
    ) -> Self {
        Self {
            service: XdriveService::OneDrive,
            remote_folder: folder.trim_matches('/').to_string(),
            secrets: vec![
                client_id.to_string(),
                client_secret.to_string(),
                refresh_token.to_string(),
            ],
            s3: None,
            segment_bytes: Some(65536),      // 64KB 分片
            flush_interval_ms: Some(80),     // 80ms 缓冲刷出
            poll_interval_ms: Some(200),     // 200ms 轮询间隔（与公网 RTT 匹配）
            max_poll_interval_ms: Some(1200),// 1200ms 空闲退避上限
            eager_window_ms: Some(2000),     // 2s 积极轮询窗口
            concurrency: Some(4),            // 4 并发读写槽（防 429 限流保护）
        }
    }

    /// 生成完整的 Microsoft Graph API XDRIVE Template JSON 对象
    pub fn generate_onedrive_template_value(&self) -> serde_json::Value {
        serde_json::json!({
            "flatten": true,
            "concurrency": self.concurrency.unwrap_or(4),
            "auth": {
                "type": "oauth2",
                "tokenUrl": "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                "form": {
                    "client_id": "{secret0}",
                    "client_secret": "{secret1}",
                    "refresh_token": "{secret2}",
                    "grant_type": "refresh_token"
                },
                "tokenPath": "access_token",
                "expiryPath": "expires_in",
                "header": {
                    "Authorization": "Bearer {token}"
                }
            },
            "put": {
                "method": "PUT",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content",
                "headers": {
                    "Content-Type": "application/octet-stream"
                }
            },
            "get": {
                "method": "GET",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
            },
            "delete": {
                "method": "DELETE",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}"
            },
            "list": {
                "method": "GET",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}:/children?select=name",
                "namesRegex": "\"name\"\\s*:\\s*\"([^\"]+)\""
            },
            "retry": {
                "status": [429, 500, 502, 503, 504],
                "rateReason": "error.code"
            }
        })
    }
}
```

### 4.3 Xray JSON 序列化转换 (`rust/aegis/src/core/xray/config.rs`)
在生成服务端和客户端 `xdriveSettings` 时：
当 `xdrive.service == XdriveService::OneDrive` 时：
- `service` 字段自动输出为 `"template"`；
- `template` 字段注入 `xdrive.generate_onedrive_template_value()`；
- `remoteFolder` 保持配置目录名；
- `secrets` 注入 `[client_id, client_secret, refresh_token]`；
- 输出经过验证的流控参数。

### 4.4 向导解析器升级 (`rust/aegis/src/shared/handlers/message.rs`)
升级 `extract_onedrive_tokens`：
- 若用户输入中含有 `refresh_token` 但未显式提供 `client_id` / `client_secret`（例如直接粘贴 `rclone authorize` 的输出 `{"access_token":"...","refresh_token":"..."}`）：
  - 自动回退填充 `ONEDRIVE_DEFAULT_CLIENT_ID` 与 `ONEDRIVE_DEFAULT_CLIENT_SECRET`；
- 彻底移除原引导文案中关于前往 Azure Portal 注册 App、获取 Application ID 的说明，替换为通过 `rclone authorize "onedrive"` 或类似工具获取 Token 的极简 30 秒指引。

---

## 5. Testing Strategy (测试驱动策略 - TDD)
1. **单元测试 (`core::xray::xdrive`)**：
   - 验证 `new_onedrive` 默认填充公共 Client ID/Secret；
   - 验证 `generate_onedrive_template_value()` 输出的 JSON 结构严格包含 `flatten: true`，且 `namesRegex` 包含捕获组；
   - 验证 `validate()` 对空 `refresh_token` 或非法目录名的拒绝保护。
2. **集成与序列化测试 (`core::xray::config`)**：
   - 构造含 `XdriveService::OneDrive` 的入站和出站，调用 `generate_xray_config`，断言生成的 JSON 中 `streamSettings.xdriveSettings.service` 为 `"template"`，且 `template` 为完整的 JSON 对象；
   - 验证生成的 VLESS 节点链接解析兼容性。
3. **向导文本解析测试 (`shared::handlers::message`)**：
   - 模拟用户仅输入包含 `refresh_token` 的 JSON 或键值对，验证自动成功提取并注入默认凭据。

---

## 6. Boundaries (行为边界契约)
- **Always do**:
  - 保持 `"flatten": true`，严禁在 OneDrive 模式中关闭展平；
  - 默认流控参数严格对齐生产基准（`segment_bytes: 65536`, `poll_interval_ms: 200`, `concurrency: 4`）；
  - 变更前后运行 `cargo test` 确保无任何回归破损。
- **Ask first**:
  - 更改其他云存储服务（如 S3/GoogleDrive/Local）的现有行为。
- **Never do**:
  - 向 Xray-core 发送未转义的 JSON String 作为 template（必须作为原生 Object）；
  - 向用户提示复杂的 Azure 开发者注册流程。

---

## 7. Success Criteria (验收标准)
- [ ] `XdriveService::OneDrive` 构造器无缝支持免输入 Client ID/Secret；
- [ ] 导出的 Xray `xdriveSettings` 为合法的 `service: "template"` 配置；
- [ ] 彻底清理所有涉及手动创建 Azure App 的文案与逻辑；
- [ ] `cargo fmt`、`cargo clippy`、`cargo test` 全绿，零告警。

---

## 8. Open Questions (当前未决事项)
- 无未决技术问题（前期沙盒与真实公网联调已完全消除技术未知数）。
