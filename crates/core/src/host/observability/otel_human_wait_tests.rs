use super::*;
use crate::{
    events::{EventRequest, ToolCallRequestedData},
    human_wait::HumanWaitState,
};

#[tokio::test]
async fn human_wait_export_survives_restart_and_omits_content() {
    let first = OtelHarness::new(false, TraceConventions::ALL);
    let mut start = EventRequest::new(
        first.session,
        first.context(None, None, None),
        first.turn_started(),
    );
    let carrier = first.listener.prepare_causal_context(&start).unwrap();
    crate::causal::attach_turn_trace(&mut start, &carrier);
    let start = start.into_event(everruns_contracts::typed_id::EventId::new(), 1);
    first.listener.on_event(&start).await;
    let mut state = HumanWaitState::default();
    state.observe(&start);
    let mut asked = Event::new(
        first.session,
        start.context.clone(),
        ToolCallRequestedData::with_definitions(
            &[ToolCall {
                id: "question-1".into(),
                name: "ask_user".into(),
                arguments: json!({"question":"PRIVATE_PROMPT", "url":"https://secret.example/token", "password":"PRIVATE_KEY"}),
            }],
            &[],
        ),
    );
    asked.ts = first.at(10);
    state.observe(&asked);
    let state: HumanWaitState =
        serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    let mut completed = EventRequest::new(
        first.session,
        start.context.clone(),
        ToolCompletedData::success(
            "question-1".into(),
            "ask_user".into(),
            vec![ContentPart::tool_result_text(
                &json!({"status":"declined", "answered_by":"timeout", "answers":["PRIVATE_ANSWER"]}),
            )],
            None,
        ),
    );
    completed.ts = first.at(6010);
    state.annotate(&mut completed);
    let completed: Event = serde_json::from_value(
        serde_json::to_value(completed.into_event(everruns_contracts::typed_id::EventId::new(), 3))
            .unwrap(),
    )
    .unwrap();
    let recovered = OtelHarness::new(false, TraceConventions::ALL);
    // No start/request replay and no live turn in the recovered listener.
    recovered.listener.on_event(&completed).await;
    recovered.listener.on_event(&completed).await;
    let spans = recovered.spans();
    let waits: Vec<_> = spans
        .iter()
        .filter(|span| span.name == "human.wait")
        .collect();
    assert_eq!(waits.len(), 1);
    let wait = waits[0];
    assert_eq!(wait.start_time, SystemTime::from(first.at(10)));
    assert_eq!(wait.end_time, SystemTime::from(first.at(6010)));
    assert_eq!(
        attr_str(wait, "everruns.human_wait.duration_ms").as_deref(),
        Some("6000")
    );
    assert_eq!(
        attr_str(wait, "everruns.human_wait.outcome").as_deref(),
        Some("timeout")
    );
    let anchor = causal::turn_anchor(start.metadata.as_ref()).unwrap();
    assert_eq!(wait.span_context.trace_id(), anchor.trace_id());
    assert_eq!(wait.parent_span_id, anchor.span_id());
    assert!(!format!("{spans:?}").contains("PRIVATE"));
    assert!(!format!("{spans:?}").contains("https://secret"));
    // Missing metadata on legacy completions never fabricates a duration.
    let mut legacy = completed;
    legacy.metadata = None;
    recovered.listener.on_event(&legacy).await;
    assert_eq!(
        recovered
            .spans()
            .iter()
            .filter(|span| span.name == "human.wait")
            .count(),
        1
    );
}
