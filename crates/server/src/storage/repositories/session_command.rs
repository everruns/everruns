//! PostgreSQL adapter for the same command reducer used beneath Engine.
use crate::storage::WaitingTurnResolutionPlan;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::{
    events::{EventContext, ToolCompletedData},
    session_commands::{self, CommandView, ControlUpdate, SessionCommand},
};
use serde_json::{Value, json};

pub(super) async fn prepare_tool_results(
    tx: &mut crate::storage::transaction::DbTx,
    session: SessionId,
    results: Vec<ToolCompletedData>,
) -> anyhow::Result<WaitingTurnResolutionPlan> {
    let now: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    let request: Option<(i32, Value, Value)> = sqlx::query_as(
        "SELECT sequence, context, data FROM events \
         WHERE session_id = $1 AND event_type = 'tool.call_requested' \
         ORDER BY sequence DESC LIMIT 1",
    )
    .bind(session)
    .fetch_optional(&mut **tx)
    .await?;
    let (sequence, context, data) = request.ok_or_else(|| {
        everruns_contracts::AgentLoopError::config("session has no pending client tools")
    })?;
    let mut view = CommandView::new(session, now.timestamp_millis());
    // The session row was found and locked by the tenant-scoped claim. Executable
    // Agent loading remains platform-owned; this command does not create a binding.
    view.binding = Some(json!({}));
    view.open = Some(serde_json::from_value::<EventContext>(context)?);
    if view
        .open
        .as_ref()
        .is_none_or(|context| context.turn_id.is_none())
    {
        // Match the platform's existing imported-history correlation convention.
        view.open = Some(EventContext::turn(
            TurnId::from_uuid(session.uuid()),
            MessageId::from_uuid(session.uuid()),
        ));
    }
    view.pending = serde_json::from_value(data.get("tool_calls").cloned().unwrap_or(json!([])))?;
    let completed: Vec<String> = sqlx::query_scalar(
        "SELECT data->>'tool_call_id' FROM events \
         WHERE session_id = $1 AND event_type = 'tool.completed' AND sequence > $2 \
         ORDER BY sequence LIMIT 257",
    )
    .bind(session)
    .bind(sequence)
    .fetch_all(&mut **tx)
    .await?;
    anyhow::ensure!(completed.len() <= 256, "pending result scan limit exceeded");
    view.pending.retain(|call| !completed.contains(&call.id));
    view.head =
        sqlx::query_scalar("SELECT coalesce(max(sequence),0) FROM events WHERE session_id=$1")
            .bind(session)
            .fetch_one(&mut **tx)
            .await?;
    let turn = view
        .open
        .as_ref()
        .and_then(|context| context.turn_id)
        .ok_or_else(|| anyhow::anyhow!("open turn lacks identity"))?;
    let accepted = results.len();
    let plan = session_commands::prepare(
        SessionCommand::Resolve {
            id: uuid::Uuid::now_v7().to_string(),
            turn,
            results,
        },
        view,
    )
    .map_err(everruns_contracts::AgentLoopError::from)?;
    Ok(WaitingTurnResolutionPlan {
        kind: "tool_results".into(),
        events: plan.events,
        session_values: Vec::new(),
        response: json!({"accepted":accepted,"resume":matches!(plan.control,ControlUpdate::Resume)}),
    })
}
