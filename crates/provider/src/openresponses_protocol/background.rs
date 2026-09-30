// OpenAI Responses background mode (EVE-1116).
//
// A long `xhigh`/`max` reasoning call can run for many minutes. In the default
// foreground mode the response lives only as long as its HTTP connection: a
// dropped connection loses the work already paid for, and the retry bills the
// whole call again. With `background: true` OpenAI keeps generating
// server-side and the stream can be re-attached from any event:
// `GET /v1/responses/{id}?stream=true&starting_after={sequence_number}`.
//
// This wrapper sits between the SSE transport and the event parser. It watches
// the response id (`response.created`) and each event's `sequence_number`, and
// when the connection fails or closes before a terminal event it resumes from
// the last event it forwarded, so the parser never sees a gap or a duplicate.
//
// Background responses outlive their connection, so abandoning one would keep
// generating and billing. Dropping the stream before a terminal event (turn
// cancellation, a stall timeout, a worker shutdown) therefore sends
// `POST /v1/responses/{id}/cancel`, which is idempotent.
//
// Policy: on for `xhigh`/`max` effort, where a call is long enough for a lost
// connection to matter; the `openai/background` driver option forces it on or
// off. Background mode requires `store: true`, so it is not zero-data-retention
// compatible: a 400 naming the background fields retries once in the
// foreground, and ZDR deployments can set the option to `false` to skip that
// round trip. Only the OpenAI driver on OpenAI and Azure hosts enables it;
// OpenRouter and custom gateways do not implement the resume API.
//
// Scope: resume covers one process. Re-attaching after a worker restart needs
// the response id persisted outside the process and is not done here.

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::{StreamExt, stream};
use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::driver_registry::LlmCallConfig;
use crate::model::ReasoningEffort;
use crate::runtime_provider::ProviderEndpoint;
use crate::stream_reconnect::{SseItem, SseStream, is_reconnectable_stream_error};

/// Driver option forcing background mode on (`true`) or off (`false`).
pub const OPENAI_BACKGROUND_OPTION: &str = "openai/background";

/// How many times one call re-attaches before surfacing the transport error.
const MAX_RESUMES: u32 = 5;
const RESUME_BACKOFF: Duration = Duration::from_millis(500);

/// Whether this call runs in background mode.
pub(super) fn wants_background(config: &LlmCallConfig) -> bool {
    match config
        .driver_options
        .get(OPENAI_BACKGROUND_OPTION)
        .and_then(Value::as_bool)
    {
        Some(forced) => forced,
        None => matches!(
            config.reasoning_effort,
            Some(ReasoningEffort::Xhigh | ReasoningEffort::Max)
        ),
    }
}

/// Set or clear the background fields on a serialized request.
pub(super) fn mark(body: &mut Value, background: bool) {
    let Some(body) = body.as_object_mut() else {
        return;
    };
    if background {
        body.insert("background".into(), true.into());
        body.insert("store".into(), true.into());
    } else {
        body.remove("background");
        body.remove("store");
    }
}

/// A 400 that names the background fields: the organization cannot store
/// responses (zero data retention) or the deployment lacks background mode.
/// The call is retried once in the foreground rather than failing the turn.
pub(super) fn is_rejection(error: &crate::error::AgentLoopError) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("400") && (text.contains("background") || text.contains("store"))
}

/// What resuming and cancelling a background response needs, owned so the
/// stream stays `'static`.
#[derive(Clone)]
pub(super) struct BackgroundResponses {
    pub(super) client: reqwest::Client,
    pub(super) endpoint: ProviderEndpoint,
    /// The `.../responses` URL the request was posted to.
    pub(super) api_url: String,
    pub(super) headers: HeaderMap,
}

impl BackgroundResponses {
    async fn send(&self, method: &str, url: String) -> Result<reqwest::Response, String> {
        let resolved = self
            .endpoint
            .resolve(method, url, b"")
            .await
            .map_err(|e| e.to_string())?;
        let mut headers = self.headers.clone();
        for (name, value) in resolved.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| e.to_string())?;
            let mut value =
                reqwest::header::HeaderValue::from_str(&value).map_err(|e| e.to_string())?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let request = match method {
            "POST" => self.client.post(&resolved.url),
            _ => self.client.get(&resolved.url),
        };
        let response = request
            .headers(headers)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {body}"));
        }
        Ok(response)
    }

    async fn resume(&self, id: &str, after: Option<u64>) -> Result<SseStream, String> {
        let mut url = format!("{}/{id}?stream=true", self.api_url);
        if let Some(after) = after {
            url.push_str(&format!("&starting_after={after}"));
        }
        let response = self.send("GET", url).await?;
        Ok(Box::pin(response.bytes_stream().eventsource()))
    }
}

/// Cancels the response when dropped before it reached a terminal state.
struct CancelOnDrop {
    responses: BackgroundResponses,
    id: Option<String>,
    finished: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let (Some(id), false) = (self.id.take(), self.finished) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let responses = self.responses.clone();
        runtime.spawn(async move {
            let url = format!("{}/{id}/cancel", responses.api_url);
            match responses.send("POST", url).await {
                Ok(_) => tracing::info!(response_id = %id, "cancelled abandoned background response"),
                Err(error) => {
                    tracing::warn!(response_id = %id, %error, "failed to cancel background response")
                }
            }
        });
    }
}

struct State {
    current: SseStream,
    guard: CancelOnDrop,
    last_sequence: Option<u64>,
    resumes: u32,
}

fn observe(state: &mut State, data: &str) {
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return;
    };
    if let Some(sequence) = event.get("sequence_number").and_then(Value::as_u64) {
        state.last_sequence = Some(sequence);
    }
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if state.guard.id.is_none()
        && let Some(id) = event.pointer("/response/id").and_then(Value::as_str)
    {
        state.guard.id = Some(id.to_string());
    }
    if matches!(
        kind,
        "response.completed" | "response.failed" | "response.incomplete" | "error"
    ) {
        state.guard.finished = true;
    }
}

/// Wrap a background response's SSE stream so a dropped connection resumes
/// where it left off and an abandoned response is cancelled.
pub(super) fn resumable(first: SseStream, responses: BackgroundResponses) -> SseStream {
    let state = State {
        current: first,
        guard: CancelOnDrop {
            responses,
            id: None,
            finished: false,
        },
        last_sequence: None,
        resumes: 0,
    };
    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            let item: Option<SseItem> = state.current.next().await;
            let failure = match item {
                Some(Ok(event)) => {
                    observe(&mut state, &event.data);
                    return Some((Ok(event), state));
                }
                Some(Err(error)) if !is_reconnectable_stream_error(&error) => {
                    return Some((Err(error), state));
                }
                Some(Err(error)) => Some(error),
                None => None,
            };
            let resumable_id = state.guard.id.clone().filter(|_| !state.guard.finished);
            let Some(id) = resumable_id.filter(|_| state.resumes < MAX_RESUMES) else {
                // Finished, never started, or out of attempts: end as the
                // transport did.
                return failure.map(|error| (Err(error), state));
            };
            state.resumes += 1;
            tracing::warn!(
                response_id = %id,
                starting_after = ?state.last_sequence,
                attempt = state.resumes,
                "background response stream dropped; resuming"
            );
            tokio::time::sleep(RESUME_BACKOFF * state.resumes).await;
            match state.guard.responses.resume(&id, state.last_sequence).await {
                Ok(stream) => state.current = stream,
                Err(error) => {
                    tracing::warn!(response_id = %id, %error, "background resume failed");
                    if let Some(error) = failure {
                        return Some((Err(error), state));
                    }
                    // A clean close with no way back: let the parser report
                    // the missing completion.
                    return None;
                }
            }
        }
    }))
}
