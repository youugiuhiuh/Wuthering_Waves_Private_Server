#!/usr/bin/env bash
# 版本同步：以 rust/aegis/Cargo.toml 为唯一真源，同步到其它声明位置。
#
# 取代已废弃的 rust/version-sync（该 crate 的正则要求 "v" 前缀，
# 而文件实际格式无前缀，导致它静默通过且从不生效）。
#
# 真源：  rust/aegis/Cargo.toml   [package] version
# 跟随：  go/installer/main.go    const version
#         rust/aegis/Cargo.lock   aegis 包的 version（cargo check 重新生成）
#
# Rust 二进制内部的版本走 env!("CARGO_PKG_VERSION")，编译期自动注入，无需处理。
#
# 用法：
#   scripts/sync-version.sh --check   仅校验，不一致则退出 1（CI / pre-commit）
#   scripts/sync-version.sh --fix     就地修正跟随位置（本地）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO="$ROOT/rust/aegis/Cargo.toml"
GO="$ROOT/go/installer/main.go"
LOCK="$ROOT/rust/aegis/Cargo.lock"

MODE="${1:---check}"
case "$MODE" in
--check | --fix) ;;
*)
  echo "用法: $0 [--check|--fix]" >&2
  exit 2
  ;;
esac

# ---- 读取真源 ----
SRC_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CARGO" | head -1)"
if [ -z "$SRC_VERSION" ]; then
  echo "❌ 无法从 $CARGO 解析 version" >&2
  exit 1
fi

# ---- 读取跟随位置（go 的 const 缩进对齐，故允许前导空白）----
GO_VERSION="$(sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([0-9][^"]*\)".*/\1/p' "$GO" | head -1)"
if [ -z "$GO_VERSION" ]; then
  echo "❌ 无法从 $GO 解析 version 常量" >&2
  exit 1
fi

# go 文件里若出现第二个独立的 version 常量，说明结构变了，需人工确认
GO_HITS="$(grep -c '^[[:space:]]*version[[:space:]]*=' "$GO" || true)"
if [ "$GO_HITS" -ne 1 ]; then
  echo "❌ $GO 中匹配到 $GO_HITS 处 version 声明（预期 1 处）" >&2
  exit 1
fi

echo "真源 Cargo.toml : $SRC_VERSION"
echo "跟随 main.go    : $GO_VERSION"

# ---- 校验 ----
if [ "$SRC_VERSION" = "$GO_VERSION" ]; then
  echo "✅ 版本一致: $SRC_VERSION"
  exit 0
fi

if [ "$MODE" = "--check" ]; then
  echo "❌ 版本不一致: rust=$SRC_VERSION go=$GO_VERSION" >&2
  echo "   修正: scripts/sync-version.sh --fix" >&2
  exit 1
fi

# ---- 修正 ----
echo ">>> 修正 go/installer/main.go: $GO_VERSION -> $SRC_VERSION"
# 保留原有对齐（const 块里 version 后跟多个空格）；只替换引号内的版本串
sed -i "s|^\([[:space:]]*version[[:space:]]*=[[:space:]]*\)\"[0-9][^\"]*\"|\1\"$SRC_VERSION\"|" "$GO"

# Cargo.lock 里的 aegis 版本由 cargo 重新生成，不手工 sed
if [ -f "$LOCK" ]; then
  echo ">>> 重新生成 Cargo.lock"
  (cd "$ROOT/rust/aegis" && cargo check --quiet)
fi

echo "✅ 已同步到 $SRC_VERSION"
echo "   请 git add 以下文件后重新提交:"
echo "     go/installer/main.go"
echo "     rust/aegis/Cargo.lock"
