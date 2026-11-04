//! Message Batches reuse the Messages request encoder, with streaming disabled.
//! Creation has no idempotency guarantee: never replay an ambiguous submission.

use std::{collections::HashSet, io::Write, time::Duration};

use async_trait::async_trait;
use everruns_contracts::{ProviderEndpoint, batch::*, driver_registry::LlmResponse};
use reqwest::Method;

use super::*;

const MAX_REQUESTS: usize = 100_000;
const MAX_REQUEST_BYTES: usize = 256 * 1024 * 1024;

// Bound serialization while writing, before a large body can be allocated.
struct BoundedBody {
    bytes: Vec<u8>,
    max_bytes: usize,
}
impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max_bytes.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("batch exceeds request byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[derive(Serialize)]
struct WireRequest {
    custom_id: String,
    params: AnthropicRequest,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn path(job_id: &str, suffix: &str) -> std::result::Result<String, BatchError> {
    if !valid_id(job_id) {
        return Err(BatchError::InvalidTicket);
    }
    Ok(format!("messages/batches/{job_id}{suffix}"))
}

fn status(
    value: &Value,
    expected_id: Option<&str>,
) -> std::result::Result<BatchStatus, BatchError> {
    let id = value["id"]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or(BatchError::Protocol("invalid job ID"))?;
    if expected_id.is_some_and(|expected| expected != id) {
        return Err(BatchError::Protocol("job ID mismatch"));
    }
    match value["processing_status"].as_str() {
        Some("in_progress") => Ok(BatchStatus::Processing),
        Some("canceling") => Ok(BatchStatus::Cancelling),
        Some("ended") => Ok(BatchStatus::Ended),
        _ => Err(BatchError::Protocol("unknown job status")),
    }
}

#[derive(Clone, Copy)]
struct HttpLimits {
    timeout: Duration,
    max_bytes: usize,
}

impl AnthropicChatDriver {
    async fn batch_http(
        &self,
        endpoint: &ProviderEndpoint,
        method: Method,
        path: &str,
        body: Vec<u8>,
        betas: &str,
        limits: HttpLimits,
    ) -> std::result::Result<Vec<u8>, BatchError> {
        let HttpLimits { timeout, max_bytes } = limits;
        let creating = method == Method::POST && path == "messages/batches";
        tokio::time::timeout(timeout, async {
            let url = endpoint
                .url(path)
                .ok_or(BatchError::InvalidRequest("provider has no base URL"))?;
            let resolved = endpoint
                .resolve(method.as_str(), url, &body)
                .await
                .map_err(|_| BatchError::InvalidRequest("provider authentication failed"))?;
            // THREAT[TM-LLM-056]: Do not follow redirects or provider-supplied result
            // URLs with credentials, and never expose provider error bodies.
            let client = driver_helpers::shared_request_http_client();
            let mut request = client
                .request(method, resolved.url)
                .timeout(timeout)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json");
            if !betas.is_empty() {
                request = request.header("anthropic-beta", betas);
            }
            for (name, value) in resolved.headers {
                request = request.header(name, value);
            }
            let mut response = request.body(body).send().await.map_err(|_| {
                if creating {
                    BatchError::SubmissionUncertain
                } else {
                    BatchError::Transport
                }
            })?;
            let code = response.status().as_u16();
            if !(200..300).contains(&code) {
                return Err(if creating && (code == 408 || code >= 500) {
                    BatchError::SubmissionUncertain
                } else {
                    BatchError::Http { status: code }
                });
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                if creating {
                    BatchError::SubmissionUncertain
                } else {
                    BatchError::Transport
                }
            })? {
                if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
                    return Err(if creating {
                        BatchError::SubmissionUncertain
                    } else {
                        BatchError::ResponseTooLarge
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| {
            if creating {
                BatchError::SubmissionUncertain
            } else {
                BatchError::Transport
            }
        })?
    }
}

#[async_trait]
impl BatchDriver for AnthropicChatDriver {
    fn batch_format(&self) -> &'static str {
        "anthropic-messages-v1"
    }

    async fn submit(
        &self,
        endpoint: &ProviderEndpoint,
        requests: Vec<BatchRequest>,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<String, BatchError> {
        if requests.is_empty() || requests.len() > MAX_REQUESTS {
            return Err(BatchError::InvalidRequest(
                "batch must contain 1 to 100000 requests",
            ));
        }
        let mut seen = HashSet::new();
        let mut betas = std::collections::BTreeSet::new();
        let mut body = BoundedBody {
            bytes: b"{\"requests\":[".to_vec(),
            max_bytes: MAX_REQUEST_BYTES - 2,
        };
        for (index, item) in requests.into_iter().enumerate() {
            if !valid_id(&item.id) || !seen.insert(item.id.clone()) {
                return Err(BatchError::InvalidRequest(
                    "invalid or duplicate request ID",
                ));
            }
            let config = &item.config;
            if item.messages.is_empty() || config.max_tokens == Some(0) {
                return Err(BatchError::InvalidRequest(
                    "messages and a positive token budget are required",
                ));
            }
            if !config.limits.is_unbounded()
                || config.capture_request
                || !config.extra_headers.is_empty()
                || !config.driver_options.is_empty()
                || config.provider_opaque_context.is_some()
            {
                return Err(BatchError::InvalidRequest(
                    "batch does not support streaming limits, capture, per-item headers or provider execution options",
                ));
            }
            if config.response_format.is_some() && !self.supports_response_format(&config.model) {
                return Err(BatchError::InvalidRequest(
                    "structured output is unsupported by this driver",
                ));
            }
            let mut prepared = self
                .prepare_request(endpoint, item.messages, config)
                .map_err(|_| BatchError::InvalidRequest("Messages request encoding failed"))?;
            prepared.request.stream = false;
            if prepared.needs_interleaved_thinking {
                betas.insert("interleaved-thinking-2025-05-14");
            }
            if prepared.wants_million_context {
                betas.insert("context-1m-2025-08-07");
            }
            if prepared.wants_cache_diagnostics {
                betas.insert(CACHE_DIAGNOSTICS_BETA);
            }
            if layout::binds_thinking_to_conversation(&config.model) {
                betas.insert(layout::THINKING_BINDING_BETA);
            }
            betas.extend(prepared.conversation_betas);
            if index > 0 {
                body.write_all(b",").map_err(|_| {
                    BatchError::InvalidRequest("batch exceeds 256 MiB; split it explicitly")
                })?;
            }
            serde_json::to_writer(
                &mut body,
                &WireRequest {
                    custom_id: item.id,
                    params: prepared.request,
                },
            )
            .map_err(|_| {
                BatchError::InvalidRequest(
                    "batch exceeds 256 MiB or cannot be serialized; split it explicitly",
                )
            })?;
        }
        body.bytes.extend_from_slice(b"]}");
        let bytes = self
            .batch_http(
                endpoint,
                Method::POST,
                "messages/batches",
                body.bytes,
                &betas.into_iter().collect::<Vec<_>>().join(","),
                HttpLimits { timeout, max_bytes },
            )
            .await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| BatchError::SubmissionUncertain)?;
        status(&value, None).map_err(|_| BatchError::SubmissionUncertain)?;
        value["id"]
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
        let bytes = self
            .batch_http(
                endpoint,
                Method::GET,
                &path(job_id, "")?,
                Vec::new(),
                "",
                HttpLimits { timeout, max_bytes },
            )
            .await?;
        let value =
            serde_json::from_slice(&bytes).map_err(|_| BatchError::Protocol("invalid job JSON"))?;
        status(&value, Some(job_id))
    }

    async fn collect(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> std::result::Result<Vec<BatchResult>, BatchError> {
        let bytes = self
            .batch_http(
                endpoint,
                Method::GET,
                &path(job_id, "/results")?,
                Vec::new(),
                "",
                HttpLimits { timeout, max_bytes },
            )
            .await?;
        let mut results = Vec::new();
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        {
            if results.len() == MAX_REQUESTS {
                return Err(BatchError::Protocol("too many results"));
            }
            let row: Value = serde_json::from_slice(line)
                .map_err(|_| BatchError::Protocol("invalid results JSONL"))?;
            let id = row["custom_id"]
                .as_str()
                .filter(|id| valid_id(id))
                .ok_or(BatchError::Protocol("invalid result ID"))?
                .to_owned();
            let result = match row["result"]["type"].as_str() {
                Some("succeeded") => Ok(decode_message(&row["result"]["message"])?),
                Some("canceled") => Err(BatchItemError::Cancelled),
                Some("expired") => Err(BatchItemError::Expired),
                Some("errored") => Err(match row["result"]["error"]["error"]["type"].as_str() {
                    Some("invalid_request_error") => BatchItemError::InvalidRequest,
                    Some("authentication_error") => BatchItemError::Authentication,
                    Some("permission_error") => BatchItemError::Permission,
                    Some("rate_limit_error") => BatchItemError::RateLimited,
                    Some("api_error" | "overloaded_error") => BatchItemError::Server,
                    _ => BatchItemError::Other,
                }),
                _ => return Err(BatchError::Protocol("unknown result type")),
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
        let bytes = self
            .batch_http(
                endpoint,
                Method::POST,
                &path(job_id, "/cancel")?,
                Vec::new(),
                "",
                HttpLimits { timeout, max_bytes },
            )
            .await?;
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| BatchError::Protocol("invalid cancellation JSON"))?;
        status(&value, Some(job_id))
    }
}

fn decode_message(message: &Value) -> std::result::Result<LlmResponse, BatchError> {
    let blocks = message["content"]
        .as_array()
        .ok_or(BatchError::Protocol("missing message content"))?;
    let mut text = String::new();
    let mut reasoning = Vec::new();
    let mut tools = Vec::new();
    for block in blocks {
        let parsed: AnthropicContentBlock = serde_json::from_value(block.clone())
            .map_err(|_| BatchError::Protocol("invalid content block"))?;
        if matches!(parsed, AnthropicContentBlock::Raw(_))
            && matches!(
                block["type"].as_str(),
                Some("text" | "thinking" | "redacted_thinking" | "tool_use")
            )
        {
            return Err(BatchError::Protocol("invalid known content block"));
        }
        match parsed {
            AnthropicContentBlock::Text { text: part, .. } => text.push_str(&part),
            AnthropicContentBlock::Thinking {
                thinking,
                signature,
            } => reasoning.push(
                ReasoningContentPart::opaque("anthropic")
                    .with_signature(signature)
                    .with_text(ReasoningText::Plain { text: thinking }),
            ),
            AnthropicContentBlock::RedactedThinking { data } => reasoning.push(
                ReasoningContentPart::opaque("anthropic")
                    .with_encrypted(data)
                    .with_text(ReasoningText::Redacted),
            ),
            AnthropicContentBlock::ToolUse { id, name, input } => {
                if id.is_empty() || name.is_empty() || !input.is_object() {
                    return Err(BatchError::Protocol("invalid function tool call"));
                }
                tools.push(ToolCall {
                    id,
                    name,
                    arguments: input,
                });
            }
            _ => {}
        }
    }
    let stop = message["stop_reason"]
        .as_str()
        .ok_or(BatchError::Protocol("missing stop reason"))?;
    if message["id"].as_str().is_none_or(str::is_empty)
        || message["model"].as_str().is_none_or(str::is_empty)
        || message["usage"]["input_tokens"].as_u64().is_none()
        || message["usage"]["output_tokens"].as_u64().is_none()
    {
        return Err(BatchError::Protocol("incomplete message identity or usage"));
    }
    let mut metadata = LlmCompletionMetadata::default();
    metadata.model = message["model"].as_str().map(str::to_owned);
    metadata.response_id = message["id"].as_str().map(str::to_owned);
    metadata.provider_finish_reason = Some(stop.to_owned());
    metadata.finish_reason = Some(
        match stop {
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            "refusal" => "refusal",
            _ => "stop",
        }
        .to_owned(),
    );
    let usage: AnthropicUsage = serde_json::from_value(message["usage"].clone())
        .map_err(|_| BatchError::Protocol("invalid message usage"))?;
    metadata.prompt_tokens = Some(usage.input_tokens);
    metadata.completion_tokens = Some(usage.output_tokens);
    metadata.cache_read_tokens = usage.cache_read_input_tokens;
    metadata.cache_creation_tokens = usage.cache_creation_input_tokens;
    metadata.cache_creation_1h_tokens = usage
        .cache_creation
        .and_then(|split| split.ephemeral_1h_input_tokens);
    metadata.total_tokens = Some(
        usage
            .input_tokens
            .saturating_add(usage.output_tokens)
            .saturating_add(metadata.cache_read_tokens.unwrap_or(0))
            .saturating_add(metadata.cache_creation_tokens.unwrap_or(0)),
    );
    if stop != "tool_use" {
        metadata.tool_calls_dropped = tools.len() as u32;
        tools.clear();
    }
    metadata.provider_opaque_content = Some(ProviderOpaqueContent::new(
        "anthropic",
        Value::Array(blocks.clone()),
    ));
    Ok(LlmResponse {
        text,
        reasoning,
        tool_calls: (!tools.is_empty()).then_some(tools),
        metadata,
    })
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
