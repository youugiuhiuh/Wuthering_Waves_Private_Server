//! 进程唯一的日志器。
//!
//! 在此之前 aegis 从未安装过日志器：`log` crate 在无 logger 时把 `log::logger()` 保持为
//! `NopLogger`（`enabled()` 恒 false），139 处 `log::*` 在生产二进制里全部退化为 no-op。
//! 运维唯一能取到 SimpleX contactId 的途径（`runtime.rs` 的「未授权联系人」告警）因此
//! 永远不出现，`--set-simplex-admin` 无值可填，onboarding 无法闭环。
//!
//! 写入 stderr 而非 stdout：安装器把 `--generate-totp-secret` 的整段 stdout 当成 TOTP 密钥
//! （见 `main.rs` 顶部注释），stdout 必须保持干净。systemd 默认把 stderr 收进 journal，
//! `Binary Integrity Hash:` 走 `eprintln!` 是既有先例。
//!
//! 故意不使用缓冲（`perf-io-buffering` 的例外）：release profile 是 `panic = "abort"`，
//! 缓冲会在崩溃时丢掉最后几行 —— 正是排障最需要的那几行。

use std::io::Write;

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // 不用 eprintln!：它会在 broken pipe 上 panic。日志失败绝不能让进程退出。
        // writeln! 直接格式化进 stderr，不走 format! 的临时分配（mem-write-over-format）。
        let _ = writeln!(std::io::stderr(), "[{}] {}", record.level(), record.args());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// 解析 `WWPS_LOG` 的级别名；无法识别时回落到 Info。
///
/// `log::LevelFilter` 已实现 `FromStr`（大小写不敏感，见 log 0.4.34 `src/lib.rs:674-685`），
/// 所以这里只补一个 `trim`（`FromStr` 不做 trim），不重复实现级别名表。
fn parse_level(raw: &str) -> log::LevelFilter {
    raw.trim().parse().unwrap_or(log::LevelFilter::Info)
}

/// 默认 Info：139 处调用站点无法逐个人工分级，Info 保住运维必需的行而不至于灌爆 journal。
/// 这个旋钮存在的理由是 3am 排查要能调级别，而不必重新编译或重发签名二进制。
/// 只认这一个变量，不引入 `RUST_LOG` 之类的外部约定。
fn level_from_env() -> log::LevelFilter {
    parse_level(&std::env::var("WWPS_LOG").unwrap_or_default())
}

/// 安装日志器。必须在 `main()` 的**第一条**语句调用。
///
/// `set_logger` 重复调用返回 Err（测试里会）—— 忽略：日志器是进程全局的，
/// 第二次安装无效但无害。
pub fn init_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level_from_env());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「没有安装日志器」的机器可判定特征：`log::logger()` 在未安装时返回 `NopLogger`，
    /// 而 `NopLogger::enabled()` 恒为 false —— 这正是 139 处 `log::*` 被静默丢弃的原因。
    /// 注意不能用 `log::max_level()`：单独的 `set_max_level` 就能把它设为非 Off，
    /// 即使一个 logger 都没有。
    #[test]
    fn init_logger_installs_a_logger_whose_records_are_enabled() {
        init_logger();
        let meta = log::Metadata::builder().level(log::Level::Info).build();
        assert!(
            log::logger().enabled(&meta),
            "没有安装日志器：所有 log::* 都会被静默丢弃"
        );
    }

    #[test]
    fn parse_level_accepts_known_names_case_insensitively() {
        assert_eq!(parse_level("off"), log::LevelFilter::Off);
        assert_eq!(parse_level("ERROR"), log::LevelFilter::Error);
        assert_eq!(parse_level(" warn "), log::LevelFilter::Warn);
        assert_eq!(parse_level("Info"), log::LevelFilter::Info);
        assert_eq!(parse_level("debug"), log::LevelFilter::Debug);
        assert_eq!(parse_level("Trace"), log::LevelFilter::Trace);
    }

    #[test]
    fn parse_level_defaults_to_info() {
        assert_eq!(parse_level(""), log::LevelFilter::Info);
        assert_eq!(parse_level("nonsense"), log::LevelFilter::Info);
    }
}
