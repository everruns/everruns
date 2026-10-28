//! OpenAI's Files + Batches lifecycle. The uploaded file and the job are two
//! separate external effects; neither ambiguous operation is replayed.
use super::*;
use crate::native_batch::{BoundedBody, Http, http_error, valid_segment, validate};
use everruns_contracts::batch::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::{collections::HashSet, io::Write, time::Duration};

const MAX_REQUESTS: usize = 50_000;
const MAX_BYTES: usize = 200_000_000;
const BOUNDARY: &str = "everruns_batch_upload";

#[async_trait]
trait Codec: Send + Sync {
    const URL: &'static str;
    fn encode(&self, item: &BatchRequest) -> std::result::Result<Value, BatchError>;
    async fn decode(&self, value: Value) -> std::result::Result<LlmResponse, BatchError>;
}
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
fn job_path(id: &str, suffix: &str) -> std::result::Result<String, BatchError> {
    if !id.starts_with("batch_") || !valid_segment(id) {
        return Err(BatchError::InvalidTicket);
    }
    Ok(format!("batches/{id}{suffix}"))
}
fn file_path(id: &str) -> std::result::Result<String, BatchError> {
    if !(id.starts_with("file-") || id.starts_with("file_")) || !valid_segment(id) {
        return Err(BatchError::Protocol("invalid file ID"));
    }
    Ok(format!("files/{id}/content"))
}
fn status(
    value: &Value,
    expected: Option<&str>,
    url: &str,
) -> std::result::Result<BatchStatus, BatchError> {
    let id = value["id"]
        .as_str()
        .ok_or(BatchError::Protocol("missing job ID"))?;
    job_path(id, "").map_err(|_| BatchError::Protocol("invalid job ID"))?;
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

async fn submit<C: Codec>(
    codec: &C,
    http: Http<'_>,
    requests: Vec<BatchRequest>,
) -> std::result::Result<String, BatchError> {
    validate(&requests, MAX_REQUESTS)?;
    let model = &requests[0].config.model;
    if requests.iter().any(|r| &r.config.model != model) {
        return Err(BatchError::InvalidRequest(
            "OpenAI requires one model per batch",
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
    file_path(file_id).map_err(|_| BatchError::SubmissionUncertain)?;
    // Do not delete the staging file on an uncertain create: it may already
    // belong to a live job. Applications reconcile provider artifacts explicitly.
    let body = serde_json::to_vec(
        &json!({"input_file_id":file_id,"endpoint":C::URL,"completion_window":"24h"}),
    )
    .map_err(|_| BatchError::InvalidRequest("batch encoding failed"))?;
    let job = http.json(Method::POST, "batches", body, true).await?;
    status(&job, None, C::URL).map_err(|_| BatchError::SubmissionUncertain)?;
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
    let job = http
        .json(Method::GET, &job_path(job_id, "")?, vec![], false)
        .await?;
    if status(&job, Some(job_id), C::URL)? != BatchStatus::Ended {
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
        let Some(id) = job.get(field).filter(|id| !id.is_null()) else {
            continue;
        };
        let path = file_path(id.as_str().ok_or(BatchError::Protocol("invalid file ID"))?)?;
        let bytes = http
            .request(Method::GET, &path, vec![], "application/json", false)
            .await?;
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
                submit(
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
                .json(Method::GET, &job_path(job_id, "")?, vec![], false)
                .await?;
                status(&job, Some(job_id), Self::URL)
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
                    &job_path(job_id, "/cancel")?,
                    b"{}".to_vec(),
                    false,
                )
                .await?;
                status(&job, Some(job_id), Self::URL)
            }
        }
    };
}
batch_driver!(OpenAIChatDriver, "openai-responses-batch-v1");
batch_driver!(
    OpenAICompletionsChatDriver,
    "openai-chat-completions-batch-v1"
);
