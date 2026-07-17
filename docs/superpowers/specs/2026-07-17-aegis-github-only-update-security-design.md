# Aegis GitHub-only 更新安全设计

## 目标

将 Aegis 自更新和 wwps-core 更新收敛为编译期固定的 GitHub 信任边界，删除第三方 Git 托管平台、自定义镜像和运行时仓库覆盖，同时强化完整性验证并阻止 `GITHUB_TOKEN` 泄漏到资产下载请求。

## 已批准决策

- Aegis 只信任 GitHub 仓库 `youugiuhiuh/Wuthering_Waves_Private_Server`。
- wwps-core 只信任 GitHub 仓库 `XTLS/Xray-core`。
- 两条更新链都使用编译期固定仓库，不提供运行时仓库白名单或兼容层。
- `GITHUB_TOKEN` 保留，但只允许发送到 `https://api.github.com`。
- Aegis 必须具有有效 Minisign 签名；缺少签名时拒绝更新。
- XTLS 当前不发布 Minisign；wwps-core 必须同时匹配 GitHub API `digest` 和对应 `.dgst` 中的 SHA2-256。
- Xray 的双摘要都处于固定 GitHub 仓库信任域内，用于完整性校验，不宣称提供独立发布者签名。

## 当前风险

### 可变更新源

`core/system/upgrade.rs` 当前支持 GitHub、Codeberg、Gitea 和任意镜像 API，并允许通过多个环境变量替换仓库和资产名。`core/system/core_upgrade.rs` 对 wwps-core 提供同类镜像和仓库覆盖。这些入口使本地配置或部署环境能够扩大更新信任域。

### Token 泄漏

现有请求构建逻辑可能对发布元数据提供的任意 URL 添加 `GITHUB_TOKEN`。恶意或被攻陷的 Release 元数据可把下载 URL 指向其他 host。

### 签名降级

现有 Aegis 更新逻辑允许签名缺失或下载失败后继续。core 更新也把签名设为可选，但其固定上游 XTLS 当前只发布 `.dgst`，没有可验证的 Minisign 资产。

## 信任边界

### 固定发布源

代码内只保留以下仓库常量：

| 更新对象 | Owner | Repository | API origin |
| --- | --- | --- | --- |
| Aegis | `youugiuhiuh` | `Wuthering_Waves_Private_Server` | `https://api.github.com` |
| wwps-core | `XTLS` | `Xray-core` | `https://api.github.com` |

Release API URL 必须由固定 origin、固定 owner、固定 repository 和代码生成的 API path 组合，不能接受完整外部 URL。

### 允许的下载域

Release 资产只接受 GitHub 的 `browser_download_url`。所有下载必须使用 HTTPS，并按每一跳验证 host：

- 初始 URL：`github.com`
- GitHub 资产重定向：`release-assets.githubusercontent.com`
- GitHub 对象存储重定向：`objects.githubusercontent.com`

不允许通配子域、后缀匹配、HTTP、IP 字面量、用户信息字段或其他 host。GitHub 将来改变资产域时，必须通过代码和测试显式扩展白名单。

### Token 边界

使用两个逻辑请求路径：

- Release API 请求：固定 `api.github.com`，可附加 `GITHUB_TOKEN`。
- 资产、SHA256 和 Minisign 请求：不附加 Token，并启用逐跳重定向校验。

即使 GitHub API 返回 `url` 字段，也不得使用该字段作为资产下载回退；只使用经过校验的 `browser_download_url`。

## 删除的配置面

以下环境变量不再读取：

- `AEGIS_RELEASE_MIRRORS`
- `AEGIS_RELEASE_REPOSITORIES`
- `AEGIS_RELEASE_REPOSITORY`
- `AEGIS_RELEASE_OWNER`
- `AEGIS_RELEASE_REPO`
- `AEGIS_RELEASE_ASSET`
- `WWPS_CORE_RELEASE_MIRRORS`
- `WWPS_CORE_RELEASE_OWNER`
- `WWPS_CORE_RELEASE_REPO`

本地安装目录、备份目录、临时目录、服务名和 CPU 架构不属于远程信任配置，可继续按现有用途配置。删除的环境变量不会获得兼容别名或迁移解析器。

## 更新流程

### Aegis

1. 从固定 GitHub API URL 获取最新 Release 元数据。
2. 选择固定名称的 Aegis 资产及其对应 `.minisig` 资产。
3. 验证两个资产的 `browser_download_url`。
4. 要求 Release API 的资产 `digest` 存在且采用 `sha256:<hex>` 格式。
5. 无 Token 下载目标资产和 Minisign，并验证资产计算 SHA256 与 API `digest` 一致。
6. 使用内置且未过期的 Minisign 公钥验证签名。
7. 验证 trusted comment 中的版本和文件名与 Release tag、目标资产一致。
8. 只有全部验证通过后才进入现有安装与重启流程。

### wwps-core

1. 从 `XTLS/Xray-core` 的固定 GitHub API URL 获取指定或最新 Release。
2. 按现有架构规则选择唯一资产及其对应 `.dgst`。
3. 要求 Release API 的资产 `digest` 存在且采用 `sha256:<hex>` 格式。
4. 无 Token 下载目标资产和 `.dgst`，解析其中的 `SHA2-256`。
5. 计算目标资产 SHA256，并要求它同时匹配 API `digest` 和 `.dgst`。
6. 对资产和 `.dgst` 执行与 Aegis 相同的 URL 与重定向校验。
7. 只有全部验证通过后才解压、替换和重启。

## 签名与摘要策略

- Aegis Release 必须包含目标资产对应的 Minisign 文件。
- Aegis 签名必须由 `MINISIGN_PUBLIC_KEYS` 中尚未过期的密钥验证通过。
- Aegis trusted comment 必须解析成功，并精确匹配 Release tag 和预期资产标识。
- Aegis 签名缺失、下载失败、格式错误、密钥过期、签名错误或 comment 不匹配均为终止错误。
- Aegis 不提供跳过签名的环境变量、命令行参数或 debug 回退。

当前 Aegis Release `v3.4.4` 已发布 `aegis.minisig`。实施前仍需用内置公钥验证实际资产及 trusted comment，验证失败时先修复本仓库发布流水线，客户端不得降级。

XTLS/Xray-core Release `v26.3.27` 已确认不包含 Minisign，Linux 资产提供对应 `.dgst`，且 GitHub API `digest` 与 `.dgst` 的 SHA2-256 一致。Xray 校验规则为：

- API `digest` 和 `.dgst` 均为必需项，缺少任一项即终止。
- 只接受严格的 SHA256 十六进制格式和 `.dgst` 中唯一的 `SHA2-256` 字段。
- 资产计算值、API 值和 `.dgst` 值必须三者相等。
- 不把 `.dgst` 当作密码学签名；其安全边界仍是固定 GitHub 仓库和 HTTPS。

## 组件改动

### `core/system/upgrade.rs`

- 将 Aegis owner、repository 和资产名改为单一固定常量。
- 删除镜像、仓库和资产环境变量解析。
- 将可选签名改为必需签名。
- 分离 API 请求与无凭据资产下载请求。

### `core/system/core_upgrade.rs`

- 固定 `XTLS/Xray-core`，删除镜像和 owner/repo 环境变量。
- 删除不可满足的可选 Minisign 路径，要求 API SHA256 digest 和对应 `.dgst`。
- 在替换前验证资产、API digest 和 `.dgst` 三者一致。
- 资产下载使用无凭据、受限重定向客户端。

### `core/network/release_api.rs`

- 将镜像轮询接口收敛为单一 GitHub API 请求。
- `ReleaseAsset::download_url` 只接受 `browser_download_url`，不再回退到 API `url`。
- 保留 Release JSON、digest、`.dgst` 和 Minisign 资产解析等真正共享的逻辑。

### `core/crypto/minisign.rs`

- 供 Aegis 自更新复用现有公钥和验证函数。
- 仅在需要精确比较 trusted comment 时收紧返回值或校验接口，不引入新的签名抽象。

## 错误处理

所有安全校验 fail closed：

- 校验失败时保留当前二进制，不替换文件，不重启服务。
- 用户侧返回统一“更新验证失败”消息和事件 ID。
- 服务端日志记录更新对象、Release tag、资产名、失败阶段和经验证的 host。
- 日志不得记录 Token、Authorization header、完整签名或带查询参数的重定向 URL。
- 不从一个固定仓库回退到另一个仓库或平台。

## 测试设计

### 固定来源

- Aegis API path 始终包含 `youugiuhiuh/Wuthering_Waves_Private_Server`。
- core API path 始终包含 `XTLS/Xray-core`。
- 已删除环境变量不能改变 owner、repository、API origin 或资产名。

### Token 隔离

- API 请求在提供 Token 时包含 Authorization header。
- 资产、`.dgst` 和 Minisign 请求不包含 Authorization header。
- 重定向后的请求也不包含 Authorization header。

### URL 校验

- 接受允许的 HTTPS GitHub 资产 URL 和逐跳重定向。
- 拒绝 HTTP、其他 host、伪造后缀、IP 字面量、userinfo 和未允许的重定向。
- 拒绝只有 API `url`、没有 `browser_download_url` 的资产。

### 签名

- 有效签名、匹配 tag 和资产名时通过。
- 缺失签名、错误签名、过期密钥、错误 tag 和错误资产名时失败。
- 任一失败都不会调用替换、解压或重启步骤。

### Xray 摘要

- API digest、`.dgst` 和资产计算值三者一致时通过。
- 缺失 digest、缺失 `.dgst`、格式错误、重复 SHA2-256 字段或任意值不一致时失败。
- 任一失败都不会调用解压、替换或重启步骤。

### 质量门禁

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`

当前检出版本已有格式和 Clippy 基线失败。实施计划必须先记录基线，并把与本改动无关的清理拆成独立步骤，不能降低 lint 标准或把历史失败隐藏在允许列表中。

## 非目标

- 不支持 Codeberg、Gitea、自建 GitHub Enterprise 或任意镜像。
- 不设计通用多仓库 trust-policy 框架。
- 不添加跳过 TLS、host 校验或签名校验的开关。
- 不在本项中处理升级回滚、single-flight、子进程回收或其他审计问题。
- 不改变 GitHub Release 的版本选择语义和本地安装路径。

## 验收标准

- 代码中不再存在第三方 Release API base 或上述远程来源环境变量读取。
- Aegis 和 core 只能从各自固定 GitHub 仓库获取 Release 元数据。
- Token 不可能出现在非 `api.github.com` 请求中。
- 所有下载 URL 和每一跳重定向都经过精确 scheme/host 校验。
- Aegis 在安装前强制验证 Minisign、tag 和资产标识。
- Xray 在安装前强制验证 API digest、`.dgst` 和资产计算 SHA256 三者一致。
- 任一安全校验失败时不会改变已安装二进制或服务状态。
- 新增测试通过，完整 Rust 质量门禁通过。
- 审计文档中的 `AEGIS-002` 和 `AEGIS-003` 在实现验证后标记为完成。
