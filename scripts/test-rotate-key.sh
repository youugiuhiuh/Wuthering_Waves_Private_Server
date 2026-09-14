#!/usr/bin/env bash
# 验证 rotate-minisign-key.sh 的不变量，以及它【不会把源码写坏】。
#
# 不轮换真实密钥：
#   - minisign 用桩替代（本机通常未安装 minisign）
#   - 目标文件用副本，通过 ROTATE_GO_FILE / ROTATE_RS_FILE 覆盖
#   - 真实文件的内容指纹在前后比对，必须完全不变
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO_FILE="$ROOT/go/installer/minisign_verify.go"
RS_FILE="$ROOT/rust/aegis/src/core/crypto/minisign.rs"
ROTATE="$ROOT/scripts/rotate-minisign-key.sh"

fail=0
check() { # 描述 条件
    if eval "$2"; then echo "  ✅ $1"; else echo "  ❌ $1"; fail=1; fi
}

echo "=== 1. rotate 脚本静态不变量 ==="
check "Go 存在 minisignActiveKeys" "grep -q 'minisignActiveKeys' '$GO_FILE'"
check "Go 存在 minisignHistoricalKeys" "grep -q 'minisignHistoricalKeys' '$GO_FILE'"
check "Rust 存在 MINISIGN_ACTIVE_KEYS" "grep -q 'MINISIGN_ACTIVE_KEYS' '$RS_FILE'"
check "Rust 存在 MINISIGN_HISTORICAL_KEYS" "grep -q 'MINISIGN_HISTORICAL_KEYS' '$RS_FILE'"
check "rotate 脚本不打印私钥内容" "! grep -q '^cat \"\$TEMP_DIR/minisign.key\"' '$ROTATE'"
check "rotate 脚本包含历史区块处理" "grep -q 'HISTORICAL\|historical' '$ROTATE'"

# ---------------------------------------------------------------
# 端到端：在副本上运行真实脚本
# ---------------------------------------------------------------
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin"

GO_SHA_BEFORE=$(sha256sum "$GO_FILE" | awk '{print $1}')
RS_SHA_BEFORE=$(sha256sum "$RS_FILE" | awk '{print $1}')

# 假 minisign：脚本只用到 -G -W -p/-s，产出可预测的密钥文件
cat >"$TMP/bin/minisign" <<'STUB'
#!/usr/bin/env bash
pub=""; sec=""
while [ $# -gt 0 ]; do
    case "$1" in
        -p) pub="$2"; shift 2 ;;
        -s) sec="$2"; shift 2 ;;
        *)  shift ;;
    esac
done
if [ -n "$pub" ]; then
    printf 'untrusted comment: minisign public key DEADBEEF\nRWQtestNEWKEYtestNEWKEYtestNEWKEYtestNEWKEYtestNEWKEY\n' >"$pub"
fi
if [ -n "$sec" ]; then
    printf 'untrusted comment: minisign encrypted secret key\nRWQtestSECRETKEY\n' >"$sec"
fi
exit 0
STUB
chmod +x "$TMP/bin/minisign"

SCENARIO_RC=0
run_scenario() { # $1=名称 $2=可选：把活跃钥到期日改成该值
    local name="$1" expiry="${2:-}"
    local dir="$TMP/$name"
    mkdir -p "$dir/go/installer" "$dir/rust/aegis/src/core/crypto"
    cp "$GO_FILE" "$dir/go/installer/minisign_verify.go"
    cp "$RS_FILE" "$dir/rust/aegis/src/core/crypto/minisign.rs"

    if [ -n "$expiry" ]; then
        sed -i "s/ExpiresAt: \"2027-07-02\"/ExpiresAt: \"$expiry\"/" \
            "$dir/go/installer/minisign_verify.go"
        sed -i "s/expires_at: \"2027-07-02\"/expires_at: \"$expiry\"/" \
            "$dir/rust/aegis/src/core/crypto/minisign.rs"
    fi

    set +e
    env PATH="$TMP/bin:$PATH" \
        ROTATE_GO_FILE="$dir/go/installer/minisign_verify.go" \
        ROTATE_RS_FILE="$dir/rust/aegis/src/core/crypto/minisign.rs" \
        ROTATE_KEY_OUT="$dir/new.key" \
        bash "$ROTATE" >"$dir/run.log" 2>&1
    SCENARIO_RC=$?
    set -e
}

GO_COPY_ACTIVE="$TMP/s1/go/installer/minisign_verify.go"
GO_COPY_HIST="$TMP/s2/go/installer/minisign_verify.go"
RS_COPY_ACTIVE="$TMP/s1/rust/aegis/src/core/crypto/minisign.rs"
RS_COPY_HIST="$TMP/s2/rust/aegis/src/core/crypto/minisign.rs"

echo
echo "=== 2. 场景 s1：活跃钥仍远离到期（净增一把新钥，旧钥保持活跃）==="
run_scenario s1 ""
check "脚本执行成功" "[ $SCENARIO_RC -eq 0 ]"
[ "$SCENARIO_RC" -eq 0 ] || sed 's/^/     /' "$TMP/s1/run.log"

check "Go 副本保留 verifyMinisign" "grep -q verifyMinisign '$GO_COPY_ACTIVE'"
check "Go 副本保留 matchTrustedComment" "grep -q matchTrustedComment '$GO_COPY_ACTIVE'"
check "Go 副本保留 requireMinisign" "grep -q requireMinisign '$GO_COPY_ACTIVE'"
check "Go 副本保留 parseTrustedComment" "grep -q parseTrustedComment '$GO_COPY_ACTIVE'"
check "Go 副本活跃表含旧钥" "awk '/^var minisignActiveKeys/,/^}/' '$GO_COPY_ACTIVE' | grep -q RWTZPf3"
check "Go 副本活跃表含新钥" "awk '/^var minisignActiveKeys/,/^}/' '$GO_COPY_ACTIVE' | grep -q testNEWKEY"
check "Go 副本历史表为空" "! awk '/^var minisignHistoricalKeys/,/^}/' '$GO_COPY_ACTIVE' | grep -q PublicKey"
check "Go 副本语法有效" "[ -z \"\$(gofmt -l '$GO_COPY_ACTIVE')\" ]"

check "Rust 副本保留 verify_minisign" "grep -q verify_minisign '$RS_COPY_ACTIVE'"
check "Rust 副本保留 parse_trusted_comment" "grep -q parse_trusted_comment '$RS_COPY_ACTIVE'"
check "Rust 副本保留 key_expired" "grep -q key_expired '$RS_COPY_ACTIVE'"
check "Rust 副本保留历史表 doc 注释" "grep -q '历史密钥' '$RS_COPY_ACTIVE'"
check "Rust 副本活跃表含旧钥" "awk '/^pub const MINISIGN_ACTIVE_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_ACTIVE' | grep -q RWTZPf3"
check "Rust 副本活跃表含新钥" "awk '/^pub const MINISIGN_ACTIVE_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_ACTIVE' | grep -q testNEWKEY"
check "Rust 副本历史表为空" "! awk '/^pub const MINISIGN_HISTORICAL_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_ACTIVE' | grep -q RWTZPf3"

echo
echo "=== 3. 场景 s2：活跃钥已过期（旧钥必须退役进历史表，而不是被删除）==="
run_scenario s2 "2020-01-01"
check "脚本执行成功" "[ $SCENARIO_RC -eq 0 ]"
[ "$SCENARIO_RC" -eq 0 ] || sed 's/^/     /' "$TMP/s2/run.log"

check "Go 旧钥已移入历史表" "awk '/^var minisignHistoricalKeys/,/^}/' '$GO_COPY_HIST' | grep -q RWTZPf3"
check "Go 历史表带退役日期" "awk '/^var minisignHistoricalKeys/,/^}/' '$GO_COPY_HIST' | grep -qE 'RetiredAt: \"20[0-9]{2}-'"
check "Go 活跃表不再含旧钥" "! awk '/^var minisignActiveKeys/,/^}/' '$GO_COPY_HIST' | grep -q RWTZPf3"
check "Go 活跃表含新钥" "awk '/^var minisignActiveKeys/,/^}/' '$GO_COPY_HIST' | grep -q testNEWKEY"
check "Go 源码函数仍存活" "grep -q verifyMinisign '$GO_COPY_HIST' && grep -q matchTrustedComment '$GO_COPY_HIST'"

check "Rust 旧钥已移入历史表" "awk '/^pub const MINISIGN_HISTORICAL_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_HIST' | grep -q RWTZPf3"
check "Rust 历史表带退役日期" "awk '/^pub const MINISIGN_HISTORICAL_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_HIST' | grep -qE 'retired_at: \"20[0-9]{2}-'"
check "Rust 活跃表不再含旧钥" "! awk '/^pub const MINISIGN_ACTIVE_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_HIST' | grep -q RWTZPf3"
check "Rust 活跃表含新钥" "awk '/^pub const MINISIGN_ACTIVE_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_HIST' | grep -q testNEWKEY"
check "Rust 源码函数仍存活" "grep -q verify_minisign '$RS_COPY_HIST' && grep -q parse_trusted_comment '$RS_COPY_HIST'"

echo
echo "=== 4. 历史表跨轮换累积（用 s2 的结果再跑一次，旧钥不得丢失）==="
# 把 s2 的产物当作新一轮输入：此时活跃钥是 testNEWKEY，历史表含 RWTZPf3
set +e
env PATH="$TMP/bin:$PATH" \
    ROTATE_GO_FILE="$GO_COPY_HIST" \
    ROTATE_RS_FILE="$RS_COPY_HIST" \
    ROTATE_KEY_OUT="$TMP/s2/new2.key" \
    bash "$ROTATE" >"$TMP/s2/run2.log" 2>&1
RC2=$?
set -e
check "第二轮执行成功" "[ $RC2 -eq 0 ]"
[ "$RC2" -eq 0 ] || sed 's/^/     /' "$TMP/s2/run2.log"
check "第二轮后 Go 历史表仍保留最初的旧钥" "awk '/^var minisignHistoricalKeys/,/^}/' '$GO_COPY_HIST' | grep -q RWTZPf3"
check "第二轮后 Rust 历史表仍保留最初的旧钥" "awk '/^pub const MINISIGN_HISTORICAL_KEYS/,/^[}\]]{1,2};$/' '$RS_COPY_HIST' | grep -q RWTZPf3"

echo
echo "=== 5. 真实文件必须完全未被改动 ==="
check "Go 真实文件 sha256 不变" "[ \"$GO_SHA_BEFORE\" = \"\$(sha256sum '$GO_FILE' | awk '{print \$1}')\" ]"
check "Rust 真实文件 sha256 不变" "[ \"$RS_SHA_BEFORE\" = \"\$(sha256sum '$RS_FILE' | awk '{print \$1}')\" ]"

echo
echo "=== 6. 私钥未泄露到日志 ==="
check "s1 日志不含私钥内容" "! grep -q 'testSECRETKEY' '$TMP/s1/run.log'"
check "s2 日志不含私钥内容" "! grep -q 'testSECRETKEY' '$TMP/s2/run.log'"
check "私钥已落盘且权限 600" "[ \"\$(stat -c %a '$TMP/s1/new.key')\" = '600' ]"

echo

# ---- 7. 负路径：锚点失效时必须中止且不写文件（P1-1 回归守卫）----
# 背景：原先 validate_output 只 grep 符号名，而未改动的文件本就含全部符号名，
# 于是锚点漂移（如 var→const）时两次 replace 都是 no-op，
# 脚本仍 exit 0 并打印「✅ 完成」，新公钥从未落地。
echo "=== 7. 负路径：锚点失效必须中止且不写文件 ==="
GO_DRIFT="$TMP/drift_go.go"
RS_DRIFT="$TMP/drift_rs.rs"
cp "$GO_FILE" "$GO_DRIFT"
cp "$RS_FILE" "$RS_DRIFT"
# 形变锚点：名字仍在，但 '^var minisignActiveKeys' 不再匹配
sed -i 's/^var minisignActiveKeys/const minisignActiveKeys/; s/^var minisignHistoricalKeys/const minisignHistoricalKeys/' "$GO_DRIFT"
DRIFT_SHA=$(sha256sum "$GO_DRIFT" | awk '{print $1}')
set +e
env PATH="$TMP/bin:$PATH" \
    ROTATE_GO_FILE="$GO_DRIFT" \
    ROTATE_RS_FILE="$RS_DRIFT" \
    ROTATE_KEY_OUT="$TMP/drift/new.key" \
    bash "$ROTATE" >"$TMP/drift.log" 2>&1
RC_DRIFT=$?
set -e
check "锚点失效时脚本必须非零退出" "[ $RC_DRIFT -ne 0 ]"
check "锚点失效时不得改动目标文件" \
    "[ \"$DRIFT_SHA\" = \"\$(sha256sum '$GO_DRIFT' | awk '{print \$1}')\" ]"
check "锚点失效时不得声称完成" "! grep -q '密钥轮换完成' '$TMP/drift.log'"

# ---- 8. 私钥必须在源码写入之前落盘（P1-2 回归守卫）----
# 否则若 Rust 段失败，EXIT trap 会连同 TEMP_DIR 删掉私钥，
# 而 Go 表已含新公钥 → 新钥永远无法签名的孤儿。
echo
echo "=== 8. 私钥落盘必须先于源码写入 ==="
KEY_LINE=$(grep -n 'cp "$TEMP_DIR/minisign.key"' "$ROTATE" | head -1 | cut -d: -f1)
GO_LINE=$(grep -n 'cp "$TEMP_DIR/go_new.go"' "$ROTATE" | head -1 | cut -d: -f1)
RS_LINE=$(grep -n 'cp "$TEMP_DIR/rs_new.rs"' "$ROTATE" | head -1 | cut -d: -f1)
check "私钥落盘行号小于 Go 写入行号" "[ -n \"$KEY_LINE\" ] && [ -n \"$GO_LINE\" ] && [ \"$KEY_LINE\" -lt \"$GO_LINE\" ]"
check "私钥落盘行号小于 Rust 写入行号" "[ -n \"$KEY_LINE\" ] && [ -n \"$RS_LINE\" ] && [ \"$KEY_LINE\" -lt \"$RS_LINE\" ]"
check "私钥落盘使用 umask 077（避免 644 窗口）" "grep -q 'umask 077' \"\$ROTATE\""

# ---- 9. 校验必须真的校验新公钥 ----
echo
echo "=== 9. 校验逻辑必须检查新公钥 ==="
check "validate_output 检查 NEW_KEY" "awk '/^validate_output\\(\\)/,/^}/' '$ROTATE' | grep -q 'NEW_KEY'"
check "validate_output 检查旧钥无丢失" "awk '/^validate_output\\(\\)/,/^}/' '$ROTATE' | grep -q 'OLD_KEYS'"

echo
if [ "$fail" -ne 0 ]; then
    echo "❌ 存在失败项"
    exit 1
fi
echo "✅ 全部通过"
