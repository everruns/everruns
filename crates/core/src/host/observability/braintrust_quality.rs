//! Privacy and completeness guarantees shared by Braintrust event conversions.
use super::{BraintrustMetrics, BraintrustState, Event, Ordering};

/// Error strings and cancellation reasons may contain user/tool content.
/// Keep a non-empty marker so the receiver still classifies failed spans.
pub(super) fn private_text(value: &str, capture: bool) -> String {
    if capture {
        value.to_owned()
    } else {
        "[REDACTED]".into()
    }
}

/// An observed terminal event closes a span even if the provider omitted usage.
pub(super) fn terminal_metrics(
    event: &Event,
    duration_ms: Option<u64>,
    usage: Option<&crate::TokenUsage>,
    first_token_ms: Option<u64>,
) -> BraintrustMetrics {
    let end = event.ts.timestamp_micros() as f64 / 1_000_000.0;
    BraintrustMetrics {
        start: duration_ms.map(|ms| end - ms as f64 / 1000.0),
        end: Some(end),
        prompt_tokens: usage.map(|u| {
            u64::from(u.input_tokens)
                + u64::from(u.cache_read_tokens.unwrap_or(0))
                + u64::from(u.cache_creation_tokens.unwrap_or(0))
        }),
        completion_tokens: usage.map(|u| u.output_tokens),
        tokens: usage.map(|u| {
            u64::from(u.input_tokens)
                + u64::from(u.cache_read_tokens.unwrap_or(0))
                + u64::from(u.cache_creation_tokens.unwrap_or(0))
                + u64::from(u.output_tokens)
        }),
        time_to_first_token: first_token_ms.map(|ms| ms as f64 / 1000.0),
        cache_read_tokens: usage.and_then(|u| u.cache_read_tokens),
        cache_creation_tokens: usage.and_then(|u| u.cache_creation_tokens),
    }
}

/// A drained queue is not evidence of delivery. Failure is sticky for this
/// listener: already-dropped telemetry cannot be recovered by a later success.
pub(super) fn delivery_result(state: &BraintrustState) -> Result<(), String> {
    if state.failed_batches.load(Ordering::Relaxed) > 0
        || state.dropped_events.load(Ordering::Relaxed) > 0
    {
        Err("Braintrust delivery lost events or failed batches".into())
    } else {
        Ok(())
    }
}

impl super::BraintrustListener {
    /// Streaming events carry exec identity, not the later generation's span ID.
    /// Derive one stable row for both thinking lifecycle events, then link it to
    /// the real generation when that record arrives.
    pub(super) fn thinking_span_linkage(
        &self,
        event: &Event,
    ) -> (Option<String>, Option<String>, Option<Vec<String>>) {
        let (mut span, root, parent) = Self::compute_child_span_linkage(event);
        if let (Some(turn), Some(exec)) = (event.context.turn_id, event.context.exec_id) {
            if event.context.span_id.is_none() {
                span = Some(format!("thinking:{}:{turn}:{exec}", event.session_id));
            }
            if let Some(id) = &span {
                self.upsert_turn_state(&turn.to_string(), |state| {
                    state.thinking_spans.insert(exec.to_string(), id.clone());
                });
            }
        }
        (span, root, parent)
    }

    pub(super) fn reparent_thinking(&self, event: &Event) {
        let (Some(turn), Some(exec)) = (event.context.turn_id, event.context.exec_id) else {
            return;
        };
        let Some(id) = self
            .current_turn_state(&turn.to_string())
            .and_then(|s| s.thinking_spans.get(&exec.to_string()).cloned())
        else {
            return;
        };
        let (Some(generation), root, _) = Self::compute_child_span_linkage(event) else {
            return;
        };
        self.enqueue_event(super::BraintrustLogEvent {
            id: id.clone(),
            created: event.ts,
            input: None,
            output: None,
            error: None,
            metadata: serde_json::json!({}),
            metrics: None,
            span_attributes: super::BraintrustSpanAttributes {
                name: "thinking".into(),
                span_type: "task".into(),
            },
            tags: None,
            span_id: Some(id),
            root_span_id: root,
            span_parents: Some(vec![generation]),
            is_merge: Some(true),
        });
    }
}

/// Preserve authoritative cost provenance separately from receiver-side estimates.
/// Never turn missing prices into zero, or add root aggregates to child costs.
pub(super) fn annotate_costs(event: &Event, metadata: &mut serde_json::Value) {
    let usage = match &event.data {
        super::EventData::LlmGeneration(data) => data.metadata.usage.as_ref(),
        super::EventData::TurnCompleted(data) => data.usage.as_ref(),
        super::EventData::TurnCancelled(data) => data.usage.as_ref(),
        super::EventData::ReasonCompleted(data) => data.usage.as_ref(),
        _ => None,
    };
    if let Some(usage) = usage {
        for (key, value) in [
            ("actual_cost_usd", usage.actual_cost_usd),
            ("estimated_cost_usd", usage.estimated_cost_usd),
            ("effective_cost_usd", usage.effective_cost_usd()),
        ] {
            if let Some(value) = value.filter(|v| v.is_finite() && *v >= 0.0) {
                metadata[key] = serde_json::json!(value);
            }
        }
    }
}

impl super::BraintrustListener {
    pub(super) fn delivery_sender(
        &self,
    ) -> Option<tokio::sync::mpsc::Sender<super::DeliveryMessage>> {
        self.state
            .sender
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Drop for super::BraintrustListener {
    fn drop(&mut self) {
        // The delivery worker retains state for pending batches, but must not
        // retain its own sender forever after a destination slot is evicted.
        self.state
            .sender
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }
}

impl std::fmt::Debug for super::BraintrustConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BraintrustConfig")
            .field("api_key", &"[REDACTED]")
            .field("project_id", &self.project_id)
            .field("api_url", &self.api_url)
            .field("delivery", &self.delivery)
            .field("content", &self.content)
            .field("deployment_grade", &self.deployment_grade)
            .finish()
    }
}
