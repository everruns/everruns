use super::*;
use crate::causal::CausalContext;

fn prepared_turn(h: &OtelHarness, causes: &CausalContext) -> Event {
    let mut request =
        crate::events::EventRequest::new(h.session, h.context(None, None, None), h.turn_started())
            .with_causal_context(causes);
    if let Some(carrier) = h.listener.prepare_causal_context(&request) {
        crate::causal::attach_turn_trace(&mut request, &carrier);
    }
    request.into_event(everruns_contracts::typed_id::EventId::new(), 1)
}

#[tokio::test]
async fn causal_child_links_reference_exported_parent_without_parenting() {
    let parent = OtelHarness::new(false, TraceConventions::ALL);
    let event = prepared_turn(&parent, &CausalContext::default());
    parent.listener.on_event(&event).await;
    let causes = parent.listener.turn_causal_context(&event.context);
    parent
        .emit(
            1,
            parent.context(None, None, None),
            TurnCompletedData {
                turn_id: parent.turn,
                ..Default::default()
            },
        )
        .await;
    let exported = parent.spans();
    let root = by_name(&exported, "durable.turn");
    assert_eq!(root.parent_span_id, SpanId::INVALID);

    // Commit and deserialize before a completely independent listener receives
    // the child. No live-parent map or global subscriber participates.
    let child = OtelHarness::new(false, TraceConventions::ALL);
    let event = prepared_turn(&child, &causes);
    let event: Event = serde_json::from_value(serde_json::to_value(event).unwrap()).unwrap();
    child.listener.on_event(&event).await;
    child
        .emit(
            1,
            child.context(None, None, None),
            TurnCompletedData {
                turn_id: child.turn,
                ..Default::default()
            },
        )
        .await;
    let exported = child.spans();
    let child = by_name(&exported, "durable.turn");
    assert_eq!(child.parent_span_id, SpanId::INVALID);
    assert_ne!(child.span_context.trace_id(), root.span_context.trace_id());
    assert_eq!(child.links.links.len(), 1);
    let link = &child.links.links[0];
    assert_eq!(link.span_context.trace_id(), root.span_context.trace_id());
    assert_eq!(link.span_context.span_id(), root.span_context.span_id());
    assert!(link.attributes.is_empty());
    assert!(link.span_context.trace_state().header().is_empty());
    assert!(!link.span_context.is_sampled());
}

#[tokio::test]
async fn causal_recovered_phase_exports_links_without_replaying_turn_start() {
    let first = OtelHarness::new(false, TraceConventions::ALL);
    let committed = prepared_turn(&first, &CausalContext::default());
    first.listener.on_event(&committed).await;
    let causes = CausalContext::from_turn_metadata(committed.metadata.as_ref());
    assert_eq!(causes.links().len(), 1);
    let mut h = OtelHarness::new(false, TraceConventions::ALL);
    h.turn = first.turn;
    h.listener
        .restore_turn_context(&committed.context, committed.metadata.as_ref());
    let inputs: Vec<_> = (1..=16)
        .map(|n| format!("00-{n:032x}-{n:016x}-01"))
        .collect();
    let mut metadata = CausalContext::from_traceparents(inputs.iter().map(String::as_str))
        .event_metadata()
        .unwrap();
    metadata["baggage"] = json!("SECRET");
    let ctx = h.context(None, Some("recovered-reason"), None);
    let event = Event::new(
        h.session,
        ctx.clone(),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .with_metadata(metadata);
    h.listener.on_event(&event).await;
    h.emit(
        10,
        ctx,
        ReasonCompletedData::success("done", false, 0, None, None),
    )
    .await;
    let exported = h.spans();
    let phase = by_name(&exported, "reason");
    assert_eq!(phase.links.links.len(), crate::causal::MAX_CAUSAL_LINKS);
    assert!(
        phase
            .links
            .links
            .iter()
            .any(|link| link.span_context.span_id() == SpanId::from(causes.links()[0].span_id())),
        "recovery anchor must survive a full input fan-in"
    );
    assert_eq!(exported.len(), 1, "history was not re-exported");
    assert!(!format!("{exported:?}").contains("SECRET"));
}

#[tokio::test]
async fn causal_input_sampling_flags_cannot_override_application_sampler() {
    let mut h = OtelHarness::new(false, TraceConventions::ALL);
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(h.exporter.clone())
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOff)
        .build();
    h.listener = OtelEventListener::with_tracer_provider(provider, false, TraceConventions::ALL);
    let causes = CausalContext::from_traceparents([
        "00-1234567890abcdef1234567890abcdef-1234567890abcdef-01",
    ]);
    let event = prepared_turn(&h, &causes);
    h.listener.on_event(&event).await;
    h.emit(
        1,
        h.context(None, None, None),
        TurnCompletedData {
            turn_id: h.turn,
            ..Default::default()
        },
    )
    .await;
    assert!(h.spans().is_empty());
}

#[tokio::test]
async fn causal_sealed_turn_releases_its_anchor() {
    let h = OtelHarness::new(false, TraceConventions::ALL);
    let event = prepared_turn(&h, &CausalContext::default());
    h.listener.on_event(&event).await;
    assert!(!h.listener.turn_causal_context(&event.context).is_empty());
    h.emit(
        1,
        event.context.clone(),
        crate::events::TurnSealedData {
            turn_id: h.turn,
            reason: "finished".into(),
            detail: None,
            iterations: Some(1),
            usage: None,
        },
    )
    .await;
    assert!(h.listener.turn_causal_context(&event.context).is_empty());
    assert!(h.spans().iter().any(|s| s.name.starts_with("invoke_agent")));
}
