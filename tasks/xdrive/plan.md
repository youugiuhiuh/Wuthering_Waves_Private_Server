# Implementation Plan: Aegis XDRIVE 隐蔽传输总框架

## Overview
在 `rust/aegis` 中实现 XDRIVE 隐蔽传输层总框架支持，首期优先打通 S3/Cloudflare R2 与 Local 兼容模式，规范 VLESS+XMUX+Encryption 配置生成，实现零公网端口暴露。

## Dependency Graph
```
Task 1: xdrive_model (src/core/xray/xdrive.rs)
   │
   ▼
Task 2: config_builder (src/core/xray/config.rs)
   │
   ▼
Task 3: client_link (src/core/xray/config.rs)
   │
   ▼
Task 4: lifecycle_adapt (src/core/xray/port_allocator.rs)
   │
   ▼
Task 5: Quality Gate & Full Verification (cargo clippy, cargo fmt, cargo test)
```

## Tasks
1. **Task 1: XDRIVE 数据模型与 S3 校验器 (xdrive_model)**
   - 文件：`rust/aegis/src/core/xray/xdrive.rs`, `rust/aegis/src/core/xray/mod.rs`
   - 目标：定义 `XdriveService`, `S3Config`, `XdriveConfig`，实现 S3 参数校验与 Cloudflare R2 预设构建器，并编写 TDD 单元测试。
2. **Task 2: XDRIVE VLESS 入站生成器 (config_builder)**
   - 文件：`rust/aegis/src/core/xray/config.rs`
   - 目标：扩展 `Proto::XDrive`，实现 `build_xdrive_vless_inbound`，严格断言遵循 Xray PR #7182 规范。
3. **Task 3: 客户端订阅链接格式化 (client_link)**
   - 文件：`rust/aegis/src/core/xray/config.rs`
   - 目标：支持 `vless://` 链接输出，完整携带 S3/R2 xdrive 参数与 VLESS 加密参数。
4. **Task 4: 端口分配与防火墙安全豁免 (lifecycle_adapt)**
   - 文件：`rust/aegis/src/core/xray/port_allocator.rs`
   - 目标：实现 XDRIVE 的零公网端口抽象，免分配跳跃端口，免开放外部防火墙。
5. **Task 5: 质量把关与规范检查 (quality_gate)**
   - 运行 `cargo fmt`, `cargo clippy`, `cargo test`。
