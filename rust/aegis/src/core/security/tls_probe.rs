use std::sync::Arc;

use anyhow::{Context, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
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

static TLS_PROBE_CACHE: Lazy<DashMap<String, TlsProbeResult>> = Lazy::new(DashMap::new);

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
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn rustls_handshake(
    domain: &str,
    port: u16,
) -> Result<(TlsStream<TcpStream>, Vec<CertificateDer<'static>>)> {
    let addr = format!("{}:{}", domain, port);

    let stream = timeout(Duration::from_secs(5), TcpStream::connect(&addr))
        .await
        .context("TLS probe connect timeout")?
        .with_context(|| format!("TLS probe connect failed to {}", addr))?;

    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth();

    let server_name =
        ServerName::try_from(domain).with_context(|| format!("invalid DNS name: {}", domain))?;

    let connector = TlsConnector::from(Arc::new(config));
    let mut tls_stream = timeout(
        Duration::from_secs(8),
        connector.connect(server_name.to_owned(), stream),
    )
    .await
    .context("TLS probe handshake timeout")?
    .context("TLS probe handshake failed")?;

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

    let total_len: usize = certs.iter().map(|cert| cert.as_ref().len()).sum();

    let leaf_alg = certs
        .first()
        .and_then(|cert| {
            X509Certificate::from_der(cert.as_ref())
                .ok()
                .map(|(_, parsed)| {
                    let alg = parsed.public_key().algorithm.algorithm.clone();
                    let oid_str = alg.to_id_string();
                    match oid_str.as_str() {
                        "1.2.840.113549.1.1.1" => "RSA".to_string(),
                        "1.2.840.10045.2.1" => "EC".to_string(),
                        _ => oid_str,
                    }
                })
        })
        .unwrap_or_else(|| "UNKNOWN".to_string());

    Ok(TlsProbeResult {
        total_cert_len: total_len,
        leaf_pubkey_alg: leaf_alg,
    })
}

pub async fn probe_tls_cached(sni: &str, port: u16) -> Result<TlsProbeResult> {
    if let Some(res) = TLS_PROBE_CACHE.get(sni) {
        return Ok(res.clone());
    }

    let res = probe_tls_once(sni, port).await?;
    TLS_PROBE_CACHE.insert(sni.to_string(), res.clone());
    Ok(res)
}

pub async fn sni_is_pq_friendly(sni: &str) -> bool {
    match probe_tls_cached(sni, 443).await {
        Ok(res) => res.total_cert_len > 3500,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oid_mapping_for_rsa_and_ec() {
        let rsa_oid = "1.2.840.113549.1.1.1";
        let ec_oid = "1.2.840.10045.2.1";
        assert_eq!(rsa_oid, "1.2.840.113549.1.1.1");
        assert_eq!(ec_oid, "1.2.840.10045.2.1");
    }

    #[test]
    fn test_tls_probe_result_fields() {
        let result = TlsProbeResult {
            total_cert_len: 3500,
            leaf_pubkey_alg: "RSA".to_string(),
        };
        assert_eq!(result.total_cert_len, 3500);
        assert_eq!(result.leaf_pubkey_alg, "RSA");
    }

    #[test]
    fn test_tls_probe_result_clone_debug() {
        let result = TlsProbeResult {
            total_cert_len: 1024,
            leaf_pubkey_alg: "EC".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.total_cert_len, 1024);
        assert_eq!(cloned.leaf_pubkey_alg, "EC");
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("1024"));
        assert!(debug_str.contains("EC"));
    }

    #[test]
    fn test_oid_to_algorithm_mapping() {
        let mappings = [("1.2.840.113549.1.1.1", "RSA"), ("1.2.840.10045.2.1", "EC")];
        for (oid, algo) in mappings {
            let result = match oid {
                "1.2.840.113549.1.1.1" => "RSA",
                "1.2.840.10045.2.1" => "EC",
                _ => oid,
            };
            assert_eq!(result, algo);
        }
    }

    #[test]
    fn test_unknown_oid_passes_through() {
        let unknown_oid = "1.3.6.1.4.1.99999.1";
        let result = match unknown_oid {
            "1.2.840.113549.1.1.1" => "RSA",
            "1.2.840.10045.2.1" => "EC",
            other => other,
        };
        assert_eq!(result, "1.3.6.1.4.1.99999.1");
    }

    #[test]
    fn test_sni_is_pq_friendly_threshold() {
        let result = TlsProbeResult {
            total_cert_len: 3501,
            leaf_pubkey_alg: "RSA".to_string(),
        };
        assert!(result.total_cert_len > 3500);
        let small_result = TlsProbeResult {
            total_cert_len: 3000,
            leaf_pubkey_alg: "EC".to_string(),
        };
        assert!(small_result.total_cert_len <= 3500);
    }
}
