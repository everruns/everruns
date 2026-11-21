//! A v2 session's Work tab reads on the data plane (step 11.6): the work
//! report of a turn, and the session's leased resources.
//!
//! Decisions:
//! - The work report is computed on read from the turn's events with the
//!   one reducer every host uses (`WorkReport::from_events`), as the
//!   control plane does for a v1 session, so the two read the same.
//! - A cell leases no resources: its sandboxes are pinned by the control
//!   plane (step 11.3) and shown by the sandbox panel. The resources list
//!   answers empty rather than 501, which is what a v2 session holds.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use everruns_contracts::runtime::work_report::{WORK_REPORT_EVENT_TYPES, WorkReport};
use everruns_core::events::{Event, EventData};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::gateway::{Gateway, parse};

type ApiError = (StatusCode, String);

#[derive(Debug, Default, Deserialize)]
pub(crate) struct WorkQuery {
    turn_id: Option<String>,
}

/// The report of `turn` (the latest turn when none), or `None` when the
/// session ran no turn or the turn has no events.
pub fn report(
    events: &[Event],
    turn: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<WorkReport> {
    let turn = match turn {
        Some(turn) => turn.to_owned(),
        None => events.iter().rev().find_map(|event| match &event.data {
            EventData::TurnStarted(data) => Some(data.turn_id.to_string()),
            _ => None,
        })?,
    };
    let chosen = events.iter().filter(|event| {
        WORK_REPORT_EVENT_TYPES.contains(&event.event_type.as_str())
            && event
                .context
                .turn_id
                .is_some_and(|id| id.to_string() == turn)
    });
    WorkReport::from_events(chosen, now)
}

/// `GET /v1/sessions/{id}/work`: `{"report": ...}`; 404 for a named turn
/// with no events.
pub(crate) async fn work(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Query(query): Query<WorkQuery>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let events = crate::gateway_v1::read_events(&gateway, session, 0).await?;
    let report = report(&events, query.turn_id.as_deref(), chrono::Utc::now());
    if report.is_none() && query.turn_id.is_some() {
        return Err((StatusCode::NOT_FOUND, "Turn not found".into()));
    }
    Ok(Json(json!({ "report": report })))
}

/// `GET /v1/sessions/{id}/resources`: none on a data plane.
pub(crate) async fn resources(Path(session): Path<String>) -> Result<Json<Value>, ApiError> {
    parse(&session)?;
    Ok(Json(json!([])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
    use everruns_core::events::{EventContext, TurnCompletedData, TurnStartedData};

    fn started(session: SessionId, turn: TurnId) -> Event {
        let context = EventContext {
            turn_id: Some(turn),
            ..Default::default()
        };
        Event::new(
            session,
            context,
            TurnStartedData {
                turn_id: turn,
                input_message_id: MessageId::new(),
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
                origin: None,
            },
        )
    }

    #[test]
    fn the_latest_turn_is_reported_unless_one_is_named() {
        let session = SessionId::new();
        let (first, second) = (TurnId::new(), TurnId::new());
        assert!(report(&[], None, chrono::Utc::now()).is_none());
        let mut done = Event::new(
            session,
            EventContext::default(),
            TurnCompletedData::default(),
        );
        done.context.turn_id = Some(first);
        let events = vec![started(session, first), done, started(session, second)];
        let latest = report(&events, None, chrono::Utc::now());
        let named = report(&events, Some(&first.to_string()), chrono::Utc::now());
        assert!(latest.is_some());
        assert!(named.is_some());
        assert_ne!(
            serde_json::to_value(&latest).unwrap(),
            serde_json::to_value(&named).unwrap()
        );
        assert!(
            report(
                &events,
                Some(&TurnId::new().to_string()),
                chrono::Utc::now()
            )
            .is_none()
        );
    }
}
