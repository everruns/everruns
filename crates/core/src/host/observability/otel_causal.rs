//! Real trace anchors committed with turns; input carriers remain link-only.
use super::*;

/// Kept separate from span delivery: a slow exporter must not hold the lock
/// used by producer-side causal lookups.
#[derive(Default)]
pub(super) struct TurnContexts {
    contexts: HashMap<String, SpanContext>,
    pending: HashMap<String, BoxedSpan>,
}

impl OtelEventListener {
    pub(super) fn phase_causes(&self, event: &Event, has_parent: bool) -> CausalContext {
        // Recovery continuity takes precedence when input fan-in fills the cap.
        let mut causes = if has_parent {
            CausalContext::default()
        } else {
            self.causal_context_for_turn(&event.context)
        };
        causes.merge(&CausalContext::from_event_metadata(event.metadata.as_ref()));
        causes
    }

    pub(super) fn trace_anchor(&self, key: &str) -> Option<SpanContext> {
        self.causal.lock().unwrap().contexts.get(key).cloned()
    }

    pub(super) fn commit_trace_anchor(&self, key: &str, ts: SystemTime) {
        let pending = self.causal.lock().unwrap().pending.remove(key);
        if let Some(mut span) = pending {
            span.end_with_timestamp(ts);
        }
    }

    pub(super) fn finish_trace_anchor(&self, key: &str, ts: SystemTime) -> Option<Context> {
        self.commit_trace_anchor(key, ts);
        self.causal
            .lock()
            .unwrap()
            .contexts
            .remove(key)
            .map(|cx| Context::new().with_remote_span_context(cx))
    }

    pub(super) fn start_span(
        &self,
        name: String,
        kind: SpanKind,
        start: SystemTime,
        attributes: Vec<KeyValue>,
        parent: Option<&Context>,
    ) -> (BoxedSpan, Context) {
        self.start_causal_span(
            name,
            kind,
            start,
            attributes,
            parent,
            &CausalContext::default(),
        )
    }

    /// Attributes every span carries: conversation/session correlation plus
    /// Everruns ids.
    pub(super) fn common_attributes(&self, event: &Event, oi_kind: &'static str) -> Vec<KeyValue> {
        let session = event.session_id.to_string();
        let mut attrs = Vec::new();
        if self.conventions.gen_ai {
            attrs.push(KeyValue::new(gen_ai::CONVERSATION_ID, session.clone()));
        }
        if self.conventions.openinference {
            attrs.push(KeyValue::new(oi::SPAN_KIND, oi_kind));
            attrs.push(KeyValue::new(oi::SESSION_ID, session));
        }
        if let Some(turn_id) = &event.context.turn_id {
            attrs.push(KeyValue::new(everruns_attr::TURN_ID, turn_id.to_string()));
        }
        if let Some(exec_id) = &event.context.exec_id {
            attrs.push(KeyValue::new(everruns_attr::EXEC_ID, exec_id.to_string()));
        }
        if let Some(id) = &event.context.input_message_id {
            attrs.push(KeyValue::new(
                everruns_attr::INPUT_MESSAGE_ID,
                id.to_string(),
            ));
        }
        attrs.extend(super::super::provider_attrs::otel_provider_attributes(
            event,
        ));
        attrs
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_causal_span(
        &self,
        name: String,
        kind: SpanKind,
        start: SystemTime,
        attributes: Vec<KeyValue>,
        parent: Option<&Context>,
        causes: &CausalContext,
    ) -> (BoxedSpan, Context) {
        let builder = self
            .tracer
            .span_builder(name)
            .with_kind(kind)
            .with_start_time(start)
            .with_attributes(attributes)
            .with_links(
                causes
                    .links()
                    .iter()
                    .map(|link| Link::new(causal_span_context(link), Vec::new(), 0))
                    .collect(),
            );
        let root = Context::new();
        let span = self
            .tracer
            .build_with_context(builder, parent.unwrap_or(&root));
        let cx = Context::new().with_remote_span_context(span.span_context().clone());
        (span, cx)
    }

    pub(super) fn prepare_turn_trace(
        &self,
        request: &crate::events::EventRequest,
    ) -> Option<String> {
        self.restore_turn_context(&request.context, request.metadata.as_ref());
        let EventData::TurnStarted(data) = &request.data else {
            return None;
        };
        let key = format!("turn:{}", data.turn_id);
        let mut state = self.causal.lock().unwrap();
        let anchor = if let Some(anchor) = state.contexts.get(&key) {
            anchor.clone()
        } else {
            // Bounded independently of observer queues. An abandoned pre-commit
            // attempt must never grow this registry without limit.
            if state.contexts.len() >= 4096 {
                return None;
            }
            let causes = CausalContext::from_event_metadata(request.metadata.as_ref());
            let (span, cx) = self.start_causal_span(
                "durable.turn".into(),
                SpanKind::Internal,
                request.ts.into(),
                vec![
                    KeyValue::new(everruns_attr::TURN_ID, data.turn_id.to_string()),
                    KeyValue::new(gen_ai::CONVERSATION_ID, request.session_id.to_string()),
                ],
                None,
                &causes,
            );
            let anchor = cx.span().span_context().clone();

            if !anchor.is_valid() {
                return None;
            }
            state.pending.insert(key.clone(), span);
            state.contexts.insert(key, anchor.clone());
            anchor
        };
        Some(traceparent(&anchor))
    }

    pub(super) fn causal_context_for_turn(
        &self,
        context: &crate::events::EventContext,
    ) -> CausalContext {
        context
            .turn_id
            .and_then(|turn| {
                self.causal
                    .lock()
                    .unwrap()
                    .contexts
                    .get(&format!("turn:{turn}"))
                    .cloned()
            })
            .map(|anchor| CausalContext::from_traceparents([traceparent(&anchor).as_str()]))
            .unwrap_or_default()
    }

    pub(super) fn restore_trace_anchor(
        &self,
        context: &crate::events::EventContext,
        metadata: Option<&serde_json::Value>,
    ) {
        if let (Some(turn), Some(anchor)) = (context.turn_id, turn_anchor(metadata)) {
            let mut state = self.causal.lock().unwrap();
            if state.contexts.len() < 4096 {
                state
                    .contexts
                    .entry(format!("turn:{turn}"))
                    .or_insert(anchor);
            }
        }
    }
}

fn causal_span_context(link: &CausalLink) -> SpanContext {
    SpanContext::new(
        TraceId::from(link.trace_id()),
        SpanId::from(link.span_id()),
        TraceFlags::default(),
        true,
        TraceState::default(),
    )
}

const TURN_ANCHOR_KEY: &str = "everruns.turn_traceparent";

pub(super) fn traceparent(context: &SpanContext) -> String {
    format!(
        "00-{}-{}-{:02x}",
        context.trace_id(),
        context.span_id(),
        context.trace_flags().to_u8() & 1
    )
}

pub(super) fn turn_anchor(metadata: Option<&serde_json::Value>) -> Option<SpanContext> {
    let value = metadata?.get(TURN_ANCHOR_KEY)?.as_str()?;
    let link = CausalLink::from_traceparent(value)?;
    // This value comes only from the committed turn event, never input metadata.
    Some(SpanContext::new(
        TraceId::from(link.trace_id()),
        SpanId::from(link.span_id()),
        if value.ends_with("01") {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        },
        true,
        TraceState::default(),
    ))
}
