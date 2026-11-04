//! Test-only workerd HTTP transport for shared provider protocol conformance.
//! The fixture runner creates every endpoint and credential locally.
use async_trait::async_trait;
use everruns_contracts::http_transport::{
    HeaderMap, HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode, header,
};
use futures::StreamExt;
use std::time::Duration;
use worker::{
    send::{SendFuture, SendWrapper},
    *,
};

pub struct FetchTransport;
struct AbortOnDrop(Option<SendWrapper<AbortController>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(controller) = self.0.take() {
            controller.0.abort();
        }
    }
}
fn transport_error() -> HttpError {
    // A JS exception may contain an authenticated URL. Keep it out of telemetry.
    HttpError::new("host fetch transport failed", true)
}
#[async_trait]
impl HttpTransport for FetchTransport {
    async fn execute(&self, outgoing: HttpRequest) -> std::result::Result<HttpResponse, HttpError> {
        SendFuture::new(async move {
            let headers = Headers::new();
            for (name, value) in outgoing.headers() {
                headers
                    .set(
                        name.as_str(),
                        value
                            .to_str()
                            .map_err(|_| HttpError::new("invalid HTTP header", false))?,
                    )
                    .map_err(|_| HttpError::new("invalid HTTP header", false))?;
            }
            let mut init = RequestInit::new();
            init.with_method(Method::from(outgoing.method().to_string()))
                .with_headers(headers)
                // THREAT[TM-API-013]: never follow redirects with provider auth.
                .with_redirect(RequestRedirect::Manual);
            if !outgoing.body().is_empty() {
                init.with_body(Some(
                    js_sys::Uint8Array::from(outgoing.body().as_slice()).into(),
                ));
            }
            let request = Request::new_with_init(&outgoing.uri().to_string(), &init)
                .map_err(|_| HttpError::new("invalid HTTP request", false))?;
            let controller = AbortController::default();
            let signal = controller.signal();
            let guard = AbortOnDrop(Some(SendWrapper::new(controller)));
            let mut response = everruns_contracts::rt::timeout(
                Duration::from_secs(60),
                Fetch::Request(request).send_with_signal(&signal),
            )
            .await
            .map_err(|_| transport_error())?
            .map_err(|_| transport_error())?;
            let status = StatusCode::from_u16(response.status_code())
                .map_err(|_| HttpError::new("invalid HTTP status", false))?;
            let mut headers = HeaderMap::new();
            for (name, value) in response.headers().entries() {
                let name = header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| HttpError::new("invalid response header", false))?;
                let value = header::HeaderValue::from_str(&value)
                    .map_err(|_| HttpError::new("invalid response header", false))?;
                headers.append(name, value);
            }
            // Fetch exposes null bodies (for example 204) without a reader.
            if matches!(response.body(), ResponseBody::Empty) {
                return Ok(HttpResponse::new(
                    status,
                    headers,
                    Box::pin(futures::stream::empty()),
                ));
            }
            let stream = SendWrapper::new(response.stream().map_err(|_| transport_error())?);
            let body = futures::stream::unfold(Some((stream, guard)), |state| async move {
                let (mut stream, guard) = state?;
                let next = everruns_contracts::rt::timeout(
                    Duration::from_secs(300),
                    SendFuture::new(async { stream.0.next().await }),
                )
                .await;
                match next {
                    Ok(Some(Ok(bytes))) => Some((Ok(bytes.into()), Some((stream, guard)))),
                    Ok(None) => None,
                    _ => Some((Err(transport_error()), None)),
                }
            });
            Ok(HttpResponse::new(status, headers, Box::pin(body)))
        })
        .await
    }
}
