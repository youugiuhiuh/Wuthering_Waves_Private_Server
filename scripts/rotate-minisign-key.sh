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

# 计算过期时间（当前日期 + 90 天）
EXPIRES=$(date -d "+90 days" +%Y-%m-%d)
EXPIRES=$(date -d "+1 year" +%Y-%m-%d)
echo ">>> 过期日期: $EXPIRES"

# ---------- Go ----------
mapfile -t GO_KEYS < <(awk '
    /^var minisignPublicKeys/ { printing=1; next }
    printing && /^}/ { exit }
    printing && /PublicKey:/ {
        gsub(/.*"([^"]+)".*/, "\\1"); print
    }
    printing && /ExpiresAt:/ {
        gsub(/.*"([^"]+)".*/, "\\1"); print
    }
' "$GO_FILE")

# Rebuild Go entries in pairs (key, expires)
printf 'var minisignPublicKeys = []minisignKeyEntry{\n' > "$TEMP_DIR/go_block"
i=0
while [ $i -lt ${#GO_KEYS[@]} ]; do
    key="${GO_KEYS[$i]}"
    exp="${GO_KEYS[$((i+1))]}"
    printf '\t{PublicKey: "%s", ExpiresAt: "%s"},\n' "$key" "$exp" >> "$TEMP_DIR/go_block"
    i=$((i+2))
done
printf '\t{PublicKey: "%s", ExpiresAt: "%s"},\n' "$NEW_KEY" "$EXPIRES" >> "$TEMP_DIR/go_block"
printf '}\n' >> "$TEMP_DIR/go_block"

awk -v block="$TEMP_DIR/go_block" '
    /^var minisignPublicKeys/ { printing=1; next }
    printing && /^}/ { printing=0; system("cat " block); next }
    !printing { print }
' "$GO_FILE" > "$TEMP_DIR/go_new.go" && cp "$TEMP_DIR/go_new.go" "$GO_FILE"
gofmt -w "$GO_FILE"

# ---------- Rust ----------
mapfile -t RS_KEYS < <(awk '
    /^pub const MINISIGN_PUBLIC_KEYS/ { printing=1; next }
    printing && /^];/ { exit }
    printing && /public_key:/ {
        gsub(/.*"([^"]+)".*/, "\\1"); print
    }
    printing && /expires_at:/ {
        gsub(/.*"([^"]+)".*/, "\\1"); print
    }
' "$RS_FILE")

printf 'pub const MINISIGN_PUBLIC_KEYS: &[MinisignKeyEntry] = &[\n' > "$TEMP_DIR/rs_block"
i=0
while [ $i -lt ${#RS_KEYS[@]} ]; do
    key="${RS_KEYS[$i]}"
    exp="${RS_KEYS[$((i+1))]}"
    printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s" },\n' "$key" "$exp" >> "$TEMP_DIR/rs_block"
    i=$((i+2))
done
printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s" },\n' "$NEW_KEY" "$EXPIRES" >> "$TEMP_DIR/rs_block"
printf '];\n' >> "$TEMP_DIR/rs_block"

awk -v block="$TEMP_DIR/rs_block" '
    /^pub const MINISIGN_PUBLIC_KEYS/ { printing=1; next }
    printing && /^];/ { printing=0; system("cat " block); next }
    !printing { print }
' "$RS_FILE" > "$TEMP_DIR/rs_new.rs" && cp "$TEMP_DIR/rs_new.rs" "$RS_FILE"
rustfmt "$RS_FILE"

echo ""
echo "============================================"
echo "✅ 新公钥已追加，过期日期: $EXPIRES"
echo "   过期后自动失效，无需手动删除"
echo ""
echo "将以下私钥添加到 GitHub Secrets（production Environment → MINISIGN_SECRET_KEY）："
echo "============================================"
cat "$TEMP_DIR/minisign.key"
echo "============================================"
echo ""
echo "!!! 请勿泄露私钥！建议运行: history -c !!!"
