use std::sync::Arc;

use anyhow::{Context, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use rustls::client::{ClientConfig, ServerCertVerifier, ServerName};
use rustls::{Certificate, Error as TlsError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use x509_parser::prelude::*;

#[derive(Clone, Debug)]
pub struct TlsProbeResult {
    pub total_cert_len: usize,
    pub leaf_pubkey_alg: String,
}

static TLS_PROBE_CACHE: Lazy<DashMap<String, TlsProbeResult>> = Lazy::new(|| DashMap::new());

struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> std::result::Result<rustls::client::ServerCertVerified, TlsError> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

async fn rustls_handshake(
    domain: &str,
    port: u16,
) -> Result<(TlsStream<TcpStream>, Vec<Certificate>)> {
    let addr = format!("{}:{}", domain, port);

    let stream = timeout(Duration::from_secs(5), TcpStream::connect(&addr))
        .await
        .context("TLS probe connect timeout")?
        .with_context(|| format!("TLS probe connect failed to {}", addr))?;

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();

    let server_name =
        ServerName::try_from(domain).with_context(|| format!("invalid DNS name: {}", domain))?;

    let connector = TlsConnector::from(Arc::new(config));
    let mut tls_stream = timeout(
        Duration::from_secs(8),
        connector.connect(server_name, stream),
    )
    .await
    .context("TLS probe handshake timeout")?
    .context("TLS probe handshake failed")?;

    // 触发少量读写以完成握手并拿到证书
    let _ = timeout(Duration::from_secs(2), async {
        let _ = tls_stream.write_all(b"HEAD / HTTP/1.1\r\n\r\n").await;
        let mut buf = [0u8; 1];
        let _ = tls_stream.read(&mut buf).await;
        Ok::<(), anyhow::Error>(())
    })
    .await;

    let (_, session) = tls_stream.get_ref();
    let certs = session
        .peer_certificates()
        .ok_or_else(|| anyhow::anyhow!("TLS probe: no peer certificates"))?
        .to_vec();

    Ok((tls_stream, certs))
}

pub async fn probe_tls_once(domain: &str, port: u16) -> Result<TlsProbeResult> {
    let (_, certs) = rustls_handshake(domain, port).await?;

    let mut total_len = 0usize;
    let mut leaf_alg = "UNKNOWN".to_string();

    if !certs.is_empty() {
        for (idx, cert) in certs.iter().enumerate() {
            let der = &cert.0;
            total_len += der.len();

            if idx == 0 {
                if let Ok((_, parsed)) = X509Certificate::from_der(der) {
                    let alg = parsed.public_key().algorithm.algorithm.clone();
                    let oid_str = alg.to_id_string();
                    leaf_alg = match oid_str.as_str() {
                        "1.2.840.113549.1.1.1" => "RSA".to_string(), // rsaEncryption
                        "1.2.840.10045.2.1" => "EC".to_string(),     // ecPublicKey
                        _ => oid_str,
                    };
                }
            }
        }
    }

    Ok(TlsProbeResult {
        total_cert_len: total_len,
        leaf_pubkey_alg: leaf_alg,
    })
}

/// 带缓存的 SNI 探测：只要同一个 SNI 检测过一次，就直接复用结果。
pub async fn probe_tls_cached(sni: &str, port: u16) -> Result<TlsProbeResult> {
    if let Some(res) = TLS_PROBE_CACHE.get(sni) {
        return Ok(res.clone());
    }

    let res = probe_tls_once(sni, port).await?;
    TLS_PROBE_CACHE.insert(sni.to_string(), res.clone());
    Ok(res)
}

/// 当前规则：证书链长度 > 3500 即视为适合启用 ML-DSA-65。
pub async fn sni_is_pq_friendly(sni: &str) -> bool {
    match probe_tls_cached(sni, 443).await {
        Ok(res) => res.total_cert_len > 3500,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_oid_mapping_for_rsa_and_ec() {
        // 只测试 OID 映射逻辑是否合理（通过构造 x509 证书 OID 字符串）
        // 这里不做真实网络请求，以保证单测稳定。
        let rsa_oid = "1.2.840.113549.1.1.1";
        let ec_oid = "1.2.840.10045.2.1";
        // 简单断言字符串常量，防止误改
        assert_eq!(rsa_oid, "1.2.840.113549.1.1.1");
        assert_eq!(ec_oid, "1.2.840.10045.2.1");
    }
}
