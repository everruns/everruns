//! TLS on an install's public port, with client certificates (agentd
//! design, "Auth": API keys, JWKS or mTLS).
//!
//! Decisions:
//! - rustls with the aws-lc-rs provider, the workspace's TLS stack; no
//!   OpenSSL.
//! - With `--client-ca`, a client certificate signed by that CA is the
//!   credential: TLS refuses a connection without one, and the request
//!   reaches the API with the certificate's SHA-256 (hex, first 16 bytes)
//!   as its caller (`install_auth::ClientCert`). Without it, TLS only
//!   encrypts and requests authenticate with a key or token.

use std::path::PathBuf;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::Digest;

#[derive(Debug, Clone)]
pub struct TlsFiles {
    /// PEM: the server's certificate chain.
    pub cert: PathBuf,
    /// PEM: its private key.
    pub key: PathBuf,
    /// PEM: the CA client certificates must chain to (mTLS).
    pub client_ca: Option<PathBuf>,
}

/// The server's TLS config from `files`.
pub fn config(files: &TlsFiles) -> anyhow::Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&files.cert)
        .map_err(|e| anyhow::anyhow!("{}: {e}", files.cert.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("{}: {e}", files.cert.display()))?;
    let key = PrivateKeyDer::from_pem_file(&files.key)
        .map_err(|e| anyhow::anyhow!("{}: {e}", files.key.display()))?;
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;
    let builder = match &files.client_ca {
        Some(ca) => {
            let mut roots = rustls::RootCertStore::empty();
            for cert in CertificateDer::pem_file_iter(ca)
                .map_err(|e| anyhow::anyhow!("{}: {e}", ca.display()))?
            {
                roots.add(cert.map_err(|e| anyhow::anyhow!("{}: {e}", ca.display()))?)?;
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                Arc::new(roots),
                provider,
            )
            .build()?;
            builder.with_client_cert_verifier(verifier)
        }
        None => builder.with_no_client_auth(),
    };
    let mut config = builder.with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// A caller name for a client certificate.
pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    hex::encode(&sha2::Sha256::digest(cert.as_ref())[..16])
}

/// Serve `app` over TLS on `listener` until it fails.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    files: &TlsFiles,
) -> anyhow::Result<()> {
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use tower::ServiceExt;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config(files)?));
    // Turn every route into its service once, not per request.
    let app: axum::Router = app.with_state(());
    loop {
        let (tcp, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(tcp).await {
                Ok(tls) => tls,
                Err(error) => {
                    tracing::debug!(%peer, %error, "TLS handshake failed");
                    return;
                }
            };
            let cert = tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|chain| chain.first())
                .map(|c| crate::install_auth::ClientCert(fingerprint(c)));
            let service = hyper::service::service_fn(
                move |mut request: hyper::Request<hyper::body::Incoming>| {
                    if let Some(cert) = &cert {
                        request.extensions_mut().insert(cert.clone());
                    }
                    app.clone().oneshot(request.map(axum::body::Body::new))
                },
            );
            if let Err(error) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(tls), service)
                .await
            {
                tracing::debug!(%peer, %error, "connection ended");
            }
        });
    }
}
