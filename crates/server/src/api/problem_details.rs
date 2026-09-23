use axum::body::{Body, HttpBody};
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use futures_util::{StreamExt, stream};
use http_body_util::{BodyStream, StreamBody};

use super::common::ErrorResponse;

const MAX_RETRY_METADATA_RESPONSE_BYTES: usize = 64 * 1024;

/// Apply standard headers to JSON problem responses.
///
/// Rewrites `Content-Type` to `application/problem+json`, per RFC 9457, and
/// mirrors `retry_after_seconds` into the standard `Retry-After` header.
/// Retry metadata inspection is limited to small, fixed-size response bodies.
/// Bodies and existing retry headers are preserved unchanged.
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

    let body_size = response.body().size_hint().exact();
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    if parts.headers.contains_key(header::RETRY_AFTER)
        || !matches!(
            status,
            axum::http::StatusCode::TOO_MANY_REQUESTS | axum::http::StatusCode::SERVICE_UNAVAILABLE
        )
    {
        return Response::from_parts(parts, body);
    }
    let Some(body_size) = body_size else {
        return Response::from_parts(parts, body);
    };
    if body_size > MAX_RETRY_METADATA_RESPONSE_BYTES as u64 {
        return Response::from_parts(parts, body);
    }

    let (bytes, body) = inspect_body_preserving_frames(body, body_size as usize).await;
    if let Some(bytes) = bytes
        && let Ok(problem) = serde_json::from_slice::<ErrorResponse>(&bytes)
        && let Some(seconds) = problem.retry_after_seconds
        && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
    {
        parts.headers.insert(header::RETRY_AFTER, value);
    }
    Response::from_parts(parts, body)
}

async fn inspect_body_preserving_frames(body: Body, capacity: usize) -> (Option<Vec<u8>>, Body) {
    let mut body_stream = BodyStream::new(body);
    let mut frames = Vec::new();
    let mut bytes = Vec::with_capacity(capacity);

    while let Some(result) = body_stream.next().await {
        match result {
            Ok(frame) => {
                if let Some(data) = frame.data_ref() {
                    if data.len() > capacity.saturating_sub(bytes.len()) {
                        frames.push(frame);
                        let replay = stream::iter(frames.into_iter().map(Ok::<_, axum::Error>))
                            .chain(body_stream);
                        return (None, Body::new(StreamBody::new(replay)));
                    }
                    bytes.extend_from_slice(data);
                }
                frames.push(frame);
            }
            Err(error) => {
                let replay = stream::iter(frames.into_iter().map(Ok::<_, axum::Error>))
                    .chain(stream::once(async { Err(error) }))
                    .chain(body_stream);
                return (None, Body::new(StreamBody::new(replay)));
            }
        }
    }

    let replay = stream::iter(frames.into_iter().map(Ok::<_, axum::Error>));
    (Some(bytes), Body::new(StreamBody::new(replay)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::{Bytes, to_bytes};
    use axum::http::StatusCode;
    use axum::routing::get;
    use tower::ServiceExt;

    #[tokio::test]
    async fn oversized_json_error_preserves_status_and_body() {
        const FORMER_INSPECTION_LIMIT: usize = 16 * 1024 * 1024;
        let mut payload = Vec::with_capacity(FORMER_INSPECTION_LIMIT + 32);
        payload.extend_from_slice(br#"{"detail":""#);
        payload.extend(std::iter::repeat_n(b'x', FORMER_INSPECTION_LIMIT + 1));
        payload.extend_from_slice(br#""}"#);
        let expected = Bytes::from(payload);
        let handler_body = expected.clone();
        let app = Router::new()
            .route(
                "/",
                get(move || {
                    let body = handler_body.clone();
                    async move {
                        Response::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body))
                            .unwrap()
                    }
                }),
            )
            .layer(axum::middleware::from_fn(standard_error_headers));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
        let actual = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn existing_retry_after_header_is_preserved() {
        let expected = Bytes::from_static(
            br#"{"title":"Service Unavailable","status":503,"retry_after_seconds":1}"#,
        );
        let handler_body = expected.clone();
        let app = Router::new()
            .route(
                "/",
                get(move || {
                    let body = handler_body.clone();
                    async move {
                        Response::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .header(header::CONTENT_TYPE, "application/json")
                            .header(header::RETRY_AFTER, "17")
                            .body(Body::from(body))
                            .unwrap()
                    }
                }),
            )
            .layer(axum::middleware::from_fn(standard_error_headers));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "17");
        let actual = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn inspection_failure_preserves_status_body_and_error() {
        let source = stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(b"before")),
            Err(std::io::Error::other("body read failed")),
            Ok(Bytes::from_static(b"after")),
        ]);
        let (inspected, replayed) =
            inspect_body_preserving_frames(Body::from_stream(source), 11).await;
        assert!(inspected.is_none());

        let response = Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(replayed)
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let mut frames = BodyStream::new(response.into_body());
        assert_eq!(
            frames.next().await.unwrap().unwrap().data_ref().unwrap(),
            "before"
        );
        assert!(
            frames
                .next()
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("body read failed")
        );
        assert_eq!(
            frames.next().await.unwrap().unwrap().data_ref().unwrap(),
            "after"
        );
    }
}
