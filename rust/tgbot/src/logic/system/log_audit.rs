use anyhow::Result;
use std::time::Duration;

use crate::logic::cmd_async::run_cmd_output;

const TIMEOUT_SHORT: Duration = Duration::from_secs(30);
const MAX_LOG_CHARS: usize = 4000;
#[allow(dead_code)]
const TAIL_LINES: usize = 50;

pub const SERVICE_WWPS_CORE: &str = "wwps-core";
pub const SERVICE_SING_BOX: &str = "wwps-box";

pub struct LogAudit;

pub struct ServiceStatus {
    pub active: bool,
    pub status_text: String,
}

impl LogAudit {
    /// 获取服务最近 N 行日志
    /// 调用: journalctl -u <service> -n <lines> --no-pager
    pub async fn tail_logs(service: &str, lines: usize) -> Result<String> {
        let (status, stdout, stderr) = run_cmd_output(
            "journalctl",
            &["-u", service, "-n", &lines.to_string(), "--no-pager"],
            TIMEOUT_SHORT,
        )
        .await?;

        if !status.success() && !stderr.is_empty() {
            anyhow::bail!("journalctl error: {}", stderr);
        }

        Ok(format_output(&stdout))
    }

    /// 检查服务状态
    /// 调用: systemctl is-active <service>
    pub async fn service_status(service: &str) -> ServiceStatus {
        match run_cmd_output("systemctl", &["is-active", service], TIMEOUT_SHORT).await {
            Ok((status, stdout, _)) => ServiceStatus {
                active: status.success() || stdout.trim() == "active",
                status_text: stdout.trim().to_string(),
            },
            Err(_) => ServiceStatus {
                active: false,
                status_text: "unknown".to_string(),
            },
        }
    }
}

fn format_output(output: &str) -> String {
    if output.len() <= MAX_LOG_CHARS {
        output.to_string()
    } else {
        format!(
            "... (Truncated)\n{}",
            &output[output.len() - (MAX_LOG_CHARS - 20)..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_output_short() {
        let short = "short log";
        assert_eq!(format_output(short), short);
    }

    #[test]
    fn test_service_status_constants() {
        assert_eq!(SERVICE_WWPS_CORE, "wwps-core");
        assert_eq!(SERVICE_SING_BOX, "wwps-box");
    }
}
