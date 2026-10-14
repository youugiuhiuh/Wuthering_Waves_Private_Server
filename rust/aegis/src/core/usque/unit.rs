//! usque 的 systemd unit 文本生成（纯函数）。
//!
//! 生成纯函数而非 I/O：unit 的内容是本模块**最重要的安全约束**的载体
//! （见 [`service_unit`] 的文档），必须能被直接断言。
//! 真正写盘与 `systemctl` 调用在 [`super::installer`]。

use crate::core::paths::usque as usque_paths;

/// `Restart=always` 的重启间隔。
pub const RESTART_SEC: u32 = 5;

/// 生成 `wwps-usque.service` 的完整文本。
///
/// # 为什么用 `socks --http2` 而**不是** `l4-socks`
///
/// `l4-socks` **根本没有 `--http2` 标志**——它是 QUIC-only 的。在本部署
/// 所在机房（出站 UDP 被封）下实测吞吐约 1 KB/s，等于不可用。
///
/// 上游 README 写着「l4-socks is faster for TCP-only SOCKS use cases」，
/// 但那个建议的前提是 **QUIC 可用**。在 UDP 被封的机器上照抄该建议会
/// 把节点搞挂，而且日志看起来一切正常。
///
/// 因此这里有一条**防回归断言**（见本文件测试
/// `test_unit_uses_socks_http2_not_l4`）：unit 必须含 `socks --http2`，
/// 且**不得**含 `l4-socks`。
///
/// # 为什么绑 `127.0.0.1`
///
/// usque 的 SOCKS **不加密也不认证**，对外暴露等于开放代理。
/// 它只应被本机 Xray 的 `socks` outbound 使用。
pub fn service_unit(bin: &str, config: &str, port: u16) -> String {
    format!(
        "[Unit]\n\
         Description=wwps-usque (WARP over TCP/443, MASQUE/HTTP-2)\n\
         After=network.target\n\
         \n\
         [Service]\n\
         User=root\n\
         Type=simple\n\
         ExecStart={bin} socks --http2 -p {port} -b 127.0.0.1 -c {config}\n\
         Restart=always\n\
         RestartSec={RESTART_SEC}\n\
         LimitNOFILE=51200\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

/// 用已 pin 的路径与端口生成 unit（供安装流程直接调用）。
pub fn default_service_unit() -> String {
    service_unit(
        usque_paths::BIN,
        usque_paths::CONFIG_FILE,
        usque_paths::SOCKS_PORT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **硬约束 1 的机械保证**：必须用 `socks --http2`，禁止 `l4-socks`。
    ///
    /// `l4-socks` 没有 `--http2` 标志，是 QUIC-only；在本部署（出站 UDP
    /// 被封）下实测约 1 KB/s。上游 README 的「l4 更快」建议前提是 QUIC
    /// 可用——照抄会把节点搞挂。
    #[test]
    fn test_unit_uses_socks_http2_not_l4() {
        let u = service_unit("/etc/wwps/usque/usque", "/etc/wwps/usque/config.json", 1080);
        assert!(
            u.contains("socks --http2"),
            "必须用 `socks --http2`（TCP 模式）。实际 unit:\n{u}"
        );
        assert!(
            !u.contains("l4-socks"),
            "禁止 `l4-socks`：它无 --http2 支持，本机房下等于不可用。实际 unit:\n{u}"
        );
        assert!(
            !u.contains("l4-http-proxy"),
            "禁止 l4-* 系列（同为 QUIC-only）"
        );
    }

    /// SOCKS 不加密不认证 → 必须只绑回环。
    #[test]
    fn test_unit_binds_loopback_only() {
        let u = service_unit("/bin/usque", "/c.json", 1080);
        assert!(u.contains("-b 127.0.0.1"), "必须只监听回环");
        assert!(!u.contains("0.0.0.0"), "不得监听所有地址");
        assert!(!u.contains("-b ::"), "不得监听 IPv6 通配");
    }

    /// 端口与配置路径必须真的出现在 ExecStart 里（防参数漏拼）。
    #[test]
    fn test_unit_carries_port_and_config() {
        let u = service_unit("/opt/usque", "/etc/wwps/usque/config.json", 1080);
        assert!(u.contains("-p 1080"), "端口必须出现在 ExecStart");
        assert!(
            u.contains("-c /etc/wwps/usque/config.json"),
            "配置路径必须出现在 ExecStart"
        );
        assert!(u.contains("/opt/usque socks"), "二进制路径必须在开头");
    }

    /// unit 结构必需项——沿用项目既有 unit 风格
    /// （`core/singbox/installer.rs` 的 `create_service`）。
    #[test]
    fn test_unit_has_required_structure() {
        let u = service_unit("/bin/usque", "/c.json", 1080);
        assert!(u.contains("[Unit]"));
        assert!(u.contains("[Service]"));
        assert!(u.contains("[Install]"));
        assert!(u.contains("After=network.target"));
        assert!(u.contains("Type=simple"));
        assert!(u.contains("Restart=always"), "崩溃需自动拉起");
        assert!(u.contains("RestartSec=5"), "与项目其它 unit 一致");
        assert!(u.contains("LimitNOFILE=51200"), "与项目其它 unit 一致");
        assert!(u.contains("WantedBy=multi-user.target"));
    }

    /// 每次重启固定间隔——不能依赖 systemd 默认值。
    #[test]
    fn test_restart_sec_is_pinned() {
        assert_eq!(RESTART_SEC, 5);
        assert!(service_unit("/b", "/c", 1).contains("RestartSec=5"));
    }

    #[test]
    fn test_default_unit_uses_pinned_paths() {
        let u = default_service_unit();
        assert!(u.contains(usque_paths::BIN));
        assert!(u.contains(usque_paths::CONFIG_FILE));
        assert!(u.contains(&format!("-p {}", usque_paths::SOCKS_PORT)));
    }

    /// 端口可变——测试用非默认端口也必须能正确生成。
    #[test]
    fn test_unit_honours_custom_port() {
        let u = service_unit("/b", "/c", 10801);
        assert!(u.contains("-p 10801"));
        assert!(!u.contains("-p 1080 "), "不得残留默认端口");
    }
}
