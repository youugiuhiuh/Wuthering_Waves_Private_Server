# 任务清单：下载瞬态故障指数退避重试

- [x] Task 1: 在 `core/network/release_api.rs` 中实现 `is_retryable_status`、`is_retryable_reqwest_error`、`retry_with_backoff` 及完备单测
- [x] Task 2: 在 `core/system/maintenance.rs` 的 `download_file` 中复用 `retry_with_backoff`
- [x] Task 3: 在 `core/system/core_upgrade.rs` 的 `download_release` 与 `download_sha256_manifest` 中复用 `retry_with_backoff`
- [x] Review 1: 收紧错误判定边界，避免端口号/哈希字符串误伤
- [x] Review 2: 消除重复的样板重试循环，统一直调 `retry_with_backoff`
- [x] Checkpoint: 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo nextest run`
