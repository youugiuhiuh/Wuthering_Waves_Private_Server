use std::{convert::Infallible, io::BufReader, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hyper::{
    Body, Method, Request, Response, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue},
    service::service_fn,
};
use rustls::{Certificate, PrivateKey, ServerConfig};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_util::sync::CancellationToken;

use super::{
    config::{SubscriptionConfig, verify_token},
    node::load_current_nodes,
    render::{render_base64, render_clash},
};

const MAX_URI_LENGTH: usize = 2048;
const MAX_PROBE_RESPONSE_LENGTH: usize = 64 * 1024;

#[derive(Clone)]
pub struct TlsFiles {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

impl TlsFiles {
    pub fn new(cert_path: PathBuf, key_path: PathBuf) -> Self {
        Self {
            cert_path,
            key_path,
        }
    }
}

#[derive(Clone)]
pub struct TlsState {
    tx: watch::Sender<Arc<ServerConfig>>,
}

impl TlsState {
    pub fn load(files: TlsFiles) -> Result<Self> {
        let config = Arc::new(load_tls_config(&files)?);
        let (tx, _) = watch::channel(config);
        Ok(Self { tx })
    }

    pub fn reload(&self, files: TlsFiles) -> Result<()> {
        let config = Arc::new(load_tls_config(&files)?);
        self.tx.send_replace(config);
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<Arc<ServerConfig>> {
        self.tx.subscribe()
    }
}

pub struct ListenerHandle {
    pub local_addr: SocketAddr,
    pub tls: TlsState,
    pub cancel: CancellationToken,
    pub task: JoinHandle<Result<()>>,
}

impl ListenerHandle {
    pub async fn shutdown(self) -> Result<()> {
        self.cancel.cancel();
        self.task
            .await
            .context("subscription listener task failed")?
    }
}

pub async fn spawn_listener(
    bind: SocketAddr,
    tls: TlsState,
    source: SubscriptionSource,
) -> Result<ListenerHandle> {
    let listener = TcpListener::bind(bind)
        .await
        .context("failed to bind subscription listener")?;
    let local_addr = listener
        .local_addr()
        .context("failed to read subscription listener address")?;
    let cancel = CancellationToken::new();
    let task_cancel = cancel.child_token();
    let mut tls_rx = tls.subscribe();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        let mut failure = None;
        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => break,
                joined = connections.join_next(), if !connections.is_empty() => {
                    if let Some(joined) = joined
                        && let Err(error) = joined
                            .context("subscription connection task failed")
                            .and_then(|result| result)
                        && failure.is_none()
                    {
                        failure = Some(error);
                    }
                }
                accepted = listener.accept() => {
                    let (socket, _) = match accepted {
                        Ok(accepted) => accepted,
                        Err(error) => {
                            failure = Some(
                                anyhow::Error::new(error)
                                    .context("subscription listener accept failed"),
                            );
                            break;
                        }
                    };
                    let config = tls_rx.borrow_and_update().clone();
                    let acceptor = TlsAcceptor::from(config);
                    let source = source.clone();
                    let connection_cancel = task_cancel.clone();
                    connections.spawn(async move {
                        let stream = tokio::select! {
                            _ = connection_cancel.cancelled() => return Ok(()),
                            accepted = acceptor.accept(socket) => accepted
                                .context("subscription TLS connection failed")?,
                        };
                        let service = service_fn(move |request| {
                            let source = source.clone();
                            async move { Ok::<_, Infallible>(handle_request(request, source).await) }
                        });
                        let mut http = hyper::server::conn::Http::new();
                        http.http1_only(true);
                        let connection = http.serve_connection(stream, service);
                        tokio::pin!(connection);
                        let result = tokio::select! {
                            result = connection.as_mut() => result,
                            _ = connection_cancel.cancelled() => {
                                connection.as_mut().graceful_shutdown();
                                connection.await
                            }
                        };
                        result.context("subscription HTTP connection failed")
                    });
                }
            }
        }
        task_cancel.cancel();
        while let Some(joined) = connections.join_next().await {
            if let Err(error) = joined
                .context("subscription connection task failed")
                .and_then(|result| result)
                && failure.is_none()
            {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            Err(error)
        } else {
            Ok(())
        }
    });
    Ok(ListenerHandle {
        local_addr,
        tls,
        cancel,
        task,
    })
}

pub async fn probe_https(
    addr: SocketAddr,
    server_name: &str,
    cert: &std::path::Path,
) -> Result<()> {
    let mut stream = open_https(addr, server_name, cert).await?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: probe\r\nConnection: close\r\n\r\n")
        .await
        .context("HTTPS probe request failed")?;
    let status = read_http_response(&mut stream).await?;
    if status != StatusCode::NOT_FOUND {
        bail!("HTTPS probe returned unexpected status");
    }
    Ok(())
}

async fn open_https(
    addr: SocketAddr,
    server_name: &str,
    cert: &std::path::Path,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let certs = read_certificates(cert)?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in certs {
        roots
            .add(&cert)
            .map_err(|_| anyhow::anyhow!("invalid probe CA certificate"))?;
    }
    let config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::ServerName::try_from(server_name)
        .map_err(|_| anyhow::anyhow!("invalid probe server name"))?;
    let stream = TcpStream::connect(addr)
        .await
        .context("HTTPS probe connection failed")?;
    TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .context("HTTPS probe handshake failed")
}

async fn read_http_response(stream: &mut (impl AsyncRead + Unpin)) -> Result<StatusCode> {
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() == MAX_PROBE_RESPONSE_LENGTH {
            bail!("HTTPS probe response exceeded limit");
        }
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .context("HTTPS probe response was incomplete")?;
        response.push(byte[0]);
    }
    let headers = std::str::from_utf8(&response).context("HTTPS probe response was not HTTP")?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .and_then(|status| StatusCode::from_u16(status).ok())
        .context("HTTPS probe response had invalid status")?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        })
        .context("HTTPS probe response omitted content length")?;
    if content_length > MAX_PROBE_RESPONSE_LENGTH - response.len() {
        bail!("HTTPS probe response exceeded limit");
    }
    let mut body = vec![0_u8; content_length];
    stream
        .read_exact(&mut body)
        .await
        .context("HTTPS probe response was incomplete")?;
    Ok(status)
}

fn load_tls_config(files: &TlsFiles) -> Result<ServerConfig> {
    let certs = read_certificates(&files.cert_path)?;
    if certs.is_empty() {
        bail!("TLS certificate chain is empty");
    }
    let raw_key = std::fs::read(&files.key_path).context("failed to read TLS private key")?;
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut BufReader::new(raw_key.as_slice()))
        .context("failed to parse TLS private key")?;
    if keys.is_empty() {
        keys = rustls_pemfile::rsa_private_keys(&mut BufReader::new(raw_key.as_slice()))
            .context("failed to parse TLS private key")?;
    }
    if keys.len() != 1 {
        bail!("TLS configuration requires exactly one private key");
    }
    ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, PrivateKey(keys.remove(0)))
        .context("TLS certificate and private key are invalid")
}

fn read_certificates(path: &std::path::Path) -> Result<Vec<Certificate>> {
    let raw = std::fs::read(path).context("failed to read TLS certificate")?;
    rustls_pemfile::certs(&mut BufReader::new(raw.as_slice()))
        .context("failed to parse TLS certificate")
        .map(|certs| certs.into_iter().map(Certificate).collect())
}

#[derive(Clone)]
pub struct SubscriptionSource {
    pub config_rx: watch::Receiver<Arc<SubscriptionConfig>>,
    pub xray_dir: PathBuf,
    pub singbox_dir: PathBuf,
}

#[derive(Clone, Copy)]
enum Format {
    Base64,
    Clash,
}

pub async fn handle_request(request: Request<Body>, source: SubscriptionSource) -> Response<Body> {
    let path = request.uri().path();
    let route = redacted_route(path);
    let Some((token, format)) = parse_route(&request) else {
        return response(StatusCode::NOT_FOUND, "text/plain", "not found");
    };
    let config = source.config_rx.borrow().clone();
    if !verify_token(token, &config.token_hash) {
        return response(StatusCode::NOT_FOUND, "text/plain", "not found");
    }

    let body =
        load_current_nodes(&source.xray_dir, &source.singbox_dir).and_then(|nodes| match format {
            Format::Base64 => render_base64(&nodes, &config.public_host),
            Format::Clash => render_clash(&nodes, &config.public_host),
        });
    let Ok(body) = body else {
        log::error!("subscription request failed route={route} category=render");
        return response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain",
            "internal server error",
        );
    };

    let content_type = match format {
        Format::Base64 => "text/plain",
        Format::Clash => "application/yaml",
    };
    let mut response = response(StatusCode::OK, content_type, body);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        "profile-title",
        HeaderValue::from_str(&format!("base64:{}", STANDARD.encode("Aegis")))
            .expect("static profile title is a valid header"),
    );
    response
        .headers_mut()
        .insert("profile-update-interval", HeaderValue::from_static("24"));
    response
}

pub fn redacted_route(path: &str) -> &'static str {
    let segments = path.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["", "sub", _] => "/sub/[token]",
        ["", "sub", _, "clash"] => "/sub/[token]/clash",
        _ => "/other",
    }
}

fn parse_route(request: &Request<Body>) -> Option<(&str, Format)> {
    if request.method() != Method::GET || request.uri().to_string().len() > MAX_URI_LENGTH {
        return None;
    }
    let segments = request.uri().path().split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["", "sub", token] if !token.is_empty() => Some((token, Format::Base64)),
        ["", "sub", token, "clash"] if !token.is_empty() => Some((token, Format::Clash)),
        _ => None,
    }
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Body>,
) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body.into())
        .expect("static response metadata is valid")
}

#[cfg(test)]
mod tests {
    use std::{fs, net::SocketAddr, path::Path, sync::Arc};

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use hyper::{
        Body, Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::watch,
        time::{Duration, timeout},
    };
    use tokio_rustls::client::TlsStream;

    use super::{
        SubscriptionSource, TlsFiles, TlsState, handle_request, open_https, probe_https,
        read_http_response, redacted_route, spawn_listener,
    };
    use crate::core::subscription::config::SubscriptionConfig;

    struct FixtureSource {
        raw_token: String,
        config_tx: watch::Sender<Arc<SubscriptionConfig>>,
        xray: TempDir,
        singbox: TempDir,
    }

    impl FixtureSource {
        fn runtime(&self) -> SubscriptionSource {
            SubscriptionSource {
                config_rx: self.config_tx.subscribe(),
                xray_dir: self.xray.path().to_owned(),
                singbox_dir: self.singbox.path().to_owned(),
            }
        }
    }

    #[tokio::test]
    async fn routes_require_get_and_valid_token_without_disclosure() {
        let source = fixture_source();
        let ok = request(Method::GET, &format!("/sub/{}", source.raw_token));
        let response = handle_request(ok, source.runtime()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
        assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
        assert_eq!(
            response.headers()["profile-title"],
            format!("base64:{}", STANDARD.encode("Aegis"))
        );
        assert_eq!(response.headers()["profile-update-interval"], "24");

        for request in [
            request(Method::POST, "/sub/secret"),
            request(Method::GET, "/sub/wrong"),
            request(Method::GET, "/other"),
        ] {
            assert_eq!(
                handle_request(request, source.runtime()).await.status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn clash_route_has_yaml_content_type_and_bounds_uri() {
        let source = fixture_source();
        let clash = request(Method::GET, &format!("/sub/{}/clash", source.raw_token));
        let response = handle_request(clash, source.runtime()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/yaml");

        for path in [
            format!("/sub/{}/extra", source.raw_token),
            format!("/sub/{}/clash/extra", source.raw_token),
            format!("/sub/%2F{}/clash", source.raw_token),
            format!("/{}", "a".repeat(2048)),
        ] {
            assert_eq!(
                handle_request(request(Method::GET, &path), source.runtime())
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[test]
    fn request_log_path_never_contains_raw_token() {
        assert_eq!(redacted_route("/sub/raw-secret"), "/sub/[token]");
        assert_eq!(
            redacted_route("/sub/raw-secret/clash"),
            "/sub/[token]/clash"
        );
        assert_eq!(redacted_route("/anything/raw-secret"), "/other");
    }

    #[tokio::test]
    async fn successful_requests_read_latest_config_and_nodes() {
        let source = fixture_source();
        let path = format!("/sub/{}", source.raw_token);
        write_reality_node(source.xray.path(), "updated");
        let mut config = (**source.config_tx.borrow()).clone();
        config.public_host = "new.example.com".to_owned();
        source.config_tx.send_replace(Arc::new(config));

        let response = handle_request(request(Method::GET, &path), source.runtime()).await;
        let encoded = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let rendered = STANDARD.decode(encoded).unwrap();
        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("@new.example.com:443"));
        assert!(rendered.contains("#updated"));
    }

    #[tokio::test]
    async fn valid_route_with_no_nodes_returns_generic_error() {
        let source = fixture_source();
        fs::remove_file(source.xray.path().join("server.json")).unwrap();
        let path = format!("/sub/{}", source.raw_token);

        let response = handle_request(request(Method::GET, &path), source.runtime()).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            hyper::body::to_bytes(response.into_body()).await.unwrap(),
            "internal server error"
        );
    }

    #[tokio::test]
    async fn established_connection_survives_tls_reload_and_new_connection_uses_latest_config() {
        let first = test_tls_state("first.local");
        let source = fixture_source();
        let route = format!("/sub/{}", source.raw_token);
        let handle = spawn_listener(localhost(), first.state, source.runtime())
            .await
            .unwrap();
        let mut established = open_https(handle.local_addr, "first.local", &first.ca)
            .await
            .unwrap();
        assert_eq!(
            request_over_keep_alive(&mut established, &route).await,
            StatusCode::OK
        );
        let bound_addr = handle.local_addr;
        let second = test_tls_state("second.local");
        handle.tls.reload(second.files()).unwrap();
        assert_eq!(handle.local_addr, bound_addr);
        assert_eq!(
            request_over_keep_alive(&mut established, &route).await,
            StatusCode::OK
        );
        probe_https(handle.local_addr, "second.local", &second.ca)
            .await
            .unwrap();
        assert_eq!(handle.local_addr, bound_addr);
        drop(established);
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_waits_until_established_connection_receives_graceful_eof() {
        let tls = test_tls_state("first.local");
        let source = fixture_source();
        let route = format!("/sub/{}", source.raw_token);
        let handle = spawn_listener(localhost(), tls.state, source.runtime())
            .await
            .unwrap();
        let mut established = open_https(handle.local_addr, "first.local", &tls.ca)
            .await
            .unwrap();
        assert_eq!(
            request_over_keep_alive(&mut established, &route).await,
            StatusCode::OK
        );

        handle.shutdown().await.unwrap();

        let mut byte = [0_u8; 1];
        let bytes_read = timeout(Duration::from_secs(1), established.read(&mut byte))
            .await
            .expect("shutdown returned before signaling the connection")
            .unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn malformed_file_preserves_valid_sibling_server_response() {
        const SECRET: &str = "must-not-leak-node-secret";
        let source = fixture_source();
        write_malformed_node_file(source.xray.path(), SECRET);
        let path = format!("/sub/{}", source.raw_token);

        let response = handle_request(request(Method::GET, &path), source.runtime()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body = String::from_utf8(STANDARD.decode(body).unwrap()).unwrap();
        assert!(!body.contains(SECRET));
        assert!(body.ends_with("#first"));
    }

    #[tokio::test]
    async fn partially_invalid_file_preserves_valid_sibling_server_response() {
        const SECRET: &str = "must-not-leak-node-secret";
        let source = fixture_source();
        write_partially_invalid_node_file(source.xray.path(), SECRET, true);
        let path = format!("/sub/{}", source.raw_token);

        let response = handle_request(request(Method::GET, &path), source.runtime()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body = String::from_utf8(STANDARD.decode(body).unwrap()).unwrap();
        assert!(!body.contains(SECRET));
        assert!(body.starts_with("vless://"));
        assert!(body.ends_with("#first"));
    }

    #[tokio::test]
    async fn combined_all_invalid_files_return_fixed_secret_safe_server_error() {
        const SECRET: &str = "must-not-leak-node-secret";
        let source = fixture_source();
        write_malformed_node_file(source.xray.path(), SECRET);
        write_partially_invalid_node_file(source.xray.path(), SECRET, false);
        let path = format!("/sub/{}", source.raw_token);

        let response = handle_request(request(Method::GET, &path), source.runtime()).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, "internal server error");
        assert!(
            !body
                .windows(SECRET.len())
                .any(|bytes| bytes == SECRET.as_bytes())
        );
    }

    #[tokio::test]
    async fn response_reader_rejects_overflowing_content_length() {
        let (mut client, mut server) = tokio::io::duplex(256);
        server
            .write_all(
                format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n",
                    usize::MAX
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        assert!(read_http_response(&mut client).await.is_err());
    }

    #[tokio::test]
    async fn invalid_tls_reload_keeps_listener_available() {
        let first = test_tls_state("first.local");
        let handle = spawn_listener(localhost(), first.state, fixture_source().runtime())
            .await
            .unwrap();
        let bad_key = first.dir.path().join("bad.key");
        fs::write(&bad_key, "not a key").unwrap();
        assert!(
            handle
                .tls
                .reload(TlsFiles::new(first.ca.clone(), bad_key))
                .is_err()
        );
        probe_https(handle.local_addr, "first.local", &first.ca)
            .await
            .unwrap();
        handle.shutdown().await.unwrap();
    }

    struct TestTls {
        state: TlsState,
        ca: std::path::PathBuf,
        key: std::path::PathBuf,
        dir: TempDir,
    }

    impl TestTls {
        fn files(&self) -> TlsFiles {
            TlsFiles::new(self.ca.clone(), self.key.clone())
        }
    }

    fn test_tls_state(name: &str) -> TestTls {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let (cert_pem, key_pem) = match name {
            "first.local" => (FIRST_CERT, FIRST_KEY),
            "second.local" => (SECOND_CERT, SECOND_KEY),
            _ => panic!("unknown TLS fixture"),
        };
        fs::write(&ca, cert_pem).unwrap();
        fs::write(&key, key_pem).unwrap();
        let state = TlsState::load(TlsFiles::new(ca.clone(), key.clone())).unwrap();
        TestTls {
            state,
            ca,
            key,
            dir,
        }
    }

    fn localhost() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    async fn request_over_keep_alive(stream: &mut TlsStream<TcpStream>, path: &str) -> StatusCode {
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: keep-alive\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        read_http_response(stream).await.unwrap()
    }

    fn write_malformed_node_file(dir: &Path, secret: &str) {
        fs::write(
            dir.join("00-malformed.json"),
            format!(r#"{{"password":"{secret}""#),
        )
        .unwrap();
    }

    fn write_partially_invalid_node_file(dir: &Path, secret: &str, include_valid: bool) {
        let valid: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("server.json")).unwrap()).unwrap();
        fs::remove_file(dir.join("server.json")).unwrap();
        let mut inbounds = vec![json!({"protocol": "socks", "password": secret})];
        if include_valid {
            inbounds.push(valid["inbounds"][0].clone());
        }
        fs::write(
            dir.join("01-partial.json"),
            serde_json::to_vec(&json!({"inbounds": inbounds})).unwrap(),
        )
        .unwrap();
    }

    const FIRST_CERT: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIBfTCCAS+gAwIBAgIUTcO+/IRlal0NTkIHxFMuSywy/v0wBQYDK2VwMBYxFDAS\n",
        "BgNVBAMMC2ZpcnN0LmxvY2FsMB4XDTI2MDcyNjA0NTAzN1oXDTM2MDcyMzA0NTAz\n",
        "N1owFjEUMBIGA1UEAwwLZmlyc3QubG9jYWwwKjAFBgMrZXADIQBWRWUDYUYSmCkF\n",
        "50N+Z0hQ1+hF1NhzypcBwUht7UjaaaOBjjCBizAdBgNVHQ4EFgQUdBBxl73LFuFF\n",
        "eBO6hQZDLBS9tXAwHwYDVR0jBBgwFoAUdBBxl73LFuFFeBO6hQZDLBS9tXAwFgYD\n",
        "VR0RBA8wDYILZmlyc3QubG9jYWwwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMC\n",
        "B4AwEwYDVR0lBAwwCgYIKwYBBQUHAwEwBQYDK2VwA0EAQZd/ONEQVrTd+0GLPjtO\n",
        "+grnzm+fQALcjD7G1H+z6m0QYwv8WXdnL+UL40HbK1EXv97ZW8bRRNCwhoyJYfTD\n",
        "BA==\n",
        "-----END CERTIFICATE-----\n"
    );
    const FIRST_KEY: &str = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MC4CAQAwBQYDK2VwBCIEIAXOm7FeRbdfdtN927lNMSX0geEm9nYauCCSWnVo8pxr\n",
        "-----END PRIVATE KEY-----\n"
    );
    const SECOND_CERT: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIBgDCCATKgAwIBAgIUFPsoMmn7K7sD4g2F7KRFOeIIh9gwBQYDK2VwMBcxFTAT\n",
        "BgNVBAMMDHNlY29uZC5sb2NhbDAeFw0yNjA3MjYwNDUwMzdaFw0zNjA3MjMwNDUw\n",
        "MzdaMBcxFTATBgNVBAMMDHNlY29uZC5sb2NhbDAqMAUGAytlcAMhABRt3XF9cRwI\n",
        "oREJDMwn8PtYST49vPTaT3pAgKectu3Ao4GPMIGMMB0GA1UdDgQWBBSdok9kFlts\n",
        "Gn3/UfLi+E9nEgRFrjAfBgNVHSMEGDAWgBSdok9kFltsGn3/UfLi+E9nEgRFrjAX\n",
        "BgNVHREEEDAOggxzZWNvbmQubG9jYWwwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8E\n",
        "BAMCB4AwEwYDVR0lBAwwCgYIKwYBBQUHAwEwBQYDK2VwA0EA8YlIDwAFF0QRKsfP\n",
        "IddxlOTOo2UXrPXogIFQkblYLRc8/nntxV+NIGFJcH43xk9MoCfCDYdkLFCiA6rx\n",
        "rnU9BA==\n",
        "-----END CERTIFICATE-----\n"
    );
    const SECOND_KEY: &str = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MC4CAQAwBQYDK2VwBCIEICbvXTerFoty+tavisN8rM4tIx8dye5RwAETj1fsBa+y\n",
        "-----END PRIVATE KEY-----\n"
    );

    fn fixture_source() -> FixtureSource {
        let raw_token = "raw-secret".to_owned();
        let mut config =
            SubscriptionConfig::new_disabled(hex::encode(Sha256::digest(raw_token.as_bytes())));
        config.public_host = "sub.example.com".to_owned();
        let (config_tx, _) = watch::channel(Arc::new(config));
        let xray = tempfile::tempdir().unwrap();
        let singbox = tempfile::tempdir().unwrap();
        write_reality_node(xray.path(), "first");
        FixtureSource {
            raw_token,
            config_tx,
            xray,
            singbox,
        }
    }

    fn request(method: Method, path: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    fn write_reality_node(dir: &Path, name: &str) {
        fs::write(
            dir.join("server.json"),
            serde_json::to_vec(&json!({"inbounds": [{
                "port": 443,
                "protocol": "vless",
                "settings": {"clients": [{"id": "123e4567-e89b-12d3-a456-426614174000", "email": name, "flow": "xtls-rprx-vision"}]},
                "streamSettings": {"network": "tcp", "security": "reality", "realitySettings": {
                    "serverNames": ["example.com"],
                    "privateKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                    "shortIds": ["0123456789abcdef"]
                }}
            }]}))
            .unwrap(),
        )
        .unwrap();
    }
}
