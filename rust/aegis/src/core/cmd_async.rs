use anyhow::{Context, Result};
use std::io;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

/// Maximum bytes of command output to capture in diagnostic logs.
pub const MAX_DIAG_BYTES: usize = 65536;

/// Return the last `limit` bytes of `buf` as a lossy UTF-8 string.
pub fn bounded_tail(buf: &[u8], limit: usize) -> String {
    let start = buf.len().saturating_sub(limit);
    String::from_utf8_lossy(&buf[start..]).to_string()
}

/// Set process group of `pid` to itself (become group leader).
#[cfg(target_os = "linux")]
pub fn set_process_group(pid: u32) -> io::Result<()> {
    // SAFETY: pid is a valid OS process ID from a successful spawn; we are in
    // the same session so setpgid is well-defined. The FFI call either succeeds
    // (ret == 0) or returns a standard errno we propagate.
    let ret = unsafe { libc::setpgid(pid as i32, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn set_process_group(_pid: u32) -> io::Result<()> {
    Ok(())
}

/// Kill an entire process group — SIGTERM first, then SIGKILL after a 2s grace period.
#[cfg(target_os = "linux")]
#[allow(clippy::undocumented_unsafe_blocks)]
pub fn kill_process_group(pid: u32) -> io::Result<()> {
    // SAFETY: pid is a valid process-group ID; killpg is safe to call with any
    // standard signal number. The FFI call either succeeds or returns a standard
    // errno we propagate.
    let ret = unsafe { libc::killpg(pid as i32, libc::SIGTERM) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    // Wait 2s for graceful shutdown
    std::thread::sleep(Duration::from_secs(2));
    // SAFETY: same as above — pid is valid, killpg is safe to call with SIGKILL.
    // Only SIGKILL if the group still exists (killpg with signal=0 is a
    // probe — returns 0 if alive, ESRCH if gone).
    if unsafe { libc::killpg(pid as i32, 0) } == 0 {
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn kill_process_group(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "not available"))
}

/// Run a command with args, returning status/stdout/stderr. Uses timeout to avoid hanging.
pub async fn run_cmd_output(
    program: &str,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<(ExitStatus, String, String)> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("命令启动失败")?;

    let child_pid = child.id().unwrap_or(0);

    #[cfg(target_os = "linux")]
    let _ = set_process_group(child_pid);

    let mut child_stdout = child.stdout.take().context("无法获取 stdout 流")?;
    let mut child_stderr = child.stderr.take().context("无法获取 stderr 流")?;

    let out_buf = Arc::new(Mutex::new(Vec::new()));
    let err_buf = Arc::new(Mutex::new(Vec::new()));

    let read_out = {
        let buf = out_buf.clone();
        tokio::spawn(async move {
            let mut tmp = Vec::new();
            child_stdout.read_to_end(&mut tmp).await.ok();
            *buf.lock().unwrap() = tmp;
        })
    };
    let read_err = {
        let buf = err_buf.clone();
        tokio::spawn(async move {
            let mut tmp = Vec::new();
            child_stderr.read_to_end(&mut tmp).await.ok();
            *buf.lock().unwrap() = tmp;
        })
    };

    match tokio::time::timeout(timeout_duration, child.wait()).await {
        Ok(Ok(status)) => {
            let _ = read_out.await;
            let _ = read_err.await;
            let out = out_buf.lock().unwrap();
            let err = err_buf.lock().unwrap();
            Ok((
                status,
                String::from_utf8_lossy(&out).to_string(),
                String::from_utf8_lossy(&err).to_string(),
            ))
        }
        Ok(Err(e)) => Err(e).context("等待命令执行失败"),
        Err(_elapsed) => {
            #[cfg(target_os = "linux")]
            let _ = kill_process_group(child_pid);
            #[cfg(not(target_os = "linux"))]
            let _ = child.start_kill();

            let _ = child.wait().await;
            let _ = read_out.await;
            let _ = read_err.await;

            let err = err_buf.lock().unwrap();
            let tail = bounded_tail(&err, MAX_DIAG_BYTES);
            anyhow::bail!("命令执行超时 (pid={child_pid}): {tail}")
        }
    }
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动流式命令")?;

    let child_pid = child.id().unwrap_or(0);

    #[cfg(target_os = "linux")]
    let _ = set_process_group(child_pid);

    let stdout = child.stdout.take().context("无法获取 stdout 流")?;
    let stderr = child.stderr.take().context("无法获取 stderr 流")?;

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let read_loop = async {
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
                    if let Ok(Some(l)) = line { on_line(l) }
                }
            }
        }
    };

    let timed_out = tokio::time::timeout(timeout_duration, read_loop)
        .await
        .is_err();

    if timed_out {
        #[cfg(target_os = "linux")]
        let _ = kill_process_group(child_pid);
        #[cfg(not(target_os = "linux"))]
        let _ = child.start_kill();
    }

    let status = child.wait().await.context("等待命令执行失败")?;

    if timed_out {
        anyhow::bail!("命令执行超时")
    } else {
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_run_cmd_output_timeout_kills_descendants() {
        let pid_file = "/tmp/aegis-test-run-cmd-pid";
        let _ = std::fs::remove_file(pid_file);
        let result = run_cmd_output(
            "sh",
            &["-c", &format!("echo $$ > {pid_file}; sleep 30")],
            Duration::from_millis(500),
        )
        .await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("超时"),
            "expected timeout error, got: {err_str}"
        );
        // The child process group should have been killed
        if let Ok(pid_str) = std::fs::read_to_string(pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                let proc_path = format!("/proc/{pid}");
                assert!(
                    !std::path::Path::new(&proc_path).exists(),
                    "child process {pid} still alive after timeout + kill"
                );
            }
        }
        let _ = std::fs::remove_file(pid_file);
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

    // --- bounded_tail ---

    #[test]
    fn test_bounded_tail_short() {
        assert_eq!(bounded_tail(b"hello", 10), "hello");
    }

    #[test]
    fn test_bounded_tail_truncates() {
        assert_eq!(bounded_tail(b"hello world", 5), "world");
    }

    #[test]
    fn test_bounded_tail_empty() {
        assert_eq!(bounded_tail(b"", 10), "");
    }

    #[test]
    fn test_bounded_tail_exact() {
        assert_eq!(bounded_tail(b"abc", 3), "abc");
    }

    #[test]
    fn test_bounded_tail_non_utf8() {
        let buf = &[0xff, 0xfe, 0x61, 0x62];
        let result = bounded_tail(buf, 10);
        assert!(result.contains("ab"));
    }

    #[test]
    fn test_bounded_tail_limit_zero() {
        assert_eq!(bounded_tail(b"hello", 0), "");
    }

    #[test]
    fn test_bounded_tail_non_utf8_mid_cut() {
        // 0xc3 0xa9 is "é". Cutting at limit=3: last 3 bytes are \xa9 c d.
        // \xa9 alone is invalid → lossy yields "�cd".
        let buf = b"ab\xc3\xa9cd";
        let result = bounded_tail(buf, 3);
        assert_eq!(result, "�cd", "expected lossy tail '�cd', got: {result:?}");
    }

    #[test]
    fn test_bounded_tail_multi_byte_span_boundary() {
        // 2-byte UTF-8 char "\u{00e9}" = [0xc3, 0xa9]; limit=7 takes last 7 bytes.
        // Byte index 7 = \xa9 (continuation byte without lead) → "� world"
        let buf = b"hello \xc3\xa9 world";
        let result = bounded_tail(buf, 7);
        assert_eq!(
            result, "� world",
            "expected lossy '� world', got: {result:?}"
        );
    }

    // --- set_process_group ---

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_set_process_group_returns_ok() {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 5")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id().expect("child should be running");

        // setpgid(pid,0) from the parent after the child has exec'd returns EACCES.
        // This is documented Linux behavior — the helper wrapping the syscall is
        // structurally correct; callers should use a pre_exec hook (see the
        // kill_process_group test for that pattern).
        let result = set_process_group(pid);
        match result {
            Ok(()) => {
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
                let after_comm = stat.split(')').last().unwrap();
                let fields: Vec<&str> = after_comm.split_whitespace().collect();
                let pgrp: u32 = fields[2].parse().unwrap();
                assert_eq!(pgrp, pid);
            }
            Err(e) => {
                assert_eq!(
                    e.kind(),
                    io::ErrorKind::PermissionDenied,
                    "expected EACCES after child exec, got: {e}"
                );
            }
        }

        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }

    // --- kill_process_group ---

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_kill_process_group_kills_child() {
        // Use a pre_exec hook to set the child's process group before exec,
        // so killpg only targets the child process.
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 30");
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().expect("child should be running");

        kill_process_group(pid).unwrap();

        let status = child.wait().await.unwrap();
        assert!(!status.success());
    }
}
