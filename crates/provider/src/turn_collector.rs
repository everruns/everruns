//! Collecting a provider stream into one finished turn.
//!
//! Every consumer that drives [`chat_completion_stream`] and wants a whole
//! answer out of it writes the same loop: fold text deltas, keep the reasoning
//! items but not the reasoning deltas that duplicate them, take the last tool
//! calls, read the terminal metadata, and decide what a stream that stops early
//! means. Getting that wrong is quiet — a truncated stream still yields
//! plausible text, just without usage or a finish reason — so the loop belongs
//! in one tested place rather than in each embedder.
//!
//! [`collect_turn`] is that loop, plus the two things a caller cannot add from
//! outside it: per-call [`TurnLimits`] and [`TurnTiming`] measured on the
//! arrival of actual events. The driver trait's non-streaming default path runs
//! through it, so an agent turn and a direct call enforce the same limits.
//!
//! ```no_run
//! # use everruns_provider::turn_collector::{collect_turn, TurnLimits};
//! # use everruns_provider::driver_registry::{LlmResponseStream, LlmStreamEvent};
//! # async fn run(stream: LlmResponseStream) -> everruns_provider::error::Result<()> {
//! let limits = TurnLimits::default()
//!     .with_total(std::time::Duration::from_secs(120))
//!     .with_max_response_bytes(4 * 1024 * 1024);
//! let turn = collect_turn(stream, &limits, |event| {
//!     if let LlmStreamEvent::TextDelta(delta) = event {
//!         print!("{delta}");
//!     }
//! })
//! .await?;
//! println!("{} tokens", turn.metadata.total_tokens.unwrap_or(0));
//! # Ok(())
//! # }
//! ```
//!
//! [`chat_completion_stream`]: crate::driver_registry::LlmDriver::chat_completion_stream

use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::time::Instant as TokioInstant;

use crate::driver_registry::{
    LlmCompletionMetadata, LlmResponse, LlmResponseStream, LlmStreamEvent,
};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::execution_phase::ExecutionPhase;
use crate::reasoning::ReasoningContentPart;
use crate::tool_types::ToolCall;

/// Bounds applied while a turn is being collected.
///
/// Every bound is off by default: a driver that sets none behaves exactly as
/// an unbounded loop would. Each one is enforced by [`collect_turn`] and by
/// the driver trait's non-streaming default, so setting them on
/// [`LlmCallConfig`](crate::driver_registry::LlmCallConfig) covers both paths.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TurnLimits {
    /// Wall-clock bound on the whole turn, from the first poll to the terminal
    /// event.
    pub total: Option<Duration>,
    /// Wall-clock bound on the wait for the *first* event.
    ///
    /// Separate from [`total`](Self::total) because the two failures differ:
    /// a provider that never starts answering is usually unreachable, while
    /// one that answers slowly may still be worth waiting for.
    pub first_event: Option<Duration>,
    /// Cap on accumulated answer text, in bytes (assistant text plus readable
    /// reasoning).
    ///
    /// Checked as the text accumulates, so an endless stream is cut off rather
    /// than buffered to exhaustion.
    pub max_response_bytes: Option<u64>,
    /// Whether a stream that ends without a terminal `Done` event is an error.
    ///
    /// Off by default, which keeps a truncated stream's partial text. Callers
    /// that need usage and a finish reason to be real — billing, evaluation,
    /// anything that records the turn — turn this on rather than reading a cut
    /// short answer as a complete one. Either way
    /// [`CollectedTurn::complete`] reports what happened.
    pub require_terminal_event: bool,
}

impl TurnLimits {
    /// Bound the whole turn.
    #[must_use]
    pub fn with_total(mut self, total: Duration) -> Self {
        self.total = Some(total);
        self
    }

    /// Bound the wait for the first event.
    #[must_use]
    pub fn with_first_event(mut self, first_event: Duration) -> Self {
        self.first_event = Some(first_event);
        self
    }

    /// Cap the accumulated answer text.
    #[must_use]
    pub fn with_max_response_bytes(mut self, bytes: u64) -> Self {
        self.max_response_bytes = Some(bytes);
        self
    }

    /// Refuse a stream that ends without its terminal event.
    #[must_use]
    pub fn requiring_terminal_event(mut self) -> Self {
        self.require_terminal_event = true;
        self
    }

    /// Whether any bound is set.
    pub fn is_unbounded(&self) -> bool {
        *self == TurnLimits::default()
    }
}

/// What a turn cost in wall-clock time, measured on event arrival.
///
/// Measured by whoever collected the stream, so it includes the queueing and
/// scheduling the caller actually experienced — not the provider's own
/// self-reported latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TurnTiming {
    /// Time until the first event arrived. `None` when none did.
    pub time_to_first_event: Option<Duration>,
    /// Mean gap between consecutive content events.
    ///
    /// `None` below two content events, where there is no gap to average.
    pub mean_inter_event: Option<Duration>,
    /// Time from the first poll to the end of the stream.
    pub total: Duration,
}

/// One turn, folded out of a provider stream.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CollectedTurn {
    /// Assistant text, in arrival order.
    pub text: String,
    /// Reasoning artifacts, in emission order.
    ///
    /// Folded from `ReasoningItem` events only: the `ReasoningDelta` events
    /// are live progress and repeat the same text, so accumulating both would
    /// double it.
    pub reasoning: Vec<ReasoningContentPart>,
    /// Tool calls the provider asked for.
    pub tool_calls: Vec<ToolCall>,
    /// Provider-native execution phase, when the stream carried one ahead of
    /// its terminal event.
    pub phase: Option<ExecutionPhase>,
    /// Terminal metadata: usage, finish reason, serving model.
    ///
    /// Default-valued when the stream ended without a terminal event; see
    /// [`complete`](Self::complete).
    pub metadata: LlmCompletionMetadata,
    /// Whether the stream ended with its terminal `Done` event.
    ///
    /// `false` means the turn was cut short: `metadata` is a default, not the
    /// provider's answer, and the text may be missing its tail.
    pub complete: bool,
    /// What the turn cost in wall-clock time.
    pub timing: TurnTiming,
}

impl CollectedTurn {
    /// Readable reasoning text, joined in emission order.
    pub fn reasoning_text(&self) -> String {
        self.reasoning
            .iter()
            .filter_map(|part| part.display_text())
            .collect::<Vec<_>>()
            .join("")
    }

    /// The turn as the driver-facing [`LlmResponse`], dropping the timing and
    /// completeness this type adds.
    pub fn into_response(self) -> LlmResponse {
        LlmResponse {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls: (!self.tool_calls.is_empty()).then_some(self.tool_calls),
            metadata: self.metadata,
        }
    }
}

/// Fold a provider stream into one finished turn, observing every event as it
/// arrives.
///
/// `observe` sees each event before it is folded, which is how a caller renders
/// deltas live; a caller with no use for them passes `|_| {}`. The limits are
/// enforced as the turn accumulates, so an over-long or over-large answer fails
/// while it is still arriving.
///
/// # Errors
///
/// - [`LlmErrorKind::Unavailable`] when a limit in [`TurnLimits`] on *time*
///   elapses: a turn that never finished may finish on a retry.
/// - [`LlmErrorKind::MalformedResponse`] when the byte cap is exceeded, or
///   when the stream ends without its terminal event and
///   [`TurnLimits::require_terminal_event`] is set.
/// - Whatever the stream itself yielded, unchanged, for a provider failure.
pub async fn collect_turn(
    mut stream: LlmResponseStream,
    limits: &TurnLimits,
    mut observe: impl FnMut(&LlmStreamEvent),
) -> Result<CollectedTurn> {
    let started = Instant::now();
    let total_deadline = limits.total.map(|total| TokioInstant::now() + total);
    let first_event_deadline = limits
        .first_event
        .map(|first_event| TokioInstant::now() + first_event);

    let mut turn = CollectedTurn::default();
    let mut accumulated: u64 = 0;
    let mut first_event_at: Option<Instant> = None;
    let mut last_event_at: Option<Instant> = None;
    let mut content_events: u32 = 0;

    loop {
        // Before the first event both deadlines apply; after it, only the
        // whole-turn one does.
        let deadline = match (total_deadline, first_event_deadline) {
            (Some(total), Some(first)) if first_event_at.is_none() => Some(total.min(first)),
            (total, first) => {
                if first_event_at.is_none() {
                    total.or(first)
                } else {
                    total
                }
            }
        };
        let next = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, stream.next()).await {
                Ok(next) => next,
                Err(_) => return Err(timed_out(limits, first_event_at.is_none(), started)),
            },
            None => stream.next().await,
        };
        let Some(event) = next else { break };
        let event = event?;
        let now = Instant::now();
        if first_event_at.is_none() {
            first_event_at = Some(now);
        }

        observe(&event);

        match event {
            LlmStreamEvent::TextDelta(delta) => {
                if delta.is_empty() {
                    continue;
                }
                accumulated += delta.len() as u64;
                check_cap(accumulated, limits)?;
                turn.text.push_str(&delta);
                content_events += 1;
                last_event_at = Some(now);
            }
            // Deltas are live progress; the terminal `ReasoningItem` below is
            // the durable artifact, and folding both would double the text.
            LlmStreamEvent::ReasoningDelta { delta, .. } => {
                if delta.is_empty() {
                    continue;
                }
                content_events += 1;
                last_event_at = Some(now);
            }
            LlmStreamEvent::ReasoningItem(item) => {
                if let Some(text) = item.display_text() {
                    accumulated += text.len() as u64;
                    check_cap(accumulated, limits)?;
                }
                turn.reasoning.push(item);
            }
            LlmStreamEvent::ToolCalls(calls) => turn.tool_calls = calls,
            LlmStreamEvent::NativeToolCall(_) => {
                return Err(AgentLoopError::config(
                    "native async/custom calls require a streaming coordinator",
                ));
            }
            LlmStreamEvent::MessagePhase(phase) => turn.phase = Some(phase),
            LlmStreamEvent::ProviderCompactionStarted => {}
            LlmStreamEvent::Done(metadata) => {
                turn.metadata = *metadata;
                turn.complete = true;
            }
            LlmStreamEvent::Error(error) => return Err(error.into_agent_error()),
        }
    }

    if !turn.complete && limits.require_terminal_event {
        return Err(AgentLoopError::llm_kind(
            LlmErrorKind::MalformedResponse,
            "provider stream ended before its terminal event: the turn is incomplete",
        ));
    }

    turn.timing = TurnTiming {
        time_to_first_event: first_event_at.map(|at| at.duration_since(started)),
        mean_inter_event: match (first_event_at, last_event_at) {
            (Some(first), Some(last)) if content_events >= 2 => {
                Some(last.duration_since(first) / (content_events - 1))
            }
            _ => None,
        },
        total: started.elapsed(),
    };
    Ok(turn)
}

/// Apply [`TurnLimits`] to a stream a caller wants to consume event by event.
///
/// The streaming counterpart to [`collect_turn`], for callers that render
/// events themselves and so never hand the stream over. The time bounds are
/// enforced per poll and the byte cap against the text seen so far; the first
/// breach is yielded as an error item and ends the stream, because a bounded
/// call that has already overrun has nothing more to offer.
///
/// An unbounded [`TurnLimits`] returns the stream untouched, so this is safe to
/// apply unconditionally.
pub fn limit_stream(stream: LlmResponseStream, limits: TurnLimits) -> LlmResponseStream {
    if limits.is_unbounded() {
        return stream;
    }
    struct State {
        stream: LlmResponseStream,
        limits: TurnLimits,
        started: Instant,
        total_deadline: Option<TokioInstant>,
        first_event_deadline: Option<TokioInstant>,
        seen_event: bool,
        accumulated: u64,
        done: bool,
    }
    let state = State {
        stream,
        limits,
        started: Instant::now(),
        total_deadline: limits.total.map(|total| TokioInstant::now() + total),
        first_event_deadline: limits.first_event.map(|first| TokioInstant::now() + first),
        seen_event: false,
        accumulated: 0,
        done: false,
    };
    Box::pin(futures::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        let deadline = match (state.total_deadline, state.first_event_deadline) {
            (Some(total), Some(first)) if !state.seen_event => Some(total.min(first)),
            (total, first) if !state.seen_event => total.or(first),
            (total, _) => total,
        };
        let next = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, state.stream.next()).await {
                Ok(next) => next,
                Err(_) => {
                    let error = timed_out(&state.limits, !state.seen_event, state.started);
                    state.done = true;
                    return Some((Err(error), state));
                }
            },
            None => state.stream.next().await,
        };
        let item = next?;
        state.seen_event = true;
        if let Ok(event) = &item {
            let produced = match event {
                LlmStreamEvent::TextDelta(delta) => delta.len() as u64,
                LlmStreamEvent::ReasoningItem(item) => {
                    item.display_text().map_or(0, |text| text.len() as u64)
                }
                _ => 0,
            };
            state.accumulated += produced;
            if let Err(error) = check_cap(state.accumulated, &state.limits) {
                state.done = true;
                return Some((Err(error), state));
            }
        }
        Some((item, state))
    }))
}

fn check_cap(accumulated: u64, limits: &TurnLimits) -> Result<()> {
    match limits.max_response_bytes {
        Some(cap) if accumulated > cap => Err(AgentLoopError::llm_kind(
            LlmErrorKind::MalformedResponse,
            format!("provider response exceeded the {cap}-byte limit for this call"),
        )),
        _ => Ok(()),
    }
}

/// A time limit elapsed. Classified [`Unavailable`](LlmErrorKind::Unavailable)
/// because a turn that ran out of time is transient by nature: the same call
/// may well complete on a retry.
fn timed_out(limits: &TurnLimits, before_first_event: bool, started: Instant) -> AgentLoopError {
    let elapsed = started.elapsed();
    if before_first_event && limits.first_event.is_some_and(|first| elapsed >= first) {
        first_event_timeout(limits.first_event.unwrap_or(elapsed))
    } else {
        turn_timeout(limits.total.unwrap_or(elapsed))
    }
}

/// The failure for a turn that ran out of its whole-turn budget.
///
/// Public so the request phase — establishing the stream, before any event
/// exists to collect — can fail the same way the fold does. A caller that hit
/// its own limit should not be able to tell which half of the call was slow
/// from the error's shape.
pub fn turn_timeout(after: Duration) -> AgentLoopError {
    AgentLoopError::llm_kind(
        LlmErrorKind::Unavailable,
        format!("provider turn did not finish within {after:?}"),
    )
}

/// The failure for a provider that never started answering.
pub fn first_event_timeout(after: Duration) -> AgentLoopError {
    AgentLoopError::llm_kind(
        LlmErrorKind::Unavailable,
        format!("provider sent no response within {after:?}"),
    )
}

/// Run the request phase under the call's time limits.
///
/// [`collect_turn`] can only start its clock once a stream exists, but
/// establishing that stream is itself a provider round trip that can hang —
/// and a provider that never sends its response headers is exactly the case
/// the limits are for. Drivers and provider wrappers put the connect phase
/// through this so the budget covers the whole call, not just its tail.
///
/// The tighter of the two time limits applies: whichever of the whole-turn and
/// first-event budgets would elapse first. Returns the elapsed time alongside
/// the value so the caller can charge it against the remaining budget.
pub async fn connect_within<T, F>(
    limits: &TurnLimits,
    future: F,
) -> Result<(T, std::time::Duration)>
where
    F: std::future::Future<Output = Result<T>>,
{
    let budget = match (limits.total, limits.first_event) {
        (Some(total), Some(first)) => Some(total.min(first)),
        (total, first) => total.or(first),
    };
    let started = Instant::now();
    let value = match budget {
        Some(budget) => match tokio::time::timeout(budget, future).await {
            Ok(value) => value?,
            Err(_) => {
                return Err(if limits.first_event == Some(budget) {
                    first_event_timeout(budget)
                } else {
                    turn_timeout(budget)
                });
            }
        },
        None => future.await?,
    };
    Ok((value, started.elapsed()))
}

impl TurnLimits {
    /// The same limits with `spent` already charged against the time budgets.
    ///
    /// Used after [`connect_within`] so the fold inherits what is left rather
    /// than restarting the clock and letting a call take twice its budget.
    #[must_use]
    pub fn after(mut self, spent: std::time::Duration) -> Self {
        // A budget already spent becomes zero, not a wrap-around to a huge
        // one: an overrun must fail on the next poll, not disable the limit.
        self.total = self.total.map(|total| total.saturating_sub(spent));
        self.first_event = self.first_event.map(|first| first.saturating_sub(spent));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::LlmStreamError;
    use crate::reasoning::{ReasoningContentPart, ReasoningText};
    use futures::stream;
    use serde_json::json;

    fn reasoning(text: &str) -> ReasoningContentPart {
        ReasoningContentPart::opaque("test").with_text(ReasoningText::Plain { text: text.into() })
    }

    fn done(finish: &str) -> LlmStreamEvent {
        LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
            finish_reason: Some(finish.to_owned()),
            total_tokens: Some(7),
            ..Default::default()
        }))
    }

    fn streamed(events: Vec<LlmStreamEvent>) -> LlmResponseStream {
        Box::pin(stream::iter(events.into_iter().map(Ok)))
    }

    #[tokio::test]
    async fn folds_text_reasoning_and_tool_calls_into_one_turn() {
        let events = vec![
            LlmStreamEvent::TextDelta("Hel".into()),
            LlmStreamEvent::TextDelta(String::new()),
            LlmStreamEvent::TextDelta("lo".into()),
            LlmStreamEvent::ReasoningDelta {
                delta: "thin".into(),
                summary: false,
            },
            LlmStreamEvent::ReasoningItem(reasoning("thinking")),
            LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: "call_1".into(),
                name: "search".into(),
                arguments: json!({"q": "everruns"}),
            }]),
            done("tool_calls"),
        ];
        let mut seen = 0;
        let turn = collect_turn(streamed(events), &TurnLimits::default(), |_| seen += 1)
            .await
            .unwrap();

        assert_eq!(turn.text, "Hello");
        // The delta repeated the item's text; only the item is folded.
        assert_eq!(turn.reasoning_text(), "thinking");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.metadata.finish_reason.as_deref(), Some("tool_calls"));
        assert!(turn.complete);
        assert_eq!(
            seen, 7,
            "every event reaches the observer, empties included"
        );
    }

    #[tokio::test]
    async fn a_cut_short_stream_is_reported_and_only_refused_on_request() {
        let events = || vec![LlmStreamEvent::TextDelta("partial".into())];

        let lenient = collect_turn(streamed(events()), &TurnLimits::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(lenient.text, "partial");
        assert!(!lenient.complete, "the missing terminal event is visible");

        let strict = collect_turn(
            streamed(events()),
            &TurnLimits::default().requiring_terminal_event(),
            |_| {},
        )
        .await
        .expect_err("a required terminal event that never arrived is an error");
        assert_eq!(
            strict.llm_error_kind(),
            Some(LlmErrorKind::MalformedResponse)
        );
    }

    #[tokio::test]
    async fn the_byte_cap_trips_while_the_answer_is_still_arriving() {
        let events = vec![
            LlmStreamEvent::TextDelta("a".repeat(8)),
            LlmStreamEvent::TextDelta("b".repeat(8)),
            done("stop"),
        ];
        let error = collect_turn(
            streamed(events),
            &TurnLimits::default().with_max_response_bytes(10),
            |_| {},
        )
        .await
        .expect_err("an over-cap answer must not pass for a turn");
        assert_eq!(
            error.llm_error_kind(),
            Some(LlmErrorKind::MalformedResponse)
        );
        assert!(error.to_string().contains("10-byte limit"), "{error}");
    }

    #[tokio::test]
    async fn reasoning_text_counts_against_the_same_cap_as_answer_text() {
        let events = vec![
            LlmStreamEvent::TextDelta("hi".into()),
            LlmStreamEvent::ReasoningItem(reasoning(&"r".repeat(20))),
            done("stop"),
        ];
        let error = collect_turn(
            streamed(events),
            &TurnLimits::default().with_max_response_bytes(10),
            |_| {},
        )
        .await
        .expect_err("reasoning is answer text too");
        assert_eq!(
            error.llm_error_kind(),
            Some(LlmErrorKind::MalformedResponse)
        );
    }

    #[tokio::test]
    async fn a_stream_error_keeps_its_kind_and_status() {
        let events = vec![Err(AgentLoopError::llm("upstream reset"))];
        let stream: LlmResponseStream = Box::pin(stream::iter(events));
        let error = collect_turn(stream, &TurnLimits::default(), |_| {})
            .await
            .expect_err("a stream failure propagates");
        assert_eq!(error.to_string(), "LLM error: upstream reset");

        let mut inline = LlmStreamError::new("rate limited");
        inline.status = Some(429);
        let stream = streamed(vec![LlmStreamEvent::Error(inline)]);
        let error = collect_turn(stream, &TurnLimits::default(), |_| {})
            .await
            .expect_err("an inline error event fails the turn");
        assert_eq!(error.http_status(), Some(429));
        assert!(error.is_rate_limited());
    }

    #[tokio::test]
    async fn a_silent_provider_trips_the_first_event_limit() {
        let stream: LlmResponseStream = Box::pin(stream::once(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(LlmStreamEvent::TextDelta("late".into()))
        }));
        let limits = TurnLimits::default().with_first_event(Duration::from_millis(20));
        let error = collect_turn(stream, &limits, |_| {})
            .await
            .expect_err("a provider that never answers must not hang the caller");
        assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Unavailable));
        assert!(error.to_string().contains("no response within"), "{error}");
        // Transient: the same call may well succeed on a retry.
        assert!(error.is_transient_llm_error());
    }

    #[tokio::test]
    async fn a_slow_turn_trips_the_total_limit_after_it_has_started() {
        let stream: LlmResponseStream = Box::pin(
            stream::once(async { Ok(LlmStreamEvent::TextDelta("start".into())) }).chain(
                stream::once(async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(done("stop"))
                }),
            ),
        );
        let limits = TurnLimits::default()
            .with_first_event(Duration::from_secs(30))
            .with_total(Duration::from_millis(20));
        let error = collect_turn(stream, &limits, |_| {})
            .await
            .expect_err("a turn that never finishes must not hang the caller");
        assert!(
            error.to_string().contains("did not finish within"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn timing_reports_first_event_and_the_mean_gap_between_content_events() {
        let stream: LlmResponseStream = Box::pin(
            stream::once(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(LlmStreamEvent::TextDelta("a".into()))
            })
            .chain(stream::once(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(LlmStreamEvent::TextDelta("b".into()))
            }))
            .chain(stream::once(async { Ok(done("stop")) })),
        );
        let turn = collect_turn(stream, &TurnLimits::default(), |_| {})
            .await
            .unwrap();
        let ttfe = turn.timing.time_to_first_event.expect("an event arrived");
        assert!(ttfe >= Duration::from_millis(20), "{ttfe:?}");
        // Two content events, so exactly one gap to average.
        let mean = turn.timing.mean_inter_event.expect("two content events");
        assert!(mean >= Duration::from_millis(20), "{mean:?}");
        assert!(turn.timing.total >= ttfe);
    }

    #[tokio::test]
    async fn a_single_content_event_has_no_gap_to_average() {
        let turn = collect_turn(
            streamed(vec![LlmStreamEvent::TextDelta("one".into()), done("stop")]),
            &TurnLimits::default(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(turn.timing.mean_inter_event, None);
    }

    #[tokio::test]
    async fn limit_stream_ends_the_stream_at_the_first_breach() {
        let stream = streamed(vec![
            LlmStreamEvent::TextDelta("a".repeat(8)),
            LlmStreamEvent::TextDelta("b".repeat(8)),
            done("stop"),
        ]);
        let limited = limit_stream(stream, TurnLimits::default().with_max_response_bytes(10));
        let items: Vec<_> = limited.collect().await;
        assert_eq!(items.len(), 2, "one good event, then the breach");
        assert!(items[0].is_ok());
        let error = items[1].as_ref().expect_err("the cap trips");
        assert_eq!(
            error.llm_error_kind(),
            Some(LlmErrorKind::MalformedResponse)
        );
    }

    #[tokio::test]
    async fn limit_stream_bounds_a_provider_that_stops_sending() {
        let stream: LlmResponseStream = Box::pin(
            stream::once(async { Ok(LlmStreamEvent::TextDelta("start".into())) }).chain(
                stream::once(async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(done("stop"))
                }),
            ),
        );
        let limited = limit_stream(
            stream,
            TurnLimits::default().with_total(Duration::from_millis(20)),
        );
        let items: Vec<_> = limited.collect().await;
        assert_eq!(items.len(), 2);
        let error = items[1].as_ref().expect_err("the turn ran out of time");
        assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Unavailable));
    }

    #[tokio::test]
    async fn limit_stream_passes_an_unbounded_stream_straight_through() {
        let stream = streamed(vec![LlmStreamEvent::TextDelta("hi".into()), done("stop")]);
        let items: Vec<_> = limit_stream(stream, TurnLimits::default()).collect().await;
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(Result::is_ok));
    }

    #[test]
    fn default_limits_are_unbounded() {
        assert!(TurnLimits::default().is_unbounded());
        assert!(
            !TurnLimits::default()
                .requiring_terminal_event()
                .is_unbounded()
        );
    }
}
