use super::*;

#[tokio::test]
async fn cached_usage_totals_include_reads_and_writes() {
    let h = OtelHarness::new(false, TraceConventions::ALL);
    run_full_turn(&h).await;
    let spans = h.spans();
    for name in ["invoke_agent Weather Helper", "chat claude-sonnet-4-6"] {
        let s = by_name(&spans, name);
        assert_eq!(attr(s, "gen_ai.usage.input_tokens"), Some(&Value::I64(168)));
        assert_eq!(attr(s, "llm.token_count.prompt"), Some(&Value::I64(168)));
        assert_eq!(attr(s, "llm.token_count.total"), Some(&Value::I64(198)));
    }
}

#[tokio::test]
async fn errors_do_not_export_content_when_capture_is_off() {
    let h = OtelHarness::new(false, TraceConventions::ALL);
    let mut data = generation(false);
    let marker = "SYNTHETIC_PRIVATE_PAYLOAD_012345";
    data.metadata.error = Some(format!("provider validation failed for {marker}"));
    h.emit(600, h.context(None, None, None), data).await;
    let spans = h.spans();
    let s = by_name(&spans, "chat claude-sonnet-4-6");
    assert!(!matches!(&s.status, Status::Error { description } if description.contains(marker)));
    assert!(!s.events.iter().any(|e| {
        e.attributes
            .iter()
            .any(|a| a.value.to_string().contains(marker))
    }));
    assert!(attr(s, "gen_ai.input.messages").is_none());
}

#[tokio::test]
async fn same_call_id_is_isolated_across_sessions() {
    let a = OtelHarness::new(true, TraceConventions::ALL);
    let b = OtelHarness::new(true, TraceConventions::ALL);
    a.emit(0, a.context(None, None, None), a.turn_started())
        .await;
    let mut event = Event::new(b.session, b.context(None, None, None), b.turn_started());
    event.ts = b.at(0);
    a.listener.on_event(&event).await;
    let mut call = tool_started();
    call.tool_call.id = "call_0".into();
    a.emit(10, a.context(None, Some("tool-a"), None), call.clone())
        .await;
    let mut event = Event::new(b.session, b.context(None, Some("tool-b"), None), call);
    event.ts = b.at(10);
    a.listener.on_event(&event).await;
    a.emit(
        30,
        a.context(None, Some("tool-a"), None),
        ToolCompletedData::success(
            "call_0".into(),
            "get_weather".into(),
            vec![ContentPart::text("synthetic-session-A-result")],
            Some(20),
        ),
    )
    .await;
    let spans = a.spans();
    let completed = spans
        .iter()
        .find(|s| {
            attr_str(s, "gen_ai.tool.call.result").as_deref() == Some("synthetic-session-A-result")
        })
        .unwrap();
    assert_eq!(
        attr_str(completed, "gen_ai.conversation.id"),
        Some(a.session.to_string())
    );
    assert_eq!(
        attr_str(completed, "everruns.turn.id"),
        Some(a.turn.to_string())
    );
}

#[derive(Debug)]
struct AuditFlushProcessor {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    delay_ms: u64,
}
impl opentelemetry_sdk::trace::SpanProcessor for AuditFlushProcessor {
    fn on_start(&self, _: &mut opentelemetry_sdk::trace::Span, _: &Context) {}
    fn on_end(&self, _: SpanData) {}
    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(self.delay_ms));
        Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
            "synthetic flush failure".into(),
        ))
    }
    fn shutdown_with_timeout(&self, _: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        Ok(())
    }
}

#[tokio::test]
async fn flush_reports_exporter_failure_and_missing_provider() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = SdkTracerProvider::builder()
        .with_span_processor(AuditFlushProcessor {
            calls: calls.clone(),
            delay_ms: 0,
        })
        .build();
    let tracer_only = OtelEventListener::with_tracer(
        BoxedTracer::new(Box::new(provider.tracer("audit"))),
        false,
        TraceConventions::ALL,
    );
    assert!(
        tracer_only
            .flush()
            .await
            .unwrap_err()
            .contains("provider unavailable")
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let retained = OtelEventListener::with_tracer_provider(provider, false, TraceConventions::ALL);
    assert!(
        retained
            .flush()
            .await
            .unwrap_err()
            .contains("synthetic flush failure")
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn blocking_flush_respects_async_timeout() {
    use std::sync::{Arc, atomic::AtomicUsize};
    let provider = SdkTracerProvider::builder()
        .with_span_processor(AuditFlushProcessor {
            calls: Arc::new(AtomicUsize::new(0)),
            delay_ms: 60,
        })
        .build();
    let retained = OtelEventListener::with_tracer_provider(provider, false, TraceConventions::ALL);
    let outcome = tokio::time::timeout(Duration::from_millis(5), retained.flush()).await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn detailed_errors_require_content_capture() {
    let h = OtelHarness::new(true, TraceConventions::ALL);
    let mut data = generation(false);
    data.metadata.error = Some("SYNTHETIC_OPTED_IN_ERROR".into());
    h.emit(1, h.context(None, None, None), data).await;
    let spans = h.spans();
    let span = by_name(&spans, "chat claude-sonnet-4-6");
    assert!(
        matches!(&span.status, Status::Error { description } if description == "SYNTHETIC_OPTED_IN_ERROR")
    );
}

#[test]
fn cache_inclusive_totals_use_wide_arithmetic() {
    let value = TokenUsage::with_cache(u32::MAX, u32::MAX, Some(u32::MAX), Some(u32::MAX));
    assert_eq!(prompt_tokens(&value), 3 * i64::from(u32::MAX));
    let attrs = oi_token_attributes(&value);
    assert!(
        attrs
            .iter()
            .any(|a| a.key.as_str() == "llm.token_count.total"
                && a.value == Value::I64(4 * i64::from(u32::MAX)))
    );
}
