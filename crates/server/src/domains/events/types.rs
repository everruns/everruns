// Events domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::common_dto::ErrorResponse;
use axum::{Json, http::StatusCode};
use everruns_contracts::typed_id::EventId;
use everruns_core::VALID_EVENT_TYPES;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// Query parameters accepted by the SSE `/sse` endpoint.
///
/// SSE supports filtering only — pagination/limit semantics are not meaningful
/// for a continuous stream. Debug-only filters live on `ListEventsQuery` and
/// are intentionally absent here so they do not appear on the SSE OpenAPI spec.
#[derive(Debug, Default, Deserialize, ToSchema, IntoParams)]
pub struct EventsQuery {
    /// Filter events with ID greater than this event ID (prefixed format: event_{32-hex})
    pub since_id: Option<EventId>,
    /// Forward cursor: replay durable events with `sequence` greater than this
    /// value before switching to live streaming. `after_sequence=0` replays the
    /// session from its first event — that is what a client with an empty
    /// snapshot must send, otherwise events written between its snapshot and
    /// this subscription are never delivered. Mutually exclusive with `since_id`.
    pub after_sequence: Option<i32>,
    /// Positive type filter: only return events matching these types (can be specified multiple times).
    /// When empty, all types are returned. Example: ?types=turn.started&types=turn.completed
    #[serde(default)]
    #[param(style = Form, explode = true)]
    pub types: Vec<String>,
    /// Event types to exclude from the response (can be specified multiple times).
    /// Applied after `types` filter. Common delta events to exclude: output.message.delta, reason.thinking.delta
    #[serde(default)]
    #[param(style = Form, explode = true)]
    pub exclude: Vec<String>,
}

impl EventsQuery {
    /// Validate types and exclude parameters.
    /// Rejects unknown event types and limits array size to prevent abuse.
    pub(crate) fn validate(&self) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
        validate_event_type_list(&self.types, "types")?;
        validate_event_type_list(&self.exclude, "exclude")?;
        if self.since_id.is_some() && self.after_sequence.is_some() {
            return Err(ErrorResponse::new(
                "since_id and after_sequence are mutually exclusive".to_string(),
            )
            .into_response(StatusCode::BAD_REQUEST));
        }
        if self.after_sequence.is_some_and(|seq| seq < 0) {
            return Err(
                ErrorResponse::new("after_sequence must be >= 0".to_string())
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
        Ok(())
    }
}

/// Max event types per filter parameter. Kept above the known event set so
/// clients can explicitly request every supported event type.
pub(crate) const MAX_EVENT_TYPE_FILTER_SIZE: usize = 64;

/// Validate a list of event type strings: checks size limit and known types.
pub(crate) fn validate_event_type_list(
    types: &[String],
    param_name: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if types.len() > MAX_EVENT_TYPE_FILTER_SIZE {
        return Err(ErrorResponse::new(format!(
            "{param_name}: too many values ({}, max {MAX_EVENT_TYPE_FILTER_SIZE})",
            types.len()
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }
    for t in types {
        if !VALID_EVENT_TYPES.contains(&t.as_str()) {
            return Err(
                ErrorResponse::new(format!("{param_name}: unknown event type '{t}'"))
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
    }
    Ok(())
}
