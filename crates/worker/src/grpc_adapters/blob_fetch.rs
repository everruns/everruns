//! Fetch image and file bytes from the signed paths the control plane hands out.
//!
//! Decisions:
//! - Bytes never travel inside gRPC messages. Resolve RPCs answer with a
//!   short-lived signed path; this module GETs it as a plain HTTP request over
//!   the worker's existing control-plane channel. That reuses its connection,
//!   TLS and address, so no deployment configures anything new, and the body
//!   streams as HTTP data frames without base64 or the gRPC message cap.
//! - The request carries the worker bearer token, as gRPC calls do; the
//!   server's auth layer rejects it otherwise. That layer answers in gRPC
//!   style (HTTP 200 with a `grpc-status` header), so any response carrying
//!   `grpc-status` is an error here.
//! - The body is read frame by frame and abandoned once it passes the caller's
//!   cap, so a wrong or hostile size cannot make the worker buffer unbounded
//!   data.
//! - Generated images go up the same way: one POST whose body is a JSON head
//!   line and the raw bytes (`blob_wire`), never a protobuf field.

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_internal_protocol::blob_wire;
use tonic::codegen::{Body as _, Service as _, http};
use tonic::transport::Channel;

/// Upper bound for any single fetched object: the image upload limit.
pub(crate) const MAX_BLOB_BYTES: usize = 100 * 1024 * 1024;

/// Upper bound for a JSON answer to an upload.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// How many paths one batch fetches at the same time.
pub(crate) const FETCH_CONCURRENCY: usize = 8;

/// GETs signed paths over the control-plane channel.
#[derive(Clone)]
pub(crate) struct BlobFetcher {
    channel: Channel,
    authorization: Option<http::HeaderValue>,
}

impl BlobFetcher {
    pub(crate) fn new(channel: Channel, token: Option<&str>) -> Self {
        let authorization = token
            .filter(|t| !t.is_empty())
            .and_then(|t| http::HeaderValue::from_str(&format!("Bearer {t}")).ok());
        Self {
            channel,
            authorization,
        }
    }

    /// GET `path` and return its body, refusing anything over `max_bytes`.
    pub(crate) async fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        if !path.starts_with("/internal/blobs/") {
            return Err(AgentLoopError::store(format!(
                "Unexpected byte path from control plane: {path}"
            )));
        }
        self.send(
            http::Request::get(path),
            tonic::body::Body::empty(),
            max_bytes,
        )
        .await
    }

    /// Store a generated image of `org_id` and return what the server stored.
    pub(crate) async fn upload_image(
        &self,
        org_id: i64,
        head: &blob_wire::ImageUploadHead,
        data: &[u8],
    ) -> Result<blob_wire::UploadedImage> {
        let body = blob_wire::encode_image_upload(head, data)
            .map_err(|e| AgentLoopError::store(format!("Failed to encode image upload: {e}")))?;
        let request =
            http::Request::post(format!("{}?org_id={org_id}", blob_wire::IMAGE_UPLOAD_PATH))
                .header(http::header::CONTENT_TYPE, "application/octet-stream");
        let body = tonic::body::Body::new(http_body_util::Full::new(bytes::Bytes::from(body)));
        let answer = self.send(request, body, MAX_ANSWER_BYTES).await?;
        serde_json::from_slice(&answer)
            .map_err(|e| AgentLoopError::store(format!("Unreadable image upload answer: {e}")))
    }

    /// Send one request over the channel and read its body, up to `max_bytes`.
    async fn send(
        &self,
        mut request: http::request::Builder,
        body: tonic::body::Body,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        let store = |what: &str, e: &dyn std::fmt::Display| {
            AgentLoopError::store(format!("Failed to {what}: {e}"))
        };
        if let Some(value) = &self.authorization {
            request = request.header(http::header::AUTHORIZATION, value.clone());
        }
        let request = request
            .body(body)
            .map_err(|e| store("build byte request", &e))?;

        let mut channel = self.channel.clone();
        std::future::poll_fn(|cx| channel.poll_ready(cx))
            .await
            .map_err(|e| store("reach control plane", &e))?;
        let response = channel
            .call(request)
            .await
            .map_err(|e| store("transfer bytes", &e))?;

        let status = response.status();
        if !status.is_success() || response.headers().contains_key("grpc-status") {
            return Err(AgentLoopError::store(format!(
                "Byte transfer refused (status {status}, grpc-status {:?})",
                response.headers().get("grpc-status")
            )));
        }
        let too_large =
            || AgentLoopError::store(format!("Fetched object exceeds {max_bytes} bytes"));
        let declared = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if declared.is_some_and(|len| len > max_bytes as u64) {
            return Err(too_large());
        }

        let mut body = response.into_body();
        let mut out = Vec::with_capacity(declared.unwrap_or(0) as usize);
        while let Some(frame) =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await
        {
            let frame = frame.map_err(|e| store("read fetched bytes", &e))?;
            if let Ok(data) = frame.into_data() {
                if out.len() + data.len() > max_bytes {
                    return Err(too_large());
                }
                out.extend_from_slice(&data);
            }
        }
        Ok(out)
    }

    /// Fetch every `(key, path)` pair, at most [`FETCH_CONCURRENCY`] at a
    /// time. Fails on the first error; results come back in no fixed order.
    pub(crate) async fn fetch_all<K: Send + 'static>(
        &self,
        links: Vec<(K, String)>,
        max_bytes: usize,
    ) -> Result<Vec<(K, Vec<u8>)>> {
        let mut pending = links.into_iter();
        let mut running = tokio::task::JoinSet::new();
        let mut out = Vec::new();
        loop {
            while running.len() < FETCH_CONCURRENCY {
                let Some((key, path)) = pending.next() else {
                    break;
                };
                let fetcher = self.clone();
                running
                    .spawn(async move { fetcher.fetch(&path, max_bytes).await.map(|b| (key, b)) });
            }
            let Some(joined) = running.join_next().await else {
                return Ok(out);
            };
            let fetched = joined
                .map_err(|e| AgentLoopError::store(format!("Byte fetch task failed: {e}")))??;
            out.push(fetched);
        }
    }
}

// The fetch path itself is covered end to end against a real worker endpoint
// in the server's gRPC tests (`worker_link::grpc_service::tests::worker_blobs`).
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_signed_blob_paths_are_fetched() {
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let fetcher = BlobFetcher::new(channel, Some("token"));
        for path in [
            "/v1/sessions",
            "http://evil.example/internal/blobs/images/x",
            "",
        ] {
            let error = fetcher.fetch(path, 1024).await.unwrap_err().to_string();
            assert!(error.contains("Unexpected byte path"), "{path}: {error}");
        }
    }
}
