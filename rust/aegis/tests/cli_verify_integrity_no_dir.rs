//! 集成测试：配置目录不存在时，正常启动路径会先执行 verify_integrity，应 exit(1) 且 stderr 含「配置文件目录不存在」类提示。

use std::process::Command;

#[test]
fn cli_verify_integrity_fails_with_stderr_when_config_dir_missing() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("does_not_exist");
    assert!(!config_dir.exists(), "测试用路径应不存在");

    let bin = env!("CARGO_BIN_EXE_aegis");
    let output = Command::new(bin)
        .env("AEGIS_CONFIG_DIR", &config_dir)
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
        stderr.contains("配置文件目录不存在") || stderr.contains("目录不存在"),
        "stderr 应包含目录不存在类提示。stderr={:?}",
        stderr
    );
}
