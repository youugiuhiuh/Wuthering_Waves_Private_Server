#!/usr/bin/env bash
# 一键发版脚本：自动升级版本号，同步 Cargo.toml、Cargo.lock 与 main.go，并提交推送。
#
# 用法：
#   ./scripts/release.sh          # 默认补丁升级 (patch)，如 2.2.6 -> 2.2.7
#   ./scripts/release.sh patch    # 同上
#   ./scripts/release.sh minor    # 次版本升级，如 2.2.6 -> 2.3.0
#   ./scripts/release.sh major    # 主版本升级，如 2.2.6 -> 3.0.0
#   ./scripts/release.sh 2.3.1    # 手动指定目标版本
#
# 可选参数：
#   --no-push                     # 只生成 commit，不自动 git push
#   --dry-run                     # 仅预览变更，不修改任何文件
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO="$ROOT/rust/aegis/Cargo.toml"
GO="$ROOT/go/installer/main.go"
LOCK="$ROOT/rust/aegis/Cargo.lock"
SYNC_SCRIPT="$ROOT/scripts/sync-version.sh"

TARGET_ARG=""
PUSH=true
DRY_RUN=false

for arg in "$@"; do
  case "$arg" in
    --no-push)
      PUSH=false
      ;;
    --dry-run)
      DRY_RUN=true
      ;;
    -h|--help)
      echo "用法: $0 [patch|minor|major|<version>] [--no-push] [--dry-run]"
      exit 0
      ;;
    *)
      if [ -z "$TARGET_ARG" ]; then
        TARGET_ARG="$arg"
      else
        echo "❌ 未知参数: $arg" >&2
        exit 1
      fi
      ;;
  esac
done

TARGET_ARG="${TARGET_ARG:-patch}"

# 1. 确保工作区干净
if ! git diff-index --quiet HEAD --; then
  echo "❌ Git 工作区存在未提交的修改，请先提交或 stash 现有改动。" >&2
  git status -s
  exit 1
fi

# 2. 读取当前版本号
CURRENT_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CARGO" | head -1)"
if [ -z "$CURRENT_VERSION" ]; then
  echo "❌ 无法从 $CARGO 读取当前版本号" >&2
  exit 1
fi

# 3. 计算新版本号
if [[ "$CURRENT_VERSION" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  V_MAJOR="${BASH_REMATCH[1]}"
  V_MINOR="${BASH_REMATCH[2]}"
  V_PATCH="${BASH_REMATCH[3]}"
else
  echo "❌ 当前版本号格式不是标准 semver (x.y.z): $CURRENT_VERSION" >&2
  exit 1
fi

TARGET_ARG="${TARGET_ARG#v}" # 去掉开头的 v

case "$TARGET_ARG" in
  patch)
    NEW_VERSION="${V_MAJOR}.${V_MINOR}.$((V_PATCH + 1))"
    ;;
  minor)
    NEW_VERSION="${V_MAJOR}.$((V_MINOR + 1)).0"
    ;;
  major)
    NEW_VERSION="$((V_MAJOR + 1)).0.0"
    ;;
  *)
    if [[ "$TARGET_ARG" =~ ^[0-9]+\.[0-9]+\.[0-9]+.*$ ]]; then
      NEW_VERSION="$TARGET_ARG"
    else
      echo "❌ 非法的目标版本: $TARGET_ARG (应为 patch, minor, major 或具体版本如 2.2.7)" >&2
      exit 1
    fi
    ;;
esac

if [ "$CURRENT_VERSION" = "$NEW_VERSION" ]; then
  echo "⚠️ 当前版本已是 $NEW_VERSION，无需更新。"
  exit 0
fi

echo "🚀 发版升级: $CURRENT_VERSION -> $NEW_VERSION"

if [ "$DRY_RUN" = true ]; then
  echo "（dry-run 模式：仅预览，不执行修改）"
  exit 0
fi

# 4. 修改 Cargo.toml 真源
echo ">>> 更新 $CARGO"
sed -i "s/^version *= *\".*\"/version = \"$NEW_VERSION\"/" "$CARGO"

# 5. 调用同步脚本（更新 main.go 与 Cargo.lock）
echo ">>> 调用 sync-version.sh 同步 Go 与 Cargo.lock"
bash "$SYNC_SCRIPT" --fix

# 6. 校验一致性
bash "$SYNC_SCRIPT" --check

# 7. Git 提交
echo ">>> 创建 Git Commit: $NEW_VERSION"
git add "$CARGO" "$GO" "$LOCK"
git commit -m "$NEW_VERSION"

# 8. 推送远端
if [ "$PUSH" = true ]; then
  CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  echo ">>> 推送至 origin $CURRENT_BRANCH"
  git push origin "$CURRENT_BRANCH"
  echo "🎉 发布提交已成功推送！GitHub Actions 将自动触发打包和 Release。"
else
  echo "✅ 本地 Commit 完成（已指定 --no-push，未推送远端）。"
fi
