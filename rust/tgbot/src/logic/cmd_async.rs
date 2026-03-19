use anyhow::{Context, Result};
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Run a command with args, returning status/stdout/stderr. Uses timeout to avoid hanging.
pub async fn run_cmd_output(
    program: &str,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<(ExitStatus, String, String)> {
    let mut cmd = Command::new(program);
    cmd.args(args);

    let output = timeout(timeout_duration, cmd.output())
        .await
        .context("命令执行超时")?
        .context("命令启动失败")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status, stdout, stderr))
}

/// Run a command, ignoring stdout/stderr, returning status only.
pub async fn run_cmd_status(
    program: &str,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<ExitStatus> {
    let (status, _out, _err) = run_cmd_output(program, args, timeout_duration).await?;
    Ok(status)
}

/// Run a command and require a successful exit status.
pub async fn run_cmd_checked(
    program: &str,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<(String, String)> {
    let (status, stdout, stderr) = run_cmd_output(program, args, timeout_duration).await?;
    if !status.success() {
        let details = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        anyhow::bail!("命令 {:?} 执行失败: {}", [program], details);
    }
    Ok((stdout, stderr))
}

/// Run a command and stream its output line by line to a callback.
pub async fn run_cmd_stream<F>(
    program: &str,
    args: &[&str],
    timeout_duration: Duration,
    mut on_line: F,
) -> Result<ExitStatus>
where
    F: FnMut(String),
{
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("无法启动流式命令")?;

    let stdout = child.stdout.take().context("无法获取 stdout 流")?;
    let stderr = child.stderr.take().context("无法获取 stderr 流")?;

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let execution = async {
        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => on_line(l),
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => on_line(l),
                        _ => {}
                    }
                }
            }
        }
        child.wait().await.context("等待命令执行失败")
    };

    timeout(timeout_duration, execution)
        .await
        .context("命令执行超时")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_cmd_checked_returns_error_on_non_zero_exit() {
        let result = run_cmd_checked("sh", &["-c", "exit 7"], Duration::from_secs(1)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_cmd_output_returns_stdout_on_success() {
        let result = run_cmd_output("sh", &["-c", "echo -n hello"], Duration::from_secs(2)).await;
        assert!(result.is_ok());
        let (status, stdout, _) = result.unwrap();
        assert!(status.success());
        assert!(stdout.trim().contains("hello"));
    }

    #[tokio::test]
    async fn run_cmd_status_success() {
        let result = run_cmd_status("true", &[], Duration::from_secs(1)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().success());
    }

    #[tokio::test]
    async fn run_cmd_output_captures_stderr() {
        let result = run_cmd_output("sh", &["-c", "echo error >&2"], Duration::from_secs(2)).await;
        assert!(result.is_ok());
        let (_, _, stderr) = result.unwrap();
        assert!(stderr.contains("error"));
    }

    #[tokio::test]
    async fn run_cmd_checked_fails_on_non_zero_with_error_message() {
        let result = run_cmd_checked(
            "sh",
            &["-c", "echo fail >&2; exit 1"],
            Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("fail") || err.to_string().contains("exit"));
    }

    #[tokio::test]
    async fn run_cmd_output_nonexistent_command_fails() {
        let result = run_cmd_output("nonexistent_command_xyz", &[], Duration::from_secs(1)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_cmd_stream_callback_receives_lines() {
        let mut lines = Vec::new();
        let result = run_cmd_stream(
            "sh",
            &["-c", "echo line1; echo line2; echo line3"],
            Duration::from_secs(2),
            |line| {
                lines.push(line);
            },
        )
        .await;
        assert!(result.is_ok());
        assert!(
            lines
                .iter()
                .any(|l| l.contains("line1") || l.contains("line2") || l.contains("line3"))
        );
    }

    #[tokio::test]
    async fn run_cmd_stream_timeout_returns_error() {
        let result = run_cmd_stream(
            "sh",
            &["-c", "sleep 10"],
            Duration::from_millis(100),
            |_line| {},
        )
        .await;
        assert!(result.is_err());
    }
}
