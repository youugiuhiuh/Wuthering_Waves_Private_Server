# 多平台 CI/CD 工作流索引

本仓库提供与 **GitHub Actions** `.github/workflows/public-release.yml` **功能等效**的配置，用于在其它代码托管上构建并发布公开分发包（tgbot + installer）。

## 文件一览

| 平台 | 配置文件 | 说明 |
|------|----------|------|
| **GitHub** | `.github/workflows/public-release.yml` | 原始工作流，手动或 push `rust/tgbot/Cargo.toml` 触发 |
| **Gitea / Forgejo** | `.gitea/workflows/public-release.yml` | 无 GitHub 依赖，纯 shell + Gitea API |
| **Gitea / Forgejo** | `.gitea/workflows/public-release-actions.yml` | 使用 GitHub Actions 兼容语法，需 Runner 能访问 GitHub |
| **GitLab** | `.gitlab-ci.yml` | GitLab CI/CD，Release 使用 GitLab API |
| **Bitbucket** | `bitbucket-pipelines.yml` | Bitbucket Pipelines，main 分支或自定义 pipeline 手动带 VERSION |
| **Azure DevOps** | `azure-pipelines.yml` | Azure Pipelines，path 触发或手动带 VERSION，公开仓 Release 走 GitHub API |
| **SourceHut** | `.build.yml` | builds.sr.ht 构建清单，公开仓 Release 走 GitHub API |

## 通用配置

各平台均需配置以下** Secrets / 变量**（名称可能略有不同）：

- **SOURCE_REPO_TOKEN**：拉取/推送**当前源码仓库**的 token（写权限）。
- **PUBLIC_REPO_TOKEN**：推送到**公开仓库**并创建/删除 Release 的 token。
- （可选）**PUBLIC_REPO_OWNER** / **PUBLIC_REPO_NAME**：公开仓库，默认 `youugiuhiuh` / `Wuthering_Waves_Private_Server`。
- （可选）**PUBLIC_SERVER_URL**：公开仓所在实例 API 根（如 `https://api.github.com` 或 GitLab 地址），用于 Azure / 部分 Gitea 场景。

## 触发方式

- **手动**：在对应平台选择“运行 pipeline / workflow”，并填入版本号（如 `0.4.4`）；Gitea/Forgejo、GitLab、Bitbucket 自定义 pipeline、Azure、SourceHut 均支持。
- **自动**：当 `rust/tgbot/Cargo.toml` 发生变更并 push 到默认分支时触发（GitHub、Gitea、GitLab、Azure 支持 path 过滤；Bitbucket 当前为 main 分支 push 即触发）。

## 各平台说明

- **Gitea/Forgejo**：见 `.gitea/workflows/README.md`。
- **GitLab**：Release 使用 GitLab Releases API，附件为“链接”形式指向公开仓 main 分支上的文件。
- **Bitbucket**：公开仓推送 + 使用 Downloads API 上传 tgbot/installer；无 GitHub 式 Release 时可仅用 Downloads。
- **Azure**：默认假定公开仓在 GitHub，清理与创建 Release 使用 GitHub API；若公开仓在 GitLab 等，需在脚本中增加对应 API 逻辑。
- **SourceHut**：需将 `sources` 与 push 地址中的 `~youruser/your-repo` 改为实际 git.sr.ht 仓库；Secrets 需在 builds.sr.ht 配置并在提交时传入 SOURCE_REPO_TOKEN、PUBLIC_REPO_TOKEN。

所有工作流均可实现：解析/写入版本、打 tag、构建 tgbot（Rust + UPX）与 installer（Go + Garble）、推送到公开仓、清理旧 Release/tag、创建新 Release 并附带两个二进制文件。
