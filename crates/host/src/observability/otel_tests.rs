//! Unit tests for [`super`], the OpenTelemetry exporter.
//!
//! Split out of `otel.rs` rather than inlined: the module is a thousand
//! lines against fifteen hundred of exporter, and the file-size ratchet
//! exists because nobody could hold the combined file in their head.

use super::*;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use everruns_core::events::{
    EventContext, LlmGenerationMetadata, LlmGenerationOutput, LlmRequestOptions,
    ToolDefinitionSummary,
};
use everruns_core::message::RuntimeMessage;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{AgentId, ExecId, HarnessId, MessageId, SessionId, TurnId};
use opentelemetry::trace::{SpanId, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::json;

struct Harness {
    listener: OtelEventListener,
    exporter: InMemorySpanExporter,
    _provider: SdkTracerProvider,
    session: SessionId,
    turn: TurnId,
    input_message: MessageId,
    t0: DateTime<Utc>,
}

impl Harness {
    fn new(record_content: bool, conventions: TraceConventions) -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = BoxedTracer::new(Box::new(provider.tracer("test")));
        Self {
            listener: OtelEventListener::with_tracer(tracer, record_content, conventions),
            exporter,
            _provider: provider,
            session: SessionId::new(),
            turn: TurnId::new(),
            input_message: MessageId::new(),
            t0: Utc::now(),
        }
    }

    fn at(&self, ms: i64) -> DateTime<Utc> {
        self.t0 + ChronoDuration::milliseconds(ms)
    }

    fn context(
        &self,
        exec: Option<ExecId>,
        span: Option<&str>,
        parent: Option<&str>,
    ) -> EventContext {
        EventContext {
            turn_id: Some(self.turn),
            input_message_id: Some(self.input_message),
            exec_id: exec,
            trace_id: Some(self.turn.to_string()),
            span_id: span.map(str::to_string),
            parent_span_id: parent.map(str::to_string),
        }
    }

    async fn emit(&self, ms: i64, ctx: EventContext, data: impl Into<EventData>) {
        let mut event = Event::new(self.session, ctx, data);
        event.ts = self.at(ms);
        self.listener.on_event(&event).await;
    }

    fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().unwrap()
    }

    fn turn_started(&self) -> TurnStartedData {
        TurnStartedData {
            turn_id: self.turn,
            input_message_id: self.input_message,
            input_content: Some("What is the weather in Paris?".to_string()),
            agent_id: Some(AgentId::new()),
            agent_name: Some("Weather Helper".to_string()),
            agent_description: Some("Answers weather questions".to_string()),
        }
    }
}

fn attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| &kv.value)
}

fn attr_str(span: &SpanData, key: &str) -> Option<String> {
    attr(span, key).map(|v| v.to_string())
}

fn by_name<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no span named {name}"))
}

fn id(span: &SpanData) -> SpanId {
    span.span_context.span_id()
}

fn usage(input: u32, output: u32) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: Some(40),
        cache_creation_tokens: Some(8),
        actual_cost_usd: Some(0.0125),
        estimated_cost_usd: None,
        effective_cost_usd: None,
    }
}

fn generation(success: bool) -> LlmGenerationData {
    let call = ToolCall {
        id: "call_1".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({ "city": "Paris" }),
    };
    LlmGenerationData {
        messages: vec![
            RuntimeMessage::system("You are a weather assistant."),
            RuntimeMessage::user("What is the weather in Paris?"),
        ],
        tools: vec![ToolDefinitionSummary {
            name: "get_weather".to_string(),
            display_name: None,
            category: None,
            capability_id: None,
            capability_name: None,
            description: "Look up current weather".to_string(),
        }],
        output: LlmGenerationOutput {
            text: if success {
                Some("Let me check.".to_string())
            } else {
                None
            },
            tool_calls: if success { vec![call] } else { vec![] },
        },
        metadata: LlmGenerationMetadata {
            model: "claude-sonnet-4-5".to_string(),
            provider: Some("anthropic".to_string()),
            response_model: None,
            usage: Some(usage(120, 30)),
            duration_ms: Some(500),
            time_to_first_token_ms: Some(250),
            success,
            error: (!success).then(|| "provider returned 503".to_string()),
            finish_reasons: Some(vec!["tool_calls".to_string()]),
            response_id: Some("msg_01".to_string()),
            retry: None,
            compaction: None,
            request_options: Some(LlmRequestOptions {
                temperature: Some(0.2),
                max_tokens: Some(1024),
                reasoning_effort: Some("high".to_string()),
                stream: Some(true),
                ..LlmRequestOptions::default()
            }),
        },
    }
}

fn tool_started() -> ToolStartedData {
    ToolStartedData {
        tool_call: ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({ "city": "Paris" }),
        },
        tool_call_fingerprint: None,
        display_name: None,
        narration: None,
    }
}

/// The full agentic loop with extended thinking, as the engine emits it.
async fn run_full_turn(h: &Harness) {
    let reason_exec = ExecId::new();
    let act_exec = ExecId::new();
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        10,
        h.context(Some(reason_exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .await;
    // Thinking events carry exec context only, no span ids.
    h.emit(
        100,
        h.context(Some(reason_exec), None, None),
        ReasonThinkingStartedData {
            turn_id: h.turn,
            model: Some("claude-sonnet-4-5".to_string()),
        },
    )
    .await;
    h.emit(
        300,
        h.context(Some(reason_exec), None, None),
        ReasonThinkingCompletedData {
            turn_id: h.turn,
            thinking: "Paris needs a lookup.".to_string(),
        },
    )
    .await;
    h.emit(
        600,
        h.context(Some(reason_exec), Some("g1"), Some("r1")),
        generation(true),
    )
    .await;
    h.emit(
        610,
        h.context(Some(reason_exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonCompletedData {
            success: true,
            text_preview: Some("Let me check.".to_string()),
            has_tool_calls: true,
            tool_call_count: 1,
            error: None,
            duration_ms: Some(600),
            usage: Some(usage(120, 30)),
        },
    )
    .await;
    h.emit(
        620,
        h.context(Some(act_exec), Some("a1"), Some(&h.turn.to_string())),
        ActStartedData {
            tool_calls: vec![],
            headline: None,
        },
    )
    .await;
    h.emit(
        630,
        h.context(Some(act_exec), Some("t1"), Some("a1")),
        tool_started(),
    )
    .await;
    h.emit(
        700,
        h.context(Some(act_exec), Some("t1"), Some("a1")),
        ToolCompletedData::success(
            "call_1".to_string(),
            "get_weather".to_string(),
            vec![ContentPart::text("rainy, 14C")],
            Some(70),
        ),
    )
    .await;
    h.emit(
        710,
        h.context(Some(act_exec), Some("a1"), Some(&h.turn.to_string())),
        ActCompletedData {
            completed: true,
            success_count: 1,
            error_count: 0,
            duration_ms: Some(90),
            headline: None,
        },
    )
    .await;
    h.emit(
        800,
        h.context(None, None, None),
        TurnCompletedData {
            turn_id: h.turn,
            iterations: 1,
            duration_ms: Some(800),
            usage: Some(usage(120, 30)),
            input_content: None,
            final_message_id: None,
            final_answer_preview: Some("It is rainy in Paris.".to_string()),
            time_to_first_token_ms: Some(250),
            tool_call_count: Some(1),
            llm_call_count: Some(1),
            status: Some("completed".to_string()),
        },
    )
    .await;
}

/// `gen_ai.response.model` must report what the provider served, not echo
/// the alias that was requested. Both attributes previously came from
/// `meta.model`, so a request for `claude-sonnet-4-5` served by
/// `claude-sonnet-4-5-20250929` reported the alias twice and the only
/// record of which weights answered was lost.
#[tokio::test]
async fn response_model_reports_what_the_provider_served() {
    let h = Harness::new(false, TraceConventions::ALL);
    let mut data = generation(true);
    data.metadata.response_model = Some("claude-sonnet-4-5-20250929".to_string());
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(600, h.context(Some(ExecId::new()), Some("g1"), None), data)
        .await;

    let spans = h.spans();
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(
        attr_str(chat, "gen_ai.request.model").as_deref(),
        Some("claude-sonnet-4-5"),
        "the request attribute keeps the alias that was asked for"
    );
    assert_eq!(
        attr_str(chat, "gen_ai.response.model").as_deref(),
        Some("claude-sonnet-4-5-20250929"),
        "the response attribute must carry the served model"
    );
}

/// A provider that echoes no model leaves `response_model` unset. Dropping
/// the attribute would break every consumer that expects it, so the
/// requested model stands in — the best answer available.
#[tokio::test]
async fn response_model_falls_back_to_the_request_when_unreported() {
    let h = Harness::new(false, TraceConventions::ALL);
    let data = generation(true);
    assert!(data.metadata.response_model.is_none());
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(600, h.context(Some(ExecId::new()), Some("g1"), None), data)
        .await;

    let spans = h.spans();
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(
        attr_str(chat, "gen_ai.response.model").as_deref(),
        Some("claude-sonnet-4-5")
    );
}

#[tokio::test]
async fn full_turn_nests_spans_per_the_conventions() {
    let h = Harness::new(false, TraceConventions::ALL);
    run_full_turn(&h).await;
    let spans = h.spans();
    assert_eq!(h.listener.active_span_count(), 0);
    assert_eq!(
        spans.len(),
        6,
        "{:?}",
        spans.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
    );

    let turn = by_name(&spans, "invoke_agent Weather Helper");
    let reason = by_name(&spans, "reason");
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    let thinking = by_name(&spans, "thinking");
    let act = by_name(&spans, "act");
    let tool = by_name(&spans, "execute_tool get_weather");

    // Parenting: turn → reason → chat → thinking; turn → act → tool.
    assert_eq!(turn.parent_span_id, SpanId::INVALID);
    assert_eq!(reason.parent_span_id, id(turn));
    assert_eq!(chat.parent_span_id, id(reason));
    assert_eq!(thinking.parent_span_id, id(chat));
    assert_eq!(act.parent_span_id, id(turn));
    assert_eq!(tool.parent_span_id, id(act));
    for span in &spans {
        assert_eq!(span.span_context.trace_id(), turn.span_context.trace_id());
    }

    // Kinds.
    assert_eq!(turn.span_kind, SpanKind::Internal);
    assert_eq!(chat.span_kind, SpanKind::Client);
    assert_eq!(tool.span_kind, SpanKind::Internal);

    // Timing comes from event timestamps: the chat span opened with
    // thinking and closed with the generation record.
    assert_eq!(chat.start_time, SystemTime::from(h.at(100)));
    assert_eq!(chat.end_time, SystemTime::from(h.at(600)));
    assert_eq!(turn.start_time, SystemTime::from(h.at(0)));
    assert_eq!(turn.end_time, SystemTime::from(h.at(800)));
    assert_eq!(tool.start_time, SystemTime::from(h.at(630)));

    // invoke_agent attributes.
    assert_eq!(
        attr_str(turn, "gen_ai.operation.name").as_deref(),
        Some("invoke_agent")
    );
    assert_eq!(
        attr_str(turn, "gen_ai.agent.name").as_deref(),
        Some("Weather Helper")
    );
    assert_eq!(
        attr_str(turn, "gen_ai.agent.description").as_deref(),
        Some("Answers weather questions")
    );
    assert!(attr(turn, "gen_ai.agent.id").is_some());
    assert_eq!(
        attr_str(turn, "gen_ai.conversation.id"),
        Some(h.session.to_string())
    );
    assert_eq!(
        attr(turn, "gen_ai.usage.input_tokens"),
        Some(&Value::I64(120))
    );
    assert_eq!(
        attr(turn, "gen_ai.usage.output_tokens"),
        Some(&Value::I64(30))
    );
    assert_eq!(attr(turn, "everruns.turn.iterations"), Some(&Value::I64(1)));
    assert_eq!(
        attr_str(turn, "openinference.span.kind").as_deref(),
        Some("AGENT")
    );
    assert_eq!(
        attr_str(turn, "agent.name").as_deref(),
        Some("Weather Helper")
    );
    assert_eq!(attr_str(turn, "session.id"), Some(h.session.to_string()));
    assert_eq!(attr(turn, "llm.token_count.total"), Some(&Value::I64(150)));

    // chat attributes.
    assert_eq!(
        attr_str(chat, "gen_ai.operation.name").as_deref(),
        Some("chat")
    );
    assert_eq!(
        attr_str(chat, "gen_ai.provider.name").as_deref(),
        Some("anthropic")
    );
    assert_eq!(
        attr_str(chat, "gen_ai.request.model").as_deref(),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(
        attr_str(chat, "gen_ai.response.id").as_deref(),
        Some("msg_01")
    );
    assert_eq!(
        attr(chat, "gen_ai.response.finish_reasons"),
        Some(&Value::Array(vec![StringValue::from("tool_calls")].into()))
    );
    assert_eq!(
        attr(chat, "gen_ai.usage.input_tokens"),
        Some(&Value::I64(120))
    );
    assert_eq!(
        attr(chat, "gen_ai.usage.cache_read.input_tokens"),
        Some(&Value::I64(40))
    );
    assert_eq!(
        attr(chat, "gen_ai.usage.cache_write.input_tokens"),
        Some(&Value::I64(8))
    );
    assert_eq!(
        attr(chat, "gen_ai.request.temperature"),
        Some(&Value::F64(0.2f32 as f64))
    );
    assert_eq!(
        attr(chat, "gen_ai.request.max_tokens"),
        Some(&Value::I64(1024))
    );
    assert_eq!(
        attr_str(chat, "gen_ai.request.reasoning.level").as_deref(),
        Some("high")
    );
    assert_eq!(
        attr(chat, "gen_ai.request.stream"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        attr(chat, "gen_ai.response.time_to_first_chunk"),
        Some(&Value::F64(0.25))
    );
    assert!(attr(chat, "gen_ai.output.type").is_none());
    assert!(attr(chat, "gen_ai.usage.cache_read_tokens").is_none());
    assert_eq!(
        attr_str(chat, "openinference.span.kind").as_deref(),
        Some("LLM")
    );
    assert_eq!(
        attr_str(chat, "llm.model_name").as_deref(),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(attr_str(chat, "llm.provider").as_deref(), Some("anthropic"));
    assert_eq!(attr(chat, "llm.token_count.prompt"), Some(&Value::I64(120)));
    assert_eq!(
        attr(chat, "llm.token_count.prompt_details.cache_read"),
        Some(&Value::I64(40))
    );
    assert_eq!(attr(chat, "llm.cost.total"), Some(&Value::F64(0.0125)));
    assert!(
        attr_str(chat, "llm.tools.0.tool.json_schema")
            .unwrap()
            .contains("get_weather")
    );
    assert!(
        attr_str(chat, "llm.invocation_parameters")
            .unwrap()
            .contains("\"temperature\":0.2")
    );
    assert_eq!(chat.status, Status::Unset);

    // Phase spans are plain internal spans, not Gen-AI operations.
    assert!(attr(reason, "gen_ai.operation.name").is_none());
    assert_eq!(
        attr_str(reason, "everruns.phase").as_deref(),
        Some("reason")
    );
    assert_eq!(
        attr_str(reason, "openinference.span.kind").as_deref(),
        Some("CHAIN")
    );
    assert_eq!(
        attr_str(thinking, "everruns.phase").as_deref(),
        Some("thinking")
    );
    assert_eq!(attr_str(act, "everruns.phase").as_deref(), Some("act"));

    // execute_tool attributes, with the description learned from the
    // generation record.
    assert_eq!(
        attr_str(tool, "gen_ai.operation.name").as_deref(),
        Some("execute_tool")
    );
    assert_eq!(
        attr_str(tool, "gen_ai.tool.name").as_deref(),
        Some("get_weather")
    );
    assert_eq!(
        attr_str(tool, "gen_ai.tool.type").as_deref(),
        Some("function")
    );
    assert_eq!(
        attr_str(tool, "gen_ai.tool.call.id").as_deref(),
        Some("call_1")
    );
    assert_eq!(
        attr_str(tool, "gen_ai.tool.description").as_deref(),
        Some("Look up current weather")
    );
    assert_eq!(
        attr_str(tool, "gen_ai.agent.name").as_deref(),
        Some("Weather Helper")
    );
    assert_eq!(
        attr_str(tool, "openinference.span.kind").as_deref(),
        Some("TOOL")
    );
    assert_eq!(attr_str(tool, "tool.name").as_deref(), Some("get_weather"));
    assert_eq!(
        attr_str(tool, "everruns.tool.status").as_deref(),
        Some("success")
    );
    assert_eq!(tool.status, Status::Unset);
}

#[tokio::test]
async fn content_stays_out_of_spans_by_default() {
    let h = Harness::new(false, TraceConventions::ALL);
    run_full_turn(&h).await;
    for span in h.spans() {
        for key in [
            "gen_ai.input.messages",
            "gen_ai.output.messages",
            "gen_ai.system_instructions",
            "gen_ai.tool.definitions",
            "gen_ai.tool.call.arguments",
            "gen_ai.tool.call.result",
            "input.value",
            "output.value",
            "llm.input_messages.0.message.content",
        ] {
            assert!(attr(&span, key).is_none(), "{} leaked {key}", span.name);
        }
    }
}

#[tokio::test]
async fn content_capture_records_spec_shaped_messages() {
    let h = Harness::new(true, TraceConventions::ALL);
    run_full_turn(&h).await;
    let spans = h.spans();
    let turn = by_name(&spans, "invoke_agent Weather Helper");
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    let thinking = by_name(&spans, "thinking");
    let tool = by_name(&spans, "execute_tool get_weather");

    let instructions: serde_json::Value =
        serde_json::from_str(&attr_str(chat, "gen_ai.system_instructions").unwrap()).unwrap();
    assert_eq!(
        instructions,
        json!([{ "type": "text", "content": "You are a weather assistant." }])
    );
    let input: serde_json::Value =
        serde_json::from_str(&attr_str(chat, "gen_ai.input.messages").unwrap()).unwrap();
    assert_eq!(input[0]["role"], "user");
    assert_eq!(
        input.as_array().unwrap().len(),
        1,
        "system prompt is not chat history"
    );
    let output: serde_json::Value =
        serde_json::from_str(&attr_str(chat, "gen_ai.output.messages").unwrap()).unwrap();
    assert_eq!(output[0]["role"], "assistant");
    assert_eq!(output[0]["finish_reason"], "tool_calls");
    assert_eq!(output[0]["parts"][0]["type"], "reasoning");
    assert_eq!(output[0]["parts"][0]["content"], "Paris needs a lookup.");
    assert_eq!(output[0]["parts"][1]["type"], "text");
    assert_eq!(output[0]["parts"][2]["type"], "tool_call");
    assert_eq!(output[0]["parts"][2]["name"], "get_weather");
    let tools: serde_json::Value =
        serde_json::from_str(&attr_str(chat, "gen_ai.tool.definitions").unwrap()).unwrap();
    assert_eq!(tools[0]["name"], "get_weather");

    assert_eq!(
        attr_str(chat, "input.mime_type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        attr_str(chat, "llm.input_messages.0.message.role").as_deref(),
        Some("system")
    );
    assert_eq!(
        attr_str(chat, "llm.input_messages.1.message.role").as_deref(),
        Some("user")
    );
    assert_eq!(
        attr_str(
            chat,
            "llm.output_messages.0.message.tool_calls.0.tool_call.function.name"
        )
        .as_deref(),
        Some("get_weather")
    );

    assert_eq!(
        attr_str(turn, "input.value").as_deref(),
        Some("What is the weather in Paris?")
    );
    assert_eq!(
        attr_str(turn, "output.value").as_deref(),
        Some("It is rainy in Paris.")
    );
    assert!(
        attr_str(turn, "gen_ai.input.messages")
            .unwrap()
            .contains("\"role\":\"user\"")
    );
    assert_eq!(
        attr_str(thinking, "output.value").as_deref(),
        Some("Paris needs a lookup.")
    );

    assert_eq!(
        attr_str(tool, "gen_ai.tool.call.arguments").as_deref(),
        Some(r#"{"city":"Paris"}"#)
    );
    assert_eq!(
        attr_str(tool, "gen_ai.tool.call.result").as_deref(),
        Some("rainy, 14C")
    );
    assert_eq!(
        attr_str(tool, "output.mime_type").as_deref(),
        Some("text/plain")
    );
}

#[tokio::test]
async fn chat_without_thinking_is_backdated_by_its_duration() {
    let h = Harness::new(false, TraceConventions::ALL);
    let exec = ExecId::new();
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        10,
        h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .await;
    h.emit(
        600,
        h.context(Some(exec), Some("g1"), Some("r1")),
        generation(true),
    )
    .await;
    let spans = h.spans();
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(chat.start_time, SystemTime::from(h.at(100)));
    assert_eq!(chat.end_time, SystemTime::from(h.at(600)));
    assert_eq!(chat.span_kind, SpanKind::Client);
}

#[tokio::test]
async fn failures_carry_error_type_status_and_exception_event() {
    let h = Harness::new(false, TraceConventions::ALL);
    let exec = ExecId::new();
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        10,
        h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .await;
    h.emit(
        600,
        h.context(Some(exec), Some("g1"), Some("r1")),
        generation(false),
    )
    .await;
    h.emit(
        620,
        h.context(Some(exec), Some("t1"), Some("r1")),
        tool_started(),
    )
    .await;
    h.emit(
        700,
        h.context(Some(exec), Some("t1"), Some("r1")),
        ToolCompletedData::failure(
            "call_1".to_string(),
            "get_weather".to_string(),
            "timeout".to_string(),
            "tool timed out after 60s".to_string(),
            Some(80),
        ),
    )
    .await;
    h.emit(
        800,
        h.context(None, None, None),
        TurnFailedData {
            turn_id: h.turn,
            error: "budget exhausted".to_string(),
            error_code: Some("budget_exhausted".to_string()),
            error_fields: None,
            error_disclosure: None,
        },
    )
    .await;

    let spans = h.spans();
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(attr_str(chat, "error.type").as_deref(), Some("503"));
    assert!(
        matches!(&chat.status, Status::Error { description } if description == "provider returned 503")
    );
    let exception = chat.events.iter().find(|e| e.name == "exception").unwrap();
    assert!(
        exception
            .attributes
            .iter()
            .any(|kv| kv.key.as_str() == "exception.message")
    );
    assert!(attr(chat, "gen_ai.output.messages").is_none());

    let tool = by_name(&spans, "execute_tool get_weather");
    assert_eq!(attr_str(tool, "error.type").as_deref(), Some("timeout"));
    assert_eq!(
        attr_str(tool, "everruns.tool.status").as_deref(),
        Some("timeout")
    );
    assert!(matches!(&tool.status, Status::Error { .. }));

    let turn = by_name(&spans, "invoke_agent Weather Helper");
    assert_eq!(
        attr_str(turn, "error.type").as_deref(),
        Some("budget_exhausted")
    );
    assert!(
        matches!(&turn.status, Status::Error { description } if description == "budget exhausted")
    );

    // The reason span never completed: the turn end closed it.
    let reason = by_name(&spans, "reason");
    assert_eq!(
        attr(reason, "everruns.span.unterminated"),
        Some(&Value::Bool(true))
    );
    assert_eq!(reason.end_time, SystemTime::from(h.at(800)));
    assert_eq!(h.listener.active_span_count(), 0);
}

#[tokio::test]
async fn cancelled_turn_closes_everything_under_it() {
    let h = Harness::new(false, TraceConventions::ALL);
    let exec = ExecId::new();
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        10,
        h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .await;
    h.emit(
        100,
        h.context(Some(exec), None, None),
        ReasonThinkingStartedData {
            turn_id: h.turn,
            model: Some("claude-sonnet-4-5".to_string()),
        },
    )
    .await;
    h.emit(
        200,
        h.context(None, None, None),
        TurnCancelledData {
            turn_id: h.turn,
            reason: Some("user stop".to_string()),
            usage: None,
        },
    )
    .await;
    let spans = h.spans();
    assert_eq!(
        spans.len(),
        4,
        "{:?}",
        spans.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
    );
    for name in ["reason", "thinking", "chat claude-sonnet-4-5"] {
        let span = by_name(&spans, name);
        assert_eq!(
            attr(span, "everruns.span.unterminated"),
            Some(&Value::Bool(true))
        );
        assert_eq!(span.end_time, SystemTime::from(h.at(200)));
    }
    let turn = by_name(&spans, "invoke_agent Weather Helper");
    assert_eq!(attr_str(turn, "error.type").as_deref(), Some("cancelled"));
    assert_eq!(
        attr_str(turn, "everruns.turn.status").as_deref(),
        Some("cancelled")
    );
    assert_eq!(h.listener.active_span_count(), 0);
}

#[tokio::test]
async fn pending_chat_is_closed_when_no_generation_record_arrives() {
    let h = Harness::new(false, TraceConventions::ALL);
    let exec = ExecId::new();
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        10,
        h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
    .await;
    h.emit(
        100,
        h.context(Some(exec), None, None),
        ReasonThinkingStartedData {
            turn_id: h.turn,
            model: Some("claude-sonnet-4-5".to_string()),
        },
    )
    .await;
    h.emit(
        300,
        h.context(Some(exec), None, None),
        ReasonThinkingCompletedData {
            turn_id: h.turn,
            thinking: "partial".to_string(),
        },
    )
    .await;
    h.emit(
        500,
        h.context(Some(exec), Some("r1"), Some(&h.turn.to_string())),
        ReasonCompletedData::failure("stream dropped".to_string(), Some(490)),
    )
    .await;
    let spans = h.spans();
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(chat.end_time, SystemTime::from(h.at(500)));
    assert!(
        matches!(&chat.status, Status::Error { description } if description == "stream dropped")
    );
    let reason = by_name(&spans, "reason");
    assert!(matches!(&reason.status, Status::Error { .. }));
    let thinking = by_name(&spans, "thinking");
    assert_eq!(thinking.parent_span_id, id(chat));
}

#[tokio::test]
async fn orphan_completions_are_reconstructed_from_their_duration() {
    let h = Harness::new(false, TraceConventions::ALL);
    h.emit(0, h.context(None, None, None), h.turn_started())
        .await;
    h.emit(
        700,
        h.context(None, Some("t1"), None),
        ToolCompletedData::success(
            "call_x".to_string(),
            "read_file".to_string(),
            vec![],
            Some(70),
        ),
    )
    .await;
    h.emit(
        800,
        h.context(None, None, None),
        TurnCompletedData {
            turn_id: h.turn,
            iterations: 1,
            duration_ms: Some(800),
            usage: None,
            input_content: None,
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
        },
    )
    .await;
    let spans = h.spans();
    let tool = by_name(&spans, "execute_tool read_file");
    assert_eq!(
        attr(tool, "everruns.span.orphaned"),
        Some(&Value::Bool(true))
    );
    assert_eq!(tool.start_time, SystemTime::from(h.at(630)));
    assert_eq!(tool.end_time, SystemTime::from(h.at(700)));
    let turn = by_name(&spans, "invoke_agent Weather Helper");
    assert_eq!(tool.parent_span_id, id(turn));

    // A turn completing with no start still leaves a record.
    let other = Harness::new(false, TraceConventions::ALL);
    other
        .emit(
            50,
            other.context(None, None, None),
            TurnCompletedData {
                turn_id: other.turn,
                iterations: 2,
                duration_ms: Some(50),
                usage: None,
                input_content: None,
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: None,
                llm_call_count: None,
                status: None,
            },
        )
        .await;
    let spans = other.spans();
    let turn = by_name(&spans, "invoke_agent");
    assert_eq!(
        attr(turn, "everruns.span.orphaned"),
        Some(&Value::Bool(true))
    );
    assert_eq!(attr(turn, "everruns.turn.iterations"), Some(&Value::I64(2)));
}

#[tokio::test]
async fn conventions_can_be_narrowed() {
    let h = Harness::new(false, TraceConventions::GEN_AI);
    run_full_turn(&h).await;
    for span in h.spans() {
        assert!(
            attr(&span, "openinference.span.kind").is_none(),
            "{}",
            span.name
        );
        assert!(attr(&span, "session.id").is_none());
        assert!(attr(&span, "llm.model_name").is_none());
    }
    let h = Harness::new(false, TraceConventions::OPENINFERENCE);
    run_full_turn(&h).await;
    let spans = h.spans();
    for span in &spans {
        assert!(
            !span
                .attributes
                .iter()
                .any(|kv| kv.key.as_str().starts_with("gen_ai.")),
            "{}",
            span.name
        );
    }
    // Span names, kinds, and hierarchy do not depend on the vocabulary.
    let chat = by_name(&spans, "chat claude-sonnet-4-5");
    assert_eq!(
        attr_str(chat, "openinference.span.kind").as_deref(),
        Some("LLM")
    );
    assert_eq!(chat.parent_span_id, id(by_name(&spans, "reason")));
}

#[test]
fn conventions_parse_from_env_syntax() {
    assert_eq!(TraceConventions::parse("gen_ai"), TraceConventions::GEN_AI);
    assert_eq!(
        TraceConventions::parse("openinference"),
        TraceConventions::OPENINFERENCE
    );
    assert_eq!(
        TraceConventions::parse("gen_ai, openinference"),
        TraceConventions::ALL
    );
    assert_eq!(TraceConventions::parse("all"), TraceConventions::ALL);
    assert_eq!(TraceConventions::parse(""), TraceConventions::ALL);
    assert_eq!(TraceConventions::parse("bogus"), TraceConventions::ALL);
    assert_eq!(TraceConventions::parse("OTEL"), TraceConventions::GEN_AI);
}

#[tokio::test]
async fn listener_subscribes_to_the_thirteen_lifecycle_events() {
    let h = Harness::new(false, TraceConventions::ALL);
    let types = h.listener.event_types().unwrap();
    assert_eq!(types.len(), 13);
    assert!(types.contains(&TURN_STARTED));
    assert!(types.contains(&LLM_GENERATION));
    assert!(types.contains(&TOOL_COMPLETED));
    assert_eq!(h.listener.name(), "OtelEventListener");
    assert!(!h.listener.record_content());
    assert_eq!(h.listener.conventions(), TraceConventions::ALL);
}
