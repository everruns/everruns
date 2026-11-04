//! Bounded, exporter-neutral causal links retained with durable input.
//!
//! Carriers contain only W3C trace/span identities. They are hints for links,
//! never parents, sampling instructions, authorization, or executable state.
use std::collections::HashMap;

use serde_json::Value;

/// Host-generated anchor on committed turn/phase events; never input metadata.
pub const TURN_TRACEPARENT_KEY: &str = "everruns.turn_traceparent";

/// Reserved causal identities shared by messages and canonical phase events.
pub const CAUSAL_LINKS_KEY: &str = "everruns.causal_links";
/// Bound both retained causes and work spent inspecting an untrusted list.
pub const MAX_CAUSAL_LINKS: usize = 16;
pub(crate) const TASK_CAUSAL_CONTEXT_KEY: &str = "__everruns_causal_links";

/// A validated trace/span identity, with no baggage or vendor state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CausalLink {
    trace_id: u128,
    span_id: u64,
}

impl CausalLink {
    /// Accept only canonical W3C version 00 traceparents. Flags are validated
    /// but deliberately discarded: remote callers cannot select sampling.
    pub fn from_traceparent(value: &str) -> Option<Self> {
        if value.len() != 55 || !value.as_bytes().starts_with(b"00-") {
            return None;
        }
        let bytes = value.as_bytes();
        if bytes[35] != b'-'
            || bytes[52] != b'-'
            || !bytes.iter().enumerate().all(|(i, b)| {
                matches!(i, 2 | 35 | 52) || b.is_ascii_digit() || (b'a'..=b'f').contains(b)
            })
        {
            return None;
        }
        let trace_id = u128::from_str_radix(&value[3..35], 16).ok()?;
        let span_id = u64::from_str_radix(&value[36..52], 16).ok()?;
        (trace_id != 0 && span_id != 0).then_some(Self { trace_id, span_id })
    }

    pub fn trace_id(&self) -> u128 {
        self.trace_id
    }
    pub fn span_id(&self) -> u64 {
        self.span_id
    }
    pub fn traceparent(&self) -> String {
        format!("00-{:032x}-{:016x}-00", self.trace_id, self.span_id)
    }
}

/// Ordered, deduplicated causes. No OpenTelemetry dependency is needed to
/// persist these across volatile, SQLite and Platform execution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CausalContext(Vec<CausalLink>);

impl CausalContext {
    /// Retain bounded identities in the task's private spec projection.
    pub fn write_task_context(&self, spec: &mut Value) {
        if let Some(spec) = spec.as_object_mut() {
            if self.is_empty() {
                spec.remove(TASK_CAUSAL_CONTEXT_KEY);
            } else {
                spec.insert(TASK_CAUSAL_CONTEXT_KEY.into(), self.value());
            }
        }
    }

    pub fn from_traceparents<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut context = Self::default();
        for value in values.into_iter().take(MAX_CAUSAL_LINKS) {
            if let Some(link) = CausalLink::from_traceparent(value) {
                context.push(link);
            }
        }
        context
    }

    pub fn links(&self) -> &[CausalLink] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn merge(&mut self, other: &Self) {
        for link in &other.0 {
            self.push(link.clone());
        }
    }

    fn push(&mut self, link: CausalLink) {
        if self.0.len() < MAX_CAUSAL_LINKS && !self.0.contains(&link) {
            self.0.push(link);
        }
    }

    fn from_value(value: Option<&Value>) -> Self {
        let Some(Value::Array(values)) = value else {
            return Self::default();
        };
        let mut context = Self::default();
        // THREAT[TM-OBS-015]: bound inspection before filtering, not after it.
        for value in values.iter().take(MAX_CAUSAL_LINKS) {
            if let Some(link) = value.as_str().and_then(CausalLink::from_traceparent) {
                context.push(link);
            }
        }
        context
    }

    pub fn from_metadata(metadata: Option<&HashMap<String, Value>>) -> Self {
        Self::from_value(metadata.and_then(|m| m.get(CAUSAL_LINKS_KEY)))
    }

    /// Read a host-generated trace anchor from a committed turn event.
    pub fn from_turn_metadata(metadata: Option<&Value>) -> Self {
        metadata
            .and_then(|m| m.get(TURN_TRACEPARENT_KEY))
            .and_then(Value::as_str)
            .map(|value| Self::from_traceparents([value]))
            .unwrap_or_default()
    }

    pub fn from_event_metadata(metadata: Option<&Value>) -> Self {
        Self::from_value(metadata.and_then(|m| m.get(CAUSAL_LINKS_KEY)))
    }

    pub fn write_metadata(&self, metadata: &mut Option<HashMap<String, Value>>) {
        if self.is_empty() {
            if let Some(metadata) = metadata {
                metadata.remove(CAUSAL_LINKS_KEY);
            }
        } else {
            metadata
                .get_or_insert_with(Default::default)
                .insert(CAUSAL_LINKS_KEY.into(), self.value());
        }
    }

    pub fn event_metadata(&self) -> Option<Value> {
        (!self.is_empty()).then(|| serde_json::json!({ CAUSAL_LINKS_KEY: self.value() }))
    }

    fn value(&self) -> Value {
        Value::Array(
            self.0
                .iter()
                .map(|link| Value::String(link.traceparent()))
                .collect(),
        )
    }
}

/// Recover real exporter identities stored in a task origin context.
pub fn event_causal_context(context: &super::events::EventContext) -> CausalContext {
    match (&context.trace_id, &context.span_id) {
        (Some(trace), Some(span)) if trace.len() == 32 && span.len() == 16 => {
            CausalContext::from_traceparents([format!("00-{trace}-{span}-00").as_str()])
        }
        _ => CausalContext::default(),
    }
}

/// Reconstruct a task's original turn after journal/registry recovery.
pub fn task_causal_context(task: &super::session_task::SessionTask) -> CausalContext {
    if let Some(value) = task.spec.get(TASK_CAUSAL_CONTEXT_KEY) {
        return CausalContext::from_value(Some(value));
    }
    super::session_task::task_origin_event_context(task)
        .map(|context| event_causal_context(&context))
        .unwrap_or_default()
}

impl super::events::EventRequest {
    /// Retain only validated causal identities on a committed lifecycle event.
    pub fn with_causal_context(mut self, context: &CausalContext) -> Self {
        if let Some(Value::Object(causes)) = context.event_metadata() {
            let metadata = self.metadata.get_or_insert_with(|| serde_json::json!({}));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.extend(causes);
            }
        }
        self
    }
}

/// Attach an observer-created identity without granting observers mutation of
/// event data, execution context, or arbitrary metadata.
pub fn attach_turn_trace(request: &mut super::events::EventRequest, carrier: &str) {
    if !matches!(request.data, super::events::EventData::TurnStarted(_))
        || CausalLink::from_traceparent(carrier).is_none()
    {
        return;
    }
    let metadata = request
        .metadata
        .get_or_insert_with(|| serde_json::json!({}));
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert(TURN_TRACEPARENT_KEY.into(), Value::String(carrier.into()));
    }
}

/// Canonicalize the untrusted input carrier and reserve host-only trace state.
pub fn sanitize_input_metadata(metadata: &mut Option<HashMap<String, Value>>) {
    CausalContext::from_metadata(metadata.as_ref()).write_metadata(metadata);
    if let Some(values) = metadata {
        values.remove(TURN_TRACEPARENT_KEY);
        if values.is_empty() {
            *metadata = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VALID: &str = "00-1234567890abcdef1234567890abcdef-1234567890abcdef-01";

    #[test]
    fn untrusted_carriers_are_bounded_and_identity_only() {
        let hostile = format!("00-{}é{}", "0".repeat(31), "0".repeat(19));
        assert_eq!(hostile.len(), 55);
        assert!(CausalLink::from_traceparent(&hostile).is_none());
        let contexts: Vec<_> = (1..=32)
            .map(|n| format!("00-{n:032x}-{n:016x}-01"))
            .collect();
        assert_eq!(
            CausalContext::from_traceparents(contexts.iter().map(String::as_str))
                .links()
                .len(),
            MAX_CAUSAL_LINKS
        );
        let valid = CausalLink::from_traceparent(VALID).unwrap();
        assert!(valid.traceparent().ends_with("-00"));
        for invalid in [
            "",
            "é",
            "00-00000000000000000000000000000000-1234567890abcdef-00",
            "00-1234567890abcdef1234567890abcdef-0000000000000000-00",
            "00-1234567890abcdef1234567890abcdef-1234567890ABCDEF-00",
            "ff-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
        ] {
            assert!(CausalLink::from_traceparent(invalid).is_none());
        }
        let mut values = vec![Value::Null; MAX_CAUSAL_LINKS];
        values.push(Value::String(VALID.into()));
        assert!(CausalContext::from_value(Some(&Value::Array(values))).is_empty());
        let context = CausalContext::from_traceparents([VALID, VALID]);
        assert_eq!(context.links().len(), 1);
        let metadata = context.event_metadata();
        assert_eq!(
            CausalContext::from_event_metadata(metadata.as_ref()),
            context
        );
        let secrets = serde_json::json!({ CAUSAL_LINKS_KEY: [VALID, {"baggage":"SECRET"}], "tracestate":"SECRET" });
        assert!(
            !CausalContext::from_event_metadata(Some(&secrets))
                .event_metadata()
                .unwrap()
                .to_string()
                .contains("SECRET")
        );
    }
}
