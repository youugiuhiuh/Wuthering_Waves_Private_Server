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

EXPIRES=$(date -d "+1 year" +%Y-%m-%d)
echo ">>> 过期日期: $EXPIRES"

# 判断密钥是否仍应保留（空=永不过期）
# 运行脚本 = 正在主动轮换，到期或即将到期（≤90天）的密钥都将移除
keep_key() {
    local expires="$1"
    if [ -z "$expires" ]; then return 0; fi
    local now_epoch=$(date +%s)
    local exp_epoch=$(date -d "$expires" +%s 2>/dev/null || return 0)
    local keep_before=$((now_epoch + 90 * 86400))
    [ "$exp_epoch" -ge "$keep_before" ]
}

# ---------- Go ----------
mapfile -t GO_KEYS < <(awk '
    /^var minisignPublicKeys/ { printing=1; next }
    printing && /^\s*}\s*$/ { exit }
    printing && /PublicKey:/ {
        line = $0
        match(line, /PublicKey: "([^"]+)"/, a)
        print a[1]
        if (match(line, /ExpiresAt: "([^"]*)"/, b)) {
            print b[1]
        } else {
            print ""
        }
    }
' "$GO_FILE")

GO_KEPT=0 GO_REMOVED=0
printf 'var minisignPublicKeys = []minisignKeyEntry{\n' > "$TEMP_DIR/go_block"
for ((i = 0; i < ${#GO_KEYS[@]}; i += 2)); do
    KEY="${GO_KEYS[$i]}"
    EXP="${GO_KEYS[$((i+1))]}"
    if keep_key "$EXP"; then
        printf '\t{PublicKey: "%s", ExpiresAt: "%s"},\n' "$KEY" "$EXP" >> "$TEMP_DIR/go_block"
        GO_KEPT=$((GO_KEPT + 1))
    else
        echo ">>> 移除旧公钥 (Go): $KEY ($EXP)"
        GO_REMOVED=$((GO_REMOVED + 1))
    fi
done
printf '\t{PublicKey: "%s", ExpiresAt: "%s"},\n' "$NEW_KEY" "$EXPIRES" >> "$TEMP_DIR/go_block"
GO_KEPT=$((GO_KEPT + 1))
printf '}\n' >> "$TEMP_DIR/go_block"

awk -v block="$TEMP_DIR/go_block" '
    /^var minisignPublicKeys/ { printing=1; next }
    printing && /^\s*}\s*$/ { printing=0; system("cat " block); next }
    !printing { print }
' "$GO_FILE" > "$TEMP_DIR/go_new.go" && cp "$TEMP_DIR/go_new.go" "$GO_FILE"
gofmt -w "$GO_FILE"

# ---------- Rust ----------
mapfile -t RS_KEYS < <(awk '
    /^pub const MINISIGN_PUBLIC_KEYS/ { printing=1; next }
    printing && /];\s*$/ { exit }
    printing && /public_key:/ {
        line = $0
        match(line, /public_key: "([^"]+)"/, a)
        print a[1]
        if (match(line, /expires_at: "([^"]*)"/, b)) {
            print b[1]
        } else {
            print ""
        }
    }
' "$RS_FILE")

RS_KEPT=0 RS_REMOVED=0
printf 'pub const MINISIGN_PUBLIC_KEYS: &[MinisignKeyEntry] = &[\n' > "$TEMP_DIR/rs_block"
for ((i = 0; i < ${#RS_KEYS[@]}; i += 2)); do
    KEY="${RS_KEYS[$i]}"
    EXP="${RS_KEYS[$((i+1))]}"
    if keep_key "$EXP"; then
        printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s" },\n' "$KEY" "$EXP" >> "$TEMP_DIR/rs_block"
        RS_KEPT=$((RS_KEPT + 1))
    else
        echo ">>> 移除旧公钥 (Rust): $KEY ($EXP)"
        RS_REMOVED=$((RS_REMOVED + 1))
    fi
done
printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s" },\n' "$NEW_KEY" "$EXPIRES" >> "$TEMP_DIR/rs_block"
RS_KEPT=$((RS_KEPT + 1))
printf '];\n' >> "$TEMP_DIR/rs_block"

awk -v block="$TEMP_DIR/rs_block" '
    /^pub const MINISIGN_PUBLIC_KEYS/ { printing=1; next }
    printing && /];\s*$/ { printing=0; system("cat " block); next }
    !printing { print }
' "$RS_FILE" > "$TEMP_DIR/rs_new.rs" && cp "$TEMP_DIR/rs_new.rs" "$RS_FILE"
rustfmt "$RS_FILE"

echo ""
echo "============================================"
echo "✅ 密钥轮换完成"
echo "   Go:   $GO_KEPT 个有效密钥（移除 $GO_REMOVED 个过期）"
echo "   Rust: $RS_KEPT 个有效密钥（移除 $RS_REMOVED 个过期）"
echo "   新密钥过期日期: $EXPIRES"
echo ""
echo "将以下私钥添加到 GitHub Secrets（production Environment → MINISIGN_SECRET_KEY）："
echo "============================================"
cat "$TEMP_DIR/minisign.key"
echo "============================================"
echo ""
echo "!!! 请勿泄露私钥！建议运行: history -c !!!"
