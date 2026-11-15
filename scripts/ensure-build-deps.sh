#!/usr/bin/env bash
# 确保 libc 开发头文件存在，供 cgo / cargo / upx 使用。
#
# 背景：codefloe 的 Forgejo runner 是精简镜像 —— 装了 gcc，但没装 libc6-dev。
# 于是任何启用 cgo 的编译都会炸在 gcc 的转发头上：
#
#   /usr/lib/gcc/aarch64-linux-gnu/15/include/stdint.h:11:16:
#       fatal error: stdint.h: No such file or directory
#    11 | # include_next <stdint.h>
#
# `# include_next` 是 gcc 自带的包装头，它去下一个搜索路径找 libc 的真身，
# 找不到就报错。日志里报缺的 stdlib.h / stdint.h / pthread.h / errno.h /
# stdio.h / grp.h 全部由 libc6-dev 提供（共 482 个头文件）。
#
# 单靠 CGO_ENABLED=0 不够：那只修 garble。cargo/rustc 链接时同样需要 libc
# 开发文件，upx 也需要系统库。补齐环境才是治本。
#
# 第二轮补充（实测 run 暴露）：cargo 编译 aegis 时 openssl-sys 报
#   Could not run `PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1 pkg-config --libs --cflags openssl`
#   The pkg-config command could not be found.
# 所以还需要 pkg-config 与 libssl-dev（后者的 .pc 文件是 pkg-config 的查找依据）：
#   /usr/include/openssl/ssl.h                    -> libssl-dev
#   /usr/lib/x86_64-linux-gnu/libssl.so           -> libssl-dev
#   /usr/lib/x86_64-linux-gnu/pkgconfig/openssl.pc -> libssl-dev
#
# OpenSSL 的来源（cargo tree -i native-tls 实测）：
#   native-tls <- hyper-tls <- reqwest <- matrix-sdk
#                                     <- serenity（本仓库选了 native_tls_backend）
# 注意 matrix-sdk 那条链路不可控，所以不能靠把 serenity 换成 rustls
# 来摆脱 OpenSSL：两条路径都要过 native-tls。必须在环境层面补齐。
#
# 幂等：已装则立即返回。
# 降级：无 apt / 无 sudo 时打印警告并以 0 退出，让 CGO_ENABLED=0 的路径兜底，
#       不因为环境限制而阻断整个 release。

set -uo pipefail

# 这两个是诊断时确认缺失的代表性头文件，分属 libc6-dev 的核心内容
REQUIRED_HEADERS=(/usr/include/stdlib.h /usr/include/pthread.h)

# pkg-config 是 openssl-sys 的查找工具；openssl.pc 由 libssl-dev 提供。
# （历史上这里还必须要求 sqlite3.pc，漏掉它会让 check_deps 在缺库时
#   仍返回 0 并谎报「已就绪」跳过安装。libsqlite3-sys 改成 bundled 之后
#   不再走 pkg-config，这条要求已删除。）
#
# 交叉编译器只在 x86-64 上跨编译 arm64 时需要；arm64 host 原生构建用系统 cc，
# 无条件要求它会让 arm runner 永远判定「缺依赖」并尝试装一个不存在的包。
REQUIRED_CMDS=(pkg-config)
if [ "$(uname -m)" = "x86_64" ]; then
  REQUIRED_CMDS+=(aarch64-linux-gnu-gcc)
fi
# sqlite3.pc 不再需要：libsqlite3-sys 已开 bundled，sqlite3.c 由 cc 编译，
# 不再走 pkg-config 找系统库（见 rust/aegis/Cargo.toml 注释）。
REQUIRED_PKGCONFIG_FILES=(openssl)

check_deps() {
  local h c p
  for h in "${REQUIRED_HEADERS[@]}"; do
    [ -f "$h" ] || return 1
  done
  for c in "${REQUIRED_CMDS[@]}"; do
    command -v "$c" >/dev/null 2>&1 || return 1
  done
  # openssl.pc 路径含架构（x86_64 / aarch64），用 pkg-config 自己判断最稳
  for p in "${REQUIRED_PKGCONFIG_FILES[@]}"; do
    pkg-config --exists "$p" 2>/dev/null || return 1
  done
  return 0
}

if check_deps; then
  echo "构建依赖已就绪（libc 头文件 / pkg-config / openssl / aarch64 叉编工具链）跳过安装"
  exit 0
fi

echo "缺少构建依赖，尝试安装 libc6-dev pkg-config libssl-dev gcc-aarch64-linux-gnu libc6-dev-arm64-cross"

if ! command -v apt-get >/dev/null 2>&1; then
  echo "::warning::无 apt-get，跳过安装。将依赖 CGO_ENABLED=0 降级构建"
  exit 0
fi

SUDO=""
if [ "$(id -u)" != "0" ]; then
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
  else
    echo "::warning::非 root 且无 sudo，跳过安装。将依赖 CGO_ENABLED=0 降级构建"
    exit 0
  fi
fi

export DEBIAN_FRONTEND=noninteractive
$SUDO apt-get update -qq || echo "::warning::apt-get update 失败，继续尝试安装"
$SUDO apt-get install -y -qq --no-install-recommends libc6-dev pkg-config libssl-dev gcc-aarch64-linux-gnu libc6-dev-arm64-cross || {
  echo "::warning::构建依赖安装失败。将依赖 CGO_ENABLED=0 降级构建（cargo 侧可能仍失败）"
  exit 0
}

# 装完复查：确认依赖真的到位，而不是包管理器谎报成功
if check_deps; then
  echo "  ✅ libc6-dev / pkg-config / libssl-dev 安装成功"
  gcc --version | head -1
  pkg-config --modversion openssl
else
  echo "::warning::依赖已安装但检查未通过（路径异常？）"
  pkg-config --exists openssl 2>/dev/null || echo "  openssl.pc 不可见"
fi

# ── arm64 交叉工具链复查 ────────────────────────────────────────────
# 发布产出 aegis-linux-arm64。ring 的 C 代码与 bundled sqlite3.c 都要过 cc，
# 交叉编译必须有 aarch64 gcc **并且** 要有 aarch64 的 libc 头文件。
# libc6-dev-arm64-cross 是 gcc-aarch64-linux-gnu 的 Recommends，
# 在 --no-install-recommends 下不会被带上，缺了会报：
#   /usr/include/stdint.h:26:10: fatal error: bits/libc-header-start.h
if [ "$(uname -m)" = "x86_64" ]; then
  for c in aarch64-linux-gnu-gcc; do
    if command -v "$c" >/dev/null 2>&1; then
      echo "  ✅ $c: $($c --version | head -1)"
    else
      echo "::warning::$c 仍不可用，arm64 产物将无法构建"
    fi
  done
  [ -f /usr/aarch64-linux-gnu/include/stdint.h ] \
    && echo "  ✅ aarch64 交叉 libc 头文件就绪" \
    || echo "::warning::缺 aarch64 交叉 libc 头（libc6-dev-arm64-cross），arm64 产物将无法构建"
else
  echo "  原生 $(uname -m) 环境，无需交叉工具链"
fi

exit 0
