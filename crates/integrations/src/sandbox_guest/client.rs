//! The Everruns side: a client for the guest protocol over any transport.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::sandbox_guest::{
    GUEST_PROTOCOL_VERSION, GUEST_TOKEN_HEADER, GuestArchiveImport, GuestError, GuestExecRequest,
    GuestExecResponse, GuestFileList, GuestFileStat, GuestHealth, GuestWriteResponse, paths,
};
use serde::de::DeserializeOwned;

/// One request to the guest, independent of how it travels.
#[derive(Debug, Clone)]
pub struct GuestRequest {
    pub method: GuestMethod,
    /// Path and query, starting with `/v1/`.
    pub path_and_query: String,
    /// `(name, value)` headers, the token included.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// How long the transport may wait for the whole answer.
    pub timeout: Duration,
}

/// HTTP methods the protocol uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl GuestMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

/// The guest's answer.
#[derive(Debug, Clone)]
pub struct GuestResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// How requests reach one guest. A provider supplies this; everything else
/// (auth, retries on connect, error mapping) lives in [`GuestClient`].
#[async_trait]
pub trait GuestTransport: Send + Sync {
    async fn send(&self, request: GuestRequest) -> Result<GuestResponse, String>;
}

/// Plain HTTP to a base URL the server can reach (a pod IP in-cluster, a
/// local port in tests).
pub struct DirectTransport {
    base_url: String,
    http: reqwest::Client,
}

impl DirectTransport {
    pub fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            // The guest is addressed by IP inside a private network; an
            // HTTP(S)_PROXY in the server's environment must not capture it.
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|err| format!("cannot build the guest HTTP client: {err}"))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        })
    }
}

#[async_trait]
impl GuestTransport for DirectTransport {
    async fn send(&self, request: GuestRequest) -> Result<GuestResponse, String> {
        let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
            .map_err(|err| err.to_string())?;
        let mut builder = self
            .http
            .request(
                method,
                format!("{}{}", self.base_url, request.path_and_query),
            )
            .timeout(request.timeout)
            .body(request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| format!("guest unreachable: {err}"))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|err| format!("guest answer cut off: {err}"))?;
        Ok(GuestResponse {
            status,
            body: body.to_vec(),
        })
    }
}

/// A failed guest call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestClientError {
    /// The guest could not be reached (not started yet, network, transport).
    Unreachable(String),
    /// The guest answered with an error.
    Guest {
        status: u16,
        code: String,
        message: String,
    },
    /// The answer did not parse.
    Protocol(String),
}

impl GuestClientError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Guest { status: 404, .. })
    }
}

impl std::fmt::Display for GuestClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(message) => write!(f, "sandbox guest unreachable: {message}"),
            Self::Guest { message, .. } => f.write_str(message),
            Self::Protocol(message) => write!(f, "sandbox guest protocol error: {message}"),
        }
    }
}

impl std::error::Error for GuestClientError {}

/// Default time a file or control call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Extra time over a command's own timeout for the answer to arrive.
const EXEC_SLACK: Duration = Duration::from_secs(30);

/// Client for one guest.
#[derive(Clone)]
pub struct GuestClient {
    transport: Arc<dyn GuestTransport>,
    token: String,
}

impl std::fmt::Debug for GuestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestClient").finish_non_exhaustive()
    }
}

fn encode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

impl GuestClient {
    pub fn new(transport: Arc<dyn GuestTransport>, token: impl Into<String>) -> Self {
        Self {
            transport,
            token: token.into(),
        }
    }

    async fn call(
        &self,
        method: GuestMethod,
        path_and_query: String,
        body: Vec<u8>,
        content_type: Option<&str>,
        timeout: Duration,
    ) -> Result<Vec<u8>, GuestClientError> {
        let mut headers = vec![(GUEST_TOKEN_HEADER.to_string(), self.token.clone())];
        if let Some(content_type) = content_type {
            headers.push(("content-type".to_string(), content_type.to_string()));
        }
        let response = self
            .transport
            .send(GuestRequest {
                method,
                path_and_query,
                headers,
                body,
                timeout,
            })
            .await
            .map_err(GuestClientError::Unreachable)?;
        if (200..300).contains(&response.status) {
            return Ok(response.body);
        }
        let (code, message) = match serde_json::from_slice::<GuestError>(&response.body) {
            Ok(error) => (error.code, error.message),
            Err(_) => (
                "unknown".to_string(),
                format!(
                    "sandbox guest answered HTTP {}: {}",
                    response.status,
                    String::from_utf8_lossy(&response.body)
                        .chars()
                        .take(300)
                        .collect::<String>()
                ),
            ),
        };
        Err(GuestClientError::Guest {
            status: response.status,
            code,
            message,
        })
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: GuestMethod,
        path_and_query: String,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<T, GuestClientError> {
        let has_body = body.is_some();
        let bytes = self
            .call(
                method,
                path_and_query,
                body.unwrap_or_default(),
                has_body.then_some("application/json"),
                timeout,
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|err| GuestClientError::Protocol(err.to_string()))
    }

    /// Health and capabilities; fails when the guest speaks another protocol.
    pub async fn health(&self) -> Result<GuestHealth, GuestClientError> {
        let health: GuestHealth = self
            .json(
                GuestMethod::Get,
                paths::HEALTH.to_string(),
                None,
                Duration::from_secs(10),
            )
            .await?;
        if health.protocol != GUEST_PROTOCOL_VERSION {
            return Err(GuestClientError::Protocol(format!(
                "the sandbox guest speaks protocol {} (version {}), this server speaks {GUEST_PROTOCOL_VERSION}; use a sandbox image built for this Everruns version",
                health.protocol, health.version
            )));
        }
        Ok(health)
    }

    /// Poll [`Self::health`] until the guest answers or `deadline` passes.
    pub async fn wait_ready(&self, deadline: Duration) -> Result<GuestHealth, GuestClientError> {
        let started = std::time::Instant::now();
        let mut delay = Duration::from_millis(200);
        loop {
            match self.health().await {
                Ok(health) => return Ok(health),
                Err(err @ GuestClientError::Protocol(_)) => return Err(err),
                Err(err) if started.elapsed() >= deadline => return Err(err),
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
            }
        }
    }

    pub async fn exec(
        &self,
        request: &GuestExecRequest,
    ) -> Result<GuestExecResponse, GuestClientError> {
        let timeout = Duration::from_millis(
            request
                .timeout_ms
                .unwrap_or(everruns_contracts::sandbox_guest::DEFAULT_EXEC_TIMEOUT_MS),
        ) + EXEC_SLACK;
        let body = serde_json::to_vec(request)
            .map_err(|err| GuestClientError::Protocol(err.to_string()))?;
        self.json(
            GuestMethod::Post,
            paths::EXEC.to_string(),
            Some(body),
            timeout,
        )
        .await
    }

    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, GuestClientError> {
        self.call(
            GuestMethod::Get,
            format!("{}?path={}", paths::FILES, encode(path)),
            Vec::new(),
            None,
            CALL_TIMEOUT,
        )
        .await
    }

    pub async fn write_file(
        &self,
        path: &str,
        content: &[u8],
    ) -> Result<GuestWriteResponse, GuestClientError> {
        let bytes = self
            .call(
                GuestMethod::Put,
                format!("{}?path={}", paths::FILES, encode(path)),
                content.to_vec(),
                Some("application/octet-stream"),
                CALL_TIMEOUT,
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|err| GuestClientError::Protocol(err.to_string()))
    }

    pub async fn delete(&self, path: &str) -> Result<(), GuestClientError> {
        self.call(
            GuestMethod::Delete,
            format!("{}?path={}", paths::FILES, encode(path)),
            Vec::new(),
            None,
            CALL_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    pub async fn stat(&self, path: &str) -> Result<GuestFileStat, GuestClientError> {
        self.json(
            GuestMethod::Get,
            format!("{}?path={}", paths::STAT, encode(path)),
            None,
            CALL_TIMEOUT,
        )
        .await
    }

    pub async fn list(&self, path: &str) -> Result<GuestFileList, GuestClientError> {
        self.json(
            GuestMethod::Get,
            format!("{}?path={}", paths::LIST, encode(path)),
            None,
            CALL_TIMEOUT,
        )
        .await
    }

    /// A gzipped tar of directory `path`.
    pub async fn export_archive(&self, path: &str) -> Result<Vec<u8>, GuestClientError> {
        self.call(
            GuestMethod::Get,
            format!("{}?path={}", paths::ARCHIVE, encode(path)),
            Vec::new(),
            None,
            CALL_TIMEOUT * 5,
        )
        .await
    }

    /// Unpack a gzipped tar into directory `path`.
    pub async fn import_archive(
        &self,
        path: &str,
        archive: Vec<u8>,
    ) -> Result<GuestArchiveImport, GuestClientError> {
        let bytes = self
            .call(
                GuestMethod::Put,
                format!("{}?path={}", paths::ARCHIVE, encode(path)),
                archive,
                Some("application/gzip"),
                CALL_TIMEOUT * 5,
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|err| GuestClientError::Protocol(err.to_string()))
    }
}
