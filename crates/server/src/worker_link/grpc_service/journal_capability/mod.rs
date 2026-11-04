//! Unactivated, journal-only identity and capability verification component.
//!
//! No production listener calls this verifier, and no production code registers
//! identities or issues capabilities. An eventual isolated listener must use
//! verified mTLS directly (no forwarded identity headers or bearer fallback).
use std::sync::Arc;

use axum::http::{Extensions, HeaderMap, Request, header::AUTHORIZATION};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_contracts::typed_id::SessionId;
use sha2::{Digest, Sha256};
use tonic::Status;
use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};

use super::authority::{self, WorkerAuthority};
use crate::storage::{StorageBackend, WorkerJournalCapabilityBinding};

fn denied() -> Status {
    Status::permission_denied("Worker assignment does not permit this operation")
}

/// Explicit middleware for a future qualified listener. Not installed by any
/// production bootstrap. Both raw HTTP and gRPC fail before body consumption;
/// authentication failure never falls through to a legacy route or bearer.
pub async fn authenticate_request(
    axum::extract::State(verifier): axum::extract::State<JournalCapabilityVerifier>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let grpc = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/grpc"));
    let service = <everruns_internal_protocol::WorkerServiceServer<super::WorkerServiceImpl>
        as tonic::server::NamedService>::NAME;
    let journal = grpc
        && request.method() == axum::http::Method::POST
        && request
            .uri()
            .path()
            .strip_prefix('/')
            .and_then(|path| path.strip_prefix(service))
            == Some("/ReadActorJournal");
    match verifier.authenticate(&mut request).await {
        // Fail closed even if a future route forgets its downstream scope gate.
        Ok(()) if journal => next.run(request).await,
        Ok(()) if grpc => denied().into_http::<axum::body::Body>(),
        Ok(()) => (
            axum::http::StatusCode::FORBIDDEN,
            "Worker assignment does not permit this operation",
        )
            .into_response(),
        Err(error) if grpc => error.into_http::<axum::body::Body>(),
        Err(error) => {
            let code = if error.code() == tonic::Code::Internal {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            } else {
                axum::http::StatusCode::UNAUTHORIZED
            };
            (code, "Worker assignment authentication failed").into_response()
        }
    }
}

/// Domain-separated digest for an existing 256-bit opaque proof. Does not issue
/// credentials. A future trusted issuer must use cryptographic randomness and
/// deliver proofs outside tool/sandbox environments, URLs and shared globals.
pub fn journal_capability_digest(proof: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"everruns-worker-journal-v1\0");
    digest.update(proof);
    digest.finalize().into()
}

fn capability_digest(headers: &HeaderMap) -> Result<[u8; 32], Status> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values.next().ok_or_else(denied)?;
    if values.next().is_some() {
        return Err(denied());
    }
    let encoded = value
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer wjc1."))
        .filter(|value| value.len() == 43)
        .ok_or_else(denied)?;
    let mut proof = [0_u8; 32];
    if URL_SAFE_NO_PAD.decode_slice(encoded, &mut proof).ok() != Some(32) {
        return Err(denied());
    }
    Ok(journal_capability_digest(&proof))
}

/// Mandatory proof bindings cannot be lost into a lease-only scope. Only this
/// verifier constructs the type; callers cannot create it from selectors.
#[derive(Clone)]
pub struct VerifiedJournalScope {
    binding: WorkerJournalCapabilityBinding,
    certificate_sha256: [u8; 32],
    capability_sha256: [u8; 32],
}

impl std::fmt::Debug for VerifiedJournalScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedJournalScope")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl VerifiedJournalScope {
    pub(super) async fn read_journal(
        &self,
        db: &StorageBackend,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: i64,
    ) -> Result<crate::storage::ActorJournalRows, Status> {
        if session_id != self.binding.session_id {
            return Err(denied());
        }
        db.read_worker_assignment_journal(
            &self.binding,
            crate::storage::WorkerJournalReadIdentity::Verified {
                certificate_sha256: &self.certificate_sha256,
                capability_sha256: &self.capability_sha256,
            },
            after_sequence,
            limit,
        )
        .await
        .map_err(|error| {
            super::worker::support::internal_status("Failed to authorize worker assignment", error)
        })?
        .ok_or_else(denied)
    }
}

/// Explicitly constructed component for a future isolated listener. Merely
/// configuring legacy mTLS does not enable this path or issue scoped access.
#[derive(Clone)]
pub struct JournalCapabilityVerifier {
    db: Arc<StorageBackend>,
}

impl JournalCapabilityVerifier {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// Shared HTTP/gRPC authentication edge, before bodies or route handlers.
    /// Requires Tonic's actual verified TCP TLS connection info. Proxy headers,
    /// executor IDs and grant/tenant/session selectors are never identity proof.
    pub async fn authenticate<B>(&self, request: &mut Request<B>) -> Result<(), Status> {
        authority::ensure_unset(request.extensions())?;
        let certificate_sha256 = verified_peer_digest(request.extensions())?;
        let capability_sha256 = capability_digest(request.headers())?;
        // Downstream handlers receive resolved authority, never the proof.
        request.headers_mut().remove(AUTHORIZATION);
        let binding = self
            .db
            .resolve_worker_journal_capability(&certificate_sha256, &capability_sha256)
            .await
            .map_err(|error| {
                super::worker::support::internal_status(
                    "Failed to authorize worker assignment",
                    error,
                )
            })?
            .ok_or_else(denied)?;
        request
            .extensions_mut()
            .insert(WorkerAuthority::VerifiedJournal(VerifiedJournalScope {
                binding,
                certificate_sha256,
                capability_sha256,
            }));
        Ok(())
    }
}

fn verified_peer_digest(extensions: &Extensions) -> Result<[u8; 32], Status> {
    // TlsConnectInfo has no public constructor; this evidence is populated by
    // the TLS transport after handshake. No certificate from HTTP is accepted.
    let certificates = extensions
        .get::<TlsConnectInfo<TcpConnectInfo>>()
        .and_then(TlsConnectInfo::peer_certs)
        .ok_or_else(denied)?;
    let leaf = certificates.first().ok_or_else(denied)?;
    Ok(Sha256::digest(leaf.as_ref()).into())
}
