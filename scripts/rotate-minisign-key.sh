#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if ! command -v minisign &>/dev/null; then
    echo "请先安装 minisign:"
    echo "  brew install minisign   # macOS"
    echo "  apt install minisign    # Debian/Ubuntu"
    exit 1
fi

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo ">>> 生成新的 Minisign 密钥对（无密码）..."
minisign -G -W -p "$TEMP_DIR/minisign.pub" -s "$TEMP_DIR/minisign.key"

PUBLIC_KEY=$(cat "$TEMP_DIR/minisign.pub" | grep -v '^untrusted comment')
echo ">>> 新公钥: $PUBLIC_KEY"

# 转义公钥中的斜杠（sed 安全）
ESCAPED_KEY=$(echo "$PUBLIC_KEY" | sed 's|/|\\/|g')

echo ">>> 更新 Go 公钥..."
sed -i '/^var minisignPublicKeys = \[\]string{/,/^}/c\
var minisignPublicKeys = []string{\
\t"'"$ESCAPED_KEY"'",\
}' "$ROOT/go/installer/minisign_verify.go"

echo ">>> 更新 Rust 公钥..."
sed -i '/^pub const MINISIGN_PUBLIC_KEYS:.*$/,/^];/c\
pub const MINISIGN_PUBLIC_KEYS: &[&str] = &[\
    "'"$PUBLIC_KEY"'",\
];' "$ROOT/rust/aegis/src/core/crypto/minisign.rs"
rustfmt "$ROOT/rust/aegis/src/core/crypto/minisign.rs"

echo ""
echo "============================================"
echo "✅ 公钥已更新至代码中！"
echo ""
echo "将以下私钥添加到 GitHub Secrets（production Environment → MINISIGN_SECRET_KEY）："
echo "============================================"
cat "$TEMP_DIR/minisign.key"
echo "============================================"
echo ""
echo "!!! 请勿泄露私钥！完成后建议运行:  history -c  !!!"
