//! B3 (#39): the service over a real TLS listener, with a certificate the
//! test generates, and the loopback-only plaintext mode.
mod common;
use common::*;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use weave_service::log::Logger;
use weave_service::tls::{self, ConnectionLimits};

async fn https_get(port: u16, roots: RootCertStore, path: &str, token: &str) -> String {
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut tls = connector
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = tls.read_to_end(&mut response).await;
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn the_service_answers_over_tls_and_refuses_plaintext_on_the_tls_port() {
    let h = Harness::new();
    let root = h.tenant("acme", "root");
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert = CertificateDer::from(generated.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der()));
    let config = tls::tls_config_from(vec![cert.clone()], key).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(tls::serve_tls(
        listener,
        config,
        h.service.router(),
        ConnectionLimits::default(),
        Logger::discard(),
        async {
            let _ = stopped.await;
        },
    ));
    let mut roots = RootCertStore::empty();
    roots.add(cert).unwrap();

    let ok = https_get(port, roots.clone(), "/v1/whoami", &root).await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.contains(r#""tenant":"acme""#), "{ok}");
    assert!(ok.to_ascii_lowercase().contains("x-request-id:"));
    let denied = https_get(port, roots.clone(), "/v1/whoami", "wvs_nope").await;
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");

    // Plaintext HTTP to the TLS port gets no HTTP response.
    let mut plain = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    plain
        .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut buffer = Vec::new();
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        plain.read_to_end(&mut buffer),
    )
    .await;
    assert!(!String::from_utf8_lossy(&buffer).contains("HTTP/1.1 200"));

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn plaintext_is_served_only_on_loopback() {
    let h = Harness::new();
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let refused = tls::serve_plaintext_loopback(
        listener,
        h.service.router(),
        ConnectionLimits::default(),
        async {},
    )
    .await;
    assert!(refused.is_err());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let served = tls::serve_plaintext_loopback(
        listener,
        h.service.router(),
        ConnectionLimits::default(),
        async {},
    )
    .await;
    assert!(served.is_ok());
}

#[test]
fn tls_configuration_rejects_missing_or_invalid_files() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.pem");
    assert!(tls::tls_config(&missing, &missing).is_err());
    let junk = dir.path().join("junk.pem");
    std::fs::write(&junk, "not a certificate").unwrap();
    assert!(tls::tls_config(&junk, &junk).is_err());
}
