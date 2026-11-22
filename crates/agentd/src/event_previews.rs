//! Small, sequence-bounded evidence reads for opt-in monitoring. They never
//! resolve an event id by scanning a session's lifetime log.
use crate::{api::Lookup, gateway::Gateway, node::Node};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::compaction_observation::{CompactionEvidenceExcerpt, bounded_json_preview};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct PreviewQuery {
    pub from: i32,
    pub to: i32,
    pub chars: usize,
}

impl PreviewQuery {
    fn validate(&self) -> Result<(), (StatusCode, String)> {
        if self.from < 1
            || self.to < self.from
            || i64::from(self.to) - i64::from(self.from) >= 64
            || !(1..=2048).contains(&self.chars)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Evidence reads require 1–64 sequence positions and 1–2048 preview bytes".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/shard/sessions/{session}/event-previews", get(previews))
        .with_state(node)
}

async fn previews(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Query(query): Query<PreviewQuery>,
) -> Result<Json<Lookup<Vec<CompactionEvidenceExcerpt>>>, (StatusCode, String)> {
    query.validate()?;
    let id =
        SessionId::parse(&session).map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    crate::api_files::read_barrier(&node).await.map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Evidence replica is not ready".into(),
        )
    })?;
    let rows = match node
        .shared
        .context(id, query.from, Some(query.to), query.from)
        .await
    {
        Some(Ok(rows)) => rows,
        Some(Err(_)) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Evidence is unavailable".into(),
            ));
        }
        None => return Ok(Json(crate::api::lookup(&node, id, |_| Vec::new()))),
    };
    let rows = rows
        .into_iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "input.message"
                    | "output.message.completed"
                    | "tool.completed"
                    | "tool.started"
                    | "conversation.message"
            )
        })
        .take(16)
        .filter_map(|event| {
            let sequence = event.sequence?;
            let id = event.id.to_string();
            let event_type = event.event_type.clone();
            let (text, truncated) = bounded_json_preview(&event.into_public().data, query.chars);
            Some(CompactionEvidenceExcerpt {
                id,
                sequence,
                event_type,
                text,
                truncated,
            })
        })
        .collect();
    Ok(Json(Lookup::Here(rows)))
}

pub(crate) async fn read(
    gateway: &Arc<Gateway>,
    session: SessionId,
    from: i32,
    to: i32,
    chars: usize,
) -> Result<Vec<CompactionEvidenceExcerpt>, (StatusCode, String)> {
    PreviewQuery { from, to, chars }.validate()?;
    let gw = gateway.clone();
    gateway
        .with_shard(session, |shard| {
            let gw = gw.clone();
            async move {
                gw.call::<Lookup<Vec<CompactionEvidenceExcerpt>>>(
                    shard,
                    &format!(
                        "/shard/sessions/{session}/event-previews?from={from}&to={to}&chars={chars}"
                    ),
                    None,
                )
                .await
            }
        })
        .await
}
