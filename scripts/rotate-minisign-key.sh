#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO_FILE="$ROOT/go/installer/minisign_verify.go"
RS_FILE="$ROOT/rust/aegis/src/core/crypto/minisign.rs"

if ! command -v minisign &>/dev/null; then
    echo "请先安装 minisign: brew install minisign / apt install minisign"
    exit 1
fi

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo ">>> 生成新的 Minisign 密钥对（无密码）..."
minisign -G -W -p "$TEMP_DIR/minisign.pub" -s "$TEMP_DIR/minisign.key"
NEW_KEY=$(grep -v '^untrusted comment' "$TEMP_DIR/minisign.pub" | tr -d '\n')
echo ">>> 新公钥: $NEW_KEY"

# ---------- Go ----------
# 读取现有密钥
mapfile -t GO_KEYS < <(sed -n '/^var minisignPublicKeys/,/^}/p' "$GO_FILE" | grep '"' | sed 's/.*"\(.*\)".*/\1/')
GO_KEYS+=("$NEW_KEY")

# 生成新代码块
printf 'var minisignPublicKeys = []string{\n' > "$TEMP_DIR/go_block"
for k in "${GO_KEYS[@]}"; do printf '\t"%s",\n' "$k" >> "$TEMP_DIR/go_block"; done
printf '}\n' >> "$TEMP_DIR/go_block"

# 替换
awk -v block="$TEMP_DIR/go_block" '
    /^var minisignPublicKeys/ { printing=1; next }
    printing && /^}/ { printing=0; system("cat " block); next }
    !printing { print }
' "$GO_FILE" > "$TEMP_DIR/go_new.go" && cp "$TEMP_DIR/go_new.go" "$GO_FILE"
gofmt -w "$GO_FILE" > /dev/null

# ---------- Rust ----------
mapfile -t RS_KEYS < <(sed -n '/^pub const MINISIGN_PUBLIC_KEYS/,/^];/p' "$RS_FILE" | grep '"' | sed 's/.*"\(.*\)".*/\1/')
RS_KEYS+=("$NEW_KEY")

printf 'pub const MINISIGN_PUBLIC_KEYS: &[&str] = &[\n' > "$TEMP_DIR/rs_block"
for k in "${RS_KEYS[@]}"; do printf '    "%s",\n' "$k" >> "$TEMP_DIR/rs_block"; done
printf '];\n' >> "$TEMP_DIR/rs_block"

awk -v block="$TEMP_DIR/rs_block" '
    /^pub const MINISIGN_PUBLIC_KEYS/ { printing=1; next }
    printing && /^];/ { printing=0; system("cat " block); next }
    !printing { print }
' "$RS_FILE" > "$TEMP_DIR/rs_new.rs" && cp "$TEMP_DIR/rs_new.rs" "$RS_FILE"
rustfmt "$RS_FILE"

echo ""
echo "============================================"
echo "✅ 新公钥已追加，共有 ${#GO_KEYS[@]} 个密钥在轮换中"
echo "   新旧签名均可通过验证"
echo ""
echo "将以下私钥添加到 GitHub Secrets（production Environment → MINISIGN_SECRET_KEY）："
echo "============================================"
cat "$TEMP_DIR/minisign.key"
echo "============================================"
echo ""
echo "!!! 请勿泄露私钥！建议运行: history -c !!!"
echo "待所有客户端升级后，手动移除旧公钥即可完成轮换。"
