// Internal presigned image endpoint for worker image resolution
//
// Decision: Workers fetch images via presigned HTTP URLs instead of base64 over gRPC.
// This removes the 150MB gRPC message limit workaround and keeps gRPC messages small.
// Presigned URLs are HMAC-signed with WORKER_GRPC_AUTH_TOKEN so no org auth is needed.

use crate::storage::StorageBackend;
use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::get,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Presigned URL expiry duration (5 minutes)
const PRESIGN_EXPIRY_SECS: i64 = 300;

/// App state for internal image routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    /// Shared secret for HMAC signature validation (WORKER_GRPC_AUTH_TOKEN)
    pub signing_secret: String,
}

/// Query parameters for presigned image fetch
#[derive(Debug, Deserialize)]
pub struct PresignedQuery {
    pub org_id: i64,
    pub expires: i64,
    pub sig: String,
}

/// Create internal image routes (no org auth — presigned URL validates access)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/internal/images/{image_id}", get(get_presigned_image))
        .with_state(state)
}

/// Generate a presigned URL for an image
pub fn generate_presigned_url(
    base_url: &str,
    image_id: Uuid,
    org_id: i64,
    signing_secret: &str,
) -> String {
    let expires = chrono::Utc::now().timestamp() + PRESIGN_EXPIRY_SECS;
    let sig = compute_signature(signing_secret, image_id, org_id, expires);
    format!(
        "{}/internal/images/{}?org_id={}&expires={}&sig={}",
        base_url.trim_end_matches('/'),
        image_id,
        org_id,
        expires,
        sig
    )
}

/// Compute HMAC-SHA256 signature for presigned URL parameters
fn compute_signature(secret: &str, image_id: Uuid, org_id: i64, expires: i64) -> String {
    let message = format!("{}:{}:{}", image_id, org_id, expires);
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Validate presigned URL signature and expiry
fn validate_presigned(
    signing_secret: &str,
    image_id: Uuid,
    query: &PresignedQuery,
) -> Result<(), (StatusCode, String)> {
    // Check expiry
    let now = chrono::Utc::now().timestamp();
    if now > query.expires {
        return Err((StatusCode::FORBIDDEN, "Presigned URL expired".to_string()));
    }

    // Validate signature
    let expected = compute_signature(signing_secret, image_id, query.org_id, query.expires);
    if !constant_time_eq(expected.as_bytes(), query.sig.as_bytes()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Invalid presigned URL signature".to_string(),
        ));
    }

    Ok(())
}

/// Constant-time comparison to prevent timing attacks on signature validation
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// GET /internal/images/{image_id}?org_id=...&expires=...&sig=...
///
/// Returns image binary data. Validates presigned URL signature against WORKER_GRPC_AUTH_TOKEN.
async fn get_presigned_image(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
    Query(query): Query<PresignedQuery>,
) -> Result<Response, (StatusCode, String)> {
    validate_presigned(&state.signing_secret, image_id, &query)?;

    let row = state
        .db
        .get_image(query.org_id, image_id)
        .await
        .map_err(|e| {
            tracing::error!(%image_id, error = %e, "Failed to get presigned image");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Image not found".to_string()))?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, row.content_type)
        .header(header::CACHE_CONTROL, "private, max-age=60")
        .body(Body::from(row.data))
        .unwrap();

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presigned_url_roundtrip() {
        let secret = "test-secret-token";
        let image_id = Uuid::new_v4();
        let org_id = 42;

        let url = generate_presigned_url("http://localhost:9000", image_id, org_id, secret);

        // Parse the URL to extract query params
        assert!(url.contains(&format!("/internal/images/{}", image_id)));
        assert!(url.contains(&format!("org_id={}", org_id)));
        assert!(url.contains("expires="));
        assert!(url.contains("sig="));
    }

    #[test]
    fn signature_validation_succeeds_for_valid_params() {
        let secret = "test-secret";
        let image_id = Uuid::new_v4();
        let org_id = 1;
        let expires = chrono::Utc::now().timestamp() + 300;
        let sig = compute_signature(secret, image_id, org_id, expires);

        let query = PresignedQuery {
            org_id,
            expires,
            sig,
        };
        assert!(validate_presigned(secret, image_id, &query).is_ok());
    }

    #[test]
    fn signature_validation_rejects_expired() {
        let secret = "test-secret";
        let image_id = Uuid::new_v4();
        let org_id = 1;
        let expires = chrono::Utc::now().timestamp() - 10; // expired
        let sig = compute_signature(secret, image_id, org_id, expires);

        let query = PresignedQuery {
            org_id,
            expires,
            sig,
        };
        assert!(validate_presigned(secret, image_id, &query).is_err());
    }

    #[test]
    fn signature_validation_rejects_wrong_sig() {
        let secret = "test-secret";
        let image_id = Uuid::new_v4();
        let org_id = 1;
        let expires = chrono::Utc::now().timestamp() + 300;

        let query = PresignedQuery {
            org_id,
            expires,
            sig: "bad-signature".to_string(),
        };
        assert!(validate_presigned(secret, image_id, &query).is_err());
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
    }
}
