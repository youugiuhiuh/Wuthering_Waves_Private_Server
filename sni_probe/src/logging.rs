//! 日志开关（对齐 Go `-debug`）。
//!
//! 默认只输出阶段汇总，逐域名明细仅在 `--debug` 时打印；
//! 否则百万级输入会把终端刷爆，也会拖慢整体吞吐。

use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

/// 打开/关闭调试日志。
pub fn set_debug(on: bool) {
    DEBUG.store(on, Ordering::Relaxed);
}

pub fn is_debug() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

/// 仅调试模式下打印。
#[macro_export]
macro_rules! debugln {
    ($($arg:tt)*) => {
        if $crate::logging::is_debug() {
            println!($($arg)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_toggles() {
        set_debug(false);
        assert!(!is_debug());
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
