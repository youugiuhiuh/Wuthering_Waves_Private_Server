//! TLS 1.3 深探测：单次 BoringSSL 握手同时取回 PQ 协商结果、证书链尺寸与首跳响应头。
//!
//! 为什么用 BoringSSL 而不是 rustls：只改 ClientHello 的扩展顺序/缺省扩展而保留 rustls
//! 的握手实现，JA3/JA4 仍与真实浏览器不符；BoringSSL 才能拿到与 Chrome 一致的
//! ClientHello wire format（含 PQ 混合 kex、GREASE）。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use boring::ssl::{SslConnector, SslMethod, SslVerifyMode, SslVersion};

use crate::h2;

/// TLS 1.3 协商结果（对应 Go 版 `TLSResult`）。
#[derive(Debug, Clone, Default)]
pub struct TlsProbe {
    pub tls13: bool,
    pub alpn: Option<String>,
    /// 协商出的密钥交换组名，如 `X25519MLKEM768`（PQ）或 `X25519`。
    pub curve: Option<String>,
    pub cert_chain_len: usize,
    pub cert_total_bytes: usize,
    pub headers: HashMap<String, String>,
    pub http_status: Option<u16>,
    /// 目标是否宣告支持 HTTP/3（`Alt-Svc: h3`）
    pub h3_support: bool,
}

impl TlsProbe {
    /// 是否使用后量子混合密钥交换。
    pub fn is_pq(&self) -> bool {
        self.curve.as_deref().is_some_and(is_pq_group)
    }

    /// 是否使用 X25519 系密钥交换（REALITY 要求）。
    pub fn is_valid_group(&self) -> bool {
        self.curve.as_deref().is_some_and(is_valid_group)
    }
}

/// 判定协商出的组名是否属于 PQ 混合。
pub fn is_pq_group(curve: &str) -> bool {
    let c = curve.to_ascii_uppercase();
    c.contains("MLKEM") || c.contains("KYBER")
}

/// 判定协商出的组名是否在 REALITY 可接受集合内。
pub fn is_valid_group(curve: &str) -> bool {
    let c = curve.to_ascii_uppercase();
    c.starts_with("X25519")
}

const SNI_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn build_connector() -> Result<SslConnector, Box<dyn std::error::Error + Send + Sync>> {
    let mut b = SslConnector::builder(SslMethod::tls())?;
    b.set_verify(SslVerifyMode::NONE);
    b.set_min_proto_version(Some(SslVersion::TLS1_3))?;
    b.set_max_proto_version(Some(SslVersion::TLS1_3))?;
    b.set_alpn_protos(b"\x02h2\x08http/1.1")?;
    // X25519MLKEM768 放首位：既拿 PQ 结果，又与 Chrome 默认顺序一致
    b.set_curves_list("X25519MLKEM768:X25519")?;
    Ok(b.build())
}

/// 对 `domain` + `ip` 发起一次 TLS 1.3 握手，并在同一条连接上取回首跳响应头。
pub fn probe(
    domain: &str,
    ip: IpAddr,
    timeout: Duration,
) -> Result<TlsProbe, Box<dyn std::error::Error + Send + Sync>> {
    let connector = build_connector()?;

    let tcp = crate::bind::connect_tcp(SocketAddr::new(ip, 443), timeout)?;
    tcp.set_read_timeout(Some(timeout))?;
    tcp.set_write_timeout(Some(timeout))?;

    let mut stream = connector.connect(domain, tcp)?;

    let mut result = TlsProbe {
        tls13: stream.ssl().version_str() == "TLSv1.3",
        alpn: stream
            .ssl()
            .selected_alpn_protocol()
            .map(|p| String::from_utf8_lossy(p).to_string()),
        curve: stream.ssl().curve_name().map(str::to_string),
        ..Default::default()
    };

    if let Some(chain) = stream.ssl().peer_cert_chain() {
        result.cert_chain_len = chain.len();
        result.cert_total_bytes = chain
            .iter()
            .map(|c| c.to_der().map(|d| d.len()).unwrap_or(0))
            .sum();
    }

    let headers = if result.alpn.as_deref() == Some("h2") {
        h2::head(&mut stream, domain, SNI_USER_AGENT).unwrap_or_default()
    } else if result.alpn.as_deref() == Some("http/1.1") {
        head_request_http11(&mut stream, domain).unwrap_or_default()
    } else {
        HashMap::new()
    };

    result.h3_support = headers
        .get("alt-svc")
        .is_some_and(|v| v.to_ascii_lowercase().contains("h3"));
    result.http_status = headers
        .get(":status")
        .and_then(|s| s.parse::<u16>().ok())
        .or_else(|| headers.get("status").and_then(|s| s.parse::<u16>().ok()));
    result.headers = headers;

    Ok(result)
}

/// HTTP/1.1 回退路径：发送 HEAD 并解析状态行与头部。
fn head_request_http11<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    host: &str,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let req = format!(
        "HEAD / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {SNI_USER_AGENT}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;

    let mut buf = vec![0u8; 8192];
    let mut total = Vec::new();
    while let Ok(n) = stream.read(&mut buf) {
        if n == 0 {
            break;
        }
        total.extend_from_slice(&buf[..n]);
        if total.windows(4).any(|w| w == b"\r\n\r\n") || total.len() > 16 * 1024 {
            break;
        }
    }

    let text = String::from_utf8_lossy(&total);
    let mut headers = HashMap::new();
    let mut lines = text.lines();
    if let Some(status_line) = lines.next() {
        // 例：HTTP/1.1 301 Moved Permanently
        if let Some(code) = status_line.split_whitespace().nth(1) {
            headers.insert("status".to_string(), code.to_string());
        }
    }
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Ok(headers)
}

/// REALITY 证书链深度上限：终端证书 + 最多 2 张中间 CA。
pub const MAX_CERT_CHAIN_DEPTH: usize = 3;
/// REALITY 证书总字节上限：Xray 内部 8192 字节缓冲区，留足裕量。
pub const MAX_CERT_BYTES: usize = 5500;

/// 只完成 TLS 1.3 握手，不解析证书链、不发任何 HTTP 请求。
///
/// 用于连通性判定（判断"能不能连上"），因此比 `probe_survival` 更轻。
pub fn handshake_only(
    domain: &str,
    ip: IpAddr,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connector = build_connector()?;
    let tcp = crate::bind::connect_tcp(SocketAddr::new(ip, 443), timeout)?;
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));
    let _stream = connector.connect(domain, tcp)?;
    Ok(())
}

/// 阶段二：极轻量存活探测，**不解析证书、不发任何 HTTP 请求**。
///
/// 握手成功后再等 150ms 做一次非阻塞读，用于捕获审查设备迟发的 TCP RST
/// （只看握手完成会把「握手成功但随即被重置」误判为存活）。
pub fn probe_survival(domain: &str, ip: IpAddr, timeout: Duration) -> bool {
    use std::io::Read;

    let Ok(connector) = build_connector() else {
        return false;
    };
    let Ok(tcp) = crate::bind::connect_tcp(SocketAddr::new(ip, 443), timeout) else {
        return false;
    };
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));

    let Ok(mut stream) = connector.connect(domain, tcp) else {
        return false;
    };

    // 后置探读：审查系统常在握手后数十毫秒才注入 RST
    let _ = stream
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(150)));
    let mut probe = [0u8; 1];
    let mut filled = 0usize;
    match stream.read(&mut probe) {
        Ok(n) => {
            filled += n;
            let _ = filled;
            true // 有数据或 EOF 都说明连接未被审查切断
        }
        Err(e) => !matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
        ),
    }
}

/// 深度质检：是否为合格 REALITY 伪装目标。
pub fn validate(p: &TlsProbe) -> bool {
    p.tls13
        && p.is_valid_group()
        && p.cert_chain_len <= MAX_CERT_CHAIN_DEPTH
        && p.cert_total_bytes <= MAX_CERT_BYTES
        && (p.alpn.as_deref() == Some("h2") || p.h3_support || has_h3_altsvc(p))
        && is_compatible_http(&p.headers)
}

/// 从已抓到的响应头里判断是否宣告了 HTTP/3（`Alt-Svc: h3=...`）。
///
/// 优先用这种方式：`probe` 已经在同一条连接上取回了响应头，
/// 无需再为 H3 判断额外发起一次请求。
pub fn has_h3_altsvc(p: &TlsProbe) -> bool {
    p.headers
        .get("alt-svc")
        .is_some_and(|v| v.to_ascii_lowercase().contains("h3"))
}

/// 在没有响应头时主动探测 H3（对齐 Go `CheckH3Support`）。
///
/// 只在 `alpn` 既不是 h2 也没有 Alt-Svc 时才需要，作为最后的补充手段。
#[allow(dead_code)] // 主动探测：仅在没有响应头可查时作为兜底
pub fn check_h3_support(domain: &str, ip: IpAddr, timeout: Duration) -> bool {
    let Ok(connector) = build_connector() else {
        return false;
    };
    let Ok(tcp) = crate::bind::connect_tcp(SocketAddr::new(ip, 443), timeout) else {
        return false;
    };
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));
    let Ok(mut stream) = connector.connect(domain, tcp) else {
        return false;
    };

    let alpn = stream
        .ssl()
        .selected_alpn_protocol()
        .map(|p| String::from_utf8_lossy(p).to_string());
    let headers = match alpn.as_deref() {
        Some("h2") => h2::head(&mut stream, domain, SNI_USER_AGENT).unwrap_or_default(),
        Some("http/1.1") => head_request_http11(&mut stream, domain).unwrap_or_default(),
        _ => HashMap::new(),
    };
    headers
        .get("alt-svc")
        .is_some_and(|v| v.to_ascii_lowercase().contains("h3"))
}

/// Cloudflare / hcdn 等 CDN 边缘会把 REALITY 伪装彻底打穿，必须在 HTTP 层再次拦截。
pub fn is_compatible_http(headers: &HashMap<String, String>) -> bool {
    if headers.contains_key("cf-ray") || headers.contains_key("cf-cache-status") {
        return false;
    }
    headers
        .get("server")
        .map(|s| {
            let s = s.to_ascii_lowercase();
            !s.contains("cloudflare") && !s.contains("hcdn")
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_pq_groups() {
        assert!(is_pq_group("X25519MLKEM768"));
        assert!(is_pq_group("X25519Kyber768Draft00"));
        assert!(is_pq_group("SecP256r1MLKEM768"));
        assert!(!is_pq_group("X25519"));
        assert!(!is_pq_group("prime256v1"));
    }

    #[test]
    fn accepts_only_x25519_family() {
        assert!(is_valid_group("X25519"));
        assert!(is_valid_group("X25519MLKEM768"));
        assert!(!is_valid_group("prime256v1"));
        assert!(!is_valid_group("ffdhe2048"));
    }

    #[test]
    fn http_cdn_headers_reject_target() {
        let mut h = HashMap::new();
        h.insert("server".to_string(), "cloudflare".to_string());
        assert!(!is_compatible_http(&h));

        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "85de".to_string());
        assert!(!is_compatible_http(&h));

        let mut h = HashMap::new();
        h.insert("server".to_string(), "hcdn".to_string());
        assert!(!is_compatible_http(&h));

        let mut h = HashMap::new();
        h.insert("server".to_string(), "AkamaiNetStorage".to_string());
        assert!(is_compatible_http(&h));

        assert!(is_compatible_http(&HashMap::new()));
    }

    #[test]
    fn h3_altsvc_is_detected_case_insensitively() {
        let mut h = HashMap::new();
        h.insert("alt-svc".to_string(), "h3=\":443\"; ma=86400".to_string());
        let p = TlsProbe {
            headers: h,
            ..Default::default()
        };
        assert!(has_h3_altsvc(&p));

        let mut h2h = HashMap::new();
        h2h.insert("alt-svc".to_string(), "H3=\":443\"".to_string());
        let p2 = TlsProbe {
            headers: h2h,
            ..Default::default()
        };
        assert!(has_h3_altsvc(&p2), "Alt-Svc 值大小写不敏感");

        let mut h3h = HashMap::new();
        h3h.insert("alt-svc".to_string(), "h2=\":443\"".to_string());
        let p3 = TlsProbe {
            headers: h3h,
            ..Default::default()
        };
        assert!(!has_h3_altsvc(&p3), "只有 h2 不算 H3");

        assert!(!has_h3_altsvc(&TlsProbe::default()), "无头不得误判");
    }

    /// H3 目标：无 h2 但宣告 Alt-Svc: h3 时也应通过质检。
    #[test]
    fn validate_accepts_h3_only_targets() {
        let mut h = HashMap::new();
        h.insert("alt-svc".to_string(), "h3=\":443\"".to_string());
        let p = TlsProbe {
            tls13: true,
            alpn: Some("http/1.1".into()),
            curve: Some("X25519MLKEM768".into()),
            cert_chain_len: 3,
            cert_total_bytes: 4000,
            headers: h,
            h3_support: true,
            ..Default::default()
        };
        assert!(validate(&p), "宣告 H3 的目标应通过");
    }

    /// 既无 h2 也无 H3 的目标必须被拒。
    #[test]
    fn validate_rejects_targets_without_h2_or_h3() {
        let p = TlsProbe {
            tls13: true,
            alpn: Some("http/1.1".into()),
            curve: Some("X25519".into()),
            cert_chain_len: 3,
            cert_total_bytes: 4000,
            ..Default::default()
        };
        assert!(!validate(&p));
    }

    #[test]
    fn validate_requires_tls13_h2_x25519_and_cert_limits() {
        let good = TlsProbe {
            tls13: true,
            alpn: Some("h2".into()),
            curve: Some("X25519MLKEM768".into()),
            cert_chain_len: 3,
            cert_total_bytes: 4042,
            ..Default::default()
        };
        assert!(validate(&good));

        // 非 TLS 1.3
        assert!(!validate(&TlsProbe {
            tls13: false,
            ..good.clone()
        }));
        // 无 h2
        assert!(!validate(&TlsProbe {
            alpn: Some("http/1.1".into()),
            ..good.clone()
        }));
        // 非 X25519 系
        assert!(!validate(&TlsProbe {
            curve: Some("prime256v1".into()),
            ..good.clone()
        }));
        // 链太深
        assert!(!validate(&TlsProbe {
            cert_chain_len: 4,
            ..good.clone()
        }));
        // 证书太大（超过 8192 缓冲区安全阈值）
        assert!(!validate(&TlsProbe {
            cert_total_bytes: 8273,
            ..good.clone()
        }));
    }

    #[test]
    fn probe_flags_derive_from_curve() {
        let pq = TlsProbe {
            curve: Some("X25519MLKEM768".into()),
            ..Default::default()
        };
        assert!(pq.is_pq() && pq.is_valid_group());

        let classic = TlsProbe {
            curve: Some("X25519".into()),
            ..Default::default()
        };
        assert!(!classic.is_pq() && classic.is_valid_group());

        let unsupported = TlsProbe {
            curve: Some("prime256v1".into()),
            ..Default::default()
        };
        assert!(!unsupported.is_pq() && !unsupported.is_valid_group());

        assert!(!TlsProbe::default().is_valid_group());
    }
}
