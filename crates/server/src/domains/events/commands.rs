use super::queries as q;
use crate::domains::common::*;
use crate::storage::models::ListEventsParams;
use chrono::{DateTime, Utc};
use everruns_core::{Event, VALID_EVENT_TYPES};
use everruns_provider::typed_id::EventId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_EVENT_TYPE_FILTER_SIZE: usize = 40;
const MAX_EVENT_LIMIT: i32 = 1000;
const MAX_TAGS_FILTER_SIZE: usize = 20;
const MAX_AROUND_WINDOW: i32 = 500;
const MAX_Q_BYTES: usize = 4096;
const MAX_Q_TOKENS: usize = 64;

#[derive(Debug, Serialize)]
pub struct ListEventsResult {
    pub data: Vec<Event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
}

fn validate_event_type_list(types: &[String], param_name: &str) -> Result<(), CommandError> {
    if types.len() > MAX_EVENT_TYPE_FILTER_SIZE {
        return Err(CommandError::bad_request(format!(
            "{param_name}: too many values ({}, max {MAX_EVENT_TYPE_FILTER_SIZE})",
            types.len()
        )));
    }
    for event_type in types {
        if !VALID_EVENT_TYPES.contains(&event_type.as_str()) {
            return Err(CommandError::bad_request(format!(
                "{param_name}: unknown event type '{event_type}'"
            )));
        }
    }
    Ok(())
}

fn validate_q_filter(q: Option<&str>) -> Result<(), CommandError> {
    let Some(q) = q else {
        return Ok(());
    };
    if q.len() > MAX_Q_BYTES {
        return Err(CommandError::bad_request(format!(
            "q: too long ({} bytes, max {MAX_Q_BYTES})",
            q.len()
        )));
    }
    let token_count = q.split_whitespace().count();
    if token_count > MAX_Q_TOKENS {
        return Err(CommandError::bad_request(format!(
            "q: too many tokens ({token_count}, max {MAX_Q_TOKENS})"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListEvents {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub since_id: Option<EventId>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Maximum number of items returned in this page.
    pub limit: Option<i32>,
    pub before_sequence: Option<i32>,

    // -------- Debugging extensions --------
    /// Forward cursor: only return events with sequence > after_sequence.
    /// Mutually exclusive with `before_sequence` and `around`.
    pub after_sequence: Option<i32>,
    /// Anchor event id: returns up to `window` events on each side
    /// (default 50, max 500). Mutually exclusive with `since_id`,
    /// `after_sequence`, and `before_sequence` — combining returns 400.
    pub around: Option<EventId>,
    /// Window size for `around` (events on each side).
    pub window: Option<i32>,
    /// `created_at >= from_ts` (RFC 3339).
    pub from_ts: Option<DateTime<Utc>>,
    /// `created_at <= to_ts` (RFC 3339).
    pub to_ts: Option<DateTime<Utc>>,
    /// Filter by `context.turn_id`.
    pub turn_id: Option<String>,
    /// Filter by `context.exec_id`.
    pub exec_id: Option<String>,
    /// Filter by `context.trace_id`.
    pub trace_id: Option<String>,
    /// Tag any-match against `events.tags`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Match `data.tool_name` (useful for `tool.*` events).
    pub tool_name: Option<String>,
    /// Full-text search via Postgres tsvector (substring fallback in-memory).
    pub q: Option<String>,
    /// When true, return newest first; default oldest first.
    #[serde(default)]
    pub order_desc: bool,
}

impl ListEvents {
    /// True when any debugging filter is set — routes to `list_events_advanced`.
    fn uses_advanced(&self) -> bool {
        self.after_sequence.is_some()
            || self.around.is_some()
            || self.window.is_some()
            || self.from_ts.is_some()
            || self.to_ts.is_some()
            || self.turn_id.is_some()
            || self.exec_id.is_some()
            || self.trace_id.is_some()
            || !self.tags.is_empty()
            || self.tool_name.is_some()
            || self.q.is_some()
            || self.order_desc
    }
}

impl Command for ListEvents {
    type Output = ListEventsResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_events",
            category: "events",
            description: "List events for a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/events",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListEventsResult, CommandError> {
        validate_event_type_list(&self.types, "types")?;
        validate_event_type_list(&self.exclude, "exclude")?;
        validate_q_filter(self.q.as_deref())?;

        if self.tags.len() > MAX_TAGS_FILTER_SIZE {
            return Err(CommandError::bad_request(format!(
                "tags: too many values ({}, max {MAX_TAGS_FILTER_SIZE})",
                self.tags.len()
            )));
        }
        if let Some(limit) = self.limit
            && !(1..=MAX_EVENT_LIMIT).contains(&limit)
        {
            return Err(CommandError::bad_request(format!(
                "limit must be between 1 and {MAX_EVENT_LIMIT}, got {limit}"
            )));
        }
        if let Some(window) = self.window
            && !(1..=MAX_AROUND_WINDOW).contains(&window)
        {
            return Err(CommandError::bad_request(format!(
                "window must be between 1 and {MAX_AROUND_WINDOW}, got {window}"
            )));
        }
        if self.window.is_some() && self.around.is_none() {
            return Err(CommandError::bad_request(
                "window requires `around` to be set",
            ));
        }
        if self.before_sequence.is_some() && self.limit.is_none() && !self.uses_advanced() {
            return Err(CommandError::bad_request(
                "before_sequence requires limit to be set",
            ));
        }
        if (self.after_sequence.is_some() || self.since_id.is_some())
            && self.before_sequence.is_some()
        {
            return Err(CommandError::bad_request(
                "forward and backward cursors are mutually exclusive",
            ));
        }
        if self.around.is_some()
            && (self.before_sequence.is_some()
                || self.after_sequence.is_some()
                || self.since_id.is_some())
        {
            return Err(CommandError::bad_request(
                "around is mutually exclusive with other cursors",
            ));
        }
        if let Some(from) = self.from_ts
            && let Some(to) = self.to_ts
            && from > to
        {
            return Err(CommandError::bad_request("from_ts must be <= to_ts"));
        }

        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        let is_paginated = self.limit.is_some();

        // Route to the advanced path when any debug filter is set; otherwise
        // keep the legacy path (preserves turn-boundary snapping for UI paging).
        if self.uses_advanced() {
            let params = ListEventsParams {
                session_id,
                after_sequence: self.after_sequence,
                since_id: self.since_id,
                before_sequence: self.before_sequence,
                around_id: self.around,
                window: self.window,
                filter_types: self.types.clone(),
                exclude_types: self.exclude.clone(),
                from_ts: self.from_ts,
                to_ts: self.to_ts,
                turn_id: self.turn_id.clone(),
                exec_id: self.exec_id.clone(),
                trace_id: self.trace_id.clone(),
                tags: self.tags.clone(),
                tool_name: self.tool_name.clone(),
                q: self.q.clone(),
                order_desc: self.order_desc,
                limit: self.limit,
            };
            let events = q::event_service(ctx)?
                .list_advanced(&params)
                .await
                .map_err(classify_anyhow)?;
            let total = if is_paginated {
                Some(
                    q::event_service(ctx)?
                        .count_events(session_id.uuid(), &self.exclude)
                        .await
                        .map_err(classify_anyhow)?,
                )
            } else {
                None
            };
            return Ok(ListEventsResult {
                data: events,
                total,
            });
        }

        let mut events = q::event_service(ctx)?
            .list(
                session_id.uuid(),
                None,
                self.since_id.map(|id| id.uuid()),
                &self.types,
                &self.exclude,
                self.before_sequence,
                self.limit,
            )
            .await
            .map_err(classify_anyhow)?;

        if is_paginated && !events.is_empty() && self.before_sequence.is_some() {
            let first_seq = events[0].sequence.unwrap_or(0);
            if events[0].event_type != "turn.started"
                && let Ok(Some(turn_seq)) = q::event_service(ctx)?
                    .find_turn_boundary(session_id.uuid(), first_seq)
                    .await
                && turn_seq < first_seq
                && let Some(prefix_limit) = first_seq.checked_sub(turn_seq)
                && let Ok(mut prefix) = q::event_service(ctx)?
                    .list(
                        session_id.uuid(),
                        Some(turn_seq - 1),
                        None,
                        &self.types,
                        &self.exclude,
                        Some(first_seq),
                        Some(prefix_limit),
                    )
                    .await
            {
                prefix.extend(events);
                events = prefix;
            }
        }

        let total = if is_paginated {
            Some(
                q::event_service(ctx)?
                    .count_events(session_id.uuid(), &self.exclude)
                    .await
                    .map_err(classify_anyhow)?,
            )
        } else {
            None
        };

        Ok(ListEventsResult {
            data: events,
            total,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListEvents>() }

/// One row of `EventsSummaryResult.by_type` — the per-event-type count
/// produced by the events summary query.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventTypeCountOut {
    /// Event-type discriminator (e.g. `turn.started`, `tool.completed`).
    pub event_type: String,
    /// Count of items matching the query.
    pub count: i64,
}

/// Aggregate result of the events summary query — total count, per-type
/// breakdown, and a few convenience rollups (turn count, failure count).
#[derive(Debug, Serialize, ToSchema)]
pub struct EventsSummaryResult {
    /// Total event count across all types.
    pub total: i64,
    /// Per-type count, sorted by event_type asc.
    pub by_type: Vec<EventTypeCountOut>,
    /// Earliest event timestamp, if any.
    pub first_ts: Option<DateTime<Utc>>,
    /// Latest event timestamp, if any.
    pub last_ts: Option<DateTime<Utc>>,
    /// Convenience: count of `turn.started` events.
    pub turn_count: i64,
    /// Convenience: count of failure-shaped event types
    /// (`turn.failed`, `tool.failed`, `*.error`, `subagent.failed`).
    pub error_count: i64,
}

/// Debug snapshot for a session: per-type counts and time span.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventsSummaryCmd {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for EventsSummaryCmd {
    type Output = EventsSummaryResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "events_summary",
            category: "events",
            description: "One-shot debug summary for a session: counts by type, first/last timestamps, turn count, error count.",
            method: "GET",
            path: "/v1/sessions/{session_id}/events/summary",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<EventsSummaryResult, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        let summary = q::event_service(ctx)?
            .summary(session_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        let mut turn_count = 0i64;
        let mut error_count = 0i64;
        let by_type: Vec<EventTypeCountOut> = summary
            .by_type
            .into_iter()
            .map(|c| {
                if c.event_type == "turn.started" {
                    turn_count = c.count;
                }
                if matches!(
                    c.event_type.as_str(),
                    "turn.failed" | "tool.failed" | "subagent.failed"
                ) || c.event_type.ends_with(".error")
                {
                    error_count += c.count;
                }
                EventTypeCountOut {
                    event_type: c.event_type,
                    count: c.count,
                }
            })
            .collect();

        Ok(EventsSummaryResult {
            total: summary.total,
            by_type,
            first_ts: summary.first_ts,
            last_ts: summary.last_ts,
            turn_count,
            error_count,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<EventsSummaryCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamSse {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for StreamSse {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "stream_sse",
            category: "events",
            description: "Stream events via SSE. Not supported in bash mode.",
            method: "GET",
            path: "/v1/sessions/{session_id}/sse",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, _ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        Err(CommandError::bad_request(
            "SSE streaming is not available in bash mode. Use list_events instead.",
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<StreamSse>() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_filter_rejects_long_input() {
        let q = "a".repeat(MAX_Q_BYTES + 1);
        let err = validate_q_filter(Some(&q)).expect_err("long q should fail");
        assert!(err.to_string().contains("q: too long"));
    }

    #[test]
    fn q_filter_rejects_too_many_tokens() {
        let q = (0..=MAX_Q_TOKENS)
            .map(|_| "token")
            .collect::<Vec<_>>()
            .join(" ");
        let err = validate_q_filter(Some(&q)).expect_err("too many tokens should fail");
        assert!(err.to_string().contains("q: too many tokens"));
    }

    #[test]
    fn q_filter_accepts_bounded_input() {
        let q = "tool failed retry";
        validate_q_filter(Some(q)).expect("bounded q should pass");
    }
}
