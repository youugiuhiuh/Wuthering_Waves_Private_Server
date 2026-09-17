//! 回归测试：matrix-sdk 0.19 的 HTTP 栈走 reqwest 0.13 的 `rustls-no-provider` 路径
//! （aegis 用 `default-features = false` 关掉了 matrix-sdk 的 `rustls-aws-lc-rs` 默认特性）。
//! reqwest 在构建 Client 时取 rustls 的进程级默认 provider，取不到就直接 panic。
//! 线上故障：wwps-aegis v1.5.9 启动即 abort、无限重启。
use aegis::bootstrap::install_crypto_provider;

#[tokio::test]
async fn matrix_client_builds_after_crypto_provider_install() {
    install_crypto_provider();

    let store = tempfile::tempdir().expect("tempdir");
    let client = matrix_sdk::Client::builder()
        .homeserver_url("https://example.org")
        .sqlite_store(store.path(), None)
        .build()
        .await
        .expect("matrix-sdk Client 构建失败");

    drop(client);
}
