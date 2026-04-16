// Request ID middleware
//
// Extracts X-Request-ID from incoming request or generates a UUID v4.
// Stores the ID in request extensions for downstream handlers.
// Echoes the ID in the X-Request-ID response header.
//
// Must sit outside (wrap) TraceLayer so the span has the ID when created.

use axum::http::{HeaderName, HeaderValue, Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use uuid::Uuid;

pub static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Opaque per-request correlation ID stored in request extensions.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

#[derive(Clone, Default)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

#[derive(Clone)]
pub struct RequestIdService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestIdService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<ResBody>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        const MAX_REQUEST_ID_LEN: usize = 256;
        let request_id = req
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= MAX_REQUEST_ID_LEN)
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        req.extensions_mut().insert(RequestId(request_id.clone()));

        let fut = self.inner.call(req);

        Box::pin(async move {
            let mut response = fut.await?;
            if let Ok(val) = HeaderValue::from_str(&request_id) {
                response
                    .headers_mut()
                    .insert(REQUEST_ID_HEADER.clone(), val);
            }
            Ok(response)
        })
    }
}
