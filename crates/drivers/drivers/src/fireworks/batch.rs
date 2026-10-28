//! Native account-scoped datasets and batch jobs. Artifacts are downloaded
//! without provider authentication, only from application-trusted origins.
use super::FireworksChatDriver;
use crate::native_batch::{BoundedBody, Http, http_error, valid_segment, validate};
use async_trait::async_trait;
use everruns_contracts::{ProviderEndpoint, batch::*};
use reqwest::Method;
use serde_json::{Value, json};
use std::{collections::HashSet, io::Write, time::Duration};

const MAX_REQUESTS: usize = 50_000;
const MAX_UPLOAD: usize = 100_000_000;
const BOUNDARY: &str = "everruns_fireworks_upload";

/// Trusted account and exact HTTPS origins permitted to serve signed artifacts.
/// No origins are inferred from a ticket or a provider response. Configure the
/// storage origin used by your Fireworks account before collecting results.
#[derive(Clone, Debug)]
pub struct FireworksBatchConfig {
    account: String,
    artifact_origins: Vec<String>,
}
impl FireworksBatchConfig {
    pub fn new(account: impl Into<String>) -> Result<Self, BatchError> {
        let account = account.into();
        if !valid_segment(&account) {
            return Err(BatchError::InvalidRequest("invalid Fireworks account"));
        }
        Ok(Self {
            account,
            artifact_origins: Vec::new(),
        })
    }
    /// Allow an exact storage origin. HTTP is allowed only on loopback for local
    /// protocol tests; production artifacts must use HTTPS. No URL paths/keys.
    pub fn artifact_origin(mut self, origin: &str) -> Result<Self, BatchError> {
        let url = reqwest::Url::parse(origin)
            .map_err(|_| BatchError::InvalidRequest("invalid artifact origin"))?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        if (url.scheme() != "https" && !(local && url.scheme() == "http"))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(BatchError::InvalidRequest(
                "artifact origin must be an HTTPS origin",
            ));
        }
        self.artifact_origins
            .push(url.origin().ascii_serialization());
        Ok(self)
    }
    fn resource(&self, name: &str, kind: &str) -> Result<(), BatchError> {
        let prefix = format!("accounts/{}/{kind}/", self.account);
        if !name.strip_prefix(&prefix).is_some_and(valid_segment) {
            return Err(BatchError::InvalidTicket);
        }
        Ok(())
    }
    fn artifact_url(&self, value: &str) -> Result<reqwest::Url, BatchError> {
        let url =
            reqwest::Url::parse(value).map_err(|_| BatchError::Protocol("invalid artifact URL"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !self
                .artifact_origins
                .contains(&url.origin().ascii_serialization())
        {
            return Err(BatchError::Protocol("artifact origin is not trusted"));
        }
        Ok(url)
    }
}
fn state(
    job: &Value,
    expected: Option<&str>,
    config: &FireworksBatchConfig,
) -> Result<BatchStatus, BatchError> {
    let name = job["name"]
        .as_str()
        .ok_or(BatchError::Protocol("missing job name"))?;
    config
        .resource(name, "batchInferenceJobs")
        .map_err(|_| BatchError::Protocol("invalid job account or ID"))?;
    if expected.is_some_and(|id| id != name) {
        return Err(BatchError::Protocol("job ID mismatch"));
    }
    match job["state"].as_str() {
        Some(
            "JOB_STATE_CREATING"
            | "JOB_STATE_RUNNING"
            | "JOB_STATE_WRITING_RESULTS"
            | "JOB_STATE_VALIDATING"
            | "JOB_STATE_PENDING"
            | "JOB_STATE_RE_QUEUEING"
            | "JOB_STATE_CREATING_INPUT_DATASET"
            | "JOB_STATE_IDLE"
            | "JOB_STATE_PAUSED",
        ) => Ok(BatchStatus::Processing),
        Some("JOB_STATE_CANCELLING") => Ok(BatchStatus::Cancelling),
        Some(
            "JOB_STATE_COMPLETED"
            | "JOB_STATE_FAILED"
            | "JOB_STATE_CANCELLED"
            | "JOB_STATE_EXPIRED"
            | "JOB_STATE_EARLY_STOPPED",
        ) => Ok(BatchStatus::Ended),
        _ => Err(BatchError::Protocol("unknown Fireworks job state")),
    }
}
impl FireworksChatDriver {
    fn batch_config(&self) -> Result<&FireworksBatchConfig, BatchError> {
        self.batch.as_ref().ok_or(BatchError::Unsupported)
    }
    async fn control(
        &self,
        http: Http<'_>,
        method: Method,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
        creating: bool,
    ) -> Result<Value, BatchError> {
        let mut url = reqwest::Url::parse(http.endpoint.base_url().unwrap_or_default())
            .map_err(|_| BatchError::InvalidRequest("invalid Fireworks endpoint"))?;
        // Inference and management APIs share the configured origin, never a
        // hardcoded credential destination. Custom gateways must serve both.
        let base = url
            .path()
            .trim_end_matches('/')
            .strip_suffix("/inference/v1")
            .or_else(|| url.path().trim_end_matches('/').strip_suffix("/v1"))
            .ok_or(BatchError::InvalidRequest(
                "Fireworks base must end in /inference/v1 or /v1",
            ))?;
        let path = format!("{base}/v1/{path}");
        let (path, query) = path
            .split_once('?')
            .map_or((path.as_str(), None), |(p, q)| (p, Some(q)));
        url.set_path(path);
        if let Some(query) = query {
            url.set_query(Some(query));
        }
        let bytes = http
            .request_url(method, url.to_string(), body, content_type, creating)
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            if creating {
                BatchError::SubmissionUncertain
            } else {
                BatchError::Protocol("invalid Fireworks JSON")
            }
        })
    }
    async fn job(&self, http: Http<'_>, id: &str) -> Result<Value, BatchError> {
        self.batch_config()?.resource(id, "batchInferenceJobs")?;
        let job = self
            .control(http, Method::GET, id, vec![], "application/json", false)
            .await?;
        state(&job, Some(id), self.batch_config()?)?;
        Ok(job)
    }
}
#[async_trait]
impl BatchDriver for FireworksChatDriver {
    fn batch_format(&self) -> &'static str {
        "fireworks-datasets-v1"
    }
    async fn submit(
        &self,
        endpoint: &ProviderEndpoint,
        requests: Vec<BatchRequest>,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<String, BatchError> {
        validate(&requests, MAX_REQUESTS)?;
        let config = self.batch_config()?;
        if config.artifact_origins.is_empty() {
            return Err(BatchError::InvalidRequest(
                "configure trusted Fireworks artifact origins before submission",
            ));
        }
        let model = &requests[0].config.model;
        if requests.iter().any(|r| &r.config.model != model) {
            return Err(BatchError::InvalidRequest(
                "Fireworks requires one model per batch",
            ));
        }
        let prefix = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"everruns-batch.jsonl\"\r\nContent-Type: application/jsonl\r\n\r\n"
        );
        let mut body = BoundedBody {
            bytes: prefix.into_bytes(),
            max_bytes: MAX_UPLOAD,
        };
        for item in &requests {
            let mut wire = self
                .inner
                .encode_batch_request_for_url(
                    "https://api.fireworks.ai/inference/v1/chat/completions",
                    &item.messages,
                    &item.config,
                )
                .map_err(|_| BatchError::InvalidRequest("Fireworks request encoding failed"))?;
            wire.as_object_mut()
                .ok_or(BatchError::InvalidRequest("invalid request"))?
                .remove("model");
            serde_json::to_writer(&mut body, &json!({"custom_id":item.id,"body":wire}))
                .map_err(|_| BatchError::InvalidRequest("Fireworks batch exceeds 100 MB"))?;
            body.write_all(b"\n")
                .map_err(|_| BatchError::InvalidRequest("Fireworks batch exceeds 100 MB"))?;
        }
        body.write_all(format!("\r\n--{BOUNDARY}--\r\n").as_bytes())
            .map_err(|_| BatchError::InvalidRequest("Fireworks batch exceeds 100 MB"))?;
        let http = Http {
            endpoint,
            timeout,
            max_bytes,
        };
        let id = format!("everruns-{}", uuid::Uuid::now_v7());
        let input = format!("accounts/{}/datasets/{id}", config.account);
        let output = format!("accounts/{}/datasets/{id}-output", config.account);
        let dataset = self
            .control(
                http,
                Method::POST,
                &format!("accounts/{}/datasets", config.account),
                serde_json::to_vec(&json!({"datasetId":id,"dataset":{"userUploaded":{}}}))
                    .map_err(|_| BatchError::InvalidRequest("dataset encoding failed"))?,
                "application/json",
                true,
            )
            .await?;
        if dataset["name"] != input {
            return Err(BatchError::SubmissionUncertain);
        }
        // Dataset creation, upload, and job creation are independent effects.
        // Never replay or delete any stage after an uncertain response.
        self.control(
            http,
            Method::POST,
            &format!("{input}:upload"),
            body.bytes,
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            true,
        )
        .await?;
        let job = self
            .control(
                http,
                Method::POST,
                &format!(
                    "accounts/{}/batchInferenceJobs?batchInferenceJobId={id}",
                    config.account
                ),
                serde_json::to_vec(
                    &json!({"model":model,"inputDatasetId":input,"outputDatasetId":output}),
                )
                .map_err(|_| BatchError::InvalidRequest("job encoding failed"))?,
                "application/json",
                true,
            )
            .await?;
        state(
            &job,
            Some(&format!(
                "accounts/{}/batchInferenceJobs/{id}",
                config.account
            )),
            config,
        )
        .map_err(|_| BatchError::SubmissionUncertain)?;
        job["name"]
            .as_str()
            .map(str::to_owned)
            .ok_or(BatchError::SubmissionUncertain)
    }
    async fn poll(
        &self,
        endpoint: &ProviderEndpoint,
        id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<BatchStatus, BatchError> {
        state(
            &self
                .job(
                    Http {
                        endpoint,
                        timeout,
                        max_bytes,
                    },
                    id,
                )
                .await?,
            Some(id),
            self.batch_config()?,
        )
    }
    async fn collect(
        &self,
        endpoint: &ProviderEndpoint,
        id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Vec<BatchResult>, BatchError> {
        let http = Http {
            endpoint,
            timeout,
            max_bytes,
        };
        let job = self.job(http, id).await?;
        let config = self.batch_config()?;
        if state(&job, Some(id), config)? != BatchStatus::Ended {
            return Err(BatchError::NotReady);
        }
        if job["state"] != "JOB_STATE_COMPLETED" {
            return Err(BatchError::JobFailed {
                reason: match job["state"].as_str() {
                    Some("JOB_STATE_CANCELLED") => BatchItemError::Cancelled,
                    Some("JOB_STATE_EXPIRED") => BatchItemError::Expired,
                    _ => BatchItemError::Other,
                },
            });
        }
        let dataset = job["outputDatasetId"]
            .as_str()
            .ok_or(BatchError::Protocol("missing output dataset"))?;
        config
            .resource(dataset, "datasets")
            .map_err(|_| BatchError::Protocol("invalid output dataset account"))?;
        let downloads = self
            .control(
                http,
                Method::GET,
                &format!("{dataset}:getDownloadEndpoint"),
                vec![],
                "application/json",
                false,
            )
            .await?;
        let files = downloads["filenameToSignedUrls"]
            .as_object()
            .ok_or(BatchError::Protocol("missing signed artifacts"))?;
        if files.is_empty() || files.len() > 16 {
            return Err(BatchError::Protocol("invalid artifact count"));
        }
        let mut remaining = max_bytes;
        let mut ids = HashSet::new();
        let mut results = Vec::new();
        for value in files.values() {
            let url = config.artifact_url(
                value
                    .as_str()
                    .ok_or(BatchError::Protocol("invalid signed artifact"))?,
            )?;
            // THREAT[TM-LLM-056]: Never attach provider auth to a signed storage
            // URL, follow redirects, or surface a credential-bearing URL.
            let bytes = tokio::time::timeout(timeout, async {
                let mut response = everruns_contracts::driver_helpers::shared_request_http_client()
                    .get(url)
                    .timeout(timeout)
                    .send()
                    .await
                    .map_err(|_| BatchError::Transport)?;
                if !response.status().is_success() {
                    return Err(BatchError::Http {
                        status: response.status().as_u16(),
                    });
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| BatchError::Transport)? {
                    if chunk.len() > remaining.saturating_sub(bytes.len()) {
                        return Err(BatchError::ResponseTooLarge);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(bytes)
            })
            .await
            .map_err(|_| BatchError::Transport)??;
            remaining -= bytes.len();
            for line in bytes
                .split(|b| *b == b'\n')
                .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            {
                if results.len() == MAX_REQUESTS {
                    return Err(BatchError::Protocol("too many results"));
                }
                let row: Value = serde_json::from_slice(line)
                    .map_err(|_| BatchError::Protocol("invalid result JSONL"))?;
                let id = row["custom_id"]
                    .as_str()
                    .ok_or(BatchError::Protocol("missing result ID"))?
                    .to_owned();
                if !ids.insert(id.clone()) {
                    return Err(BatchError::Protocol("duplicate result ID"));
                }
                let result = if let Some(error) = row.get("error").filter(|v| !v.is_null()) {
                    if !error.is_object() {
                        return Err(BatchError::Protocol("invalid row error"));
                    }
                    Err(error["code"]
                        .as_u64()
                        .map(http_error)
                        .unwrap_or(BatchItemError::Other))
                } else {
                    let response = &row["response"];
                    let body = if let Some(code) = response["status_code"].as_u64() {
                        if code != 200 {
                            results.push(BatchResult {
                                id,
                                result: Err(http_error(code)),
                            });
                            continue;
                        }
                        response["body"].clone()
                    } else {
                        response.clone()
                    };
                    Ok(self
                        .inner
                        .decode_batch_response(body)
                        .map_err(|_| BatchError::Protocol("invalid Fireworks completion"))?)
                };
                results.push(BatchResult { id, result });
            }
        }
        Ok(results)
    }
    async fn cancel(
        &self,
        _endpoint: &ProviderEndpoint,
        _id: &str,
        _timeout: Duration,
        _max_bytes: usize,
    ) -> Result<BatchStatus, BatchError> {
        // The published API offers deletion, which is not a cancellation
        // contract and can destroy collectable metadata. Never substitute it.
        Err(BatchError::UnsupportedOperation("Fireworks cancellation"))
    }
}
