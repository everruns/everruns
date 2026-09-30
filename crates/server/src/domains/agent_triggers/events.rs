//! Trigger event pipeline: the one path every event source takes into a run.
//!
//! Design decision: sources (the durable scheduler, webhook ingress, GitHub App
//! deliveries, MCP events later) only authenticate and normalize what they
//! receive into a [`TriggerEvent`]. Everything after that is shared and lives
//! here, so each source gets the same guarantees:
//!
//! 1. **Filter** against the trigger's conditions. A miss is recorded as a
//!    `filtered` delivery and starts nothing.
//! 2. **Deduplicate** on the source's event id. The delivery row claims the id
//!    under a partial unique index, so a redelivered event is recorded as a
//!    `duplicate` instead of a second run, even across concurrent requests.
//! 3. **Route to a session**: shared, per invocation, or per subject
//!    (`per_thread` keyed on the event's subject, e.g. one session per pull
//!    request).
//! 4. **Dispatch** the rendered message on the agent's harness as the agent's
//!    own identity, and record the outcome.
//!
//! Delivery history is bounded per trigger ([`DELIVERY_HISTORY_LIMIT`]) so a
//! busy source cannot grow the table without limit.

use super::commands::{
    AgentTriggerInvocationResult, TriggerSessionRoute, WebhookCompatibilityContext,
    dispatch_trigger_message, emit_agent_trigger_audit_event, find_or_create_trigger_session,
    resolve_trigger_execution_context,
};
use crate::domains::apps::invocation::{render_message_template, template_lookup};
use crate::domains::common::{CommandError, classify_anyhow};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use crate::storage::models::{AgentRow, AgentTriggerRow, CreateAgentTriggerDeliveryRow};
use everruns_platform::{SessionBinding, TriggerDeliveryStatus, TriggerEventFilter};
use serde_json::Value;
use std::sync::Arc;

/// Deliveries kept per trigger; older rows are pruned after each event.
pub const DELIVERY_HISTORY_LIMIT: i64 = 200;

/// Longest event id or subject we store; longer values are truncated.
const MAX_EVENT_FIELD_LEN: usize = 512;

/// A normalized event, as handed to the pipeline by a source.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// Source name recorded on the delivery (`schedule`, `webhook`, `github`).
    pub source: &'static str,
    /// Idempotency key from the source, if it has one.
    pub event_id: Option<String>,
    /// Source event type, if it has one (e.g. `pull_request.opened`).
    pub event_type: Option<String>,
    /// What the event is about (e.g. `owner/repo#12`); keys `per_thread` sessions.
    pub subject: Option<String>,
    /// Template and filter context. The pipeline adds an `event` object with
    /// the fields above before filtering and rendering.
    pub context: Value,
}

/// How one trigger should handle an event.
pub struct TriggerEventRoute<'a> {
    pub trigger: &'a AgentTriggerRow,
    pub agent: &'a AgentRow,
    pub message_template: &'a str,
    pub session_mode: SessionBinding,
    pub filter: Option<&'a TriggerEventFilter>,
    pub session_source: everruns_platform::SessionSource,
    pub webhook_compat: Option<&'a WebhookCompatibilityContext>,
}

/// Result of handing one event to one trigger.
#[derive(Debug, Clone)]
pub enum TriggerEventOutcome {
    Dispatched(AgentTriggerInvocationResult),
    Filtered,
    Duplicate,
}

/// Run one event through filter, dedupe, session routing and dispatch.
pub async fn dispatch_trigger_event(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    route: TriggerEventRoute<'_>,
    mut event: TriggerEvent,
    request_id: Option<String>,
) -> Result<TriggerEventOutcome, CommandError> {
    event.event_id = clean_field(event.event_id);
    event.subject = clean_field(event.subject);
    event.event_type = clean_field(event.event_type);
    if let Value::Object(map) = &mut event.context {
        map.insert(
            "event".to_string(),
            serde_json::json!({
                "source": event.source,
                "id": event.event_id,
                "type": event.event_type,
                "subject": event.subject,
            }),
        );
    }

    let trigger = route.trigger;
    let new_row =
        |status: TriggerDeliveryStatus, reason: Option<String>| CreateAgentTriggerDeliveryRow {
            org_id: trigger.org_id,
            trigger_id: trigger.id,
            source: event.source.to_string(),
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            subject: event.subject.clone(),
            status: status.as_str().to_string(),
            reason,
        };

    if let Some(reason) = route
        .filter
        .and_then(|filter| filter_miss(filter, &event.context))
    {
        let claimed = db
            .record_agent_trigger_delivery(new_row(TriggerDeliveryStatus::Filtered, Some(reason)))
            .await
            .map_err(classify_anyhow)?;
        if claimed.is_none() {
            record_duplicate(db, new_row(TriggerDeliveryStatus::Duplicate, None)).await?;
            return Ok(TriggerEventOutcome::Duplicate);
        }
        prune(db, trigger).await;
        return Ok(TriggerEventOutcome::Filtered);
    }

    let Some(delivery) = db
        .record_agent_trigger_delivery(new_row(TriggerDeliveryStatus::Dispatched, None))
        .await
        .map_err(classify_anyhow)?
    else {
        record_duplicate(db, new_row(TriggerDeliveryStatus::Duplicate, None)).await?;
        return Ok(TriggerEventOutcome::Duplicate);
    };

    let result = run(
        db,
        session_service,
        message_service,
        &route,
        &event,
        request_id,
    )
    .await;
    let (status, reason, session_id) = match &result {
        Ok(result) => (
            TriggerDeliveryStatus::Dispatched,
            None,
            Some(result.session_id.uuid()),
        ),
        Err(error) => (
            TriggerDeliveryStatus::Failed,
            Some(truncate(&error.to_string(), MAX_EVENT_FIELD_LEN)),
            None,
        ),
    };
    if let Err(error) = db
        .finish_agent_trigger_delivery(delivery.id, status.as_str(), reason.as_deref(), session_id)
        .await
    {
        tracing::warn!(%error, delivery_id = %delivery.id, "failed to record trigger delivery outcome");
    }
    prune(db, trigger).await;
    result.map(TriggerEventOutcome::Dispatched)
}

async fn run(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    route: &TriggerEventRoute<'_>,
    event: &TriggerEvent,
    request_id: Option<String>,
) -> Result<AgentTriggerInvocationResult, CommandError> {
    let rendered_message = render_message_template(route.message_template, &event.context);
    if rendered_message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }
    let trigger = route.trigger;
    let agent = route.agent;
    let execution_context =
        resolve_trigger_execution_context(db, trigger.org_id, agent, trigger).await?;
    let (session_id, created_session) = find_or_create_trigger_session(
        db,
        session_service,
        trigger.org_id,
        agent,
        &execution_context,
        TriggerSessionRoute {
            trigger_id: trigger.id,
            session_mode: route.session_mode,
            subject: event.subject.as_deref(),
            source: route.session_source,
            webhook: route.webhook_compat,
        },
    )
    .await?;
    dispatch_trigger_message(
        message_service,
        trigger.org_id,
        agent,
        trigger.id,
        session_id,
        execution_context.harness_id,
        execution_context.owner_principal_id,
        rendered_message,
        request_id,
    )
    .await?;
    emit_agent_trigger_audit_event(
        Arc::clone(db),
        trigger.org_id,
        agent,
        trigger.id,
        session_id,
        execution_context.owner_principal_id,
        created_session,
    );
    Ok(AgentTriggerInvocationResult {
        session_id,
        created_session,
    })
}

async fn record_duplicate(
    db: &Arc<StorageBackend>,
    row: CreateAgentTriggerDeliveryRow,
) -> Result<(), CommandError> {
    db.record_agent_trigger_delivery(row)
        .await
        .map_err(classify_anyhow)?;
    Ok(())
}

async fn prune(db: &Arc<StorageBackend>, trigger: &AgentTriggerRow) {
    if let Err(error) = db
        .prune_agent_trigger_deliveries(trigger.id, DELIVERY_HISTORY_LIMIT)
        .await
    {
        tracing::warn!(%error, trigger_id = %trigger.id, "failed to prune trigger deliveries");
    }
}

/// Return why the event misses the filter, or `None` when every condition holds.
pub fn filter_miss(filter: &TriggerEventFilter, context: &Value) -> Option<String> {
    filter.conditions.iter().find_map(|condition| {
        let actual = template_lookup(context, &condition.path);
        let matched = actual.is_some_and(|actual| {
            condition
                .any_of
                .iter()
                .any(|expected| values_match(actual, expected))
        });
        (!matched).then(|| {
            let shown = actual
                .map(|value| truncate(&value_label(value), 80))
                .unwrap_or_else(|| "missing".to_string());
            format!("{} is {shown}", condition.path)
        })
    })
}

fn values_match(actual: &Value, expected: &Value) -> bool {
    if actual == expected {
        return true;
    }
    // A string condition matches the same scalar written differently
    // (`"12"` matches `12`, `"true"` matches `true`), since conditions are
    // often typed in a form.
    match (actual, expected) {
        (Value::Number(number), Value::String(text)) => number.to_string() == text.as_str(),
        (Value::Bool(flag), Value::String(text)) => text.parse::<bool>().ok() == Some(*flag),
        _ => false,
    }
}

fn value_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn clean_field(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| truncate(&value, MAX_EVENT_FIELD_LEN))
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_platform::TriggerFilterCondition;
    use serde_json::json;

    fn filter(conditions: Vec<(&str, Vec<Value>)>) -> TriggerEventFilter {
        TriggerEventFilter {
            conditions: conditions
                .into_iter()
                .map(|(path, any_of)| TriggerFilterCondition {
                    path: path.to_string(),
                    any_of,
                })
                .collect(),
        }
    }

    #[test]
    fn filter_passes_when_every_condition_matches() {
        let context = json!({"payload": {"action": "opened", "pull_request": {"draft": false}}});
        let filter = filter(vec![
            (
                "payload.action",
                vec![json!("opened"), json!("synchronize")],
            ),
            ("payload.pull_request.draft", vec![json!(false)]),
        ]);
        assert_eq!(filter_miss(&filter, &context), None);
    }

    #[test]
    fn filter_reports_the_first_failing_condition() {
        let context = json!({"payload": {"action": "labeled"}});
        let filter = filter(vec![("payload.action", vec![json!("opened")])]);
        assert_eq!(
            filter_miss(&filter, &context).as_deref(),
            Some("payload.action is labeled")
        );
    }

    #[test]
    fn filter_treats_a_missing_path_as_a_miss() {
        let filter = filter(vec![("payload.action", vec![json!("opened")])]);
        assert_eq!(
            filter_miss(&filter, &json!({})).as_deref(),
            Some("payload.action is missing")
        );
    }

    #[test]
    fn string_conditions_match_scalars() {
        let context = json!({"payload": {"number": 12, "draft": true}});
        let filter = filter(vec![
            ("payload.number", vec![json!("12")]),
            ("payload.draft", vec![json!("true")]),
        ]);
        assert_eq!(filter_miss(&filter, &context), None);
    }

    #[test]
    fn empty_filter_passes() {
        assert_eq!(
            filter_miss(&TriggerEventFilter::default(), &json!({})),
            None
        );
    }

    #[test]
    fn fields_are_trimmed_and_bounded() {
        assert_eq!(clean_field(Some("  ".to_string())), None);
        assert_eq!(clean_field(Some(" a ".to_string())).as_deref(), Some("a"));
        let long = "é".repeat(MAX_EVENT_FIELD_LEN);
        let cleaned = clean_field(Some(long)).unwrap();
        assert!(cleaned.len() <= MAX_EVENT_FIELD_LEN);
    }
}
