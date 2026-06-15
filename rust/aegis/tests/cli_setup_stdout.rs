//! 集成测试：`aegis --setup` 成功时 stdout 仅一行且包含成功提示，避免混入其它输出。

use std::process::Command;

#[test]
fn cli_setup_success_stdout_is_single_line_with_success_message() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path();
    let bin = env!("CARGO_BIN_EXE_tgbot");
    let totp_secret = "JBSWY3DPEHPK3PXP"; // 合法 base32，至少 16 位

    let output = Command::new(bin)
        .env("TGBOT_CONFIG_DIR", config_dir)
        .args(["--setup", "dummy_token", "123456", totp_secret])
        .output()
        .expect("执行 aegis --setup 失败");

    assert!(
        output.status.success(),
        "setup 应成功。stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "stdout 应仅一行，避免污染管道。stdout={:?}",
        stdout
    );
    assert!(
        lines[0].contains("Setup completed") || lines[0].contains("✅"),
        "该行应包含成功提示。got: {:?}",
        lines[0]
    );
}
