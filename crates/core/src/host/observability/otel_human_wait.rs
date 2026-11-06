//! Completed, backdated human waits survive listener and worker replacement.
use super::*;
use crate::human_wait::{METADATA_KEY, ResolvedWait, WaitKind, WaitOutcome};
use sha2::{Digest, Sha256};

impl OtelEventListener {
    pub(super) fn handle_human_waits(&self, event: &Event) {
        if !crate::human_wait::may_resolve(&event.data) {
            return;
        }
        let Some(waits) = event
            .metadata
            .as_ref()
            .and_then(|m| m.get(METADATA_KEY))
            .and_then(serde_json::Value::as_array)
        else {
            return;
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.wait_events.contains(&event.id) {
            return;
        }
        if state.wait_events.len() >= 4096 {
            state.wait_events.pop_front();
        }
        state.wait_events.push_back(event.id);
        drop(state);
        for value in waits.iter().take(256) {
            let Ok(resolved) = serde_json::from_value::<ResolvedWait>(value.clone()) else {
                continue;
            };
            let wait = resolved.wait;
            if Some(wait.turn_id) != event.context.turn_id || wait.started_at > event.ts {
                continue;
            }
            // Stable request identity lets backends correlate the same durable wait.
            let digest = Sha256::digest(format!(
                "{}:{}:{}:{}",
                event.session_id, wait.turn_id, wait.tool_call_id, wait.started_at
            ));
            let request_id: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            let parent_metadata =
                serde_json::json!({crate::causal::TURN_TRACEPARENT_KEY: wait.turn_traceparent});
            let anchor = turn_anchor(Some(&parent_metadata));
            let parent = anchor.map(|cx| Context::new().with_remote_span_context(cx));
            // Deliberately don't inherit provider attrs, prompts, result/error text,
            // tool arguments or URLs, even with capture-content enabled.
            let kind = match wait.kind {
                WaitKind::Approval => "approval",
                WaitKind::Question => "question",
            };
            let outcome = match resolved.outcome {
                WaitOutcome::Approved => "approved",
                WaitOutcome::Rejected => "rejected",
                WaitOutcome::NotApproved => "not_approved",
                WaitOutcome::Answered => "answered",
                WaitOutcome::Declined => "declined",
                WaitOutcome::Timeout => "timeout",
                WaitOutcome::Cancelled => "cancelled",
                WaitOutcome::Failed => "failed",
                WaitOutcome::Abandoned => "abandoned",
                WaitOutcome::Unknown => "unknown",
            };
            let builder = self
                .tracer
                .span_builder("human.wait")
                .with_kind(SpanKind::Internal)
                .with_start_time(SystemTime::from(wait.started_at))
                .with_attributes(vec![
                    KeyValue::new(gen_ai::CONVERSATION_ID, event.session_id.to_string()),
                    KeyValue::new(everruns_attr::TURN_ID, wait.turn_id.to_string()),
                    KeyValue::new("everruns.human_wait.kind", kind),
                    KeyValue::new("everruns.human_wait.request_id", request_id),
                    KeyValue::new("everruns.human_wait.outcome", outcome),
                    KeyValue::new("gen_ai.tool.call.id", wait.tool_call_id),
                    KeyValue::new(
                        "everruns.human_wait.duration_ms",
                        (event.ts - wait.started_at).num_milliseconds(),
                    ),
                ]);
            let mut span = self
                .tracer
                .build_with_context(builder, parent.as_ref().unwrap_or(&Context::new()));
            span.end_with_timestamp(event.ts.into());
        }
    }
}
