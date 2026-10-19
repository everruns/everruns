// Turn latency breakdown — one structured log line and a few histograms per turn.
//
// Why: a chat turn spends time in four places — the queue (message to
// `turn.started`, and every hand-off between durable phases), context
// assembly before each LLM call, the LLM itself, and tools. Per-event spans
// existed (OTel, when enabled), but nothing said how much of a slow turn was
// the platform and how much the model. Production runs with OTel off and ships
// only logs, so the summary is a log line first: `turn latency` with numeric
// fields, which the SaaS log pipeline lifts into queryable columns.
//
// Decision: computed on the server from the event stream every worker already
// sends, so it needs no worker change and covers every runtime. Timestamps are
// the events' own (`Event.ts`), written by the process that emitted them; the
// server and workers share a host clock in the hosted deployment.
//
// Decision: the input time comes from the turn's `input_message_id` when it is
// a UUIDv7 (the chat API mints those), read from the id's own timestamp. The
// `input.message` event is the fallback, not the source: the HTTP message path
// emits it through an `EventService` built without listeners, so the listener
// never sees it there. The id travels on every turn event, so this also works
// when the input and the turn land on different server replicas.
//
// Decision: in-memory per-session state, bounded. A turn whose events land on
// two server replicas yields a partial summary on each; the fields that need a
// missing event are omitted rather than guessed.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{
    ACT_COMPLETED, ACT_STARTED, Event, EventData, EventListener, INPUT_MESSAGE, LLM_GENERATION,
    REASON_COMPLETED, REASON_STARTED, TOOL_COMPLETED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED,
    TURN_STARTED,
};

use crate::metrics_names as names;

/// Sessions tracked at once. Past this, entries idle for `STALE_AFTER` are
/// dropped, then everything if that is not enough: losing a summary is fine,
/// unbounded growth is not.
const MAX_TRACKED_SESSIONS: usize = 10_000;
const STALE_AFTER: chrono::Duration = chrono::Duration::minutes(30);

/// What the listener needs from one event, independent of `Event`'s shape.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Observation<'a> {
    pub event_type: &'a str,
    pub ts: DateTime<Utc>,
    /// `llm.generation`: generation duration and time to first token.
    pub llm_duration_ms: Option<u64>,
    pub ttft_ms: Option<u64>,
    /// `tool.completed`: tool duration.
    pub tool_duration_ms: Option<u64>,
    /// When the turn's input message was created, from its UUIDv7 id.
    pub input_created_at: Option<DateTime<Utc>>,
}

impl<'a> Observation<'a> {
    fn from_event(event: &'a Event) -> Self {
        let (llm_duration_ms, ttft_ms, tool_duration_ms) = match &event.data {
            EventData::LlmGeneration(data) => (
                data.metadata.duration_ms,
                data.metadata.time_to_first_token_ms,
                None,
            ),
            EventData::ToolCompleted(data) => (None, None, data.duration_ms),
            _ => (None, None, None),
        };
        Self {
            event_type: &event.event_type,
            ts: event.ts,
            llm_duration_ms,
            ttft_ms,
            tool_duration_ms,
            input_created_at: event
                .context
                .input_message_id
                .and_then(|id| uuid_v7_time(id.uuid())),
        }
    }
}

/// Creation time embedded in a UUIDv7; `None` for any other version.
fn uuid_v7_time(id: uuid::Uuid) -> Option<DateTime<Utc>> {
    if id.get_version_num() != 7 {
        return None;
    }
    let (secs, nanos) = id.get_timestamp()?.to_unix();
    DateTime::from_timestamp(secs as i64, nanos)
}

/// Where one finished turn spent its time. Every `*_ms` is wall-clock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TurnLatency {
    pub outcome: &'static str,
    /// Input message persisted to `turn.started`: enqueue plus worker pickup.
    pub pickup_ms: Option<u64>,
    /// Input message to the first streamed token of the first LLM call: what
    /// the user waits before anything appears.
    pub first_token_ms: Option<u64>,
    /// `turn.started` to the terminal turn event.
    pub turn_ms: u64,
    /// Input message to the terminal turn event.
    pub total_ms: Option<u64>,
    pub llm_ms: u64,
    pub tool_ms: u64,
    /// Sum of hand-offs between durable phases (`turn.started` or a phase's
    /// end, to the next phase's start): queue wait inside the turn.
    pub phase_gap_ms: u64,
    pub max_phase_gap_ms: u64,
    /// Sum, over reason phases, of `reason.started` to the LLM request start:
    /// context assembly before the model is called.
    pub prep_ms: u64,
    /// `turn_ms` not spent in the LLM or tools.
    pub overhead_ms: u64,
    pub phases: u32,
    pub llm_calls: u32,
    pub tool_calls: u32,
}

#[derive(Debug, Default)]
struct TurnClock {
    input_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    last_phase_end: Option<DateTime<Utc>>,
    reason_started_at: Option<DateTime<Utc>>,
    first_token_at: Option<DateTime<Utc>>,
    touched: Option<DateTime<Utc>>,
    llm_ms: u64,
    tool_ms: u64,
    phase_gap_ms: u64,
    max_phase_gap_ms: u64,
    prep_ms: u64,
    phases: u32,
    llm_calls: u32,
    tool_calls: u32,
}

fn ms_between(from: DateTime<Utc>, to: DateTime<Utc>) -> u64 {
    (to - from).num_milliseconds().max(0) as u64
}

/// One phase hand-off, reported as it happens so the histogram sees each gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhaseGap {
    pub phase: &'static str,
    pub ms: u64,
}

impl TurnClock {
    /// Feed one event. Returns the phase gap it closed, if any.
    fn observe(&mut self, obs: &Observation<'_>) -> Option<PhaseGap> {
        self.touched = Some(obs.ts);
        match obs.event_type {
            INPUT_MESSAGE => {
                // A message steering a running turn does not restart its clock.
                if self.started_at.is_none() {
                    self.input_at = Some(obs.ts);
                }
                None
            }
            TURN_STARTED => {
                let input_at = obs.input_created_at.or(self.input_at);
                *self = Self {
                    input_at,
                    started_at: Some(obs.ts),
                    last_phase_end: Some(obs.ts),
                    touched: Some(obs.ts),
                    ..Self::default()
                };
                None
            }
            REASON_STARTED | ACT_STARTED => {
                let phase = if obs.event_type == REASON_STARTED {
                    self.reason_started_at = Some(obs.ts);
                    "reason"
                } else {
                    "act"
                };
                self.phases += 1;
                let gap = self.last_phase_end.take().map(|end| PhaseGap {
                    phase,
                    ms: ms_between(end, obs.ts),
                });
                if let Some(gap) = gap {
                    self.phase_gap_ms += gap.ms;
                    self.max_phase_gap_ms = self.max_phase_gap_ms.max(gap.ms);
                }
                gap
            }
            REASON_COMPLETED | ACT_COMPLETED => {
                self.last_phase_end = Some(obs.ts);
                None
            }
            LLM_GENERATION => {
                self.llm_calls += 1;
                let duration = obs.llm_duration_ms.unwrap_or(0);
                self.llm_ms += duration;
                // The event is written when the generation ends.
                let request_start = obs.ts - chrono::Duration::milliseconds(duration as i64);
                if let Some(reason_start) = self.reason_started_at.take() {
                    self.prep_ms += ms_between(reason_start, request_start);
                }
                if self.first_token_at.is_none()
                    && let Some(ttft) = obs.ttft_ms
                {
                    self.first_token_at =
                        Some(request_start + chrono::Duration::milliseconds(ttft as i64));
                }
                None
            }
            TOOL_COMPLETED => {
                self.tool_calls += 1;
                self.tool_ms += obs.tool_duration_ms.unwrap_or(0);
                None
            }
            _ => None,
        }
    }

    fn finish(&self, outcome: &'static str, ended_at: DateTime<Utc>) -> Option<TurnLatency> {
        let started_at = self.started_at?;
        let turn_ms = ms_between(started_at, ended_at);
        Some(TurnLatency {
            outcome,
            pickup_ms: self.input_at.map(|input| ms_between(input, started_at)),
            first_token_ms: self
                .input_at
                .zip(self.first_token_at)
                .map(|(input, token)| ms_between(input, token)),
            turn_ms,
            total_ms: self.input_at.map(|input| ms_between(input, ended_at)),
            llm_ms: self.llm_ms,
            tool_ms: self.tool_ms,
            phase_gap_ms: self.phase_gap_ms,
            max_phase_gap_ms: self.max_phase_gap_ms,
            prep_ms: self.prep_ms,
            // Parallel tools can sum past wall-clock; clamp rather than wrap.
            overhead_ms: turn_ms.saturating_sub(self.llm_ms + self.tool_ms),
            phases: self.phases,
            llm_calls: self.llm_calls,
            tool_calls: self.tool_calls,
        })
    }
}

/// Outcome name for a terminal turn event, or `None` if the event is not one.
fn terminal_outcome(event_type: &str) -> Option<&'static str> {
    match event_type {
        TURN_COMPLETED => Some("completed"),
        TURN_FAILED => Some("failed"),
        TURN_CANCELLED => Some("cancelled"),
        _ => None,
    }
}

/// Per-session turn clocks fed from the event stream.
#[derive(Default)]
pub(crate) struct TurnLatencyTracker {
    sessions: HashMap<SessionId, TurnClock>,
}

impl TurnLatencyTracker {
    /// Feed one event. Returns the phase gap it closed and, on a terminal turn
    /// event, the finished turn's breakdown.
    pub(crate) fn observe(
        &mut self,
        session_id: SessionId,
        obs: &Observation<'_>,
    ) -> (Option<PhaseGap>, Option<TurnLatency>) {
        if let Some(outcome) = terminal_outcome(obs.event_type) {
            let latency = self
                .sessions
                .remove(&session_id)
                .and_then(|clock| clock.finish(outcome, obs.ts));
            return (None, latency);
        }
        if !self.sessions.contains_key(&session_id) {
            self.evict(obs.ts);
        }
        let gap = self.sessions.entry(session_id).or_default().observe(obs);
        (gap, None)
    }

    fn evict(&mut self, now: DateTime<Utc>) {
        if self.sessions.len() < MAX_TRACKED_SESSIONS {
            return;
        }
        self.sessions
            .retain(|_, clock| clock.touched.is_some_and(|t| now - t < STALE_AFTER));
        if self.sessions.len() >= MAX_TRACKED_SESSIONS {
            self.sessions.clear();
        }
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.sessions.len()
    }
}

/// Logs `turn latency` and records the turn histograms for every finished turn.
#[derive(Default)]
pub struct TurnLatencyListener {
    tracker: Mutex<TurnLatencyTracker>,
}

impl TurnLatencyListener {
    pub fn new() -> Self {
        Self::default()
    }
}

fn secs(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

#[async_trait]
impl EventListener for TurnLatencyListener {
    async fn on_event(&self, event: &Event) {
        let obs = Observation::from_event(event);
        let (gap, latency) = match self.tracker.lock() {
            Ok(mut tracker) => tracker.observe(event.session_id, &obs),
            Err(_) => return,
        };

        if let Some(gap) = gap {
            metrics::histogram!(names::TURN_PHASE_GAP_DURATION, "phase" => gap.phase)
                .record(secs(gap.ms));
        }
        let Some(l) = latency else {
            return;
        };
        let outcome = l.outcome;
        if let Some(pickup) = l.pickup_ms {
            metrics::histogram!(names::TURN_PICKUP_DURATION).record(secs(pickup));
        }
        if let Some(first_token) = l.first_token_ms {
            metrics::histogram!(names::TURN_FIRST_TOKEN_DURATION).record(secs(first_token));
        }
        metrics::histogram!(names::TURN_OVERHEAD_DURATION, "outcome" => outcome)
            .record(secs(l.overhead_ms));

        // Field names are a contract with the SaaS log pipeline
        // (`infra/vector.yaml` in everruns/saas): rename them together.
        tracing::info!(
            target: "everruns_server::turn_latency",
            session_id = %event.session_id,
            turn_id = event.context.turn_id.as_ref().map(tracing::field::display),
            outcome,
            pickup_ms = l.pickup_ms,
            first_token_ms = l.first_token_ms,
            turn_ms = l.turn_ms,
            total_ms = l.total_ms,
            llm_ms = l.llm_ms,
            tool_ms = l.tool_ms,
            phase_gap_ms = l.phase_gap_ms,
            max_phase_gap_ms = l.max_phase_gap_ms,
            prep_ms = l.prep_ms,
            overhead_ms = l.overhead_ms,
            phases = l.phases,
            llm_calls = l.llm_calls,
            tool_calls = l.tool_calls,
            "turn latency"
        );
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            INPUT_MESSAGE,
            TURN_STARTED,
            REASON_STARTED,
            REASON_COMPLETED,
            ACT_STARTED,
            ACT_COMPLETED,
            LLM_GENERATION,
            TOOL_COMPLETED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
        ])
    }

    fn name(&self) -> &'static str {
        "TurnLatencyListener"
    }
}

#[cfg(test)]
mod tests;
