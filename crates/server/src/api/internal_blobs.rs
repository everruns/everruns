// Signed byte links for workers: images and file attachments.
//
// Decisions:
// - The worker gRPC link carries references, never file or image bytes. A
//   resolve RPC answers with a short-lived signed path; the worker GETs it as
//   a plain HTTP request on the same control-plane connection, so the bytes
//   travel as an HTTP body, not inside a protobuf message: no base64, no
//   message-size cap, no whole-message buffering.
// - Served by the worker gRPC server itself (`app_builder::background`), not
//   the public API listener. Workers already reach that address with the same
//   TLS and bearer token, so no deployment needs a new address, port or
//   network rule, and the route is never exposed through the public edge.
// - Legacy paths are HMAC-signed with WORKER_GRPC_AUTH_TOKEN over kind, id,
//   org and expiry. Holders of that shared worker key can mint paths within
//   the existing trusted-worker boundary. Journal-only assignment scopes are
//   refused before signature or body processing. Missing or ambiguous server
//   authority also fails closed.
// - Uploads go the other way on the same server: a worker POSTs a generated
//   image as raw bytes (`blob_wire`), authorized by the worker bearer token
//   like the gRPC call it replaces, which also named its org.

use crate::security::constant_time_eq;
use crate::storage::StorageBackend;
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_internal_protocol::blob_wire::{
    IMAGE_UPLOAD_PATH, UploadedImage, decode_image_upload,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// How long a signed link stays valid.
const LINK_EXPIRY_SECS: i64 = 300;

/// What a signed link points at. Part of the signed message, so a link for an
/// image can never be replayed against a file with the same id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobKind {
    Image,
    File,
}

impl BlobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::File => "files",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "images" => Some(Self::Image),
            "files" => Some(Self::File),
            _ => None,
        }
    }
}

/// Signs worker byte links. Held by the gRPC service.
#[derive(Clone)]
pub struct BlobLinkSigner {
    secret: String,
}

impl std::fmt::Debug for BlobLinkSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobLinkSigner").finish_non_exhaustive()
    }
}

impl BlobLinkSigner {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Signed path, relative to the worker gRPC endpoint.
    pub fn path(&self, kind: BlobKind, id: Uuid, org_id: i64) -> String {
        let expires = chrono::Utc::now().timestamp() + LINK_EXPIRY_SECS;
        self.path_expiring(kind, id, org_id, expires)
    }

    fn path_expiring(&self, kind: BlobKind, id: Uuid, org_id: i64, expires: i64) -> String {
        let sig = compute_signature(&self.secret, kind, id, org_id, expires);
        format!(
            "/internal/blobs/{}/{}?org_id={}&expires={}&sig={}",
            kind.as_str(),
            id,
            org_id,
            expires,
            sig
        )
    }
}

/// App state for the internal byte routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    /// Shared secret for signature validation (WORKER_GRPC_AUTH_TOKEN)
    pub signing_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct SignedQuery {
    pub org_id: i64,
    pub expires: i64,
    pub sig: String,
}

/// Internal byte routes, merged into the worker gRPC server. The server's
/// bearer-token layer covers them too; the signature authorizes the object.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/internal/blobs/{kind}/{id}", get(get_blob))
        .route(
            IMAGE_UPLOAD_PATH,
            // The head line and slack on top of the image limit.
            post(upload_image).layer(DefaultBodyLimit::max(
                crate::api::images::MAX_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        // A journal reader has no byte authority, even for a valid legacy URL.
        // Run before extractors: do not buffer an upload or touch storage first.
        .layer(axum::middleware::from_fn_with_state(
            state.db.clone(),
            require_blob_authority,
        ))
        .with_state(state)
}

async fn require_blob_authority(
    State(db): State<Arc<StorageBackend>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use crate::worker_link::grpc_service::authority::{self, WorkerAuthority};
    match authority::resolve(request.extensions()) {
        Ok(authority @ (WorkerAuthority::LegacyWorker | WorkerAuthority::Development)) => {
            // Read-link authentication must reject invalid signatures without
            // any database access. The handler checks the floor before lookup.
            if matches!(
                *request.method(),
                axum::http::Method::GET | axum::http::Method::HEAD
            ) {
                return next.run(request).await;
            }
            if let Err(error) = authority::admit_ingress(&db, authority).await {
                return ingress_error(error).into_response();
            }
            next.run(request).await
        }
        Ok(WorkerAuthority::ActorJournal(_) | WorkerAuthority::VerifiedJournal(_)) => (
            StatusCode::FORBIDDEN,
            "Worker assignment does not permit blob access",
        )
            .into_response(),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            "Worker authority context required",
        )
            .into_response(),
    }
}

fn ingress_error(error: tonic::Status) -> (StatusCode, &'static str) {
    let status = if error.code() == tonic::Code::PermissionDenied {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, "Worker byte authority unavailable")
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    pub org_id: i64,
}

/// POST /internal/blobs/images?org_id=...: a head line, then the image bytes.
async fn upload_image(
    State(state): State<AppState>,
    Query(query): Query<UploadQuery>,
    Extension(authority): Extension<crate::worker_link::grpc_service::authority::WorkerAuthority>,
    body: Bytes,
) -> Result<Json<UploadedImage>, (StatusCode, &'static str)> {
    // A body admitted before cutover may arrive later. Refuse before decoding
    // or storing it once its legacy authority is no longer accepted.
    crate::worker_link::grpc_service::authority::admit_ingress(&state.db, &authority)
        .await
        .map_err(ingress_error)?;
    let (head, data) =
        decode_image_upload(&body).ok_or((StatusCode::BAD_REQUEST, "Malformed image upload"))?;
    if data.len() > crate::api::images::MAX_IMAGE_SIZE {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Image too large"));
    }
    let (thumbnail_data, thumbnail_content_type) =
        crate::api::images::generate_thumbnail(data, &head.content_type)
            .map(|(data, content_type)| (Some(data), Some(content_type)))
            .unwrap_or((None, None));
    let metadata = if head.metadata.is_null() {
        serde_json::json!({})
    } else {
        head.metadata
    };
    let row = state
        .db
        .create_image(
            query.org_id,
            crate::storage::CreateImageRow {
                org_id: query.org_id,
                filename: head.filename,
                content_type: head.content_type,
                size_bytes: data.len() as i64,
                data: data.to_vec(),
                thumbnail_data,
                thumbnail_content_type,
                metadata,
            },
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "Failed to store a worker image upload");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?;
    Ok(Json(UploadedImage {
        id: row.id.uuid(),
        filename: row.filename,
        content_type: row.content_type,
        size_bytes: row.size_bytes,
        metadata: row.metadata,
        created_at: row.created_at,
    }))
}

fn compute_signature(secret: &str, kind: BlobKind, id: Uuid, org_id: i64, expires: i64) -> String {
    let message = format!("{}:{}:{}:{}", kind.as_str(), id, org_id, expires);
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn validate_signature(
    signing_secret: &str,
    kind: BlobKind,
    id: Uuid,
    query: &SignedQuery,
) -> Result<(), (StatusCode, &'static str)> {
    if chrono::Utc::now().timestamp() > query.expires {
        return Err((StatusCode::FORBIDDEN, "Link expired"));
    }
    let expected = compute_signature(signing_secret, kind, id, query.org_id, query.expires);
    if !constant_time_eq(expected.as_bytes(), query.sig.as_bytes()) {
        return Err((StatusCode::FORBIDDEN, "Invalid link signature"));
    }
    Ok(())
}

/// GET /internal/blobs/{kind}/{id}?org_id=...&expires=...&sig=...
async fn get_blob(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
    Query(query): Query<SignedQuery>,
    Extension(authority): Extension<crate::worker_link::grpc_service::authority::WorkerAuthority>,
) -> Result<Response, (StatusCode, &'static str)> {
    let kind = BlobKind::parse(&kind).ok_or((StatusCode::NOT_FOUND, "Unknown blob kind"))?;
    validate_signature(&state.signing_secret, kind, id, &query)?;
    crate::worker_link::grpc_service::authority::admit_ingress(&state.db, &authority)
        .await
        .map_err(ingress_error)?;

    let internal = |error: anyhow::Error| {
        tracing::error!(%id, kind = kind.as_str(), error = %error, "Failed to load blob for worker");
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    };
    let (content_type, data) = match kind {
        BlobKind::Image => state
            .db
            .get_image(query.org_id, id)
            .await
            .map_err(internal)?
            .map(|row| (row.content_type, row.data)),
        BlobKind::File => state
            .db
            .get_file(query.org_id, id)
            .await
            .map_err(internal)?
            .map(|row| (row.content_type, row.data)),
    }
    .ok_or((StatusCode::NOT_FOUND, "Not found"))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, data.len())
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(data))
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query_from(path: &str) -> (String, SignedQuery) {
        let (path, query) = path.split_once('?').unwrap();
        (path.to_string(), serde_urlencoded::from_str(query).unwrap())
    }

    #[test]
    fn signed_path_validates_for_its_kind_id_and_org() {
        let signer = BlobLinkSigner::new("secret");
        let id = Uuid::new_v4();
        let (path, query) = query_from(&signer.path(BlobKind::File, id, 42));

        assert_eq!(path, format!("/internal/blobs/files/{id}"));
        assert_eq!(query.org_id, 42);
        assert!(validate_signature("secret", BlobKind::File, id, &query).is_ok());
    }

    #[test]
    fn signature_does_not_transfer_across_kind_id_org_or_secret() {
        let signer = BlobLinkSigner::new("secret");
        let id = Uuid::new_v4();
        let (_, query) = query_from(&signer.path(BlobKind::Image, id, 7));

        assert!(validate_signature("secret", BlobKind::File, id, &query).is_err());
        assert!(validate_signature("secret", BlobKind::Image, Uuid::new_v4(), &query).is_err());
        assert!(validate_signature("other", BlobKind::Image, id, &query).is_err());
        let other_org = SignedQuery {
            org_id: 8,
            expires: query.expires,
            sig: query.sig.clone(),
        };
        assert!(validate_signature("secret", BlobKind::Image, id, &other_org).is_err());
    }

    #[test]
    fn expired_link_is_rejected() {
        let signer = BlobLinkSigner::new("secret");
        let id = Uuid::new_v4();
        let expired = chrono::Utc::now().timestamp() - 10;
        let (_, query) = query_from(&signer.path_expiring(BlobKind::Image, id, 1, expired));
        assert_eq!(
            validate_signature("secret", BlobKind::Image, id, &query).unwrap_err(),
            (StatusCode::FORBIDDEN, "Link expired")
        );
    }

    #[tokio::test]
    async fn bad_signature_is_refused_before_any_lookup() {
        use tower::ServiceExt;
        // A lazy pool never connects: the signature check must answer first.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let db = Arc::new(StorageBackend::from_database(
            crate::storage::Database::new(pool),
        ));
        let app = routes(AppState {
            db,
            signing_secret: "secret".into(),
        })
        .layer(axum::Extension(
            crate::worker_link::grpc_service::authority::WorkerAuthority::Development,
        ));
        let id = Uuid::new_v4();
        let expires = chrono::Utc::now().timestamp() + 60;
        for (kind, expected) in [
            ("images", StatusCode::FORBIDDEN),
            ("videos", StatusCode::NOT_FOUND),
        ] {
            let uri = format!("/internal/blobs/{kind}/{id}?org_id=1&expires={expires}&sig=bad");
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
    }
}
