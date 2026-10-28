//! Completion waits at the authorized server edge. Subscribe before the live
//! state probe, wake only for lifecycle changes, and reconcile periodically
//! after a lost notification. Deltas never trigger a database read.
use super::*;
use crate::domains::sessions::record::SessionStatus;
use std::time::Duration;

async fn completed_status(
    current: &Session,
    session: &str,
    ctx: &Ctx,
) -> Result<Option<String>, CommandError> {
    match current.status {
        SessionStatus::Idle => {
            let output = dispatch(
                "list_events",
                json!({
                    "session_id":session,
                    "types":["turn.completed","turn.failed","turn.cancelled","turn.sealed"],
                    "limit":1,"order_desc":true
                }),
                ctx,
            )
            .await?;
            let output: Value = decode(&output)?;
            Ok(output["data"]
                .as_array()
                .and_then(|events| events.first())
                .and_then(|event| event["type"].as_str())
                .and_then(|kind| kind.strip_prefix("turn."))
                .map(str::to_string))
        }
        SessionStatus::WaitingForToolResults => Ok(Some("waiting_for_tool_results".into())),
        SessionStatus::Paused => Ok(Some("paused".into())),
        SessionStatus::Started | SessionStatus::Active => Ok(None),
    }
}

pub(super) async fn wait(params: Value, ctx: &Ctx) -> Result<String, CommandError> {
    let session = params
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::bad_request("session_id is required"))?;
    let session_id: everruns_contracts::typed_id::SessionId = session
        .parse()
        .map_err(|_| CommandError::bad_request("Invalid session_id"))?;
    let timeout = params
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(120);
    if timeout > 21_600 {
        return Err(CommandError::bad_request(
            "timeout_secs must be at most 21600",
        ));
    }
    let lookup = json!({"session_id": session});
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    // THREAT[TM-AUTHZ-004]: authorize before subscribing. Already-finished children and zero-timeout
    // reconciliation probes need one lookup and no NATS consumer allocation.
    let initial: Session = decode(&dispatch("get_session", lookup.clone(), ctx).await?)?;
    if let Some(status) = completed_status(&initial, session, ctx).await? {
        return encode(&status);
    }
    if timeout == 0 {
        return encode(&format!("timeout (last status: {:?})", initial.status));
    }
    let delivery = ctx
        .event_service
        .as_ref()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Event service unavailable")))?
        .event_delivery();
    let mut subscription = tokio::time::timeout(
        Duration::from_secs(1),
        delivery.subscribe(session_id.uuid()),
    )
    .await
    .ok()
    .and_then(Result::ok);
    loop {
        // Re-read after subscribing so a completion during subscription setup
        // cannot be missed. Every probe retains the command's tenant policy.
        let current: Session = decode(&dispatch("get_session", lookup.clone(), ctx).await?)?;
        if let Some(status) = completed_status(&current, session, ctx).await? {
            return encode(&status);
        }
        // Idle without a terminal event is not guessed successful: those writes
        // commit independently, so wait for notification or durable reconciliation.
        if tokio::time::Instant::now() >= deadline {
            return encode(&format!("timeout (last status: {:?})", current.status));
        }
        let interval = if subscription.is_some() {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(500)
        };
        let reconcile_at = (tokio::time::Instant::now() + interval).min(deadline);
        if let Some(events) = subscription.as_mut() {
            let wake = async {
                while let Some(event) = events.recv().await {
                    if event.session_id == session_id
                        && matches!(
                            event.event_type.as_str(),
                            "turn.completed"
                                | "turn.failed"
                                | "turn.cancelled"
                                | "turn.sealed"
                                | "session.idled"
                                | "session.activated"
                                | "budget.paused"
                                | "tool.call_requested"
                        )
                    {
                        return true;
                    }
                }
                false
            };
            tokio::select! {
                open = wake => if !open { subscription = None; },
                _ = tokio::time::sleep_until(reconcile_at) => {}
            }
        } else {
            tokio::time::sleep_until(reconcile_at).await;
        }
    }
}
