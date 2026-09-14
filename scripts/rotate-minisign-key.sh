#!/usr/bin/env bash
# Minisign 密钥轮换。
#
# 硬化后的两个关键行为（见 docs/superpowers/specs/2026-09-13-minisign-hardening.md §7.1）：
#
#   1. 旧公钥【退役】到历史表，而不是删除。
#      历史公钥永久保留，用于验证「用旧钥签的历史版本」。活跃/历史是两张
#      独立的表：历史表的既有条目【跨轮换累积】，本次轮换不会覆盖掉它们。
#      退役判据：到期日 < now + 90 天（keep_key）。
#
#   2. 私钥【不打印】到终端，改为写入权限 600 的独立文件。
#      此前 `cat` 到 stdout 会把私钥写进终端回滚缓冲与 shell history。
#
# 实现注意（踩过的坑）：
#   - awk 必须是 mawk 兼容写法。gawk 专有的 `match(str, re, arr)` 3 参形式
#     在 mawk 上是语法错误，会让读取 pass 静默拿到空列表。
#   - 区块替换用「起始行 + 收尾行」定位。收尾行形态因语言/格式化而异：
#       Go   : 单行 `}`；单行形态 `...Entry{}`
#       Rust : `}];`（当前）或 `];`（rustfmt 后）；单行形态 `...&[];`
#   - 写回前必须校验必需符号仍在，否则宁可中止也不写坏源码：
#     锚点若失配，替换 pass 会从起始行一路吞到下一个收尾行，把中间的函数删光。
#
# 环境变量（仅测试需要，正常使用无需设置）：
#   ROTATE_GO_FILE  Go 密钥表文件路径（默认仓库内真实文件）
#   ROTATE_RS_FILE  Rust 密钥表文件路径（同上）
#   ROTATE_KEY_OUT  新私钥落盘路径（默认 $HOME/minisign-new.key）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO_FILE="${ROTATE_GO_FILE:-$ROOT/go/installer/minisign_verify.go}"
RS_FILE="${ROTATE_RS_FILE:-$ROOT/rust/aegis/src/core/crypto/minisign.rs}"
KEY_OUT="${ROTATE_KEY_OUT:-$HOME/minisign-new.key}"

if ! command -v minisign &>/dev/null; then
    echo "请先安装 minisign: brew install minisign / apt install minisign"
    exit 1
fi

for f in "$GO_FILE" "$RS_FILE"; do
    [ -f "$f" ] || {
        echo "::error::找不到密钥表文件: $f" >&2
        exit 1
    }
done

# rustfmt 必须用项目声明的 edition，否则会引入与 cargo fmt 不一致的无关格式差异。
# 实测：默认 edition 会把 `use anyhow::{Result, anyhow};` 重排成 `{anyhow, Result}`，
# 导致轮换顺带改动一行与密钥无关的代码。项目当前为 2024。
RS_EDITION="2024"
if [ -f "$ROOT/rust/aegis/Cargo.toml" ]; then
    _detected=$(sed -n 's/^edition *= *"\([^"]*\)".*/\1/p' "$ROOT/rust/aegis/Cargo.toml" | head -1)
    [ -n "$_detected" ] && RS_EDITION="$_detected"
fi

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo ">>> 生成新的 Minisign 密钥对（无密码）..."
minisign -G -W -p "$TEMP_DIR/minisign.pub" -s "$TEMP_DIR/minisign.key"
NEW_KEY=$(grep -v '^untrusted comment' "$TEMP_DIR/minisign.pub" | tr -d '\n')
echo ">>> 新公钥: $NEW_KEY"

EXPIRES=$(date -d "+1 year" +%Y-%m-%d)
RETIRED=$(date +%Y-%m-%d)
echo ">>> 新密钥过期日期: $EXPIRES"

# 判断某公钥是否仍应【保持活跃】。运行脚本 = 主动轮换：≤90 天到期即退役。
# 注意：退役 ≠ 删除 —— 退役后移入历史表并永久保留。
keep_key() {
    local expires="$1"
    local now_epoch exp_epoch keep_before
    now_epoch=$(date +%s)
    exp_epoch=$(date -d "$expires" +%s 2>/dev/null || return 1)
    keep_before=$((now_epoch + 90 * 86400))
    [ "$exp_epoch" -ge "$keep_before" ]
}

# ---- 通用：从密钥表区块读出已有条目 ----
# 输出每行 "KEY<TAB>EXPIRES<TAB>RETIRED"。
# 兼容两种排版：三字段同行（Go、以及 rustfmt 后的 Rust），
# 或每字段各占一行（当前 Rust 源码）。
# 做法：按引号切分，把「标签段 → 值段」配对，而不是假定字段位置。
read_block() {
    local file="$1" start="$2" closeend="$3" endline="$4"
    awk -v start="$start" -v closeend="$closeend" -v endline="$endline" '
        function p_flush() {
            if (pkey != "") print pkey "\t" pexp "\t" pret
            pkey = ""; pexp = ""; pret = ""
        }
        function p_parse(line,   parts, n, i, lbl, val) {
            n = split(line, parts, "\"")
            for (i = 1; i < n; i += 2) {
                lbl = parts[i]; val = parts[i + 1]
                if      (lbl ~ /[Pp]ublic_?[Kk]ey:/)  { p_flush(); pkey = val }
                else if (lbl ~ /[Ee]xpires_?[Aa]t:/)  { pexp = val }
                else if (lbl ~ /[Rr]etired_?[Aa]t:/)  { pret = val }
            }
        }
        $0 ~ start {
            if ($0 ~ closeend) { p_parse($0); p_flush(); next }
            skipping = 1; next
        }
        skipping && $0 ~ endline { skipping = 0; p_flush(); next }
        skipping { p_parse($0) }
        END { p_flush() }
    ' "$file"
}

# ---- 通用：用新内容替换某个区块 ----
# start   : 区块起始行正则
# closeend: 「本行即以收尾符号结束」的正则 —— 用于识别单行形态
# endline : 多行形态的收尾行正则
replace_block() {
    local file="$1" start="$2" closeend="$3" endline="$4" blockfile="$5"
    awk -v start="$start" -v closeend="$closeend" -v endline="$endline" -v blockfile="$blockfile" '
        function p_emit() { system("cat " blockfile) }
        $0 ~ start {
            if ($0 ~ closeend) { p_emit(); next }
            skipping = 1; next
        }
        skipping && $0 ~ endline { skipping = 0; p_emit(); next }
        !skipping { print }
    ' "$file"
}

# ---- 通用：写回前校验 ----
# 直接拒绝「区块替换把源码写坏」这一后果：任何必需符号消失即中止。
# 此时真实文件尚未被触碰，安全。
validate_output() {
    local file="$1" label="$2"
    shift 2
    local sym
    for sym in "$@"; do
        if ! grep -q -- "$sym" "$file"; then
            echo "::error::$label 轮换后丢失符号 '$sym'；已中止，原文件未被修改" >&2
            return 1
        fi
    done
    return 0
}

GO_END='^}$'
GO_CLOSE='\}$'
RS_END='^[}\]]{1,2};$'
RS_CLOSE='\];$'

# ================= Go =================
GO_ACTIVE_BLOCK="$TEMP_DIR/go_active"
GO_HIST_BLOCK="$TEMP_DIR/go_hist"
printf 'var minisignActiveKeys = []minisignKeyEntry{\n' >"$GO_ACTIVE_BLOCK"
printf 'var minisignHistoricalKeys = []minisignKeyEntry{\n' >"$GO_HIST_BLOCK"

GO_ACTIVE_N=0
GO_RETIRED_N=0
GO_HIST_N=0

# 原活跃表：仍有效 → 留活跃；临近/已过期 → 退役进历史
while IFS=$'\t' read -r key exp retired; do
    [ -n "${key:-}" ] || continue
    if keep_key "$exp"; then
        printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: ""},\n' \
            "$key" "$exp" >>"$GO_ACTIVE_BLOCK"
        GO_ACTIVE_N=$((GO_ACTIVE_N + 1))
    else
        printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: "%s"},\n' \
            "$key" "$exp" "$RETIRED" >>"$GO_HIST_BLOCK"
        GO_RETIRED_N=$((GO_RETIRED_N + 1))
        echo ">>> 退役旧公钥 (Go): $key ($exp)"
    fi
done < <(read_block "$GO_FILE" '^var minisignActiveKeys' "$GO_CLOSE" "$GO_END")

# 原历史表：原样累积保留（跨轮换不丢）
while IFS=$'\t' read -r key exp retired; do
    [ -n "${key:-}" ] || continue
    printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: "%s"},\n' \
        "$key" "$exp" "$retired" >>"$GO_HIST_BLOCK"
    GO_HIST_N=$((GO_HIST_N + 1))
done < <(read_block "$GO_FILE" '^var minisignHistoricalKeys' "$GO_CLOSE" "$GO_END")

# 新公钥进活跃表
printf '\t{PublicKey: "%s", ExpiresAt: "%s", RetiredAt: ""},\n' \
    "$NEW_KEY" "$EXPIRES" >>"$GO_ACTIVE_BLOCK"
GO_ACTIVE_N=$((GO_ACTIVE_N + 1))
printf '}\n' >>"$GO_ACTIVE_BLOCK"
printf '}\n' >>"$GO_HIST_BLOCK"

replace_block "$GO_FILE" '^var minisignActiveKeys' "$GO_CLOSE" "$GO_END" "$GO_ACTIVE_BLOCK" \
    >"$TEMP_DIR/go_step1.go"
replace_block "$TEMP_DIR/go_step1.go" '^var minisignHistoricalKeys' "$GO_CLOSE" "$GO_END" "$GO_HIST_BLOCK" \
    >"$TEMP_DIR/go_new.go"

if command -v gofmt >/dev/null 2>&1; then
    gofmt -w "$TEMP_DIR/go_new.go" 2>/dev/null || echo "::warning::gofmt 失败，保留未格式化内容"
fi

validate_output "$TEMP_DIR/go_new.go" "go/installer/minisign_verify.go" \
    verifyMinisign parseTrustedComment requireMinisign matchTrustedComment \
    minisignActiveKeys minisignHistoricalKeys || exit 1

cp "$TEMP_DIR/go_new.go" "$GO_FILE"

# ================= Rust =================
RS_ACTIVE_BLOCK="$TEMP_DIR/rs_active"
RS_HIST_BLOCK="$TEMP_DIR/rs_hist"
printf 'pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry] = &[\n' >"$RS_ACTIVE_BLOCK"
printf 'pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry] = &[\n' >"$RS_HIST_BLOCK"

RS_ACTIVE_N=0
RS_RETIRED_N=0
RS_HIST_N=0

while IFS=$'\t' read -r key exp retired; do
    [ -n "${key:-}" ] || continue
    if keep_key "$exp"; then
        printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "" },\n' \
            "$key" "$exp" >>"$RS_ACTIVE_BLOCK"
        RS_ACTIVE_N=$((RS_ACTIVE_N + 1))
    else
        printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "%s" },\n' \
            "$key" "$exp" "$RETIRED" >>"$RS_HIST_BLOCK"
        RS_RETIRED_N=$((RS_RETIRED_N + 1))
        echo ">>> 退役旧公钥 (Rust): $key ($exp)"
    fi
done < <(read_block "$RS_FILE" '^pub const MINISIGN_ACTIVE_KEYS' "$RS_CLOSE" "$RS_END")

while IFS=$'\t' read -r key exp retired; do
    [ -n "${key:-}" ] || continue
    printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "%s" },\n' \
        "$key" "$exp" "$retired" >>"$RS_HIST_BLOCK"
    RS_HIST_N=$((RS_HIST_N + 1))
done < <(read_block "$RS_FILE" '^pub const MINISIGN_HISTORICAL_KEYS' "$RS_CLOSE" "$RS_END")

printf '    MinisignKeyEntry { public_key: "%s", expires_at: "%s", retired_at: "" },\n' \
    "$NEW_KEY" "$EXPIRES" >>"$RS_ACTIVE_BLOCK"
RS_ACTIVE_N=$((RS_ACTIVE_N + 1))
printf '];\n' >>"$RS_ACTIVE_BLOCK"
printf '];\n' >>"$RS_HIST_BLOCK"

replace_block "$RS_FILE" '^pub const MINISIGN_ACTIVE_KEYS' "$RS_CLOSE" "$RS_END" "$RS_ACTIVE_BLOCK" \
    >"$TEMP_DIR/rs_step1.rs"
replace_block "$TEMP_DIR/rs_step1.rs" '^pub const MINISIGN_HISTORICAL_KEYS' "$RS_CLOSE" "$RS_END" "$RS_HIST_BLOCK" \
    >"$TEMP_DIR/rs_new.rs"

if command -v rustfmt >/dev/null 2>&1; then
    rustfmt --edition "$RS_EDITION" "$TEMP_DIR/rs_new.rs" 2>/dev/null \
        || echo "::warning::rustfmt 失败，保留未格式化内容"
fi

validate_output "$TEMP_DIR/rs_new.rs" "rust/aegis/src/core/crypto/minisign.rs" \
    verify_minisign parse_trusted_comment key_expired \
    MINISIGN_ACTIVE_KEYS MINISIGN_HISTORICAL_KEYS || exit 1

cp "$TEMP_DIR/rs_new.rs" "$RS_FILE"

# ================= 私钥落盘（不打印）=================
mkdir -p "$(dirname "$KEY_OUT")"
cp "$TEMP_DIR/minisign.key" "$KEY_OUT"
chmod 600 "$KEY_OUT"

echo ""
echo "============================================"
echo "✅ 密钥轮换完成"
echo "   Go:   活跃 $GO_ACTIVE_N 个（本次退役 $GO_RETIRED_N 个），历史累计 $GO_HIST_N 个"
echo "   Rust: 活跃 $RS_ACTIVE_N 个（本次退役 $RS_RETIRED_N 个），历史累计 $RS_HIST_N 个"
echo "   新密钥过期日期: $EXPIRES"
echo ""
echo "私钥已写入: $KEY_OUT（权限 600）"
echo "  已【不打印】其内容 —— 避免泄露到终端回滚缓冲与 shell history。"
echo "  请手动打开该文件，把内容填入 CI Secret: MINISIGN_SECRET_KEY"
echo "  填入后建议删除该文件: rm -f \"$KEY_OUT\""
echo ""
echo "⚠ 历史公钥永久保留，请勿手工删除 —— 删掉后旧版本将无法通过签名校验。"
echo "============================================"
