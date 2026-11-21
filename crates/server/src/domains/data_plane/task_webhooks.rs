//! Webhooks of a v2 session's tasks (data plane v2 parity, Phase B).
//!
//! A v2 task lives in its session's cell, with its push configs. The cell
//! puts a transition a webhook may want on the feed
//! (`Record::TaskTransition`, `crates/agentd/src/tasks.rs`), with the push
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
//!   own webhooks, which it keeps, and the signing secrets of push configs
//!   made through the API: the node that adds one hands its secret over
//!   ([`keep_secret`], the `task_push_secret` host call) and the cell keeps
//!   only that it has one, so no such secret is in replicated state or on
//!   the feed. A target whose secret cannot be found here is sent unsigned
//!   no more than v1 would: it is skipped, and logged.

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

/// Keep push config `config_id`'s signing secret for `session_id` of
/// `org_id` (the `task_push_secret` host call), encrypted with the
/// server's secrets key. Only a session of the org on its data plane.
pub async fn keep_secret(cp: &ControlPlane, org_id: i64, params: &Value) -> anyhow::Result<()> {
    let text = |name: &str| {
        params[name]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{name} is required"))
    };
    let (session, config, secret) = (text("session_id")?, text("config_id")?, text("secret")?);
    let session_id: everruns_contracts::typed_id::SessionId = session.parse()?;
    anyhow::ensure!(
        super::sessions::runs_on_data_plane(cp, org_id, session_id).await?
            && cp.db.dp_session_org(session).await? == Some(org_id),
        "session {session} is not the org's"
    );
    anyhow::ensure!(config.starts_with("tpc_"), "not a push config id");
    let encrypted = cp.encryption()?.encrypt_string(secret)?;
    cp.db
        .dp_put_task_push_secret(config, org_id, session, &encrypted)
        .await
}

/// The deliveries one transition makes: the org's webhooks on a terminal
/// state, then `targets`, with the secrets held here filled in.
pub async fn targets(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
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
                    secret_held: false,
                }),
        );
    }
    for target in targets {
        let mut target = target.clone();
        if target.secret_held {
            let held = cp
                .db
                .dp_task_push_secret(org_id, session_id, &target.label)
                .await?;
            let Some(encrypted) = held else {
                tracing::warn!(org_id, config = %target.label, "push config secret not found; not delivered");
                continue;
            };
            target.secret = Some(cp.encryption()?.decrypt_to_string(&encrypted)?);
            target.secret_held = false;
        }
        out.push(target);
    }
    Ok(out)
}

/// Deliver `transition` of `task` to its webhooks, in the background.
pub async fn deliver(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
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
    let targets = targets(cp, org_id, session_id, transition, pushed).await?;
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
