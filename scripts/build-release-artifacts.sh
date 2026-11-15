#!/usr/bin/env bash
# 构建发布产物：dist/aegis-linux-<arch> 与 dist/installer-linux-<arch>。
#
# build job 与 publish 的回退重建都调用本脚本 —— 架构清单与编译基线只能有一份
# 真源，否则两处漂移就会出现「CI 产出的和二进制作出行为不一致」。
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
# ── arm64 交叉编译 ───────────────────────────────────────────────────
# ring 含 C 代码，必须用 aarch64 交叉 gcc（scripts/ensure-build-deps.sh 装）。
# Go/garbled 侧 CGO_ENABLED=0，交叉编译无额外依赖。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"

# 与 .github/.forgejo public-release.yml 的打包步骤保持同名。
ASSET_SLUG="aegis"
INSTALLER_SLUG="installer"

# x86-64 通用基线：见文件头注释。
X86_BASELINE_CFLAGS="-march=x86-64 -mtune=generic"

mkdir -p "$DIST"

build_arch() {
  local arch="$1" rust_target="$2" goarch="$3" cflags="$4" linker="$5"

  echo "==> 构建 $arch (target=$rust_target)"

  local cc_var="CC_${rust_target//-/_}"
  local cflags_var="CFLAGS_${rust_target//-/_}"

  # cargo 的交叉链接器只能通过 target 专属变量传，全局 CC 会污染构建脚本本身。
  if [ -n "$linker" ]; then
    export "CARGO_TARGET_${rust_target^^}_LINKER=$linker"
    export "$cc_var=$linker"
  fi
  # cc-rs 读 target 专属 CFLAGS，优先级高于全局 CFLAGS。
  export "$cflags_var=$cflags"

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
}

# rustup 管理的工具链不会自带交叉 std。
# mise 的 rust 插件用 rustup-init 装，rustup 通常在 mise shims 或 ~/.cargo/bin；
# 两处都找不到时不能静默继续，否则 arm64 构建会在链接阶段报个无关的错。
RUSTUP="$(command -v rustup 2>/dev/null || true)"
if [ -z "$RUSTUP" ] && [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/rustup" ]; then
  RUSTUP="${CARGO_HOME:-$HOME/.cargo}/bin/rustup"
fi
if [ -n "$RUSTUP" ]; then
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    "$RUSTUP" target add "$t" >/dev/null 2>&1 || echo "::warning::rustup target add $t 失败（可能已安装）"
  done
else
  echo "::warning::未找到 rustup，假定 aarch64 目标 std 已就绪；若 arm64 构建报缺少 std 请先安装"
fi

# 可选参数：只构建指定架构（默认两个都建）。CI 不传；本地调试时可
# `scripts/build-release-artifacts.sh amd64`，不必先装交叉 gcc。
SELECTED=("$@")
[ ${#SELECTED[@]} -eq 0 ] && SELECTED=(amd64 arm64)

wanted() {
  local a
  for a in "${SELECTED[@]}"; do [ "$a" = "$1" ] && return 0; done
  return 1
}

if wanted amd64; then
  build_arch amd64 x86_64-unknown-linux-gnu amd64 "$X86_BASELINE_CFLAGS" ""
fi

if wanted arm64; then
  build_arch arm64 aarch64-unknown-linux-gnu arm64 "-march=armv8-a" aarch64-linux-gnu-gcc
fi

# 兼容别名：存量部署的一键指令硬编码了 /releases/latest/download/installer
# 与 asset 名 `aegis`，指向 amd64 产物。
#
# 注意：arm64 机器**不能**用这两个别名 —— 裸二进制是 amd64，装上只会
# Exec format error。README 已按架构分别给出下载地址。
if wanted amd64; then
  cp "$DIST/$ASSET_SLUG-linux-amd64" "$DIST/$ASSET_SLUG"
  cp "$DIST/$INSTALLER_SLUG-linux-amd64" "$DIST/$INSTALLER_SLUG"
fi

ls -lh "$DIST"
