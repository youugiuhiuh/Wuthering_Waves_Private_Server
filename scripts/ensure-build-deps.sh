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

# pkg-config 是 openssl-sys 的查找工具。
# openssl.pc -> libssl-dev；sqlite3.pc -> libsqlite3-dev。
# 后者曾被漏掉：装在列表里、打印在 echo 里，却零校验，导致 sqlite3 缺失时
# check_deps 仍返回 0 并谎报「已就绪」跳过安装。
# 来源：rust/aegis/Cargo.toml 的 matrix-sdk sqlite feature ->
#   matrix-sdk-sqlite -> rusqlite -> libsqlite3-sys（非 bundled 时走 pkg-config）
REQUIRED_CMDS=(pkg-config)
REQUIRED_PKGCONFIG_FILES=(openssl sqlite3)

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
  echo "构建依赖已就绪（libc 头文件 / pkg-config / openssl / libsqlite3-dev）跳过安装"
  exit 0
fi

echo "缺少构建依赖，尝试安装 libc6-dev pkg-config libssl-dev libsqlite3-dev"

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
$SUDO apt-get install -y -qq --no-install-recommends libc6-dev pkg-config libssl-dev libsqlite3-dev || {
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

exit 0
