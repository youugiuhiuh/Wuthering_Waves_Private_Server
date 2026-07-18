//! 集成测试：`aegis --setup-stdin` 从 stdin 读 JSON 并写入 .key + config.enc。

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_setup_stdin_creates_key_and_config_enc() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("aegis");
    let key_path = config_dir.join(".key");
    let config_path = config_dir.join("config.enc");

    let payload = r#"{"token":"x","admin_id":"1","totp_secret":"JBSWY3DPEHPK3PXP"}"#;
    let bin = env!("CARGO_BIN_EXE_aegis");

    let mut child = Command::new(bin)
        .env("AEGIS_CONFIG_DIR", config_dir)
        .args(["--setup-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aegis --setup-stdin");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("wait aegis");

    assert!(
        output.status.success(),
        "setup-stdin 应成功。stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(key_path.exists(), ".key 应存在");
    assert!(config_path.exists(), "config.enc 应存在");
}
