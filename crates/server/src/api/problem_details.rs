use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::common::ErrorResponse;

const MAX_PROBLEM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Apply standard headers to JSON problem responses.
///
/// Rewrites `Content-Type` to `application/problem+json`, per RFC 9457, and
/// mirrors `retry_after_seconds` into the standard `Retry-After` header.
/// Bodies are preserved unchanged. Non-JSON error bodies pass through.
pub async fn standard_error_headers(req: Request, next: Next) -> Response {
    let response = next.run(req).await;
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    if parts.headers.contains_key(header::RETRY_AFTER) {
        return Response::from_parts(parts, body);
    }

    let bytes = match to_bytes(body, MAX_PROBLEM_RESPONSE_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, "failed to read problem JSON response body");
            return ErrorResponse::internal_error().into_response();
        }
    };
    if let Ok(problem) = serde_json::from_slice::<ErrorResponse>(&bytes)
        && let Some(seconds) = problem.retry_after_seconds
        && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
    {
        parts.headers.insert(header::RETRY_AFTER, value);
    }
    Response::from_parts(parts, Body::from(bytes))
}
