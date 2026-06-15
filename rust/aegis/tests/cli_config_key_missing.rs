//! 集成测试：仅有 config.enc、无 .key 时进程应失败且 stderr 含「.key 不存在」类提示。
//!
//! 通过 AEGIS_CONFIG_DIR 指向临时目录，仅创建 config.enc，断言非零退出且 stderr 包含关键提示。

use std::process::Command;

#[test]
fn cli_fails_with_stderr_about_key_missing_when_config_enc_exists_without_key() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path();
    let config_enc = config_dir.join("config.enc");
    std::fs::create_dir_all(config_dir).unwrap();
    // 仅写入 config.enc，不创建 .key；内容可为任意合法 JSON（进程在读取前就会 bail）
    std::fs::write(&config_enc, b"{}").unwrap();

    let bin = env!("CARGO_BIN_EXE_aegis");
    let output = Command::new(bin)
        .env("AEGIS_CONFIG_DIR", config_dir)
        .output()
        .expect("执行 aegis 失败");

    assert!(
        !output.status.success(),
        "应失败退出。stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".key") || stderr.contains("不存在"),
        "stderr 应包含 .key 或「不存在」提示。stderr={:?}",
        stderr
    );
}
