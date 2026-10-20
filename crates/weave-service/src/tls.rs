//! HTTP/1.1 over TLS (rustls with the ring provider), plus a plaintext mode
//! that only binds loopback addresses, for local development.
use crate::log::Logger;
use axum::Router;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::json;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_rustls::TlsAcceptor;

/// Connection limits of the listener.
#[derive(Debug, Clone)]
pub struct ConnectionLimits {
    /// Most open connections; further connections wait to be accepted.
    pub max_connections: usize,
    /// Longest TLS handshake.
    pub handshake_timeout: Duration,
    /// Longest wait for a complete request head.
    pub header_read_timeout: Duration,
    /// Largest request head buffer.
    pub max_header_bytes: usize,
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            max_connections: 1024,
            handshake_timeout: Duration::from_secs(10),
            header_read_timeout: Duration::from_secs(10),
            max_header_bytes: 64 * 1024,
        }
    }
}

/// A TLS server configuration from a PEM certificate chain and a PEM private
/// key. TLS 1.2 and 1.3 only, HTTP/1.1 by ALPN.
pub fn tls_config(cert_pem: &Path, key_pem: &Path) -> Result<Arc<ServerConfig>, String> {
    let certs = CertificateDer::pem_file_iter(cert_pem)
        .map_err(|_| "cannot read the TLS certificate file".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "the TLS certificate file is not valid PEM".to_string())?;
    if certs.is_empty() {
        return Err("the TLS certificate file holds no certificate".into());
    }
    let key = PrivateKeyDer::from_pem_file(key_pem)
        .map_err(|_| "cannot read a private key from the TLS key file".to_string())?;
    tls_config_from(certs, key)
}

/// A TLS server configuration from DER values.
pub fn tls_config_from(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>, String> {
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS configuration failed: {e}"))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("TLS certificate or key rejected: {e}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Serve `router` over TLS on `listener` until `shutdown` completes.
pub async fn serve_tls(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    router: Router,
    limits: ConnectionLimits,
    log: Logger,
    shutdown: impl Future<Output = ()>,
) {
    let acceptor = TlsAcceptor::from(tls);
    let connections = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    tokio::pin!(shutdown);
    loop {
        let permit = tokio::select! {
            _ = &mut shutdown => return,
            permit = connections.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        let (stream, _peer) = tokio::select! {
            _ = &mut shutdown => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(_) => {
                    log.event("warn", "accept_failed", json!({}));
                    continue;
                }
            },
        };
        let acceptor = acceptor.clone();
        let router = router.clone();
        let limits = limits.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(Ok(tls)) =
                tokio::time::timeout(limits.handshake_timeout, acceptor.accept(stream)).await
            else {
                return;
            };
            serve_connection(TokioIo::new(tls), router, &limits).await;
        });
    }
}

/// Serve `router` without TLS. Refuses any address that is not loopback.
pub async fn serve_plaintext_loopback(
    listener: TcpListener,
    router: Router,
    limits: ConnectionLimits,
    shutdown: impl Future<Output = ()>,
) -> Result<(), String> {
    let address: SocketAddr = listener
        .local_addr()
        .map_err(|_| "cannot read the listener address".to_string())?;
    if !address.ip().is_loopback() {
        return Err("plaintext HTTP is only served on loopback addresses".into());
    }
    let connections = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    tokio::pin!(shutdown);
    loop {
        let permit = tokio::select! {
            _ = &mut shutdown => return Ok(()),
            permit = connections.clone().acquire_owned() => permit.map_err(|_| "closed".to_string())?,
        };
        let (stream, _) = tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(_) => continue,
            },
        };
        let router = router.clone();
        let limits = limits.clone();
        tokio::spawn(async move {
            let _permit = permit;
            serve_connection(TokioIo::new(stream), router, &limits).await;
        });
    }
}

async fn serve_connection<I>(io: I, router: Router, limits: &ConnectionLimits)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = TowerToHyperService::new(router);
    let _ = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout)
        .max_buf_size(limits.max_header_bytes.max(8192))
        .serve_connection(io, service)
        .await;
}
