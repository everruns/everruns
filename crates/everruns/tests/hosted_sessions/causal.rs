//! Real Engine/authority execution with a local in-memory exporter only.
use super::*;
use everruns::observability::{OpenTelemetry, SdkTracerProvider};
use everruns::{CausalContext, FunctionTool, InputMessage, ToolCall, ToolCallContext};
use everruns_core::host::observability::InMemorySpanExporter;
use serde_json::json;
use std::time::Duration;

const A: &str = "00-1234567890abcdef1234567890abcdef-1234567890abcdef-01";
const B: &str = "00-2234567890abcdef1234567890abcdef-2234567890abcdef-01";
fn input(text: &str, cause: &str) -> InputMessage {
    InputMessage::user(text).with_causal_context(&CausalContext::from_traceparents([cause]))
}
fn observed(profile: impl SessionHosting, agent: Agent) -> (Engine, InMemorySpanExporter) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (
        Engine::builder()
            .hosting(profile)
            .register(build(), agent)
            .observe(OpenTelemetry::with_tracer_provider(provider))
            .build(),
        exporter,
    )
}
async fn wait(sent: &everruns::SentMessage) -> everruns::Turn {
    tokio::time::timeout(Duration::from_secs(10), sent.wait())
        .await
        .unwrap()
        .unwrap()
}

#[cfg(feature = "local")]
#[tokio::test]
async fn causal_batch_survives_sqlite_reopen_and_duplicate_carrier_changes() {
    use everruns::hosting::SqliteHosting;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("causal.db");
    let session_id = SessionId::new();
    let message_a = MessageId::new();
    let message_b = MessageId::new();
    {
        let first = engine(External(SqliteHosting::open(&path).unwrap()), agent());
        let session = first.create_registered(session_id, &build()).await.unwrap();
        session
            .send_with_id(message_a, input("first", A))
            .await
            .unwrap();
        session
            .send_with_id(message_b, input("second", B))
            .await
            .unwrap();
        // Retry the first request under another process/span. First commit wins.
        session
            .send_with_id(message_a, input("first", B))
            .await
            .unwrap();
    }
    let (replacement, exporter) = observed(SqliteHosting::open(&path).unwrap(), agent());
    let session = replacement.resume(session_id).await.unwrap();
    replacement.activate(session_id).await.unwrap();
    let first = session.admission(message_a).await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), first.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let second = session.admission(message_b).await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), second.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type() == "input.message")
            .count(),
        2
    );
    let accepted = events
        .iter()
        .find(|e| e.event_type() == "input.message")
        .unwrap()
        .canonical_json();
    assert_eq!(
        accepted["data"]["message"]["metadata"]["everruns.causal_links"][0],
        A.replace("-01", "-00")
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type() == "turn.started")
            .count(),
        1
    );
    replacement.shutdown(Duration::from_secs(5)).await;
    let spans = exporter.get_finished_spans().unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.name == "reason" && s.links.links.len() == 2),
        "{spans:?}"
    );
    let anchor = spans.iter().find(|s| s.name == "durable.turn").unwrap();
    assert_eq!(anchor.links.links.len(), 1);
    assert_eq!(
        anchor.links.links[0].span_context.trace_id().to_string(),
        &A[3..35]
    );
}

#[tokio::test]
async fn causal_steering_links_both_inputs_and_child_links_real_exported_anchor() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let engine = Engine::builder()
        .observe(OpenTelemetry::with_tracer_provider(provider))
        .build();
    let child_engine = engine.clone();
    let tool_entered = entered.clone();
    let tool_release = release.clone();
    let tool = FunctionTool::with_context(
        "child",
        "Run a child",
        json!({"type":"object"}),
        move |context: ToolCallContext, _: serde_json::Value| {
            let engine = child_engine.clone();
            let entered = tool_entered.clone();
            let release = tool_release.clone();
            async move {
                let causes = context.causal_context();
                assert_eq!(causes.links().len(), 1);
                entered.notify_one();
                release.notified().await;
                engine
                    .create(agent())
                    .run(InputMessage::user("child").with_causal_context(&causes))
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(json!({"done":true}))
            }
        },
    );
    let parent = Agent::builder()
        .instructions("Call child once")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_child".into(),
                    name: "child".into(),
                    arguments: json!({}),
                }],
                vec![],
            ]),
        ))
        .tool(tool)
        .build()
        .unwrap();
    let session = engine.create(parent);
    let first = session.send(input("start", A)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    let second = session.send(input("steer", B)).await.unwrap();
    release.notify_one();
    assert!(wait(&first).await.success);
    assert!(wait(&second).await.success);
    engine.shutdown(Duration::from_secs(5)).await;
    let spans = exporter.get_finished_spans().unwrap();
    let anchors: Vec<_> = spans.iter().filter(|s| s.name == "durable.turn").collect();
    assert_eq!(anchors.len(), 2, "{spans:?}");
    let parent = anchors
        .iter()
        .find(|s| s.links.links[0].span_context.trace_id().to_string() == A[3..35])
        .unwrap();
    let child = anchors
        .iter()
        .find(|s| s.span_context.trace_id() != parent.span_context.trace_id())
        .unwrap();
    assert_eq!(child.links.links.len(), 1);
    assert_eq!(
        child.links.links[0].span_context.trace_id(),
        parent.span_context.trace_id()
    );
    assert_eq!(
        child.links.links[0].span_context.span_id(),
        parent.span_context.span_id()
    );
    assert!(
        spans
            .iter()
            .any(|s| s.name == "reason" && s.links.links.len() == 2),
        "steering causes {spans:?}"
    );
    println!(
        "causal_link_evidence={}",
        json!({
            "parent_trace": parent.span_context.trace_id().to_string(),
            "parent_span": parent.span_context.span_id().to_string(),
            "child_trace": child.span_context.trace_id().to_string(),
            "child_link_trace": child.links.links[0].span_context.trace_id().to_string(),
            "child_link_span": child.links.links[0].span_context.span_id().to_string(),
            "reason_input_links": spans.iter().filter(|s| s.name == "reason").map(|s| s.links.links.len()).max()
        })
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn causal_parked_turn_restores_exported_anchor_in_a_new_engine() {
    use everruns::hosting::SqliteHosting;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parked.db");
    let id = SessionId::new();
    let message = MessageId::new();
    let definition = Agent::builder()
        .instructions("Use the client tool")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("resolved").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "client_call".into(),
                    name: "client".into(),
                    arguments: json!({}),
                }],
                vec![],
            ]),
        ))
        .client_tool("client", "External operation", json!({"type":"object"}))
        .build()
        .unwrap();
    let (first, first_exporter) = observed(SqliteHosting::open(&path).unwrap(), definition.clone());
    let session = first.create_registered(id, &build()).await.unwrap();
    session
        .send_with_id(message, input("work", A))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while session.pending_tools().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let turn = first.session_status(id).await.unwrap().open_turn.unwrap();
    first.shutdown(Duration::from_secs(5)).await;
    let spans = first_exporter.get_finished_spans().unwrap();
    let anchor = spans
        .iter()
        .find(|s| s.name == "durable.turn")
        .unwrap()
        .span_context
        .clone();
    drop(session);
    drop(first);
    let (replacement, exporter) = observed(SqliteHosting::open(&path).unwrap(), definition);
    let session = replacement.resume(id).await.unwrap();
    session
        .resolve_tools(
            uuid::Uuid::new_v4(),
            turn,
            vec![everruns::ToolAnswer {
                tool_call_id: "client_call".into(),
                tool_name: "client".into(),
                result: json!("done"),
                error: None,
            }],
        )
        .await
        .unwrap();
    replacement.activate(id).await.unwrap();
    let admitted = session.admission(message).await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), admitted.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    replacement.shutdown(Duration::from_secs(5)).await;
    let spans = exporter.get_finished_spans().unwrap();
    assert!(
        !spans.iter().any(|s| s.name == "durable.turn"),
        "must not re-export the original anchor"
    );
    assert!(
        spans
            .iter()
            .filter(|s| s.name == "reason")
            .any(|s| s
                .links
                .links
                .iter()
                .any(|link| link.span_context.trace_id() == anchor.trace_id()
                    && link.span_context.span_id() == anchor.span_id())),
        "{spans:?}"
    );
    assert_eq!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.event_type() == "turn.started")
            .count(),
        1
    );
}
