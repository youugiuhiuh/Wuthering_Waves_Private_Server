//! 集成测试：`tgbot --generate-totp-secret` 的 stdout 必须仅有一行合法 Base32
//!
//! ## 为何之前的测试没捕获到“整段 stdout 被当 TOTP”的 bug？
//!
//! - **integration_totp_trim.rs** 只测了「给定字符串带换行时 trim 是否能让 TotpManager 接受」，
//!   没有测 **main() 的 CLI 行为**，也没有测「运行二进制时 stdout 里到底有什么」。
//! - **logic/totp 的单元测试**只测 TotpManager::new / generate_new_secret，不测进程边界。
//! - 因此「main 里先跑 verify_integrity() 再跑 --generate-totp-secret，导致 stdout 多出一行」
//!   这类问题，只有**对二进制做端到端测试、断言 stdout 内容**才能发现。
//!
//! ## 本测试补的是什么？
//!
//! 安装器会捕获该命令的**整段 stdout** 作为 TOTP 密钥写入配置；若出现 "Binary Integrity Hash"
//! 等额外输出，会写入无效密钥、启动报错。本测试通过**实际执行二进制**确保该 CLI 路径只输出一行 base32。

use std::process::Command;

/// 运行 `tgbot --generate-totp-secret` 并检查 stdout 仅有一行、且为合法 base32，且不含 "Binary Integrity Hash"。
#[test]
fn cli_generate_totp_secret_stdout_is_single_base32_line() {
    let bin = env!("CARGO_BIN_EXE_tgbot");
    let output = Command::new(bin)
        .arg("--generate-totp-secret")
        .output()
        .expect("执行 tgbot 失败");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();

    assert_eq!(
        lines.len(),
        1,
        "stdout 必须只有一行，否则安装器会把整段当 TOTP 密钥。lines={} stdout={:?} stderr={:?}",
        lines.len(),
        stdout,
        stderr
    );

    let line = lines[0].trim();
    assert!(!line.is_empty(), "该行不能为空");
    assert!(
        !stdout.contains("Binary Integrity Hash"),
        "stdout 中不得包含完整性哈希，否则会污染 TOTP 密钥。stdout={:?}",
        stdout
    );
    assert!(
        line.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
        "该行应为 Base32 (A-Z2-7)。got: {:?}",
        line
    );
    assert!(
        line.len() >= 16,
        "TOTP 密钥长度至少 16。got len={}",
        line.len()
    );
}

/// 确保 `tgbot -v` 的 stdout 仅一行且包含版本号，避免其它输出混入。
#[test]
fn cli_version_stdout_is_single_line() {
    let bin = env!("CARGO_BIN_EXE_tgbot");
    let output = Command::new(bin)
        .arg("-v")
        .output()
        .expect("执行 tgbot 失败");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 1, "stdout 应仅一行。stdout={:?}", stdout);
    assert!(
        lines[0].contains("tgbot"),
        "应包含版本信息。got: {:?}",
        lines[0]
    );
}
