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
