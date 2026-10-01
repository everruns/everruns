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
// Across a restart (EVE-1134): when the host supplies a durable journal
// (`BackgroundCallContext`), the response id is saved as soon as
// `response.created` arrives, before any event reaches the parser. The durable
// retry of the same call (equal request fingerprint) re-attaches with
// `GET /responses/{id}?stream=true` from the first event, because its parser
// starts empty, instead of posting the call again. With a journal, dropping
// the stream no longer cancels: the drop may be a worker shutdown or a stall
// whose retry is about to re-attach. An explicit turn cancel, delivered on the
// context's signal, cancels the response even while this process is still
// reading it. The record is cleared at the terminal event, and a record that
// does not match the call (a different request) is cancelled, not resumed.
//
// Remaining window: a worker that dies after OpenAI accepted the POST but
// before `response.created` was saved is still retried by a second POST.

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::{StreamExt, stream};
use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::background_call::{BackgroundCallContext, BackgroundResponseRecord};
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
    client: reqwest::Client,
    endpoint: ProviderEndpoint,
    /// The `.../responses` URL the request was posted to.
    api_url: String,
    headers: HeaderMap,
    /// Host-supplied journal and turn-cancel signal (EVE-1134).
    call: BackgroundCallContext,
    /// Identity of this call for re-attachment after a restart.
    fingerprint: String,
}

impl BackgroundResponses {
    pub(super) fn new(
        client: reqwest::Client,
        endpoint: &ProviderEndpoint,
        api_url: &str,
        extension_headers: &HeaderMap,
        config: &LlmCallConfig,
        request_body: &Value,
    ) -> Self {
        let mut headers = extension_headers.clone();
        for (name, value) in
            crate::driver_helpers::merge_request_headers(Vec::new(), &config.extra_headers)
        {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(&value),
            ) {
                headers.insert(name, value);
            }
        }
        Self {
            client,
            endpoint: endpoint.clone(),
            api_url: api_url.to_string(),
            headers,
            call: config.background_call.clone(),
            fingerprint: crate::background_call::request_fingerprint(request_body),
        }
    }

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

    async fn cancel(&self, id: &str) {
        let url = format!("{}/{id}/cancel", self.api_url);
        match self.send("POST", url).await {
            Ok(_) => tracing::info!(response_id = %id, "cancelled background response"),
            Err(error) => {
                tracing::warn!(response_id = %id, %error, "failed to cancel background response")
            }
        }
    }

    /// Re-attach to the response an earlier attempt of this call left at the
    /// provider, replaying it from the first event. `None` means post the call.
    pub(super) async fn reattach(&self) -> Option<(SseStream, String)> {
        let journal = self.call.journal()?;
        let record = match journal.load().await {
            Ok(record) => record?,
            Err(error) => {
                tracing::warn!(%error, "background journal unavailable; posting the call");
                return None;
            }
        };
        if record.request_fingerprint != self.fingerprint {
            // A different request owns the record: nothing will read that
            // response again, so stop it before posting this one.
            tracing::warn!(response_id = %record.response_id, "cancelling a stale background response");
            self.cancel(&record.response_id).await;
            self.clear().await;
            return None;
        }
        match self.resume(&record.response_id, None).await {
            Ok(stream) => {
                tracing::info!(response_id = %record.response_id, "re-attached to a background response after a restart");
                Some((stream, record.response_id))
            }
            Err(error) => {
                tracing::warn!(response_id = %record.response_id, %error, "background response cannot be re-attached; posting the call");
                self.clear().await;
                None
            }
        }
    }

    /// Persist the id; `true` when a later attempt can now re-attach.
    async fn record(&self, id: &str) -> bool {
        let Some(journal) = self.call.journal() else {
            return false;
        };
        let record = BackgroundResponseRecord {
            response_id: id.to_string(),
            request_fingerprint: self.fingerprint.clone(),
        };
        match journal.store(Some(&record)).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(response_id = %id, %error, "failed to persist background response id; a restart posts the call again");
                false
            }
        }
    }

    async fn clear(&self) {
        if let Some(journal) = self.call.journal()
            && let Err(error) = journal.store(None).await
        {
            tracing::warn!(%error, "failed to clear background response record");
        }
    }
}

/// Cancels the response when dropped before it reached a terminal state,
/// unless a durable record lets a later attempt re-attach to it.
struct CancelOnDrop {
    responses: BackgroundResponses,
    id: Option<String>,
    finished: bool,
    durable: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let (Some(id), false, false) = (self.id.take(), self.finished, self.durable) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let responses = self.responses.clone();
        runtime.spawn(async move { responses.cancel(&id).await });
    }
}

struct State {
    current: SseStream,
    guard: CancelOnDrop,
    last_sequence: Option<u64>,
    resumes: u32,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
}

/// What one event changed.
#[derive(Default)]
struct Observed {
    started: bool,
    finished: bool,
}

fn observe(state: &mut State, data: &str) -> Observed {
    let mut observed = Observed::default();
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return observed;
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
        observed.started = true;
    }
    if matches!(
        kind,
        "response.completed" | "response.failed" | "response.incomplete" | "error"
    ) && !state.guard.finished
    {
        state.guard.finished = true;
        observed.finished = true;
    }
    observed
}

/// Resolves once the turn is explicitly cancelled; never when the signal's
/// sender goes away without cancelling.
async fn turn_cancelled(signal: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *signal.borrow_and_update() {
            return;
        }
        if signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

enum Next {
    Item(Option<SseItem>),
    Cancelled,
}

async fn next(state: &mut State) -> Next {
    let State {
        current, cancel, ..
    } = state;
    let Some(signal) = cancel.as_mut() else {
        return Next::Item(current.next().await);
    };
    // The cancel future is polled first, so a pending cancel wins over an
    // event that is already buffered.
    let cancelled = std::pin::pin!(turn_cancelled(signal));
    match futures::future::select(cancelled, current.next()).await {
        futures::future::Either::Left(_) => Next::Cancelled,
        futures::future::Either::Right((item, _)) => Next::Item(item),
    }
}

/// Wrap a background response's SSE stream so a dropped connection resumes
/// where it left off, an explicit turn cancel stops the response, and an
/// abandoned one without a durable record is cancelled. `attached` is the id
/// of a response re-attached from the journal, whose record already exists.
pub(super) fn resumable(
    first: SseStream,
    responses: BackgroundResponses,
    attached: Option<String>,
) -> SseStream {
    let state = State {
        current: first,
        cancel: responses.call.cancel_signal(),
        guard: CancelOnDrop {
            responses,
            durable: attached.is_some(),
            id: attached,
            finished: false,
        },
        last_sequence: None,
        resumes: 0,
    };
    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            let item = match next(&mut state).await {
                Next::Item(item) => item,
                Next::Cancelled => {
                    if let (Some(id), false) = (state.guard.id.clone(), state.guard.finished) {
                        tracing::info!(response_id = %id, "turn cancelled; cancelling background response");
                        state.guard.responses.cancel(&id).await;
                    }
                    state.guard.finished = true;
                    if state.guard.durable {
                        state.guard.responses.clear().await;
                    }
                    return None;
                }
            };
            let failure = match item {
                Some(Ok(event)) => {
                    let observed = observe(&mut state, &event.data);
                    if observed.started
                        && let Some(id) = state.guard.id.clone()
                    {
                        // Saved before the parser sees anything, so a restart
                        // from here on re-attaches instead of posting again.
                        state.guard.durable = state.guard.responses.record(&id).await;
                    }
                    if observed.finished && state.guard.durable {
                        state.guard.responses.clear().await;
                    }
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
