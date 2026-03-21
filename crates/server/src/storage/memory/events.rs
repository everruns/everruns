// In-memory storage: Events

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::{EventId, SessionId};
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

        // Apply limit: take last N events (backward pagination).
        // Safety cap of 10 000 when no explicit limit to prevent unbounded results.
        let effective_limit = limit.map(|l| l as usize).unwrap_or(10_000);
        if result.len() > effective_limit {
            result = result.split_off(result.len() - effective_limit);
        }

        Ok(result)
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
        if let Some(limit) = limit {
            let len = result.len();
            let limit = limit as usize;
            if len > limit {
                result = result.split_off(len - limit);
            }
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
                            // Match PostgreSQL tsvector behavior: search data->>'content'
                            let content =
                                e.data.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            if !content
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
            result.truncate(limit as usize);
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
