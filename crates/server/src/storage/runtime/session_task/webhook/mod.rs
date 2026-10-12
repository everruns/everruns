//! Outbound HTTP webhooks on task transitions (EVE-579, EVE-682).
//!
//! Decision: the notifier lives beside the registry it observes rather than in
//! the in-process worker's adapters: the worker's registry is now the internal
//! session-task commands (`domains/session_tasks/commands/worker`), which
//! attach it whenever their context carries an egress service.

use super::{TaskTransition, TaskTransitionObserver};
use crate::kernel_imports::{EgressRequest, EgressRequestKind, EgressService};
use crate::storage::StorageBackend;
use std::sync::Arc;

/// Delivers org task webhooks and per-task push configs for one transition.
pub(crate) struct TaskWebhookNotifier {
    pub(crate) db: Arc<StorageBackend>,
    pub(crate) egress_service: Arc<dyn EgressService>,
}

/// One resolved delivery target for a webhook notification. `label` is only for
/// logging (a public id or a spec-embedded marker) and never affects delivery.
struct WebhookTarget {
    label: String,
    url: String,
    secret: Option<String>,
}

/// Parse spec-embedded push configs (EVE-682) that match `event`.
///
/// Spawn-time configs live in `task.spec["push_configs"]` as an array of
/// `{ url, secret?, event_filter? }`. `event_filter` defaults to
/// `["terminal"]`, matching org-webhook behavior. URLs were SSRF-validated at
/// spawn time and delivery pins DNS, so no re-validation is needed here.
fn spec_push_config_targets(spec: &serde_json::Value, event: TaskTransition) -> Vec<WebhookTarget> {
    let Some(entries) = spec.get("push_configs").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for entry in entries {
        let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
            continue;
        };
        let matches = match entry.get("event_filter").and_then(|v| v.as_array()) {
            Some(filters) => filters
                .iter()
                .filter_map(|f| f.as_str())
                .any(|f| f == event.filter_value()),
            // Absent filter defaults to terminal-only.
            None => event.filter_value() == "terminal",
        };
        if !matches {
            continue;
        }
        targets.push(WebhookTarget {
            label: "spec".to_string(),
            url: url.to_string(),
            secret: entry
                .get("secret")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    targets
}

#[async_trait::async_trait]
impl TaskTransitionObserver for TaskWebhookNotifier {
    async fn on_transition(
        &self,
        task: &everruns_core::session_task::SessionTask,
        event: TaskTransition,
    ) -> anyhow::Result<()> {
        // Resolve org_id from session (unscoped lookup — no harness required).
        // Server-internal dispatch only: this is never a user-facing read, so
        // the unscoped lookup does not widen any caller's tenant scope.
        let session = self
            .db
            .get_session_unscoped(task.session_id)
            .await
            .map_err(|e| anyhow::anyhow!("webhook notifier: session lookup failed: {e}"))?;
        let Some(session) = session else {
            return Ok(());
        };

        let mut targets: Vec<WebhookTarget> = Vec::new();

        // Org webhooks are terminal-only and unchanged (EVE-579): they never
        // fire on non-terminal events.
        if event == TaskTransition::Terminal {
            let webhooks = self
                .db
                .list_enabled_org_task_webhooks(session.org_id)
                .await
                .map_err(|e| anyhow::anyhow!("webhook notifier: webhook lookup failed: {e}"))?;
            targets.extend(webhooks.into_iter().map(|w| WebhookTarget {
                label: w.public_id,
                url: w.url,
                secret: w.secret,
            }));
        }

        // Per-task push configs (EVE-682): DB-persisted (endpoint-created) plus
        // spec-embedded (spawn-time) configs share this delivery path. Both are
        // filtered by whether their event_filter includes this event.
        let configs = self
            .db
            .list_task_push_configs(task.session_id, &task.id)
            .await
            .map_err(|e| anyhow::anyhow!("webhook notifier: push-config lookup failed: {e}"))?;
        targets.extend(
            configs
                .into_iter()
                .filter(|c| c.event_filter.iter().any(|f| f == event.filter_value()))
                .map(|c| WebhookTarget {
                    label: c.public_id,
                    url: c.url,
                    secret: c.secret,
                }),
        );
        targets.extend(spec_push_config_targets(&task.spec, event));

        if targets.is_empty() {
            return Ok(());
        }

        let payload = serde_json::json!({
            "event": event.event_name(),
            "task": {
                "id": task.id,
                "display_name": task.display_name,
                "kind": task.kind,
                "state": task.state.to_string(),
                "session_id": task.session_id,
                "summary": task.summary,
                "result_path": task.result_path,
            }
        });
        let body = serde_json::to_vec(&payload)?;

        for target in targets {
            let req = build_task_webhook_request(&target.url, &body, target.secret.as_deref());

            if let Err(e) = self.egress_service.send(req).await {
                tracing::warn!(
                    webhook = %target.label,
                    url = %target.url,
                    task_id = %task.id,
                    event = ?event,
                    "Task webhook delivery failed (best-effort): {e}"
                );
            }
        }

        Ok(())
    }
}

/// Build the egress request for a task-webhook delivery.
///
/// TM-API-020 (EVE-625): task-webhook URLs are org-configured and SSRF-validated
/// only at create time. Delivery must pin DNS to the IPs resolved during the
/// request-time SSRF check, otherwise a DNS rebind between create and delivery
/// can point the previously-validated host at a private IP / cloud metadata
/// endpoint. `require_dns_pinning()` closes that rebinding window.
fn build_task_webhook_request(url: &str, body: &[u8], secret: Option<&str>) -> EgressRequest {
    let mut req = EgressRequest::new("POST", url, EgressRequestKind::Integration)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .timeout_ms(10_000)
        .require_dns_pinning();

    if let Some(secret) = secret {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        req = req.header("X-Everruns-Signature", format!("sha256={sig}"));
    }

    req
}

#[cfg(test)]
mod tests;
