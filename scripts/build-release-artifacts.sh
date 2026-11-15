#!/usr/bin/env bash
# 构建发布产物：dist/aegis-linux-<arch> 与 dist/installer-linux-<arch>。
#
# 架构清单与编译基线只有这一份真源：CI 的 build job 每个 matrix 腿调一次
# （`... amd64` / `... arm64`），publish 的回退重建调不带参数（两个都建）。
#
# 支持原生与交叉两种模式，由 host 架构自动判定：
#   host == target  -> 原生构建，用系统 cc
#   host != target  -> 交叉构建，用 aarch64-linux-gnu-gcc
# 两条路径都保留，是因为 GitHub/Forgejo 都有原生 arm64 runner（矩阵首选），
# 但单 runner 环境（含 publish 的兜底重建）仍需交叉能力。
#
# ── 为什么必须显式锁 x86-64 基线 ────────────────────────────────────────
# 2.5.1 的 aegis 在 Intel Xeon E5-2680 v2 (Ivy Bridge，无 AVX2/BMI1) 上
# 启动即 SIGILL（systemd 记 status=4/ILL，dmesg: trap invalid opcode）。
# 反汇编定位到文件偏移 0x21d4bbf 的 `andn rdx,rdx,rax`（VEX 编码，BMI1），
# 属于 ring 的 crypto/curve25519/curve25519.c::table_select —— 一个**没有**
# target_feature 运行时探测的静态函数，被构建环境的 C 编译器默认架构抬高
# 后编进了 BMI1 指令。
#
# 因此 x86-64 产物固定传 -march=x86-64 -mtune=generic：不跟随构建机的
# 编译器默认，保证在任何 64 位 x86 上都能执行。
# 需要更高性能时的正确做法是方案 B（额外产 -v3 变体 + 运行时 CPUID 选择），
# 不是放宽这里。
#
# ── 本脚本涉及的两处 C 编译 ──────────────────────────────────────────
#   ring          —— TLS/curve25519，含 C 与 .S
#   libsqlite3-sys —— 已开 bundled（见 rust/aegis/Cargo.toml 注释），
#                     sqlite3.c 也走 cc，因此同样吃 CFLAGS 基线
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"

# 与 .github/.forgejo 发布流程的资产命名保持一致。
ASSET_SLUG="aegis"
INSTALLER_SLUG="installer"

# x86-64 通用基线：见文件头注释。
X86_BASELINE_CFLAGS="-march=x86-64 -mtune=generic"
# armv8-a 是 AArch64 的要求基线，等价于「不许抬到 v8.2+ 的可选扩展」。
ARM64_BASELINE_CFLAGS="-march=armv8-a"

# 把 uname 的机器名映射成与资产后缀一致的架构名。
host_arch() {
  case "$(uname -m)" in
    x86_64) echo amd64 ;;
    aarch64 | arm64) echo arm64 ;;
    *) uname -m ;;
  esac
}

mkdir -p "$DIST"

build_arch() {
  local arch="$1" rust_target="$2" goarch="$3" cflags="$4" cross_linker="$5"

  local linker=""
  if [ "$(host_arch)" != "$arch" ]; then
    linker="$cross_linker"
  fi

  echo "==> 构建 $arch (target=$rust_target, host=$(host_arch), 模式=$([ -n "$linker" ] && echo 交叉 || echo 原生))"

  # cargo 的 target 专属环境变量名要求 triple 里的 `-` 全部换成 `_`：
  #   CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER
  # 只做 `${x^^}` 得到的是带连字符的非法标识符，export 会直接报错。
  local triple="${rust_target//-/_}"
  local cc_var="CC_$triple"
  local cflags_var="CFLAGS_$triple"
  local linker_var="CARGO_TARGET_${triple^^}_LINKER"

  # 交叉链接器只能通过 target 专属变量传，全局 CC 会污染构建脚本本身。
  if [ -n "$linker" ]; then
    command -v "$linker" >/dev/null 2>&1 || {
      echo "::error::缺少交叉链接器 $linker（scripts/ensure-build-deps.sh 负责安装）"
      return 1
    }
  fi

  # cc-rs 读 target 专属 CFLAGS，优先级高于全局 CFLAGS。
  export "$cflags_var=$cflags"
  local prev_linker="${!linker_var-}"
  local prev_cc="${!cc_var-}"
  if [ -n "$linker" ]; then
    export "$linker_var=$linker"
    export "$cc_var=$linker"
  fi

  (
    cd "$ROOT/rust/aegis"
    RUSTFLAGS="--remap-path-prefix $ROOT=/project -C target-cpu=generic" \
      cargo build --release --target "$rust_target"
  )

  (
    cd "$ROOT/go/installer"
    CGO_ENABLED=0 GOTOOLCHAIN=local GOOS=linux GOARCH="$goarch" \
      garble -literals -tiny -seed=random build -ldflags="-s -w" -o "$DIST/$INSTALLER_SLUG-linux-$arch" .
  )

  cp "$ROOT/rust/aegis/target/$rust_target/release/$ASSET_SLUG" "$DIST/$ASSET_SLUG-linux-$arch"
  chmod +x "$DIST/$ASSET_SLUG-linux-$arch" "$DIST/$INSTALLER_SLUG-linux-$arch"

  # 同一个进程里可能连续构建两个架构，交叉变量必须还原，否则后者被污染。
  if [ -n "$linker" ]; then
    if [ -n "$prev_linker" ]; then export "$linker_var=$prev_linker"; else unset "$linker_var"; fi
    if [ -n "$prev_cc" ]; then export "$cc_var=$prev_cc"; else unset "$cc_var"; fi
  fi
}

# rustup 管理的工具链不会自带交叉 std。
# mise 的 rust 插件用 rustup-init 装，rustup 通常在 mise shims 或 ~/.cargo/bin；
# 两处都找不到时不能静默继续，否则交叉构建会在链接阶段报个无关的错。
RUSTUP="$(command -v rustup 2>/dev/null || true)"
if [ -z "$RUSTUP" ] && [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/rustup" ]; then
  RUSTUP="${CARGO_HOME:-$HOME/.cargo}/bin/rustup"
fi

add_target() {
  [ -n "$RUSTUP" ] || { echo "::warning::未找到 rustup，假定 $1 的标准库已就绪"; return 0; }
  "$RUSTUP" target add "$1" >/dev/null 2>&1 || echo "::warning::rustup target add $1 失败（可能已安装）"
}

# 可选参数：只构建指定架构（默认两个都建）。
# CI 的 matrix 腿各传一个；本地调试可 `scripts/build-release-artifacts.sh amd64`。
SELECTED=("$@")
[ ${#SELECTED[@]} -eq 0 ] && SELECTED=(amd64 arm64)

wanted() {
  local a
  for a in "${SELECTED[@]}"; do [ "$a" = "$1" ] && return 0; done
  return 1
}

if wanted amd64; then
  add_target x86_64-unknown-linux-gnu
  build_arch amd64 x86_64-unknown-linux-gnu amd64 "$X86_BASELINE_CFLAGS" ""
fi

if wanted arm64; then
  add_target aarch64-unknown-linux-gnu
  build_arch arm64 aarch64-unknown-linux-gnu arm64 "$ARM64_BASELINE_CFLAGS" aarch64-linux-gnu-gcc
fi

ls -lh "$DIST"
