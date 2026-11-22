//! Webhooks of a v2 session's tasks (data plane v2 parity, Phase B).
//!
//! A v2 task lives in its session's shard, with its push configs. The shard
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
//!   own webhooks, which it keeps. A push config made through the API
//!   carries its signing secret sealed with the org's data key (the edge
//!   that took it sealed it, `PushTarget::sealed`), so only ciphertext is
//!   in replicated state or on the feed; it is opened here to sign. A
//!   target whose secret does not open is never sent unsigned: it is
//!   skipped, and logged.

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
                    sealed: None,
                }),
        );
    }
    for target in targets {
        let mut target = target.clone();
        if let Some(sealed) = target.sealed.take() {
            match open(cp, org_id, session_id, &target.label, &sealed).await {
                Ok(secret) => target.secret = Some(secret),
                Err(error) => {
                    tracing::warn!(org_id, config = %target.label, %error, "push config secret did not open; not delivered");
                    continue;
                }
            }
        }
        out.push(target);
    }
    Ok(out)
}

/// Open push config `config`'s sealed secret: it must be sealed for this
/// session and config, with the org's data key.
async fn open(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
    config: &str,
    sealed: &everruns_contracts::dataplane::SealedSecret,
) -> anyhow::Result<String> {
    use everruns_contracts::dataplane_seal::{push_secret_id, unseal};
    anyhow::ensure!(
        sealed.secret_id == push_secret_id(session_id, config),
        "sealed for another config"
    );
    let row = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the org is not on a data plane"))?;
    anyhow::ensure!(
        sealed.data_key_id == row.data_key_id,
        "sealed under {}, the org's key is {}",
        sealed.data_key_id,
        row.data_key_id
    );
    let key = cp.data_key(&row)?;
    let plain =
        unseal(&key, &super::org_public(org_id), sealed).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(String::from_utf8(plain)?)
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
