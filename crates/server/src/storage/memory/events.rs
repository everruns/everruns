// In-memory storage: Events

use super::super::models::*;
use super::super::repository::MESSAGE_SAFETY_LIMIT;
use super::InMemoryDatabase;
use crate::kernel_imports::{
    everruns_provider::typed_id::EventId, everruns_provider::typed_id::SessionId,
};
use anyhow::Result;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        let now = Self::now();
        let id = EventId::new();

        // Get next sequence for this session
        let sequence = {
            let mut sequences = self.event_sequences.write();
            let seq = sequences.entry(input.session_id).or_insert(0);
            *seq += 1;
            *seq
        };

        let row = EventRow {
            id,
            session_id: input.session_id,
            sequence,
            event_type: input.event_type,
            ts: input.ts,
            context: input.context,
            data: input.data,
            metadata: input.metadata,
            tags: input.tags,
            created_at: now,
        };
        self.events.write().insert(id, row.clone());
        {
            let mut sessions = self.sessions.write();
            if let Some(session) = sessions.get_mut(&row.session_id) {
                match row.event_type.as_str() {
                    "turn.completed" | "turn.failed" | "turn.cancelled" => {
                        session.turn_count += 1;
                        // Mirrors the sessions_last_turn_insert trigger
                        // (migration 118) so the derived activity facet is the
                        // same on both backends.
                        session.last_turn_status = Some(
                            row.event_type
                                .strip_prefix("turn.")
                                .unwrap_or(&row.event_type)
                                .to_string(),
                        );
                        session.last_turn_at = Some(row.ts);
                    }
                    "tool.completed" => session.tool_call_count += 1,
                    _ => {}
                }
            }
        }
        let org_id = {
            let sessions = self.sessions.read();
            sessions.get(&row.session_id).map(|session| session.org_id)
        };
        if let Some(org_id) = org_id {
            self.enqueue_reporting_outbox(
                org_id,
                "event",
                &row.id.uuid().to_string(),
                Some(&row.id.uuid().to_string()),
                "event_projection",
            )
            .await?;
        }
        Ok(row)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                if e.session_id != session_id {
                    return false;
                }
                // Positive type filter: only include matching types (when non-empty)
                if !filter_types.is_empty() && !filter_types.contains(&e.event_type) {
                    return false;
                }
                // Negative type filter: exclude matching types
                if !exclude_types.is_empty() && exclude_types.contains(&e.event_type) {
                    return false;
                }
                // before_sequence cursor for backward pagination
                if let Some(before_seq) = before_sequence
                    && e.sequence >= before_seq
                {
                    return false;
                }
                // Resolve since_id to its sequence number for reliable ordering.
                // UUID v7 is NOT guaranteed monotonically increasing across
                // concurrent inserts, so always filter by sequence.
                if let Some(id) = since_id {
                    if let Some(ref_event) = events.get(&id) {
                        if e.sequence <= ref_event.sequence {
                            return false;
                        }
                    } else {
                        return false;
                    }
                } else if let Some(seq) = since_sequence
                    && e.sequence <= seq
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        result.sort_by_key(|e| e.sequence);

        // Apply limit.
        // - Explicit limit keeps backward pagination behavior (most recent N).
        // - Implicit safety cap (no explicit limit) keeps forward catch-up semantics:
        //   return the earliest rows first so callers can advance cursors safely.
        const FORWARD_SAFETY_LIMIT: usize = 10_000;
        if let Some(limit) = limit {
            let limit = limit as usize;
            if result.len() > limit {
                result = result.split_off(result.len() - limit);
            }
        } else if result.len() > FORWARD_SAFETY_LIMIT {
            if since_id.is_some() || since_sequence.is_some() {
                result.truncate(FORWARD_SAFETY_LIMIT);
            } else {
                result = result.split_off(result.len() - FORWARD_SAFETY_LIMIT);
            }
        }

        Ok(result)
    }

    /// Advanced event listing for debugging (in-memory mirror of the PG path).
    ///
    /// `since_id` and `around_id` lookups are scoped to `params.session_id` so a
    /// foreign event id silently produces an empty result. When `order_desc` is
    /// true, rows are returned newest-first.
    pub async fn list_events_advanced(&self, params: &ListEventsParams) -> Result<Vec<EventRow>> {
        let events = self.events.read();

        // Resolve around_id within the requested session only.
        let anchor_seq = match params.around_id {
            Some(id) => match events
                .get(&id)
                .filter(|e| e.session_id == params.session_id)
                .map(|e| e.sequence)
            {
                Some(s) => Some(s),
                None => return Ok(Vec::new()),
            },
            None => None,
        };

        // Resolve since_id only when the referenced event is in this session.
        let since_id_seq = params.since_id.and_then(|id| {
            events
                .get(&id)
                .filter(|e| e.session_id == params.session_id)
                .map(|e| e.sequence)
        });

        let q_lower: Option<String> = params.q.as_ref().map(|s| s.to_lowercase());

        let event_matches = |e: &EventRow| -> bool {
            if e.session_id != params.session_id {
                return false;
            }
            if !params.filter_types.is_empty() && !params.filter_types.contains(&e.event_type) {
                return false;
            }
            if !params.exclude_types.is_empty() && params.exclude_types.contains(&e.event_type) {
                return false;
            }
            if let Some(from) = params.from_ts
                && e.created_at < from
            {
                return false;
            }
            if let Some(to) = params.to_ts
                && e.created_at > to
            {
                return false;
            }
            if let Some(turn_id) = params.turn_id.as_deref() {
                let ctx_turn = e.context.get("turn_id").and_then(|v| v.as_str());
                if ctx_turn != Some(turn_id) {
                    return false;
                }
            }
            if let Some(exec_id) = params.exec_id.as_deref() {
                let ctx_exec = e.context.get("exec_id").and_then(|v| v.as_str());
                if ctx_exec != Some(exec_id) {
                    return false;
                }
            }
            if let Some(trace_id) = params.trace_id.as_deref() {
                let ctx_trace = e.context.get("trace_id").and_then(|v| v.as_str());
                if ctx_trace != Some(trace_id) {
                    return false;
                }
            }
            if !params.tags.is_empty() {
                let row_tags = e.tags.as_deref().unwrap_or(&[]);
                if !params.tags.iter().any(|t| row_tags.contains(t)) {
                    return false;
                }
            }
            if let Some(tool_name) = params.tool_name.as_deref() {
                let data_tool = e.data.get("tool_name").and_then(|v| v.as_str());
                if data_tool != Some(tool_name) {
                    return false;
                }
            }
            if let Some(needle) = q_lower.as_deref() {
                let haystack = serde_json::to_string(&e.data).unwrap_or_default();
                if !haystack.to_lowercase().contains(needle) {
                    return false;
                }
            }
            true
        };

        let mut result: Vec<EventRow> = if let Some(seq) = anchor_seq {
            const DEFAULT_WINDOW: i32 = 50;
            const MAX_WINDOW: i32 = 500;
            let window = params.window.unwrap_or(DEFAULT_WINDOW).clamp(1, MAX_WINDOW);
            let lo = seq - window;
            let hi = seq + window;
            events
                .values()
                .filter(|e| e.sequence >= lo && e.sequence <= hi && event_matches(e))
                .cloned()
                .collect()
        } else {
            events
                .values()
                .filter(|e| {
                    if let Some(seq) = since_id_seq
                        && e.sequence <= seq
                    {
                        return false;
                    }
                    if let Some(seq) = params.after_sequence
                        && e.sequence <= seq
                    {
                        return false;
                    }
                    if let Some(seq) = params.before_sequence
                        && e.sequence >= seq
                    {
                        return false;
                    }
                    event_matches(e)
                })
                .cloned()
                .collect()
        };

        result.sort_by_key(|e| e.sequence);

        // around_id always returns the contiguous window in ASC order; the
        // direction flag only applies to non-anchored queries.
        let limit = params.limit.unwrap_or(1_000).clamp(1, 10_000) as usize;
        if anchor_seq.is_none() {
            if params.order_desc {
                if result.len() > limit {
                    result = result.split_off(result.len() - limit);
                }
                result.reverse();
            } else if result.len() > limit {
                result.truncate(limit);
            }
        }
        Ok(result)
    }

    /// One-shot debug summary: per-type counts + first/last timestamps.
    pub async fn events_summary(&self, session_id: SessionId) -> Result<EventsSummary> {
        use std::collections::BTreeMap;

        let events = self.events.read();
        let mut by_type: BTreeMap<String, i64> = BTreeMap::new();
        let mut total = 0i64;
        let mut first_ts: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_ts: Option<chrono::DateTime<chrono::Utc>> = None;

        for e in events.values().filter(|e| e.session_id == session_id) {
            total += 1;
            *by_type.entry(e.event_type.clone()).or_insert(0) += 1;
            first_ts = Some(match first_ts {
                Some(prev) if prev < e.ts => prev,
                _ => e.ts,
            });
            last_ts = Some(match last_ts {
                Some(prev) if prev > e.ts => prev,
                _ => e.ts,
            });
        }

        Ok(EventsSummary {
            total,
            by_type: by_type
                .into_iter()
                .map(|(event_type, count)| EventTypeCount { event_type, count })
                .collect(),
            first_ts,
            last_ts,
        })
    }

    /// Find the nearest turn.started sequence at or before the given sequence.
    pub async fn find_turn_boundary(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> Result<Option<i32>> {
        let events = self.events.read();
        let seq = events
            .values()
            .filter(|e| {
                e.session_id == session_id
                    && e.sequence <= before_sequence
                    && e.event_type == "turn.started"
            })
            .map(|e| e.sequence)
            .max();
        Ok(seq)
    }

    /// Check if an input.message event with a given slack_ts already exists in a session.
    pub async fn has_event_with_slack_ts(
        &self,
        session_id: SessionId,
        slack_ts: &str,
    ) -> Result<bool> {
        let events = self.events.read();
        let found = events.values().any(|e| {
            e.session_id == session_id
                && e.event_type == "input.message"
                && e.data
                    .pointer("/message/metadata/slack_ts")
                    .and_then(|v| v.as_str())
                    == Some(slack_ts)
        });
        Ok(found)
    }

    /// Count events for a session without materializing rows.
    pub async fn count_events(
        &self,
        session_id: SessionId,
        exclude_types: &[String],
    ) -> Result<i64> {
        let events = self.events.read();
        let count = events
            .values()
            .filter(|e| {
                e.session_id == session_id
                    && (exclude_types.is_empty() || !exclude_types.contains(&e.event_type))
            })
            .count();
        Ok(count as i64)
    }

    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        self.list_message_events_limited(session_id, None).await
    }

    /// List message events with an optional limit.
    /// When `limit` is Some, returns the most recent N messages in sequence order.
    pub async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        let message_types = [
            "input.message",
            "output.message.completed",
            "tool.completed",
        ];
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                e.session_id == session_id && message_types.contains(&e.event_type.as_str())
            })
            .cloned()
            .collect();
        result.sort_by_key(|e| e.sequence);
        if let Some(limit) = limit.filter(|limit| *limit > 0) {
            let len = result.len();
            let limit = limit as usize;
            if len > limit {
                result = result.split_off(len - limit);
            }
        } else if limit.is_some() {
            result.clear();
        } else if result.len() > MESSAGE_SAFETY_LIMIT {
            // No explicit limit: cap to the latest N rows, matching the
            // Postgres backend's safety cap on this path.
            let drain_end = result.len() - MESSAGE_SAFETY_LIMIT;
            result.drain(0..drain_end);
        }
        Ok(result)
    }

    /// Count message events for a session — no row materialization.
    pub async fn count_message_events(&self, session_id: SessionId) -> Result<i64> {
        let message_types = [
            "input.message",
            "output.message.completed",
            "tool.completed",
        ];
        let events = self.events.read();
        let count = events
            .values()
            .filter(|e| {
                e.session_id == session_id && message_types.contains(&e.event_type.as_str())
            })
            .count();
        Ok(count as i64)
    }

    /// List message events for a session with filters applied.
    ///
    /// This method applies filters in-memory, mirroring the behavior of the
    /// PostgreSQL implementation but using Rust predicates instead of SQL.
    ///
    /// Note: Injections are NOT applied here - they should be applied at the
    /// MessageRetriever layer after converting events to messages.
    pub async fn list_message_events_filtered(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<EventRow>> {
        // Default event types if not specified
        let default_types = vec![
            "input.message".to_string(),
            "output.message.completed".to_string(),
            "tool.completed".to_string(),
        ];

        // Check for EventTypes filter, else use defaults
        let event_types = query
            .filters
            .iter()
            .find_map(|f| match f {
                MessageFilter::EventTypes(types) => Some(types.clone()),
                _ => None,
            })
            .unwrap_or(default_types);

        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                // Session filter
                if e.session_id != query.session_id {
                    return false;
                }
                if query
                    .after_sequence
                    .is_some_and(|sequence| i64::from(e.sequence) <= sequence)
                {
                    return false;
                }

                // Event type filter
                if !event_types.iter().any(|t| t == &e.event_type) {
                    return false;
                }

                // Apply all filters
                for filter in &query.filters {
                    match filter {
                        MessageFilter::EventTypes(_) => {
                            // Already handled above
                        }
                        MessageFilter::TimeRange { from, to } => {
                            if let Some(f) = from
                                && e.created_at < *f
                            {
                                return false;
                            }
                            if let Some(t) = to
                                && e.created_at > *t
                            {
                                return false;
                            }
                        }
                        MessageFilter::ToolName(name) => {
                            if e.event_type != "tool.completed" {
                                return false;
                            }
                            let tool_match = e
                                .data
                                .get("tool_name")
                                .and_then(|v| v.as_str())
                                .map(|n| n == name)
                                .unwrap_or(false);
                            if !tool_match {
                                return false;
                            }
                        }
                        MessageFilter::Search(search_query) => {
                            // Match PostgreSQL tsvector behavior: search canonical event text
                            // fields (message.content/result/content/delta/accumulated).
                            let searchable = e
                                .data
                                .pointer("/message/content")
                                .or_else(|| e.data.pointer("/result"))
                                .or_else(|| e.data.get("content"))
                                .or_else(|| e.data.get("delta"))
                                .or_else(|| e.data.get("accumulated"))
                                .map(|v| match v {
                                    serde_json::Value::String(s) => s.to_string(),
                                    _ => v.to_string(),
                                })
                                .unwrap_or_default();
                            if !searchable
                                .to_lowercase()
                                .contains(&search_query.to_lowercase())
                            {
                                return false;
                            }
                        }
                        MessageFilter::ExcludeIds(ids) => {
                            if ids.contains(&e.id) {
                                return false;
                            }
                        }
                        MessageFilter::IncludeIds(ids) => {
                            if !ids.contains(&e.id) {
                                return false;
                            }
                        }
                        MessageFilter::Custom(_) => {
                            // Custom filters are applied at the Message level,
                            // not the EventRow level. Skip here.
                        }
                    }
                }

                true
            })
            .cloned()
            .collect();

        result.sort_by_key(|e| e.sequence);

        // Apply offset and limit
        if let Some(offset) = query.offset {
            result = result.into_iter().skip(offset as usize).collect();
        }
        if let Some(limit) = query.limit {
            let limit = limit.max(0) as usize;
            // Head+tail window: keep the first `keep_head` (the task anchor) plus
            // the latest `limit`, dropping the middle. De-dups on overlap. Mirrors
            // the Postgres backend so both honor `MessageQuery::keep_head`.
            let keep_head = query.keep_head.unwrap_or(0).min(result.len());
            if result.len() > keep_head + limit {
                let drain_end = result.len() - limit;
                result.drain(keep_head..drain_end);
            }
        } else if result.len() > MESSAGE_SAFETY_LIMIT {
            // No explicit limit: apply the same safety cap as the Postgres
            // backend's (None, None) branch, keeping the most recent N rows so
            // an unbounded full-history candidate load cannot grow without bound.
            let drain_end = result.len() - MESSAGE_SAFETY_LIMIT;
            result.drain(0..drain_end);
        }

        Ok(result)
    }

    /// Get preview text for multiple sessions (in-memory implementation)
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        let mut previews = std::collections::HashMap::new();
        let events = self.events.read();

        for &session_id in session_ids {
            // Find the first user message for this session
            let first_user_msg = events
                .values()
                .filter(|e| e.session_id == session_id && e.event_type == "input.message")
                .min_by_key(|e| e.sequence);

            if let Some(event) = first_user_msg {
                // Extract text from the message data
                if let Some(text) = event
                    .data
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Truncate to 200 chars
                    let preview: String = text.chars().take(200).collect();
                    previews.insert(session_id, preview);
                }
            }
        }

        Ok(previews)
    }

    /// Get output preview text for multiple sessions (in-memory implementation)
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        let mut previews = std::collections::HashMap::new();
        let events = self.events.read();

        for &session_id in session_ids {
            // Find the last agent message for this session
            let last_agent_msg = events
                .values()
                .filter(|e| {
                    e.session_id == session_id && e.event_type == "output.message.completed"
                })
                .max_by_key(|e| e.sequence);

            if let Some(event) = last_agent_msg {
                // Extract text from the message data
                if let Some(text) = event
                    .data
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Truncate to 200 chars
                    let preview: String = text.chars().take(200).collect();
                    previews.insert(session_id, preview);
                }
            }
        }

        Ok(previews)
    }
}
