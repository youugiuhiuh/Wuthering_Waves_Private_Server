# Task Breakdown & Checklist

- [ ] Task 1: Calibrate R2 and S3 Presets in xdrive.rs
  - Acceptance: `new_cloudflare_r2_with_endpoint` 设为 `poll: 50, max_poll: 3000, flush: 30, eager: 3000, concurrency: 16`；`new_aws_s3`、`new_oracle_cloud_s3`、`new_s3` 及向导解析补充 `flush: 50, eager: 2000, max_poll: 2000, poll: 50`。
  - Verify: `cargo check --package aegis`
  - Files: `rust/aegis/src/core/xray/xdrive.rs`

- [ ] Task 2: Update Unit Test Assertions in xdrive.rs
  - Acceptance: 所有测试用例期望值与新参数同步对齐，`core::xray::xdrive` 单元测试全绿。
  - Verify: `cargo test --package aegis --lib core::xray::xdrive`
  - Files: `rust/aegis/src/core/xray/xdrive.rs`

- [ ] Task 3: Full Quality Gates & Clean Baseline Verification
  - Acceptance: `cargo fmt --check`、`cargo clippy`（0 警告）、`cargo nextest run` 全量通过。
  - Verify: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run`
  - Files: All workspace files
