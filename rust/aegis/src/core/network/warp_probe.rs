//! WARP 出口连通性探测。
//!
//! ## 为什么需要它
//!
//! Xray 的 `proxy/wireguard` 是 **UDP-only**（WireGuard 协议本身不支持跑在 TCP 上）。
//! 若机房在出站方向丢弃非 53 端口的 UDP，隧道会呈现「发得出去、收不到回包」的
//! 静默黑洞：`wg0` 的 `tx_bytes` 持续增长而 `rx_bytes` 恒为 0，握手永不完成，
//! 经 WARP 的流量全部超时。管理员从 bot 侧**完全无从察觉**——这正是本模块要
//! 消除的盲区（见 `WarpInstaller::status` 的假阳性）。
//!
//! ## 双路探测
//!
//! 两路并行（`tokio::join!`，总超时 3s），互不依赖：
//!
//! 1. **UDP**：向 [`crate::core::paths::warp::UDP_PROBE_ENDPOINT`] 发一个合法的
//!    WireGuard 握手初始化包（type=1，共 148 字节）。收到**任何**回应即说明
//!    出站 UDP 可用。纯超时说明该端口被丢弃。
//! 2. **TCP**：连 [`crate::core::paths::warp::TCP_PROBE_ENDPOINT`] 并做 TLS 握手，
//!    检查 ALPN 是否协商出 `h2`。这条路径对应 MASQUE over HTTP/2（TCP/443），
//!    是出站 UDP 被封时的替代通路。
//!
//! ## 副作用
//!
//! 只发一个 UDP 包 + 一次 TLS 握手，**不建立隧道、不修改配置、不重启服务**。
//! 结果不缓存——每次调用都实测。

use crate::core::paths::warp as warp_paths;
use anyhow::{Context, Result};
use rand::Rng as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::pki_types::{CertificateDer, UnixTime};
use tokio_rustls::rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

/// 单路探测的超时预算。两路并行，故总耗时上限即此值。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// WireGuard 握手初始化包的总长度（含 4 字节消息头）。
///
/// 权威来源：`wireguard-go/device/noise-protocol.go` 的
/// `MessageInitiationSize = 148`。字段布局（小端）：
///
/// ```text
/// Type       u32   4    值 1 = handshake initiation
/// Sender     u32   4
/// Ephemeral  [32]  32
/// Static     [48]  48   (32 公钥 + 16 poly1305 tag)
/// Timestamp  [28]  28   (12 tai64n + 16 tag)
/// MAC1       [16]  16
/// MAC2       [16]  16
/// ─────────────────────
///                  148
/// ```
const WG_HANDSHAKE_INIT_LEN: usize = 148;

/// UDP 路探测结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpVerdict {
    /// 收到任何回应 → 出站 UDP 对该端口可用。
    Available,
    /// 静默超时 → 该端口被丢弃（上游 ACL 或防火墙）。
    Blocked,
    /// 本地发包失败（无路由、权限等），与「被封锁」不同。
    ProbeFailed,
}

/// TCP 路探测结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpVerdict {
    /// TLS 握手成功且 ALPN 协商出 `h2` → 可走 MASQUE over HTTP/2。
    AvailableH2,
    /// TLS 握手成功但未协商出 `h2`。
    OpenNoH2,
    /// 连接或握手失败。
    Blocked,
}

/// 两路结果的组合结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeConclusion {
    /// UDP 与 TCP 都可用 → WireGuard 隧道可直接用。
    WgUsable,
    /// 仅 TCP 可用 → 需走 MASQUE over HTTP/2（见后续 usque 集成）。
    TcpOnly,
    /// 仅 UDP 可用 → WireGuard 可用，TCP 路径不可用。
    UdpOnly,
    /// 两路都不可用 → 出口完全受限。
    FullyBlocked,
}

impl ProbeConclusion {
    /// 由两路结论推导组合结论。
    ///
    /// `UdpVerdict::ProbeFailed` 在组合语义上按「不可用」处理：本地都没发出去，
    /// 自然不能声称 UDP 可用。
    pub fn from_verdicts(udp: UdpVerdict, tcp: TcpVerdict) -> Self {
        let udp_ok = udp == UdpVerdict::Available;
        let tcp_ok = tcp == TcpVerdict::AvailableH2;
        match (udp_ok, tcp_ok) {
            (true, true) => ProbeConclusion::WgUsable,
            (false, true) => ProbeConclusion::TcpOnly,
            (true, false) => ProbeConclusion::UdpOnly,
            (false, false) => ProbeConclusion::FullyBlocked,
        }
    }
}

/// 一次完整探测的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeReport {
    pub udp: UdpVerdict,
    pub tcp: TcpVerdict,
    pub conclusion: ProbeConclusion,
}

/// 构造一个合法的 WireGuard 握手初始化包。
///
/// 字段布局见 [`WG_HANDSHAKE_INIT_LEN`] 的文档；除 `Type` 外全部填随机字节。
///
/// 负载随机是刻意的：探测**不**携带真实密钥，因此即使被对端解析也无法
/// 泄露任何本机凭证；对端对无效 MAC 的处理（回 cookie 或静默）恰好能区分
/// 「端口可达」与「端口被丢弃」。
///
/// 注意 `Type` 是 **u32**（小端），不是 u8——写成单字节会让整包变成 152 字节，
/// 被对端当作畸形包（这正是本函数单测要钉死的事）。
pub fn build_wg_handshake_probe() -> Vec<u8> {
    let mut pkt = Vec::with_capacity(WG_HANDSHAKE_INIT_LEN);

    // Type = 1（handshake initiation），u32 小端。
    pkt.extend_from_slice(&1u32.to_le_bytes());
    // Sender index = 0（探测无需区分）。
    pkt.extend_from_slice(&0u32.to_le_bytes());

    // 余下 140 字节：ephemeral(32) + static(48) + timestamp(28) + mac1(16) + mac2(16)。
    // 复用项目既有的 `rand` 入口（见 `security/crypto.rs`），不额外引入 RNG 依赖。
    let mut payload = [0u8; WG_HANDSHAKE_INIT_LEN - 8];
    rand::rng().fill_bytes(&mut payload);
    pkt.extend_from_slice(&payload);

    debug_assert_eq!(pkt.len(), WG_HANDSHAKE_INIT_LEN);
    pkt
}

/// 向 Cloudflare WARP 的 WireGuard 端点发一个握手包，看是否收到任何回应。
pub async fn probe_udp() -> UdpVerdict {
    let endpoint: SocketAddr = match warp_paths::UDP_PROBE_ENDPOINT.parse() {
        Ok(addr) => addr,
        Err(_) => return UdpVerdict::ProbeFailed,
    };

    // 绑定 0.0.0.0:0 让内核挑源端口。
    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("UDP 探测：本地 socket 绑定失败: {e}");
            return UdpVerdict::ProbeFailed;
        }
    };

    if let Err(e) = sock.send_to(&build_wg_handshake_probe(), endpoint).await {
        log::warn!("UDP 探测：发包到 {endpoint} 失败: {e}");
        return UdpVerdict::ProbeFailed;
    }

    let mut buf = [0u8; 2048];
    match timeout(PROBE_TIMEOUT, sock.recv_from(&mut buf)).await {
        // 收到任何长度的回应都算可达（type 2 为握手响应，type 3 为 cookie 回复）。
        Ok(Ok((n, from))) => {
            log::info!("UDP 探测：{endpoint} 回应 {n} 字节（来自 {from}）");
            UdpVerdict::Available
        }
        Ok(Err(e)) => {
            log::warn!("UDP 探测：接收错误: {e}");
            UdpVerdict::ProbeFailed
        }
        Err(_) => {
            log::info!("UDP 探测：{endpoint} 在 {PROBE_TIMEOUT:?} 内无回应");
            UdpVerdict::Blocked
        }
    }
}

/// 连 TCP/443 并检查 ALPN 是否协商出 `h2`。
pub async fn probe_tcp() -> TcpVerdict {
    match probe_tcp_inner().await {
        Ok(verdict) => verdict,
        Err(e) => {
            log::info!("TCP 探测失败: {e}");
            TcpVerdict::Blocked
        }
    }
}

async fn probe_tcp_inner() -> Result<TcpVerdict> {
    let endpoint = warp_paths::TCP_PROBE_ENDPOINT;

    // 探测只要 ALPN 结果，不需要验签；且引入根证书会多出一个与本探测无关的
    // 失败面（系统根证书缺失）。故用空根存储 + 自定义 verifier 接受任何证书，
    // 与 `core/security/tls_probe.rs` 同一取舍。
    let config = ClientConfig::builder_with_provider(Arc::new(rustls_ring_provider()))
        .with_safe_default_protocol_versions()
        .context("构造 TLS 版本策略失败")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();

    let mut config = config;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let stream = timeout(PROBE_TIMEOUT, TcpStream::connect(endpoint))
        .await
        .context("TCP 探测连接超时")?
        .with_context(|| format!("TCP 探测无法连接 {endpoint}"))?;

    let server_name = ServerName::try_from(warp_paths::TCP_PROBE_SNI)
        .context("非法 SNI")?
        .to_owned();

    let connector = TlsConnector::from(Arc::new(config));
    let mut tls = timeout(PROBE_TIMEOUT, connector.connect(server_name, stream))
        .await
        .context("TCP 探测 TLS 握手超时")?
        .context("TCP 探测 TLS 握手失败")?;

    // 主动关闭，避免对端等我们发请求。
    let _ = tls.shutdown().await;

    let alpn = tls.get_ref().1.alpn_protocol().map(|p| p.to_vec());
    Ok(match alpn.as_deref() {
        Some(b"h2") => TcpVerdict::AvailableH2,
        _ => TcpVerdict::OpenNoH2,
    })
}

fn rustls_ring_provider() -> tokio_rustls::rustls::crypto::CryptoProvider {
    tokio_rustls::rustls::crypto::ring::default_provider()
}

/// 接受任何服务端证书。
///
/// 探测只关心 ALPN 协商结果，证书链验证会引入与本探测无关的失败面
/// （系统根证书缺失等）。取舍与 `core/security/tls_probe.rs` 一致。
#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls_ring_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls_ring_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls_ring_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// 并行跑两路探测并给出组合结论。
///
/// 两路互不依赖，用 `join!` 并发；总耗时上限为单路超时（3s），而非两路之和。
pub async fn run_probe() -> ProbeReport {
    let (udp, tcp) = tokio::join!(probe_udp(), probe_tcp());
    ProbeReport {
        udp,
        tcp,
        conclusion: ProbeConclusion::from_verdicts(udp, tcp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wg_handshake_probe_format() {
        let pkt = build_wg_handshake_probe();
        assert_eq!(
            pkt.len(),
            WG_HANDSHAKE_INIT_LEN,
            "WireGuard 握手初始化包必须是 148 字节"
        );
        // Type 是 u32 小端，不是 u8——写成单字节会得到 152 字节的畸形包。
        assert_eq!(
            u32::from_le_bytes([pkt[0], pkt[1], pkt[2], pkt[3]]),
            1,
            "前 4 字节（u32 小端）必须是 message type = 1"
        );
        assert_eq!(
            u32::from_le_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]),
            0,
            "随后 4 字节（u32 小端）是 sender index，探测用 0"
        );
    }

    #[test]
    fn test_wg_handshake_probe_payload_is_random() {
        // 两次构造的负载应不同——若相同说明随机源没接上，
        // 会让探测包变成可被指纹识别的固定串。
        let a = build_wg_handshake_probe();
        let b = build_wg_handshake_probe();
        assert_ne!(a[8..], b[8..], "握手包负载应为随机，两次构造不应相同");
    }

    /// 字段布局必须与 wireguard-go 的 `MessageInitiationSize = 148` 一致。
    /// 这条测试把「Type 是 u32」这一关键事实钉死。
    #[test]
    fn test_wg_handshake_layout_matches_spec() {
        // Type(4) + Sender(4) + Ephemeral(32) + Static(48) + Timestamp(28)
        // + MAC1(16) + MAC2(16) = 148
        const EXPECTED: usize = 4 + 4 + 32 + 48 + 28 + 16 + 16;
        assert_eq!(EXPECTED, 148);
        assert_eq!(build_wg_handshake_probe().len(), EXPECTED);
    }

    #[test]
    fn test_conclusion_matrix_all_four_states() {
        use ProbeConclusion::{FullyBlocked, TcpOnly, UdpOnly, WgUsable};

        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::Available, TcpVerdict::AvailableH2),
            WgUsable
        );
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::Blocked, TcpVerdict::AvailableH2),
            TcpOnly
        );
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::Available, TcpVerdict::Blocked),
            UdpOnly
        );
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::Blocked, TcpVerdict::Blocked),
            FullyBlocked
        );
    }

    #[test]
    fn test_conclusion_treats_probe_failed_as_unavailable() {
        use ProbeConclusion::{FullyBlocked, TcpOnly};

        // 本地发包失败 + TCP 无 h2 → 不能声称 UDP 可用
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::ProbeFailed, TcpVerdict::OpenNoH2),
            FullyBlocked
        );
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::ProbeFailed, TcpVerdict::AvailableH2),
            TcpOnly
        );
    }

    #[test]
    fn test_open_no_h2_is_not_tcp_usable() {
        use ProbeConclusion::UdpOnly;

        // 连得上但没协商出 h2，不能算 MASQUE 路径可用
        assert_eq!(
            ProbeConclusion::from_verdicts(UdpVerdict::Available, TcpVerdict::OpenNoH2),
            UdpOnly
        );
    }
}
