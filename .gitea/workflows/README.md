# Gitea/Forgejo 工作流说明

与 `.github/workflows/public-release.yml` 功能一致，用于在 Gitea 或 Forgejo 上构建并发布公开分发包。

## 两版区别

| 文件 | 适用场景 | 依赖 |
|------|----------|------|
| **public-release.yml** | Runner 不能或不想访问 GitHub | 仅需 `git`、`curl`、`jq`、`rustup`、Go、UPX（apt 安装），无第三方 Action |
| **public-release-actions.yml** | Runner 能访问 GitHub，希望复用 GitHub Actions | 使用 `actions/checkout`、`dtolnay/rust-toolchain`、`Swatinem/rust-cache`、`crazy-max/ghaction-upx`、`actions/setup-go` |

任选其一启用即可（可删另一份或保留作备用）。

## 配置

1. **Secrets**（仓库或组织）  
   - `SOURCE_REPO_TOKEN`：拉取/推送**当前源码仓库**的 token（需写权限，用于 push 与打 tag）。  
   - `PUBLIC_REPO_TOKEN`：推送到**公开仓库**并操作 Release 的 token（需写权限 + 删除/创建 Release 权限）。  
   - （可选）`PUBLIC_SERVER_URL`：公开仓库所在实例地址（如 `https://codeberg.org`）。与当前实例一致可不设。

2. **公开仓库**  
   默认：`youugiuhiuh/Wuthering_Waves_Private_Server`。  
   若不同，在 **public-release.yml** 的 “Set API and Public Repo” 步骤里改 `PUBLIC_OWNER` / `PUBLIC_REPO`，或在支持的环境里用变量覆盖。

3. **放置位置**  
   - 放在 **.gitea/workflows/** 下即可。  
   - 若实例同时支持 **.github/workflows/**，可把对应 yml 复制到 `.github/workflows/` 使用。

## Gitea/Forgejo Release API 说明

- 删除 Release：`DELETE /api/v1/repos/{owner}/{repo}/releases/{id}`  
- 删除 Tag：`DELETE /api/v1/repos/{owner}/{repo}/git/refs/tags/{tag}`（部分版本或为 `/tags/{tag}`，失败时可改）  
- 创建 Release：`POST /api/v1/repos/{owner}/{repo}/releases`，body：`tag_name`、`name`、`body`  
- 上传附件：`POST /api/v1/repos/{owner}/{repo}/releases/{id}/assets?name=文件名`，body：multipart form，字段名 `attachment`

## 触发方式

与 GitHub 版相同：

- **手动**：在 Gitea/Forgejo 的 Actions 页选择 “Public Distribution Sync”，填写版本号（如 `0.4.4`）后运行。  
- **自动**：当 `rust/tgbot/Cargo.toml` 有 push 且 commit message 中不含 `[skip ci]` 时触发。
