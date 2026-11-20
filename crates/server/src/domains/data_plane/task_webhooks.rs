//! Webhooks of a v2 session's tasks (data plane v2 parity, Phase B).
//!
//! A v2 task lives in its session's cell, with its push configs. The cell
//! puts a transition a webhook may want on the feed
//! (`Record::TaskTransition`, `crates/cell/src/tasks.rs`), with the push
//! configs that ask for it, and this delivers it the way a v1 task's is
//! (`storage/runtime/session_task/webhook`): the org's task webhooks on a
//! terminal state, then the push configs, same payload, same signature,
//! DNS pinned against rebinding (TM-API-020).
//!
//! Decisions:
//! - Delivery is best-effort and off the feed's path, as on v1: a slow or
//!   failing endpoint never holds up a batch of usage and summaries, and
//!   the feed position is not held for it either. A control plane that
//!   stops before sending loses that delivery, as a v1 worker that stops
//!   after a transition does.
//! - The record names the targets; the control plane adds only the org's
//!   own webhooks, which it keeps.

use std::sync::Arc;

use everruns_contracts::dataplane::PushTarget;
use everruns_core::EgressService;
use serde_json::{Value, json};

use super::ControlPlane;

/// The event name a payload carries for `transition` (`task.terminal`, ...).
fn event_name(transition: &str) -> Option<&'static str> {
    match transition {
        "terminal" => Some("task.terminal"),
        "awaiting_input" => Some("task.awaiting_input"),
        "message" => Some("task.message"),
        _ => None,
    }
}

/// The deliveries one transition makes: the org's webhooks on a terminal
/// state, then `targets`.
pub async fn targets(
    cp: &ControlPlane,
    org_id: i64,
    transition: &str,
    targets: &[PushTarget],
) -> anyhow::Result<Vec<PushTarget>> {
    let mut out = Vec::new();
    // Org webhooks fire on terminal states only (EVE-579).
    if transition == "terminal" {
        out.extend(
            cp.db
                .list_enabled_org_task_webhooks(org_id)
                .await?
                .into_iter()
                .map(|w| PushTarget {
                    label: w.public_id,
                    url: w.url,
                    secret: w.secret,
                }),
        );
    }
    out.extend(targets.iter().cloned());
    Ok(out)
}

/// Deliver `transition` of `task` to its webhooks, in the background.
pub async fn deliver(
    cp: &ControlPlane,
    org_id: i64,
    transition: &str,
    task: &Value,
    pushed: &[PushTarget],
) -> anyhow::Result<()> {
    let Some(event) = event_name(transition) else {
        return Ok(());
    };
    let Some(egress) = cp.egress.clone() else {
        if !pushed.is_empty() {
            tracing::warn!(org_id, "no egress for task webhooks; delivery dropped");
        }
        return Ok(());
    };
    let targets = targets(cp, org_id, transition, pushed).await?;
    if targets.is_empty() {
        return Ok(());
    }
    let body = serde_json::to_vec(&json!({ "event": event, "task": task }))?;
    let task_id = task["id"].as_str().unwrap_or_default().to_owned();
    tokio::spawn(send(egress, targets, body, task_id, event));
    Ok(())
}

async fn send(
    egress: Arc<dyn EgressService>,
    targets: Vec<PushTarget>,
    body: Vec<u8>,
    task_id: String,
    event: &'static str,
) {
    for target in targets {
        let request = crate::storage::runtime::session_task::webhook::build_task_webhook_request(
            &target.url,
            &body,
            target.secret.as_deref(),
        );
        if let Err(error) = egress.send(request).await {
            tracing::warn!(
                webhook = %target.label,
                url = %target.url,
                task_id = %task_id,
                event,
                "task webhook delivery failed (best-effort): {error}"
            );
        }
    }
}
