//! 核心健康检查：升级后判定「新核心是否真的活着」。
//!
//! # 为什么不能只看 `systemctl is-active`
//!
//! 本项目的 unit 模板是 `Type=simple` + `Restart=always` + `RestartSec=5`
//! （见 `core/xray/installer.rs` 的 unit 文本）。Xray 的配置解析发生在
//! **exec 之后**，因此：
//!
//! 1. 进程一 exec，systemd 立刻把 unit 标记为 `active`——哪怕 50ms 后 core
//!    因为配置不兼容而 `exit 23`；
//! 2. `Restart=always` 又会在 5s 后把它拉起来，于是 unit 再次变回 `active`。
//!
//! 结果：**单次 `is-active` 既会误判成功（撞上瞬时 active），也无法识破
//! crash-loop**（崩溃又被拉起时它同样是 active）。抓 crash-loop 的确定性
//! 信号是 `NRestarts` 计数——只有它会在崩溃重启时递增。
//!
//! 因此判定需要两条信号 + 一个跨崩溃周期（> `RestartSec`）的观察窗口。

use crate::core::cmd_async::run_cmd_output;
use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;

/// 观察窗口内的采样间隔。必须**大于** `RestartSec`（5s），否则会在崩溃重启
/// 发生之前就判定为健康，漏掉 crash-loop。
pub const HEALTH_SAMPLE_INTERVAL: Duration = Duration::from_secs(3);

/// 采样次数。默认 3s × 5 = 15s，覆盖至少 2 个完整崩溃周期。
pub const HEALTH_SAMPLE_COUNT: usize = 5;

/// 配置预检结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightOutcome {
    /// 配置能被新核心正常加载。
    Passed,
    /// 预检能力不可用（如上游移除了 `-test` / `check` flag），应 fail-open 放行。
    Unsupported,
    /// 配置与新核心不兼容，升级应在**替换之前**中止。
    Invalid {
        /// 提炼后的根因，用于推送消息网关。
        reason: String,
    },
}

/// 用指定命令做配置预检，并归类结果。
///
/// 预检是升级前的零风险防线：不通过就不碰现网二进制。两个核心的预检命令
/// 不同（Xray `run -test -confdir`，Sing-box `check -C`），但**判定逻辑完全
/// 相同**，故抽到这里共用。
pub async fn run_config_preflight(
    binary: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<PreflightOutcome> {
    let (status, stdout, stderr) = run_cmd_output(&binary.to_string_lossy(), args, timeout)
        .await
        .with_context(|| format!("执行配置预检失败: {} {:?}", binary.display(), args))?;

    let combined = format!("{stderr}\n{stdout}");
    if status.success() {
        return Ok(PreflightOutcome::Passed);
    }
    if is_preflight_unsupported(&stderr) {
        log::warn!(
            "配置预检不可用（疑似上游移除了预检子命令），fail-open 继续升级: {}",
            combined.trim()
        );
        return Ok(PreflightOutcome::Unsupported);
    }

    Ok(PreflightOutcome::Invalid {
        reason: summarize_preflight_failure(&stdout, &stderr),
    })
}

/// 单次 `systemctl` 探测的超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 健康判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthVerdict {
    /// 全程存活，且没有发生过崩溃重启。
    Healthy,
    /// 观察窗口内出现过非 active。
    Inactive,
    /// 一直 active，但 `NRestarts` 递增——崩溃后被 `Restart=always` 拉起。
    CrashLooping,
    /// 探测本身失败（systemctl 不可用、超时等），无法判定。
    ///
    /// 归为「不可判定」而非「不健康」：探测故障不应触发不可逆的回滚。
    Unknown,
}

impl HealthVerdict {
    /// 是否应当触发自动回滚。
    ///
    /// `Unknown` **不**触发：探测工具坏了不等于核心坏了，回滚是不可逆动作，
    /// 不能因为「查不到」就覆盖掉刚装好的新版本。
    pub fn should_rollback(self) -> bool {
        matches!(self, HealthVerdict::Inactive | HealthVerdict::CrashLooping)
    }
}

/// 单次采样的观测值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthSample {
    /// `systemctl is-active` 是否成功。
    pub active: bool,
    /// `NRestarts`，不可得时为 `None`。
    pub restarts: Option<u64>,
}

/// 由采样序列推导最终判定。纯函数，便于表驱动测试。
pub fn classify_samples(samples: &[HealthSample]) -> HealthVerdict {
    if samples.is_empty() {
        return HealthVerdict::Unknown;
    }
    // 探测整体失败：没有任何一次拿到 active 判定。
    if samples.iter().all(|s| !s.active) {
        return HealthVerdict::Inactive;
    }
    if samples.iter().any(|s| !s.active) {
        return HealthVerdict::Inactive;
    }

    // 崩溃重启判定：至少两次成功读到 NRestarts，且末值大于首值。
    let restarts: Vec<u64> = samples.iter().filter_map(|s| s.restarts).collect();
    if restarts.len() >= 2 && restarts.last() > restarts.first() {
        return HealthVerdict::CrashLooping;
    }

    HealthVerdict::Healthy
}

/// 解析 `systemctl show -p NRestarts --value` 的输出。
pub fn parse_nrestarts(stdout: &str) -> Option<u64> {
    stdout.trim().parse::<u64>().ok()
}

/// 判定预检失败是「配置不兼容」还是「预检能力不可用」。
///
/// 上游可能重命名/移除预检 flag（`-test` / `check`）。若把「flag 不认识」
/// 误判成「配置坏了」，升级会被永久阻断——那是灾难性的误伤。因此这类输出
/// 必须 fail-open：跳过预检、继续升级，只靠事后健康检查兜底。
///
/// **只看 stderr**：Go 的 `flag` 包与 cobra 在 flag/命令解析失败时都把错误与
/// usage 写到 stderr；而 Xray 把普通日志（含 warning 与 `Configuration OK.`）
/// 写到 stdout。若把 stdout 也纳入判定，一份恰好含 `usage:` 字样的配置内容
/// 就能让真实的配置错误被误放行。
pub fn is_preflight_unsupported(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "flag provided but not defined",
        "unknown flag",
        "unknown shorthand flag",
        "unknown command",
        "no such flag",
        "usage:",
        "invalid syntax",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// 从预检输出里抽取一句可读的错误，用于推送消息网关。
///
/// Xray 的典型失败输出是 `Failed to start: main: failed to load config files:
/// [...] > infra/conf: <原因>`，整段太长；取第一条含错误语义的行即可。
pub fn summarize_preflight_failure(stdout: &str, stderr: &str) -> String {
    const NOISE: &[&str] = &["failed to start", "error", "failed to load config files"];
    let combined = format!("{stderr}\n{stdout}");

    for line in combined.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if NOISE.iter().any(|n| lower.contains(n)) {
            // Xray 的报错是 `A: B > C` 的链式结构，冒号后的细节才是根因。
            if let Some((_, detail)) = line.rsplit_once(": ") {
                let detail = detail.trim();
                if !detail.is_empty() {
                    return truncate(detail, 300);
                }
            }
            return truncate(line, 300);
        }
    }

    let first = combined
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("未知原因");
    truncate(first, 300)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 对一个 systemd unit 做一次采样。
pub async fn probe_unit(unit: &str) -> Result<HealthSample> {
    let (status, _, _) = run_cmd_output("systemctl", &["is-active", unit], PROBE_TIMEOUT)
        .await
        .with_context(|| format!("执行 systemctl is-active {unit} 失败"))?;

    let restarts = run_cmd_output(
        "systemctl",
        &["show", unit, "-p", "NRestarts", "--value"],
        PROBE_TIMEOUT,
    )
    .await
    .ok()
    .and_then(|(_, stdout, _)| parse_nrestarts(&stdout));

    Ok(HealthSample {
        active: status.success(),
        restarts,
    })
}

/// 在观察窗口内连续采样，返回最终判定。
pub async fn wait_for_health(unit: &str) -> HealthVerdict {
    let mut samples = Vec::with_capacity(HEALTH_SAMPLE_COUNT);

    for index in 0..HEALTH_SAMPLE_COUNT {
        if index > 0 {
            tokio::time::sleep(HEALTH_SAMPLE_INTERVAL).await;
        }
        // 单次探测失败不终止观察：后续采样仍可能拿到有效数据。
        if let Ok(sample) = probe_unit(unit).await {
            samples.push(sample);
        }
    }

    classify_samples(&samples)
}

/// 备份保留策略：返回应当**删除**的备份目录名（不含保留的最近 `keep` 份）。
///
/// 纯函数：输入目录名列表，输出待删列表，便于表驱动测试。
/// 只认以 `prefix` 开头且后缀可解析为时间戳的目录，其他内容一律忽略——
/// 宁可留下垃圾，也不能误删管理员放在同一目录下的东西。
pub fn select_backups_to_delete(names: &[String], prefix: &str, keep: usize) -> Vec<String> {
    let mut candidates: Vec<&String> = names
        .iter()
        .filter(|name| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
        .collect();

    // 目录名后缀是 `%Y%m%d%H%M%S`，字典序即时间序；倒序后取前 keep 份保留。
    candidates.sort_by(|a, b| b.cmp(a));

    // 删除顺序取**最旧优先**：若删除过程被中断，丢掉的总是最陈旧的备份。
    candidates.into_iter().skip(keep).rev().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(restarts: u64) -> HealthSample {
        HealthSample {
            active: true,
            restarts: Some(restarts),
        }
    }

    fn down() -> HealthSample {
        HealthSample {
            active: false,
            restarts: Some(0),
        }
    }

    #[test]
    fn test_classify_samples_table() {
        let cases: Vec<(&str, Vec<HealthSample>, HealthVerdict)> = vec![
            ("无采样", vec![], HealthVerdict::Unknown),
            (
                "全程健康且无重启",
                vec![active(0), active(0), active(0)],
                HealthVerdict::Healthy,
            ),
            (
                "中途掉线",
                vec![active(0), down(), active(0)],
                HealthVerdict::Inactive,
            ),
            (
                "首采样即掉线（配置解析失败）",
                vec![down(), down(), down()],
                HealthVerdict::Inactive,
            ),
            (
                "crash-loop：始终 active 但重启计数递增",
                vec![active(0), active(1), active(2)],
                HealthVerdict::CrashLooping,
            ),
            (
                "重启计数只在最后一次跳变",
                vec![active(3), active(3), active(4)],
                HealthVerdict::CrashLooping,
            ),
            (
                "拿不到 NRestarts 时退化为只看 active",
                vec![
                    HealthSample {
                        active: true,
                        restarts: None,
                    },
                    HealthSample {
                        active: true,
                        restarts: None,
                    },
                ],
                HealthVerdict::Healthy,
            ),
            (
                "NRestarts 恒定且 active",
                vec![active(7), active(7), active(7)],
                HealthVerdict::Healthy,
            ),
        ];

        for (name, samples, expected) in cases {
            assert_eq!(classify_samples(&samples), expected, "case: {name}");
        }
    }

    #[test]
    fn test_should_rollback() {
        assert!(HealthVerdict::Inactive.should_rollback());
        assert!(HealthVerdict::CrashLooping.should_rollback());
        assert!(!HealthVerdict::Healthy.should_rollback());
        // 探测故障不得触发不可逆回滚。
        assert!(!HealthVerdict::Unknown.should_rollback());
    }

    #[test]
    fn test_parse_nrestarts() {
        assert_eq!(parse_nrestarts("0\n"), Some(0));
        assert_eq!(parse_nrestarts("  12  "), Some(12));
        assert_eq!(parse_nrestarts(""), None);
        assert_eq!(parse_nrestarts("N/A"), None);
    }

    #[test]
    fn test_is_preflight_unsupported() {
        let cases = [
            ("flag provided but not defined: -test", true),
            ("unknown flag: --test", true),
            ("Usage:\n  xray run [-c config.json]", true),
            ("unknown command \"check\" for \"wwps-box\"", true),
            (
                "Failed to start: main: failed to load config files: [conf/00_base.json] > infra/conf: invalid \"privateKey\"",
                false,
            ),
            (
                "maxConnections cannot be specified together with maxConcurrency",
                false,
            ),
        ];

        for (stderr, expected) in cases {
            assert_eq!(is_preflight_unsupported(stderr), expected, "{stderr}");
        }
    }

    /// 真实目标机的 Xray 把 warning 与 `Configuration OK.` 打到 **stdout**。
    /// 真正的保证在**调用方**只传 stderr 给分类器——这里用假二进制驱动
    /// `run_config_preflight` 来验证：标记只出现在 stdout 时，必须判为
    /// `Invalid`（中止升级），而不是 `Unsupported`（fail-open 放行）。
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preflight_ignores_unsupported_markers_on_stdout() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("fake-core");
        // 配置错误写 stdout（Xray 日志的真实形态），并故意在 stdout 里
        // 混进 usage 字样——旧实现会因此误判 fail-open。
        std::fs::write(
            &bin,
            "#!/bin/sh\necho \"Failed to start: main: failed to load config files\"\necho \"see usage: docs\"\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let outcome =
            run_config_preflight(&bin, &["run", "-test"], std::time::Duration::from_secs(5))
                .await
                .unwrap();
        // 关键断言：不得是 Unsupported——那会把一个坏配置 fail-open 放行。
        match outcome {
            PreflightOutcome::Invalid { reason } => assert!(
                !reason.is_empty(),
                "Invalid 必须带根因，否则管理员收不到可诊断信息"
            ),
            other => panic!("stdout 中的 usage 字样不应把配置错误误判为 fail-open: {other:?}"),
        }
    }

    /// 相反方向：标记出现在 **stderr** 时才判 Unsupported 并 fail-open。
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preflight_treats_stderr_flag_error_as_unsupported() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("fake-core");
        std::fs::write(
            &bin,
            "#!/bin/sh\necho \"flag provided but not defined: -test\" >&2\necho \"Usage:\" >&2\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let outcome =
            run_config_preflight(&bin, &["run", "-test"], std::time::Duration::from_secs(5))
                .await
                .unwrap();
        assert_eq!(outcome, PreflightOutcome::Unsupported);
    }

    #[test]
    fn test_summarize_preflight_failure_prefers_root_cause() {
        let stdout = "";
        let stderr = "Failed to start: main: failed to load config files: [conf/00_base.json] > infra/conf: invalid \"privateKey\"";
        assert_eq!(
            summarize_preflight_failure(stdout, stderr),
            "invalid \"privateKey\""
        );
    }

    #[test]
    fn test_summarize_preflight_failure_falls_back_to_first_line() {
        let stderr = "something went wrong\ndetail line";
        assert_eq!(
            summarize_preflight_failure("", stderr),
            "something went wrong"
        );
    }

    #[test]
    fn test_summarize_preflight_failure_handles_empty() {
        assert_eq!(summarize_preflight_failure("", ""), "未知原因");
    }

    #[test]
    fn test_summarize_preflight_failure_truncates() {
        let long = format!("error: {}", "x".repeat(500));
        let summary = summarize_preflight_failure("", &long);
        assert!(
            summary.chars().count() <= 301,
            "{}",
            summary.chars().count()
        );
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn test_select_backups_to_delete_keeps_newest() {
        let names: Vec<String> = [
            "wwps-core-backup-20260101000000",
            "wwps-core-backup-20260301000000",
            "wwps-core-backup-20260201000000",
            "wwps-core-backup-20260401000000",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let deleted = select_backups_to_delete(&names, "wwps-core-backup-", 2);
        assert_eq!(
            deleted,
            vec![
                "wwps-core-backup-20260101000000".to_string(),
                "wwps-core-backup-20260201000000".to_string(),
            ]
        );
    }

    #[test]
    fn test_select_backups_to_delete_ignores_foreign_entries() {
        let names: Vec<String> = [
            "wwps-core-backup-20260101000000",
            "wwps-core-backup-20260201000000",
            "README.txt",
            "wwps-core-backup-notatimestamp",
            ".upgrade-journal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        // 保留 1 份，只有时间戳目录会被删；README 等一律不碰。
        let deleted = select_backups_to_delete(&names, "wwps-core-backup-", 1);
        assert_eq!(deleted, vec!["wwps-core-backup-20260101000000".to_string()]);
    }

    #[test]
    fn test_select_backups_to_delete_keep_all_when_under_limit() {
        let names = vec!["wwps-core-backup-20260101000000".to_string()];
        assert!(select_backups_to_delete(&names, "wwps-core-backup-", 3).is_empty());
    }
}
