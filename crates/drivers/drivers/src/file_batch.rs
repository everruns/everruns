//! Native chat batch lifecycles sharing result/count validation. OpenAI and
//! Groq upload files before creation; Mistral submits inline. Never replay an
//! ambiguous external mutation.
#[cfg(feature = "groq")]
use crate::groq::GroqChatDriver;
use crate::native_batch::{BoundedBody, Http, http_error, valid_segment, validate};
#[cfg(feature = "openai")]
use crate::openai::{OpenAIChatDriver, OpenAICompletionsChatDriver};
use async_trait::async_trait;
use everruns_contracts::batch::*;
use everruns_contracts::{LlmResponse, ProviderEndpoint};
use reqwest::Method;
use serde_json::{Value, json};
use std::{collections::HashSet, io::Write, time::Duration};

const MAX_REQUESTS: usize = 50_000;
const MAX_BYTES: usize = 200_000_000;
const BOUNDARY: &str = "everruns_batch_upload";

#[async_trait]
trait Codec: Send + Sync {
    const URL: &'static str;
    const JOBS: &'static str = "batches";
    const LIMIT: usize = MAX_REQUESTS;
    fn valid_job_id(id: &str) -> bool {
        id.starts_with("batch_")
    }
    fn valid_file_id(id: &str) -> bool {
        id.starts_with("file-") || id.starts_with("file_")
    }
    fn normalize(&self, job: Value) -> Result<Value, BatchError> {
        Ok(job)
    }
    async fn create(
        &self,
        http: Http<'_>,
        requests: Vec<BatchRequest>,
    ) -> Result<String, BatchError> {
        submit(self, http, requests).await
    }
    fn encode(&self, item: &BatchRequest) -> std::result::Result<Value, BatchError>;
    async fn decode(&self, value: Value) -> std::result::Result<LlmResponse, BatchError>;
}
#[cfg(feature = "openai")]
#[async_trait]
impl Codec for OpenAIChatDriver {
    const URL: &'static str = "/v1/responses";
    fn encode(&self, item: &BatchRequest) -> std::result::Result<Value, BatchError> {
        self.inner
            .encode_batch_request(&item.messages, &item.config)
            .map_err(|_| BatchError::InvalidRequest("Responses request encoding failed"))
    }
    async fn decode(&self, value: Value) -> std::result::Result<LlmResponse, BatchError> {
        self.inner
            .decode_batch_response(value)
            .await
            .map_err(|_| BatchError::Protocol("invalid Responses completion"))
    }
}
#[cfg(feature = "openai")]
#[async_trait]
impl Codec for OpenAICompletionsChatDriver {
    const URL: &'static str = "/v1/chat/completions";
    fn encode(&self, item: &BatchRequest) -> std::result::Result<Value, BatchError> {
        self.inner
            .encode_batch_request(&item.messages, &item.config)
            .map_err(|_| BatchError::InvalidRequest("Chat Completions request encoding failed"))
    }
    async fn decode(&self, value: Value) -> std::result::Result<LlmResponse, BatchError> {
        self.inner
            .decode_batch_response(value)
            .map_err(|_| BatchError::Protocol("invalid Chat Completions completion"))
    }
}
fn job_path<C: Codec + ?Sized>(id: &str, suffix: &str) -> std::result::Result<String, BatchError> {
    if !valid_segment(id) || !C::valid_job_id(id) {
        return Err(BatchError::InvalidTicket);
    }
    Ok(format!("{}/{id}{suffix}", C::JOBS))
}
fn file_path<C: Codec + ?Sized>(id: &str) -> std::result::Result<String, BatchError> {
    if !valid_segment(id) || !C::valid_file_id(id) {
        return Err(BatchError::Protocol("invalid file ID"));
    }
    Ok(format!("files/{id}/content"))
}
fn status<C: Codec + ?Sized>(
    value: &Value,
    expected: Option<&str>,
    url: &str,
) -> std::result::Result<BatchStatus, BatchError> {
    let id = value["id"]
        .as_str()
        .ok_or(BatchError::Protocol("missing job ID"))?;
    job_path::<C>(id, "").map_err(|_| BatchError::Protocol("invalid job ID"))?;
    if expected.is_some_and(|expected| id != expected) {
        return Err(BatchError::Protocol("job ID mismatch"));
    }
    if value["endpoint"].as_str() != Some(url) {
        return Err(BatchError::Protocol("job endpoint mismatch"));
    }
    match value["status"].as_str() {
        Some("validating" | "in_progress" | "finalizing") => Ok(BatchStatus::Processing),
        Some("cancelling") => Ok(BatchStatus::Cancelling),
        Some("completed" | "failed" | "expired" | "cancelled") => Ok(BatchStatus::Ended),
        _ => Err(BatchError::Protocol("unknown job status")),
    }
}
fn code_error(value: &Value) -> BatchItemError {
    match value["code"].as_str().or_else(|| value["type"].as_str()) {
        Some("batch_expired") => BatchItemError::Expired,
        Some("batch_cancelled" | "batch_canceled") => BatchItemError::Cancelled,
        Some("invalid_request_error" | "invalid_json_line" | "model_not_found") => {
            BatchItemError::InvalidRequest
        }
        Some("authentication_error" | "invalid_api_key") => BatchItemError::Authentication,
        Some("permission_error") => BatchItemError::Permission,
        Some("rate_limit_exceeded" | "insufficient_quota") => BatchItemError::RateLimited,
        Some("server_error" | "internal_error") => BatchItemError::Server,
        _ => BatchItemError::Other,
    }
}
fn refused(body: &Value) -> bool {
    body.pointer("/choices/0/message/refusal")
        .and_then(Value::as_str)
        .is_some()
        || body["output"].as_array().into_iter().flatten().any(|item| {
            item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|part| part["type"] == "refusal")
        })
}

async fn submit<C: Codec + ?Sized>(
    codec: &C,
    http: Http<'_>,
    requests: Vec<BatchRequest>,
) -> std::result::Result<String, BatchError> {
    validate(&requests, C::LIMIT)?;
    let model = &requests[0].config.model;
    if requests.iter().any(|r| &r.config.model != model) {
        return Err(BatchError::InvalidRequest(
            "provider requires one model per batch",
        ));
    }
    // Compact JSON escapes CR/LF in caller text. A CRLF multipart boundary
    // therefore cannot collide with a JSONL line, including malicious prompts.
    let prefix = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nbatch\r\n\
         --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"everruns-batch.jsonl\"\r\n\
         Content-Type: application/jsonl\r\n\r\n"
    );
    let mut body = BoundedBody {
        bytes: prefix.into_bytes(),
        max_bytes: MAX_BYTES,
    };
    #[derive(serde::Serialize)]
    struct Row<'a> {
        custom_id: &'a str,
        method: &'static str,
        url: &'static str,
        body: Value,
    }
    for item in &requests {
        serde_json::to_writer(
            &mut body,
            &Row {
                custom_id: &item.id,
                method: "POST",
                url: C::URL,
                body: codec.encode(item)?,
            },
        )
        .map_err(|_| {
            BatchError::InvalidRequest(
                "batch exceeds 200 MB or cannot be serialized; split it explicitly",
            )
        })?;
        body.write_all(b"\n")
            .map_err(|_| BatchError::InvalidRequest("batch exceeds 200 MB"))?;
    }
    body.write_all(format!("\r\n--{BOUNDARY}--\r\n").as_bytes())
        .map_err(|_| BatchError::InvalidRequest("batch exceeds 200 MB"))?;
    let uploaded = http
        .request(
            Method::POST,
            "files",
            body.bytes,
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            true,
        )
        .await?;
    let file: Value =
        serde_json::from_slice(&uploaded).map_err(|_| BatchError::SubmissionUncertain)?;
    let file_id = file["id"].as_str().ok_or(BatchError::SubmissionUncertain)?;
    file_path::<C>(file_id).map_err(|_| BatchError::SubmissionUncertain)?;
    // Do not delete the staging file on an uncertain create: it may already
    // belong to a live job. Applications reconcile provider artifacts explicitly.
    let body = serde_json::to_vec(
        &json!({"input_file_id":file_id,"endpoint":C::URL,"completion_window":"24h"}),
    )
    .map_err(|_| BatchError::InvalidRequest("batch encoding failed"))?;
    let job = http.json(Method::POST, "batches", body, true).await?;
    status::<C>(&job, None, C::URL).map_err(|_| BatchError::SubmissionUncertain)?;
    job["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or(BatchError::SubmissionUncertain)
}
async fn collect<C: Codec>(
    codec: &C,
    http: Http<'_>,
    job_id: &str,
    expected: Option<&[String]>,
) -> std::result::Result<Vec<BatchResult>, BatchError> {
    let job = codec.normalize(
        http.json(Method::GET, &job_path::<C>(job_id, "")?, vec![], false)
            .await?,
    )?;
    if status::<C>(&job, Some(job_id), C::URL)? != BatchStatus::Ended {
        return Err(BatchError::NotReady);
    }
    if job["status"] == "failed" {
        return Err(BatchError::JobFailed {
            reason: job
                .pointer("/errors/data/0")
                .map(code_error)
                .unwrap_or(BatchItemError::Other),
        });
    }
    let expected_ids: Option<HashSet<&str>> =
        expected.map(|ids| ids.iter().map(String::as_str).collect());
    let mut results = Vec::new();
    let mut ids = HashSet::new();
    let mut successes = 0u64;
    let mut failures = 0u64;
    for field in ["output_file_id", "error_file_id"] {
        let bytes = if field == "output_file_id"
            && job["outputs"].is_array()
            && job.get(field).is_none_or(Value::is_null)
        {
            let mut bytes = Vec::new();
            for row in job["outputs"]
                .as_array()
                .ok_or(BatchError::Protocol("invalid inline outputs"))?
            {
                serde_json::to_writer(&mut bytes, row)
                    .map_err(|_| BatchError::Protocol("invalid inline output"))?;
                bytes.push(b'\n');
            }
            bytes
        } else {
            let Some(id) = job.get(field).filter(|id| !id.is_null()) else {
                continue;
            };
            let path = file_path::<C>(id.as_str().ok_or(BatchError::Protocol("invalid file ID"))?)?;
            http.request(Method::GET, &path, vec![], "application/json", false)
                .await?
        };
        for line in bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        {
            if results.len() == C::LIMIT {
                return Err(BatchError::Protocol("too many results"));
            }
            let row: Value = serde_json::from_slice(line)
                .map_err(|_| BatchError::Protocol("invalid result JSONL"))?;
            let id = row["custom_id"]
                .as_str()
                .ok_or(BatchError::Protocol("missing result ID"))?
                .to_owned();
            if !ids.insert(id.clone())
                || expected_ids
                    .as_ref()
                    .is_some_and(|ids| !ids.contains(id.as_str()))
            {
                return Err(BatchError::Protocol("duplicate or unknown result ID"));
            }
            let result = if let Some(error) = row.get("error").filter(|e| !e.is_null()) {
                if !error.is_object() {
                    return Err(BatchError::Protocol("invalid item error"));
                }
                failures += 1;
                Err(code_error(error))
            } else {
                let response = &row["response"];
                let code = response["status_code"]
                    .as_u64()
                    .ok_or(BatchError::Protocol("missing result status"))?;
                if code != 200 {
                    failures += 1;
                    Err(http_error(code))
                } else {
                    // Native counts describe HTTP request completion. A valid
                    // refusal is still completed, even when the item is unusable.
                    successes += 1;
                    let body = &response["body"];
                    if let Some(error) = body.get("error").filter(|e| !e.is_null()) {
                        Err(code_error(error))
                    } else if body["status"] == "cancelled" {
                        Err(BatchItemError::Cancelled)
                    } else if body["status"] == "failed" {
                        Err(BatchItemError::Server)
                    } else if refused(body) {
                        Err(BatchItemError::Other)
                    } else {
                        Ok(codec.decode(body.clone()).await?)
                    }
                }
            };
            results.push(BatchResult { id, result });
        }
    }
    let counts = &job["request_counts"];
    let total = counts["total"]
        .as_u64()
        .ok_or(BatchError::Protocol("missing request counts"))?;
    if counts["completed"].as_u64() != Some(successes)
        || counts["failed"].as_u64() != Some(failures)
        || total < successes + failures
        || expected.is_some_and(|ids| ids.len() as u64 != total)
    {
        return Err(BatchError::Protocol("result counts mismatch"));
    }
    let missing = match job["status"].as_str() {
        Some("cancelled") => Some(BatchItemError::Cancelled),
        Some("expired") => Some(BatchItemError::Expired),
        _ => None,
    };
    if let (Some(expected), Some(reason)) = (expected, missing) {
        // Only trusted submission IDs can identify unprocessed cancelled items.
        // Counts above prevent a missing success/error file being disguised.
        for id in expected {
            if !ids.contains(id) {
                results.push(BatchResult {
                    id: id.clone(),
                    result: Err(reason),
                });
            }
        }
    } else if total != results.len() as u64 {
        return Err(BatchError::Protocol("missing item results"));
    }
    Ok(results)
}
macro_rules! batch_driver {
    ($driver:ty,$format:literal) => {
        #[async_trait]
        impl BatchDriver for $driver {
            fn batch_format(&self) -> &'static str {
                $format
            }
            async fn submit(
                &self,
                endpoint: &ProviderEndpoint,
                requests: Vec<BatchRequest>,
                timeout: Duration,
                max_bytes: usize,
            ) -> std::result::Result<String, BatchError> {
                Codec::create(
                    self,
                    Http {
                        endpoint,
                        timeout,
                        max_bytes,
                    },
                    requests,
                )
                .await
            }
            async fn poll(
                &self,
                endpoint: &ProviderEndpoint,
                job_id: &str,
                timeout: Duration,
                max_bytes: usize,
            ) -> std::result::Result<BatchStatus, BatchError> {
                let job = Http {
                    endpoint,
                    timeout,
                    max_bytes,
                }
                .json(Method::GET, &job_path::<Self>(job_id, "")?, vec![], false)
                .await?;
                status::<Self>(&self.normalize(job)?, Some(job_id), Self::URL)
            }
            async fn collect(
                &self,
                endpoint: &ProviderEndpoint,
                job_id: &str,
                timeout: Duration,
                max_bytes: usize,
            ) -> std::result::Result<Vec<BatchResult>, BatchError> {
                collect(
                    self,
                    Http {
                        endpoint,
                        timeout,
                        max_bytes,
                    },
                    job_id,
                    None,
                )
                .await
            }
            async fn collect_for_requests(
                &self,
                endpoint: &ProviderEndpoint,
                job_id: &str,
                ids: &[String],
                timeout: Duration,
                max_bytes: usize,
            ) -> std::result::Result<Vec<BatchResult>, BatchError> {
                collect(
                    self,
                    Http {
                        endpoint,
                        timeout,
                        max_bytes,
                    },
                    job_id,
                    Some(ids),
                )
                .await
            }
            async fn cancel(
                &self,
                endpoint: &ProviderEndpoint,
                job_id: &str,
                timeout: Duration,
                max_bytes: usize,
            ) -> std::result::Result<BatchStatus, BatchError> {
                let current = self.poll(endpoint, job_id, timeout, max_bytes).await?;
                if current != BatchStatus::Processing {
                    return Ok(current);
                }
                let job = Http {
                    endpoint,
                    timeout,
                    max_bytes,
                }
                .json(
                    Method::POST,
                    &job_path::<Self>(job_id, "/cancel")?,
                    b"{}".to_vec(),
                    false,
                )
                .await?;
                status::<Self>(&self.normalize(job)?, Some(job_id), Self::URL)
            }
        }
    };
}
#[cfg(feature = "openai")]
batch_driver!(OpenAIChatDriver, "openai-responses-batch-v1");
#[cfg(feature = "openai")]
batch_driver!(
    OpenAICompletionsChatDriver,
    "openai-chat-completions-batch-v1"
);

#[cfg(feature = "groq")]
#[async_trait]
impl Codec for GroqChatDriver {
    const URL: &'static str = "/v1/chat/completions";
    fn encode(&self, item: &BatchRequest) -> Result<Value, BatchError> {
        self.inner
            .encode_batch_request_for_url(
                "https://api.groq.com/openai/v1/chat/completions",
                &item.messages,
                &item.config,
            )
            .map_err(|_| BatchError::InvalidRequest("Groq request encoding failed"))
    }
    async fn decode(&self, value: Value) -> Result<LlmResponse, BatchError> {
        self.inner
            .decode_batch_response(value)
            .map_err(|_| BatchError::Protocol("invalid Groq completion"))
    }
}
#[cfg(feature = "groq")]
batch_driver!(GroqChatDriver, "groq-chat-completions-v1");

#[cfg(feature = "mistral")]
#[async_trait]
impl Codec for crate::mistral::MistralChatDriver {
    const URL: &'static str = "/v1/chat/completions";
    const JOBS: &'static str = "batch/jobs";
    const LIMIT: usize = 10_000;
    fn valid_job_id(_id: &str) -> bool {
        true
    }
    fn valid_file_id(_id: &str) -> bool {
        true
    }
    fn encode(&self, item: &BatchRequest) -> Result<Value, BatchError> {
        self.inner
            .encode_batch_request_for_url(
                "https://api.mistral.ai/v1/chat/completions",
                &item.messages,
                &crate::mistral::supported_effort(&item.config),
            )
            .map_err(|_| BatchError::InvalidRequest("Mistral request encoding failed"))
    }
    async fn decode(&self, value: Value) -> Result<LlmResponse, BatchError> {
        self.inner
            .decode_batch_response(value)
            .map_err(|_| BatchError::Protocol("invalid Mistral completion"))
    }
    fn normalize(&self, mut job: Value) -> Result<Value, BatchError> {
        job["status"] = match job["status"].as_str() {
            Some("QUEUED" | "RUNNING") => "in_progress",
            Some("CANCELLATION_REQUESTED") => "cancelling",
            Some("SUCCESS") => "completed",
            Some("FAILED") => "failed",
            Some("TIMEOUT_EXCEEDED") => "expired",
            Some("CANCELLED") => "cancelled",
            _ => return Err(BatchError::Protocol("unknown Mistral job status")),
        }
        .into();
        let count = |key: &str| {
            job[key]
                .as_u64()
                .or_else(|| job[key].as_str()?.parse::<u64>().ok())
        };
        let total = count("total_requests").ok_or(BatchError::Protocol("missing total"))?;
        let success =
            count("succeeded_requests").ok_or(BatchError::Protocol("missing success count"))?;
        let failed =
            count("failed_requests").ok_or(BatchError::Protocol("missing failure count"))?;
        if success.checked_add(failed) != count("completed_requests") {
            return Err(BatchError::Protocol("Mistral counts mismatch"));
        }
        job["request_counts"] = json!({"total":total,"completed":success,"failed":failed});
        job["output_file_id"] = job["output_file"].clone();
        job["error_file_id"] = job["error_file"].clone();
        Ok(job)
    }
    async fn create(
        &self,
        http: Http<'_>,
        requests: Vec<BatchRequest>,
    ) -> Result<String, BatchError> {
        validate(&requests, Self::LIMIT)?;
        let model = &requests[0].config.model;
        if requests.iter().any(|r| &r.config.model != model) {
            return Err(BatchError::InvalidRequest(
                "Mistral requires one model per batch",
            ));
        }
        let mut body = BoundedBody {
            bytes: Vec::new(),
            max_bytes: 20_000_000,
        };
        #[derive(serde::Serialize)]
        struct Row<'a> {
            custom_id: &'a str,
            body: Value,
        }
        // Serialize incrementally; bound before building an oversized requests array.
        body.write_all(b"{\"endpoint\":\"/v1/chat/completions\",\"timeout_hours\":24,\"model\":")
            .map_err(|_| BatchError::InvalidRequest("batch too large"))?;
        serde_json::to_writer(&mut body, model)
            .map_err(|_| BatchError::InvalidRequest("batch too large"))?;
        body.write_all(b",\"requests\":[")
            .map_err(|_| BatchError::InvalidRequest("batch too large"))?;
        for (index, item) in requests.iter().enumerate() {
            if index > 0 {
                body.write_all(b",")
                    .map_err(|_| BatchError::InvalidRequest("batch too large"))?;
            }
            serde_json::to_writer(
                &mut body,
                &Row {
                    custom_id: &item.id,
                    body: self.encode(item)?,
                },
            )
            .map_err(|_| BatchError::InvalidRequest("inline batch exceeds 20 MB"))?;
        }
        body.write_all(b"]}")
            .map_err(|_| BatchError::InvalidRequest("inline batch exceeds 20 MB"))?;
        let job = self
            .normalize(
                http.json(Method::POST, Self::JOBS, body.bytes, true)
                    .await?,
            )
            .map_err(|_| BatchError::SubmissionUncertain)?;
        status::<Self>(&job, None, Self::URL).map_err(|_| BatchError::SubmissionUncertain)?;
        job["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or(BatchError::SubmissionUncertain)
    }
}
#[cfg(feature = "mistral")]
batch_driver!(
    crate::mistral::MistralChatDriver,
    "mistral-chat-completions-v1"
);
