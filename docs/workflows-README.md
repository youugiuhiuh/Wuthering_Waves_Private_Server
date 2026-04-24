# 多平台 CI/CD 工作流索引

本仓库提供多平台 CI/CD 配置，用于构建并在本仓库创建 Release（tgbot + installer）。

## 文件一览

| 平台 | 配置文件 | 说明 |
|------|----------|------|
| **GitHub** | `.github/workflows/public-release.yml` | 原始工作流，手动或 push `rust/tgbot/Cargo.toml` 触发 |
| **Gitea / Forgejo** | `.gitea/workflows/public-release.yml` | 无 GitHub 依赖，纯 shell + Gitea API |
| **Gitea / Forgejo** | `.gitea/workflows/public-release-actions.yml` | 使用 GitHub Actions 兼容语法 |
| **GitLab** | `.gitlab-ci.yml` | GitLab CI/CD，Release 使用 GitLab API |
| **Bitbucket** | `bitbucket-pipelines.yml` | Bitbucket Pipelines，main 分支或自定义 pipeline 手动带 VERSION |
| **Azure DevOps** | `azure-pipelines.yml` | Azure Pipelines，path 触发或手动带 VERSION，Release 走 GitHub API |
| **SourceHut** | `.build.yml` | builds.sr.ht 构建清单，Release 走 GitHub API |

## 通用配置

各平台均需配置以下 **Secrets / 变量**：

- **SOURCE_REPO_TOKEN**：对**源仓库**有写权限的 token（用于推送版本 tag 和创建 Release）。
- （可选）**SOURCE_REPO_OWNER** / **SOURCE_REPO_NAME**：源仓库，默认从当前仓库推断。

### 各平台 Token 说明

| 平台 | Token 类型 | 用途 |
|------|------------|------|
| GitHub Actions | `GITHUB_TOKEN` | 自动可用，无需配置 |
| GitLab CI | `GITLAB_TOKEN` (或 `SOURCE_REPO_TOKEN`) | 需要有 repo 写权限的 Personal Access Token |
| Azure DevOps | `SOURCE_REPO_TOKEN` | GitHub token（用于在 GitHub 创建 Release） |
| Bitbucket | `SOURCE_REPO_TOKEN` | Bitbucket repository token |
| SourceHut | `SOURCE_REPO_TOKEN` | GitHub token（用于在 GitHub 创建 Release） |

## 触发方式

- **手动**：在对应平台选择"运行 pipeline / workflow"，并填入版本号（如 `0.4.4`）。
- **自动**：当 `rust/tgbot/Cargo.toml` 发生变更并 push 到默认分支时触发。

## 各平台说明

- **GitHub**：使用 `GITHUB_TOKEN` 在源仓库创建 Release，上传 `tgbot` 和 `installer` 作为 assets。自动保留最近 3 个 Release。
- **GitLab**：使用 GitLab API 在源项目创建 Release，附件为"链接"形式指向 main 分支上的文件。
- **Bitbucket**：使用 Downloads API 上传 `tgbot` 和 `installer` 到源仓库。
- **Azure**：使用 GitHub API 在 GitHub 源仓库创建 Release（需配置 GitHub token）。
- **SourceHut**：使用 GitHub API 在 GitHub 源仓库创建 Release（需配置 GitHub token）。

所有工作流均可实现：解析/写入版本、打 tag、构建 tgbot（Rust + UPX）与 installer（Go + Garble）、创建 Release 并附带两个二进制文件。

## Release 下载

installer 和 tgbot 运行时自更新会从以下默认仓库下载：

- `https://github.com/NicholasDewar/Wuthering_Waves_Private_Server/releases/latest/download/installer`
- `https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest/download/installer`（备用）

可通过环境变量覆盖：
- `TGBOT_RELEASE_REPOSITORIES`（逗号分隔的 `owner/repo` 列表）
- `TGBOT_RELEASE_OWNER` + `TGBOT_RELEASE_REPO`
- `TGBOT_RELEASE_MIRRORS`（逗号分隔的 API 根地址列表）