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
# 幂等：已装则立即返回。
# 降级：无 apt / 无 sudo 时打印警告并以 0 退出，让 CGO_ENABLED=0 的路径兜底，
#       不因为环境限制而阻断整个 release。

set -uo pipefail

# 这两个是诊断时确认缺失的代表性头文件，分属 libc6-dev 的核心内容
REQUIRED_HEADERS=(/usr/include/stdlib.h /usr/include/pthread.h)

check_headers() {
  local h
  for h in "${REQUIRED_HEADERS[@]}"; do
    [ -f "$h" ] || return 1
  done
  return 0
}

if check_headers; then
  echo "libc 开发头文件已存在，跳过安装"
  exit 0
fi

echo "缺少 libc 开发头文件，尝试安装 libc6-dev"

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
$SUDO apt-get install -y -qq --no-install-recommends libc6-dev || {
  echo "::warning::libc6-dev 安装失败。将依赖 CGO_ENABLED=0 降级构建"
  exit 0
}

# 装完复查：确认头文件真的到位，而不是包管理器谎报成功
if check_headers; then
  echo "  ✅ libc6-dev 安装成功"
  gcc --version | head -1
else
  echo "::warning::libc6-dev 已安装但头文件仍缺失（路径异常？）"
fi

exit 0
