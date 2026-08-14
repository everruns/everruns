//! Stable service-provider interface for advanced, code-defined capabilities.
//!
//! Most application tools should use [`#[everruns::tool]`](macro@crate::tool).
//! This module is for reusable capability packages that need several typed
//! tools, capability-level instructions and metadata, execution context,
//! progress events, or call-scoped cancellation. It deliberately projects the
//! engine contracts needed by capability authors without exposing registries,
//! stores, tenancy, or host implementation types.
//!
//! The authoring types are the neutral `everruns-capability` contract
//! (EVE-873), re-exported here at their stable Framework paths. Capability
//! packages that want a dependency-light build can depend on
//! `everruns-capability` directly (its `definition` module is this same
//! surface); applications keep depending only on `everruns`. This module
//! additionally owns the private adapters that install a [`Definition`] onto
//! the in-process runtime.
//!
//! # Example
//!
//! ```
//! use everruns::{Agent, Model, capability};
//!
//! #[derive(capability::Deserialize, capability::JsonSchema)]
//! #[serde(crate = "everruns::capability::serde")]
//! #[schemars(crate = "everruns::capability::schemars")]
//! struct LookupInput {
//!     city: String,
//! }
//!
//! #[derive(capability::Serialize, capability::JsonSchema)]
//! #[serde(crate = "everruns::capability::serde")]
//! #[schemars(crate = "everruns::capability::schemars")]
//! struct LookupOutput {
//!     city: String,
//!     forecast: String,
//! }
//!
//! struct Lookup;
//!
//! #[capability::async_trait]
//! impl capability::Handler for Lookup {
//!     type Input = LookupInput;
//!     type Output = LookupOutput;
//!     type Error = capability::Error;
//!
//!     fn name(&self) -> &str { "lookup_weather" }
//!     fn description(&self) -> &str { "Look up the weather for a city." }
//!
//!     async fn execute(
//!         &self,
//!         input: Self::Input,
//!         context: capability::Context,
//!     ) -> Result<Self::Output, Self::Error> {
//!         context.progress("Looking up the forecast").await;
//!         Ok(LookupOutput {
//!             city: input.city,
//!             forecast: "sunny".into(),
//!         })
//!     }
//! }
//!
//! let weather = capability::Definition::new(
//!     "weather",
//!     "Weather",
//!     "Typed weather lookup tools.",
//! )
//! .instructions("Use weather data only when a user asks about a location.")
//! .tool(Lookup);
//!
//! let agent = Agent::builder()
//!     .instructions("You are a concise assistant.")
//!     .model(Model::simulated("done"))
//!     .capability(weather)
//!     .build()?;
//! # let _ = agent;
//! # Ok::<(), everruns::BuildError>(())
//! ```

#![deny(missing_docs)]

use std::sync::Arc;

use everruns_core::capabilities::Capability as CoreCapability;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool as CoreTool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use serde_json::{Value, json};

pub use everruns_capability::definition::{
    CallCancellation, CancellationSignal, Context, Definition, Deserialize, Error, ErrorVisibility,
    Handler, Hints, JsonSchema, ProgressSink, Serialize, Tool, ToolSpec, async_trait, schemars,
    serde, serde_json,
};

// ============================================================================
// Runtime adapters (private): neutral contract -> in-process engine
// ============================================================================

/// Adapt a neutral [`Definition`] into the engine's `Capability` trait for
/// private registration on this agent's runtime.
pub(crate) fn runtime_adapter(definition: &Definition) -> RuntimeDefinition {
    RuntimeDefinition(definition.clone())
}

pub(crate) struct RuntimeDefinition(Definition);

#[async_trait]
impl CoreCapability for RuntimeDefinition {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        self.0.instructions_text()
    }

    fn metadata(&self) -> Option<Value> {
        self.0.metadata_value().cloned()
    }

    fn tools(&self) -> Vec<Box<dyn CoreTool>> {
        self.0
            .tools()
            .iter()
            .cloned()
            .map(|tool| Box::new(CoreToolAdapter(tool)) as Box<dyn CoreTool>)
            .collect()
    }
}

/// Adapt one neutral capability [`Tool`] onto the engine tool contract,
/// bridging execution context, progress, cancellation, and structured errors.
pub(crate) struct CoreToolAdapter(pub(crate) Tool);

#[async_trait]
impl CoreTool for CoreToolAdapter {
    fn name(&self) -> &str {
        self.0.spec().name()
    }

    fn display_name(&self) -> Option<&str> {
        self.0.spec().display_name()
    }

    fn description(&self) -> &str {
        self.0.spec().description()
    }

    fn parameters_schema(&self) -> Value {
        self.0.spec().input_schema().clone()
    }

    fn hints(&self) -> ToolHints {
        hints_to_core(self.0.spec().hints())
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::internal_error_msg(
            "advanced capability tool execution requires runtime context",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let context = context_from_core(self.0.spec().name(), context);
        match self.0.invoke(arguments, context).await {
            Ok(output) => ToolExecutionResult::success(output),
            Err(error) => error_into_execution_result(error),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

fn hints_to_core(hints: &Hints) -> ToolHints {
    ToolHints {
        readonly: hints.readonly,
        destructive: hints.destructive,
        idempotent: hints.idempotent,
        open_world: hints.open_world,
        long_running: hints.long_running,
        concurrency_class: hints.concurrency_class.clone(),
        metadata: hints.metadata.clone(),
        ..ToolHints::default()
    }
}

/// Build the neutral execution [`Context`] from the engine's `ToolContext`.
fn context_from_core(tool_name: &str, inner: &ToolContext) -> Context {
    Context::new(
        tool_name,
        inner.session_id.to_string(),
        inner.workspace_id.to_string(),
    )
    .with_locale(inner.locale.clone())
    .with_progress_sink(Arc::new(CoreProgressSink {
        tool_name: tool_name.to_string(),
        inner: inner.clone(),
    }))
    .with_cancellation_signal(Arc::new(TokenCancellationSignal(
        inner.cancellation.clone().unwrap_or_default(),
    )))
}

struct CoreProgressSink {
    tool_name: String,
    inner: ToolContext,
}

#[async_trait]
impl ProgressSink for CoreProgressSink {
    async fn emit(&self, tool_name: &str, message: &str) {
        // Prefer the tool name the context was armed with; the neutral layer
        // passes the same value through.
        let name = if tool_name.is_empty() {
            &self.tool_name
        } else {
            tool_name
        };
        self.inner.emit_progress(name, message).await;
    }
}

struct TokenCancellationSignal(tokio_util::sync::CancellationToken);

impl CancellationSignal for TokenCancellationSignal {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    fn cancelled<'a>(&'a self) -> everruns_capability::definition::BoxFuture<'a, ()> {
        Box::pin(self.0.cancelled())
    }
}

/// Serialize a structured capability [`Error`] onto the engine's tool result
/// contract, preserving the model-visibility boundary: user errors keep code,
/// message, and details; internal errors are logged and replaced with a
/// generic model-visible message.
fn error_into_execution_result(error: Error) -> ToolExecutionResult {
    let (visibility, code, message, details) = error.into_parts();
    match visibility {
        ErrorVisibility::User => {
            let payload = json!({
                "code": code,
                "message": message,
                "details": details,
            });
            ToolExecutionResult::tool_error(payload.to_string())
        }
        // `ErrorVisibility` is non-exhaustive; treat anything unknown as
        // internal so new variants never leak details to the model.
        _ => ToolExecutionResult::internal_error_msg(format!(
            "capability error [{}]: {}{}",
            code,
            message,
            details
                .map(|details| format!("; details={details}"))
                .unwrap_or_default()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use everruns_core::tool_context::ToolContext;
    use everruns_core::tools::Tool as _;
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::SessionId;
    use everruns_test_support::LlmSimConfig;
    use serde_json::{Value, json};
    use tokio::sync::Notify;
    use tokio::time::timeout;

    use super::*;
    use crate::{Agent, CancellationToken, Model, RunOptions, SessionEventKind, TurnStopReason};

    #[derive(Deserialize, JsonSchema)]
    struct LookupInput {
        city: String,
    }

    #[derive(Debug, Serialize, JsonSchema)]
    struct LookupOutput {
        city: String,
        temperatures: Vec<i32>,
    }

    struct Lookup;

    #[async_trait]
    impl Handler for Lookup {
        type Input = LookupInput;
        type Output = LookupOutput;
        type Error = Error;

        fn name(&self) -> &str {
            "lookup_weather"
        }

        fn description(&self) -> &str {
            "Look up a typed weather forecast."
        }

        fn hints(&self) -> Hints {
            Hints::default().readonly(true).idempotent(true)
        }

        async fn execute(
            &self,
            input: Self::Input,
            context: Context,
        ) -> Result<Self::Output, Self::Error> {
            context.progress("forecast ready").await;
            Ok(LookupOutput {
                city: input.city,
                temperatures: vec![18, 21],
            })
        }
    }

    fn lookup_capability() -> Definition {
        Definition::new("weather", "Weather", "Typed weather tools.")
            .metadata(json!({ "owner": "example" }))
            .tool(Lookup)
    }

    fn scripted_model(calls: Vec<Vec<ToolCall>>) -> Model {
        let sim = LlmSimConfig::fixed("done").with_tool_call_sequence(calls);
        Model::simulated_with_config(sim)
    }

    #[test]
    fn exposes_input_output_schemas_and_hints() {
        let capability = lookup_capability();
        let spec = capability.tools()[0].spec();

        assert_eq!(spec.name(), "lookup_weather");
        assert_eq!(spec.input_schema()["type"], "object");
        assert_eq!(spec.output_schema()["type"], "object");
        assert_eq!(
            spec.output_schema()["properties"]["temperatures"]["type"],
            "array"
        );
        assert_eq!(spec.hints().readonly, Some(true));
        assert_eq!(spec.hints().idempotent, Some(true));
        assert_eq!(
            capability.metadata_value(),
            Some(&json!({ "owner": "example" }))
        );
    }

    #[tokio::test]
    async fn typed_non_string_result_serializes_as_json() {
        let tool = CoreToolAdapter(lookup_capability().tools()[0].clone());
        let context = ToolContext::new(SessionId::new())
            .with_cancellation(tokio_util::sync::CancellationToken::new());
        let result = tool
            .execute_with_context(json!({ "city": "Kyiv" }), &context)
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success, got {result:?}");
        };
        assert_eq!(value, json!({ "city": "Kyiv", "temperatures": [18, 21] }));
    }

    struct Failing;

    #[async_trait]
    impl Handler for Failing {
        type Input = LookupInput;
        type Output = LookupOutput;
        type Error = Error;

        fn name(&self) -> &str {
            "failing_lookup"
        }

        fn description(&self) -> &str {
            "Return a structured not-found error."
        }

        async fn execute(
            &self,
            input: Self::Input,
            _context: Context,
        ) -> Result<Self::Output, Self::Error> {
            Err(Error::user("city_not_found", "No forecast exists")
                .details(json!({ "city": input.city })))
        }
    }

    #[tokio::test]
    async fn structured_user_error_keeps_code_message_and_details() {
        let capability = Definition::new("failures", "Failures", "Error test.").tool(Failing);
        let tool = CoreToolAdapter(capability.tools()[0].clone());
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(json!({ "city": "Atlantis" }), &context)
            .await;

        let ToolExecutionResult::ToolError(payload) = result else {
            panic!("expected tool error, got {result:?}");
        };
        let payload: Value = serde_json::from_str(&payload).expect("structured JSON error");
        assert_eq!(payload["code"], "city_not_found");
        assert_eq!(payload["message"], "No forecast exists");
        assert_eq!(payload["details"]["city"], "Atlantis");
    }

    #[test]
    fn internal_error_details_do_not_cross_the_model_boundary() {
        let result = error_into_execution_result(
            Error::internal("upstream_failed", "private endpoint returned token=secret")
                .details(json!({ "trace": "private-trace" })),
        )
        .into_tool_result("call_private", "private_tool");

        let visible = result.error.expect("generic model-visible error");
        assert_eq!(
            visible,
            "An internal error occurred while executing the tool"
        );
        assert!(!visible.contains("secret"));
        assert!(!visible.contains("private-trace"));
    }

    #[tokio::test]
    async fn capability_runs_end_to_end_and_emits_progress() {
        let model = scripted_model(vec![
            vec![ToolCall {
                id: "call_weather".into(),
                name: "lookup_weather".into(),
                arguments: json!({ "city": "Kyiv" }),
            }],
            vec![],
        ]);
        let agent = Agent::builder()
            .instructions("Use the weather capability.")
            .model(model)
            .capability(lookup_capability())
            .build()
            .expect("valid agent");
        let session = agent.session();
        let mut events = session.events();

        let turn = session.run("Weather?").await.expect("turn runs");
        assert!(turn.success, "turn should recover: {:?}", turn.error);
        assert_eq!(turn.tool_calls, 1);

        let mut saw_progress = false;
        while let Ok(Ok(Some(event))) = timeout(Duration::from_millis(50), events.recv()).await {
            if matches!(
                event.kind,
                SessionEventKind::ToolProgress {
                    ref tool_name,
                    ref message,
                    ..
                } if tool_name == "lookup_weather" && message == "forecast ready"
            ) {
                saw_progress = true;
                break;
            }
        }
        assert!(saw_progress, "advanced context emitted correlated progress");
    }

    #[tokio::test]
    async fn user_error_is_a_recoverable_runtime_tool_error() {
        let model = scripted_model(vec![
            vec![ToolCall {
                id: "call_failure".into(),
                name: "failing_lookup".into(),
                arguments: json!({ "city": "Atlantis" }),
            }],
            vec![],
        ]);
        let capability = Definition::new("failures", "Failures", "Error test.").tool(Failing);
        let agent = Agent::builder()
            .instructions("Try the lookup and handle errors.")
            .model(model)
            .capability(capability)
            .build()
            .expect("valid agent");

        let turn = agent
            .session()
            .run("Find Atlantis")
            .await
            .expect("turn runs");
        assert!(turn.success, "model should recover from a tool error");
        assert_eq!(turn.tool_calls, 1);
    }

    struct Completing {
        child_cancelled: Arc<Notify>,
    }

    #[async_trait]
    impl Handler for Completing {
        type Input = LookupInput;
        type Output = LookupOutput;
        type Error = Error;

        fn name(&self) -> &str {
            "completing_lookup"
        }

        fn description(&self) -> &str {
            "Return successfully after starting call-scoped child work."
        }

        async fn execute(
            &self,
            input: Self::Input,
            context: Context,
        ) -> Result<Self::Output, Self::Error> {
            let cancellation = context.cancellation().clone();
            let child_cancelled = self.child_cancelled.clone();
            tokio::spawn(async move {
                cancellation.cancelled().await;
                child_cancelled.notify_one();
            });
            Ok(LookupOutput {
                city: input.city,
                temperatures: vec![],
            })
        }
    }

    #[tokio::test]
    async fn successful_call_completion_notifies_child_work() {
        let child_cancelled = Arc::new(Notify::new());
        let capability =
            Definition::new("completing", "Completing", "Completion test.").tool(Completing {
                child_cancelled: child_cancelled.clone(),
            });
        let model = scripted_model(vec![
            vec![ToolCall {
                id: "call_completing".into(),
                name: "completing_lookup".into(),
                arguments: json!({ "city": "Kyiv" }),
            }],
            vec![],
        ]);
        let agent = Agent::builder()
            .instructions("Run the completing lookup.")
            .model(model)
            .capability(capability)
            .build()
            .expect("valid agent");

        let turn = agent.session().run("start").await.expect("turn runs");
        assert!(turn.success);
        timeout(Duration::from_secs(2), child_cancelled.notified())
            .await
            .expect("child observed successful call completion");
    }

    struct Hanging {
        started: Arc<Notify>,
        child_cancelled: Arc<Notify>,
        cancellation_seen: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Handler for Hanging {
        type Input = LookupInput;
        type Output = LookupOutput;
        type Error = Error;

        fn name(&self) -> &str {
            "hanging_lookup"
        }

        fn description(&self) -> &str {
            "Wait until the enclosing turn is cancelled."
        }

        async fn execute(
            &self,
            _input: Self::Input,
            context: Context,
        ) -> Result<Self::Output, Self::Error> {
            let cancellation = context.cancellation().clone();
            let seen = self.cancellation_seen.clone();
            let child_cancelled = self.child_cancelled.clone();
            tokio::spawn(async move {
                cancellation.cancelled().await;
                seen.store(true, Ordering::SeqCst);
                child_cancelled.notify_one();
            });
            self.started.notify_one();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn turn_cancellation_stops_tool_and_notifies_child_work() {
        let started = Arc::new(Notify::new());
        let child_cancelled = Arc::new(Notify::new());
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let capability =
            Definition::new("hanging", "Hanging", "Cancellation test.").tool(Hanging {
                started: started.clone(),
                child_cancelled: child_cancelled.clone(),
                cancellation_seen: cancellation_seen.clone(),
            });
        let model = scripted_model(vec![vec![ToolCall {
            id: "call_hanging".into(),
            name: "hanging_lookup".into(),
            arguments: json!({ "city": "Kyiv" }),
        }]]);
        let agent = Agent::builder()
            .instructions("Run the hanging lookup.")
            .model(model)
            .capability(capability)
            .build()
            .expect("valid agent");
        let cancellation = CancellationToken::new();
        let cancel_from_test = cancellation.clone();

        let run = tokio::spawn(async move {
            agent
                .session()
                .run_with("start", RunOptions::new().cancel_token(cancellation))
                .await
        });
        timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("tool started");
        cancel_from_test.cancel();

        let turn = timeout(Duration::from_secs(2), run)
            .await
            .expect("run stopped")
            .expect("task joined")
            .expect("run result");
        assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
        timeout(Duration::from_secs(2), child_cancelled.notified())
            .await
            .expect("child observed call cancellation");
        assert!(cancellation_seen.load(Ordering::SeqCst));
    }
}
