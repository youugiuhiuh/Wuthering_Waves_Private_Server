# Implementation Plan: XDRIVE 存储专属零账单流控参数调优

## Overview
调整 Rust 服务端（`rust/aegis`）内置的 XDRIVE 存储预设构建器参数：
- 将 Cloudflare R2 最大空闲退避拉长至 3000ms（3秒），日空闲轮询上限降至 2.8 万次，全月在 100 万次免费额度内，彻底杜绝扣费账单；同时将 eager 窗口设定为 3000ms，保证唤醒后极速响应。
- 补齐 AWS S3、Oracle S3、标准 S3 与区域向导解析中的批量刷出（50ms）与积极窗口（2000ms），避免空转风暴。
- 同步更新测试断言并执行质量门禁。

## Task 1: Calibrate R2 and S3 Presets in xdrive.rs
**Description:** 调整 `rust/aegis/src/core/xray/xdrive.rs` 中的构造器参数。
- `new_cloudflare_r2_with_endpoint`: `poll: 50, max_poll: 3000, flush: 30, eager: 3000, concurrency: 16`
- `new_aws_s3`: `poll: 50, max_poll: 2000, flush: 50, eager: 2000, concurrency: 16`
- `new_oracle_cloud_s3`: `poll: 50, max_poll: 2000, flush: 50, eager: 2000, concurrency: 8`
- `new_s3`: `poll: 50, max_poll: 2000, flush: 50, eager: 2000, concurrency: 8`
- `parse_region_wizard_input` ("ru", "ir", "custom"): `poll: 50, max_poll: 2000, flush: 50, eager: 2000, concurrency: 8`

**Acceptance criteria:**
- 构造器参数正确配置并能成功通过 `validate()`。

**Verification:**
- `cargo check --package aegis`

**Files:**
- `rust/aegis/src/core/xray/xdrive.rs`

## Task 2: Update Unit Test Assertions in xdrive.rs
**Description:** 同步更新 `rust/aegis/src/core/xray/xdrive.rs` 中的断言期望值。
- 更新 `test_valid_cloudflare_r2_preset`、`test_cloudflare_r2_optimized_parameters`、`test_cloudflare_r2_with_custom_endpoint`。
- 更新 AWS、Oracle、S3 及 region wizard 解析测试中的断言。

**Acceptance criteria:**
- `core::xray::xdrive` 下的所有单元测试全部绿灯通过。

**Verification:**
- `cargo test --package aegis --lib core::xray::xdrive`

**Files:**
- `rust/aegis/src/core/xray/xdrive.rs`

## Task 3: Full Quality Gates & Clean Baseline Verification
**Description:** 运行全量质量门禁检查和集成测试。
- 执行 `cargo fmt --check`
- 执行 `cargo clippy --workspace --all-targets -- -D warnings`
- 执行 `cargo nextest run`

**Acceptance criteria:**
- 格式、Clippy、Nextest 全量无警告无报错。

**Verification:**
- `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run`

**Files:**
- All workspace files
