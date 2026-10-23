// Work report of a session's turn: what it is doing now, what it got done.
//
// Decision: computed on read from the turn's events with the one reducer in
// `everruns_contracts::runtime::work_report`, so nothing new is stored and a
// past turn reads the same as it did live. Clients refetch on the events they
// already stream instead of a second, client-side reducer.
// See knowledge/execution/work-reports.md.

use super::queries as q;
use crate::domains::common::*;
use crate::storage::ListEventsParams;
use chrono::Utc;
use everruns_contracts::runtime::events::{EventData, TURN_STARTED};
use everruns_contracts::runtime::work_report::{WORK_REPORT_EVENT_TYPES, WorkReport};
use everruns_core::Event;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Events read per page while loading one turn.
const PAGE: i32 = 1000;
/// Pages read before giving up on a pathological turn; the reducer keeps the
/// newest steps anyway, so a report from the first pages is still useful.
const MAX_PAGES: usize = 10;

/// Work report of one turn, or of the session's latest turn.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct GetSessionWork {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Turn to report on. Defaults to the session's latest turn.
    pub turn_id: Option<String>,
}

/// Response of `GET /v1/sessions/{session_id}/work`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionWorkResult {
    /// The report, or `null` when the session has not run a turn.
    pub report: Option<WorkReport>,
}

#[command(
    name = "get_session_work",
    category = "events",
    description = "What a session's turn is doing now, or what it got done: headline, steps, plan, open request and result.",
    method = "GET",
    cli = CliRoute::new(&["sessions", "work"], "get").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("See what a session's agent is doing or what it got done", "everruns sessions work get session_01h9")]),
    path = "/v1/sessions/{session_id}/work",
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session or turn not found")),
)]
impl Command for GetSessionWork {
    type Output = SessionWorkResult;

    async fn execute(self, ctx: &Ctx) -> Result<SessionWorkResult, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        let events = q::event_service(ctx)?;

        let turn_id = match self.turn_id {
            Some(turn_id) => turn_id,
            None => {
                let latest = events
                    .list_advanced(&ListEventsParams {
                        session_id,
                        filter_types: vec![TURN_STARTED.to_string()],
                        order_desc: true,
                        limit: Some(1),
                        ..Default::default()
                    })
                    .await?;
                match latest.first().map(|event| &event.data) {
                    Some(EventData::TurnStarted(data)) => data.turn_id.to_string(),
                    _ => return Ok(SessionWorkResult { report: None }),
                }
            }
        };

        let mut turn_events: Vec<Event> = Vec::new();
        let mut after_sequence = None;
        for _ in 0..MAX_PAGES {
            let page = events
                .list_advanced(&ListEventsParams {
                    session_id,
                    after_sequence,
                    filter_types: WORK_REPORT_EVENT_TYPES
                        .iter()
                        .map(|t| t.to_string())
                        .collect(),
                    turn_id: Some(turn_id.clone()),
                    limit: Some(PAGE),
                    ..Default::default()
                })
                .await?;
            let full = page.len() == PAGE as usize;
            after_sequence = page.last().and_then(|event| event.sequence);
            turn_events.extend(page);
            if !full || after_sequence.is_none() {
                break;
            }
        }
        match WorkReport::from_events(&turn_events, Utc::now()) {
            Some(report) => Ok(SessionWorkResult {
                report: Some(report),
            }),
            None => Err(CommandError::not_found("Turn")),
        }
    }
}
