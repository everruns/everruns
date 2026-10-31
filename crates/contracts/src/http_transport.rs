//! Host-owned HTTP for the shared provider protocols.
//!
//! Adapters send the already encoded request and return headers plus a lazy byte
//! stream. They do not interpret provider JSON or SSE. Native drivers use the
//! shared, SSRF-hardened reqwest client unless a transport is injected. Standard
//! forward-proxy settings select a destination-validated proxy path; the trusted
//! proxy owns DNS pinning on its outbound connection. Isolate
//! hosts must inject one and own URL policy, DNS, redirect refusal, timeouts and
//! cancellation (including cancelling the response reader when dropped).

use std::{pin::Pin, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, TryStreamExt};
pub use http::{HeaderMap, Method, Request, StatusCode, header};
use serde::de::DeserializeOwned;

#[cfg(not(target_arch = "wasm32"))]
mod proxy;

pub type HttpRequest = Request<Vec<u8>>;
pub type HttpBody = Pin<Box<dyn Stream<Item = Result<Bytes, HttpError>> + Send>>;

/// A transport failure. Messages must omit credentials and sensitive URLs.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HttpError {
    message: String,
    transient: bool,
}

impl HttpError {
    /// `transient` permits bounded send retries or reconnects before output.
    /// Use false for invalid requests/configuration, true for network failures.
    pub fn new(message: impl Into<String>, transient: bool) -> Self {
        Self {
            message: message.into(),
            transient,
        }
    }
}

/// The two retry decisions needed at the HTTP boundary.
pub trait TransportFailure: std::fmt::Display {
    fn retry_send(&self) -> bool;
    fn reconnect(&self) -> bool;
}

impl TransportFailure for HttpError {
    fn retry_send(&self) -> bool {
        self.transient
    }
    fn reconnect(&self) -> bool {
        self.transient
    }
}

/// Status used by the shared retry executor, before protocol classification.
pub trait HttpStatus {
    fn status(&self) -> StatusCode;
}

/// Headers are available before the body is read; SSE stays incremental.
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: HttpBody,
}

impl HttpStatus for HttpResponse {
    fn status(&self) -> StatusCode {
        self.status
    }
}

impl HttpResponse {
    pub fn new(status: StatusCode, headers: HeaderMap, body: HttpBody) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
    pub fn status(&self) -> StatusCode {
        self.status
    }
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    pub fn bytes_stream(self) -> HttpBody {
        self.body
    }
    pub async fn bytes(self) -> Result<Bytes, HttpError> {
        self.body
            .try_fold(Vec::new(), |mut out, chunk| async move {
                out.extend_from_slice(&chunk);
                Ok(out)
            })
            .await
            .map(Bytes::from)
    }
    pub async fn text(self) -> Result<String, HttpError> {
        Ok(String::from_utf8_lossy(&self.bytes().await?).into_owned())
    }
    pub async fn json<T: DeserializeOwned>(self) -> Result<T, HttpError> {
        serde_json::from_slice(&self.bytes().await?)
            .map_err(|e| HttpError::new(e.to_string(), false))
    }
}

/// Applications implement this with their host's `fetch`. Never follow
/// redirects with provider credentials. Drop must release the body reader.
/// Return HTTP errors (4xx/5xx) as responses so shared protocol classification
/// and Retry-After handling apply. Reserve `Err` for setup/network failures.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// A lazy default client or an application-supplied transport.
#[derive(Clone, Default)]
pub struct HttpClient(Option<Arc<dyn HttpTransport>>);

impl HttpClient {
    pub fn new(transport: Arc<dyn HttpTransport>) -> Self {
        Self(Some(transport))
    }
    pub fn get(&self, url: &str) -> HttpRequestBuilder {
        self.request(Method::GET, url)
    }
    pub fn post(&self, url: &str) -> HttpRequestBuilder {
        self.request(Method::POST, url)
    }
    fn request(&self, method: Method, url: &str) -> HttpRequestBuilder {
        HttpRequestBuilder {
            client: self.clone(),
            builder: Request::builder().method(method).uri(url),
            body: Vec::new(),
        }
    }
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        if let Some(transport) = &self.0 {
            return transport.execute(request).await;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            proxy::execute(request).await
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = request;
            Err(HttpError::new(
                "WASM provider drivers require an injected HTTP transport",
                false,
            ))
        }
    }
}

pub struct HttpRequestBuilder {
    client: HttpClient,
    builder: http::request::Builder,
    body: Vec<u8>,
}

impl HttpRequestBuilder {
    pub fn header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.builder = self.builder.header(name.as_ref(), value.as_ref());
        self
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        if let Some(out) = self.builder.headers_mut() {
            out.extend(headers);
        }
        self
    }
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
    pub async fn send(self) -> Result<HttpResponse, HttpError> {
        let request = self
            .builder
            .body(self.body)
            .map_err(|e| HttpError::new(e.to_string(), false))?;
        self.client.execute(request).await
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;

    impl TransportFailure for reqwest::Error {
        fn retry_send(&self) -> bool {
            self.is_connect() || self.is_timeout() || self.is_request()
        }
        fn reconnect(&self) -> bool {
            self.retry_send() || self.is_body() || self.is_decode()
        }
    }
    impl From<reqwest::Error> for HttpError {
        fn from(error: reqwest::Error) -> Self {
            let transient = error.reconnect();
            Self::new(error.without_url().to_string(), transient)
        }
    }
    impl HttpStatus for reqwest::Response {
        fn status(&self) -> StatusCode {
            self.status()
        }
    }
    impl From<reqwest::Response> for HttpResponse {
        fn from(response: reqwest::Response) -> Self {
            Self::new(
                response.status(),
                response.headers().clone(),
                Box::pin(response.bytes_stream().map_err(HttpError::from)),
            )
        }
    }
    #[async_trait]
    impl HttpTransport for reqwest::Client {
        async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let (parts, body) = request.into_parts();
            self.request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map(HttpResponse::from)
                .map_err(HttpError::from)
        }
    }
    #[async_trait]
    impl HttpSend for reqwest::RequestBuilder {
        async fn send_response(self) -> Result<HttpResponse, HttpError> {
            self.send()
                .await
                .map(HttpResponse::from)
                .map_err(HttpError::from)
        }
    }
}

/// Send an authenticated discovery request without coupling parsing to a client.
#[async_trait]
pub trait HttpSend: Send {
    async fn send_response(self) -> Result<HttpResponse, HttpError>;
}

#[async_trait]
impl HttpSend for HttpRequestBuilder {
    async fn send_response(self) -> Result<HttpResponse, HttpError> {
        self.send().await
    }
}
