use super::queries as q;
use crate::domains::common::*;
use everruns_core::{Event, VALID_EVENT_TYPES};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_EVENT_TYPE_FILTER_SIZE: usize = 40;
const MAX_EVENT_LIMIT: i32 = 1000;

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

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListEvents {
    pub session_id: String,
    pub since_id: Option<everruns_core::typed_id::EventId>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub limit: Option<i32>,
    pub before_sequence: Option<i32>,
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

        if let Some(limit) = self.limit
            && !(1..=MAX_EVENT_LIMIT).contains(&limit)
        {
            return Err(CommandError::bad_request(format!(
                "limit must be between 1 and {MAX_EVENT_LIMIT}, got {limit}"
            )));
        }
        if self.before_sequence.is_some() && self.limit.is_none() {
            return Err(CommandError::bad_request(
                "before_sequence requires limit to be set",
            ));
        }

        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        let is_paginated = self.limit.is_some();
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamSse {
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
