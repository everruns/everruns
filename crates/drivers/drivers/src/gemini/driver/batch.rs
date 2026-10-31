//! Gemini Developer API inline jobs, using the REST discovery wire schema.
use super::*;
use crate::native_batch::{BoundedBody, Http, valid_segment, validate};
use everruns_contracts::{
    ProviderEndpoint,
    batch::*,
    turn_collector::{TurnLimits, collect_turn},
};
use reqwest::Method;
use std::{io::Write, time::Duration};

const MAX_BYTES: usize = 20_000_000;

#[derive(Serialize)]
struct Metadata {
    key: String,
}
#[derive(Serialize)]
struct InlineRequest {
    request: GeminiRequest,
    metadata: Metadata,
}

fn job_path(id: &str) -> std::result::Result<&str, BatchError> {
    id.strip_prefix("batches/")
        .filter(|id| valid_segment(id))
        .ok_or(BatchError::InvalidTicket)?;
    Ok(id)
}
fn status(value: &Value, expected: Option<&str>) -> std::result::Result<BatchStatus, BatchError> {
    let id = value["name"]
        .as_str()
        .ok_or(BatchError::Protocol("missing job name"))?;
    job_path(id).map_err(|_| BatchError::Protocol("invalid job name"))?;
    if expected.is_some_and(|expected| expected != id) {
        return Err(BatchError::Protocol("job name mismatch"));
    }
    match value.get("done") {
        Some(Value::Bool(true)) => Ok(BatchStatus::Ended),
        None | Some(Value::Bool(false)) => Ok(BatchStatus::Processing),
        _ => Err(BatchError::Protocol("invalid operation status")),
    }
}
fn item_error(error: &Value) -> BatchItemError {
    match error["code"].as_i64() {
        Some(1) => BatchItemError::Cancelled,
        Some(3 | 5 | 9 | 11) => BatchItemError::InvalidRequest,
        Some(4) => BatchItemError::Expired,
        Some(7) => BatchItemError::Permission,
        Some(8) => BatchItemError::RateLimited,
        Some(13 | 14) => BatchItemError::Server,
        Some(16) => BatchItemError::Authentication,
        _ => BatchItemError::Other,
    }
}
async fn decode(
    value: Value,
    model: &str,
) -> std::result::Result<everruns_contracts::LlmResponse, BatchError> {
    if value["candidates"].as_array().is_none_or(|c| c.is_empty()) {
        return Err(BatchError::Protocol("missing Gemini candidates"));
    }
    let response = serde_json::from_value(value)
        .map_err(|_| BatchError::Protocol("invalid Gemini response"))?;
    let mut state = GeminiStreamState {
        model: model.to_owned(),
        ..Default::default()
    };
    state.response(response);
    state.finish();
    let stream = Box::pin(futures::stream::iter(state.pending.into_iter().map(Ok)));
    collect_turn(stream, &TurnLimits::default(), |_| {})
        .await
        .map(|turn| turn.into_response())
        .map_err(|_| BatchError::Protocol("invalid Gemini completion"))
}
#[async_trait]
impl BatchDriver for GeminiChatDriver {
    fn batch_format(&self) -> &'static str {
        "gemini-inline-v1"
    }
    async fn submit(
        &self,
        endpoint: &ProviderEndpoint,
        requests: Vec<BatchRequest>,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<String, BatchError> {
        validate(&requests, 100_000)?;
        let model = requests[0]
            .config
            .model
            .strip_prefix("models/")
            .unwrap_or(&requests[0].config.model)
            .to_owned();
        if !valid_segment(&model) {
            return Err(BatchError::InvalidRequest("invalid Gemini model ID"));
        }
        let mut body = BoundedBody {
            bytes: Vec::new(),
            max_bytes: MAX_BYTES,
        };
        body.write_all(b"{\"batch\":{\"displayName\":\"everruns-batch\",\"model\":")
            .map_err(|_| BatchError::InvalidRequest("batch exceeds 20 MB"))?;
        serde_json::to_writer(&mut body, &format!("models/{model}"))
            .map_err(|_| BatchError::InvalidRequest("batch encoding failed"))?;
        body.write_all(b",\"inputConfig\":{\"requests\":{\"requests\":[")
            .map_err(|_| BatchError::InvalidRequest("batch exceeds 20 MB"))?;
        for (index, item) in requests.into_iter().enumerate() {
            let mut config = item.config;
            if config
                .model
                .strip_prefix("models/")
                .unwrap_or(&config.model)
                != model
            {
                return Err(BatchError::InvalidRequest(
                    "Gemini requires one model per batch",
                ));
            }
            if config.response_format.is_some()
                || !config.metadata.is_empty()
                || config.speed.is_some()
                || config.verbosity.is_some()
            {
                return Err(BatchError::InvalidRequest(
                    "Gemini batch does not support this completion option",
                ));
            }
            config.model = model.clone();
            // Batch only produces data. A function named web_search remains a
            // function; it must not silently become a provider-executed search.
            let (request, _) = self
                .prepare_request(&item.messages, &config, false)
                .map_err(|_| BatchError::InvalidRequest("Gemini request encoding failed"))?;
            if index > 0 {
                body.write_all(b",")
                    .map_err(|_| BatchError::InvalidRequest("batch exceeds 20 MB"))?;
            }
            serde_json::to_writer(
                &mut body,
                &InlineRequest {
                    request,
                    metadata: Metadata { key: item.id },
                },
            )
            .map_err(|_| {
                BatchError::InvalidRequest(
                    "batch exceeds 20 MB or cannot be serialized; split it explicitly",
                )
            })?;
        }
        body.write_all(b"]}}}}")
            .map_err(|_| BatchError::InvalidRequest("batch exceeds 20 MB"))?;
        let value = Http {
            endpoint,
            timeout,
            max_bytes,
        }
        .json(
            Method::POST,
            &format!("models/{model}:batchGenerateContent"),
            body.bytes,
            true,
        )
        .await?;
        status(&value, None).map_err(|_| BatchError::SubmissionUncertain)?;
        value["name"]
            .as_str()
            .map(str::to_owned)
            .ok_or(BatchError::SubmissionUncertain)
    }
    async fn poll(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<BatchStatus, BatchError> {
        let value = Http {
            endpoint,
            timeout,
            max_bytes,
        }
        .json(Method::GET, job_path(job_id)?, vec![], false)
        .await?;
        status(&value, Some(job_id))
    }
    async fn collect(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<Vec<BatchResult>, BatchError> {
        let value = Http {
            endpoint,
            timeout,
            max_bytes,
        }
        .json(Method::GET, job_path(job_id)?, vec![], false)
        .await?;
        if status(&value, Some(job_id))? != BatchStatus::Ended {
            return Err(BatchError::NotReady);
        }
        let rows = value
            .pointer("/response/inlinedResponses/inlinedResponses")
            .or_else(|| value.pointer("/metadata/output/inlinedResponses/inlinedResponses"))
            .and_then(Value::as_array);
        let Some(rows) = rows else {
            if value.get("error").is_some() {
                return Err(BatchError::JobFailed {
                    reason: item_error(&value["error"]),
                });
            }
            return Err(BatchError::Protocol("missing inline Gemini results"));
        };
        if rows.len() > 100_000 {
            return Err(BatchError::Protocol("too many results"));
        }
        let model = value
            .pointer("/metadata/model")
            .and_then(Value::as_str)
            .unwrap_or("gemini")
            .trim_start_matches("models/");
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row
                .pointer("/metadata/key")
                .and_then(Value::as_str)
                .ok_or(BatchError::Protocol("missing result ID"))?
                .to_owned();
            let result = match (row.get("response"), row.get("error")) {
                (Some(response), None) => {
                    if response
                        .get("promptFeedback")
                        .and_then(|v| v.get("blockReason"))
                        .is_some()
                    {
                        Err(BatchItemError::Other)
                    } else {
                        Ok(decode(response.clone(), model).await?)
                    }
                }
                (None, Some(error)) => Err(item_error(error)),
                _ => return Err(BatchError::Protocol("invalid result envelope")),
            };
            results.push(BatchResult { id, result });
        }
        Ok(results)
    }
    async fn cancel(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<BatchStatus, BatchError> {
        if self.poll(endpoint, job_id, timeout, max_bytes).await? == BatchStatus::Ended {
            return Ok(BatchStatus::Ended);
        }
        Http {
            endpoint,
            timeout,
            max_bytes,
        }
        .request(
            Method::POST,
            &format!("{}:cancel", job_path(job_id)?),
            b"{}".to_vec(),
            "application/json",
            false,
        )
        .await?;
        Ok(BatchStatus::Cancelling)
    }
}
