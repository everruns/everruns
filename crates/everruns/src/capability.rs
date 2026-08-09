//! Stable service-provider interface for advanced, code-defined capabilities.
//!
//! Most application tools should use [`#[everruns::tool]`](macro@crate::tool).
//! This module is for reusable capability packages that need several typed
//! tools, capability-level instructions and metadata, execution context,
//! progress events, or call-scoped cancellation. It deliberately projects the
//! engine contracts needed by capability authors without exposing registries,
//! stores, tenancy, or host implementation types.
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
//!     .advanced_capability(weather)
//!     .build()?;
//! # let _ = agent;
//! # Ok::<(), everruns::BuildError>(())
//! ```

#![deny(missing_docs)]

use std::fmt;
use std::sync::Arc;

use everruns_core::capabilities::Capability as CoreCapability;
use everruns_core::tools::{Tool as CoreTool, ToolExecutionResult};
use everruns_core::{ToolContext, ToolHints};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub use async_trait::async_trait;
pub use schemars::{self, JsonSchema};
pub use serde::{self, Deserialize, Serialize};
pub use serde_json;

/// A reusable code-defined capability.
///
/// A definition is an immutable value: clone it to install the same capability
/// on several agents. The runtime registers it privately when the agent's
/// session starts; capability authors never manipulate an engine registry.
#[derive(Clone)]
pub struct Definition {
    id: String,
    name: String,
    description: String,
    instructions: Option<String>,
    metadata: Option<Value>,
    tools: Vec<Tool>,
}

impl Definition {
    /// Start a capability definition.
    ///
    /// `id` is the stable persisted identifier. `name` and `description` are
    /// human-facing catalog text. These values, tool names, and tool input
    /// schemas are validated by [`AgentBuilder::build`](crate::AgentBuilder::build).
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            instructions: None,
            metadata: None,
            tools: Vec::new(),
        }
    }

    /// Add capability-level behavioral guidance to the agent's system prompt.
    ///
    /// Do not repeat facts already expressed by tool names, descriptions, or
    /// schemas. Use this for cross-tool ordering, constraints, and semantics.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Attach host-owned, JSON metadata for catalogs and embedding hosts.
    ///
    /// The engine does not interpret this value. It may be persisted or shown
    /// to clients, so it must never contain credentials or sensitive payloads.
    pub fn metadata(mut self, metadata: impl Into<Value>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    /// Add one typed tool handler.
    pub fn tool<H>(mut self, handler: H) -> Self
    where
        H: Handler,
    {
        self.tools.push(Tool::new(handler));
        self
    }

    /// The stable capability identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The human-readable capability name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The capability description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Capability-level instructions, when configured.
    pub fn instructions_text(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Host-owned capability metadata, when configured.
    pub fn metadata_value(&self) -> Option<&Value> {
        self.metadata.as_ref()
    }

    /// Typed tool descriptors in registration order.
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "capability")?;
        if self.name.trim().is_empty() {
            return Err("capability name must not be blank".to_string());
        }
        if self.description.trim().is_empty() {
            return Err("capability description must not be blank".to_string());
        }
        if self
            .instructions
            .as_ref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err("capability instructions must not be blank".to_string());
        }
        if self.tools.is_empty() {
            return Err("capability must define at least one tool".to_string());
        }
        if let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.spec.description.trim().is_empty())
        {
            return Err(format!(
                "tool {:?} description must not be blank",
                tool.spec.name
            ));
        }
        Ok(())
    }

    pub(crate) fn runtime_adapter(&self) -> RuntimeDefinition {
        RuntimeDefinition(self.clone())
    }
}

fn validate_identifier(value: &str, noun: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{noun} id must not be empty"));
    }
    if value.len() > 64 {
        return Err(format!("{noun} id must be at most 64 characters"));
    }
    let mut chars = value.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!("{noun} id must start with a letter or underscore"));
    }
    if value
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
    {
        return Err(format!(
            "{noun} id may only contain letters, digits, '_' or '-'"
        ));
    }
    Ok(())
}

impl fmt::Debug for Definition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Definition")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("instructions", &self.instructions)
            .field("metadata", &self.metadata)
            .field("tools", &self.tools)
            .finish()
    }
}

pub(crate) struct RuntimeDefinition(Definition);

#[async_trait]
impl CoreCapability for RuntimeDefinition {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn name(&self) -> &str {
        &self.0.name
    }

    fn description(&self) -> &str {
        &self.0.description
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        self.0.instructions.as_deref()
    }

    fn metadata(&self) -> Option<Value> {
        self.0.metadata.clone()
    }

    fn tools(&self) -> Vec<Box<dyn CoreTool>> {
        self.0
            .tools
            .iter()
            .cloned()
            .map(|tool| Box::new(tool) as Box<dyn CoreTool>)
            .collect()
    }
}

/// Implemented by one typed tool inside an advanced capability.
///
/// Input values are deserialized after the model calls the tool. Output values
/// are serialized as JSON without a `String` intermediary. Both types also
/// provide schemas, allowing hosts and documentation to inspect the full
/// protocol even though the current model protocol consumes only the input
/// schema.
#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// Typed tool arguments.
    type Input: DeserializeOwned + JsonSchema + Send + 'static;
    /// Typed, JSON-serializable tool result.
    type Output: Serialize + JsonSchema + Send + 'static;
    /// Structured handler error. Custom error enums can implement `Into<Error>`.
    type Error: Into<Error> + Send + 'static;

    /// Stable model-facing tool name.
    fn name(&self) -> &str;
    /// Model-facing description of when and how to use the tool.
    fn description(&self) -> &str;

    /// Optional human-readable display name for clients.
    fn display_name(&self) -> Option<&str> {
        None
    }

    /// Semantic and host-owned tool metadata.
    fn hints(&self) -> Hints {
        Hints::default()
    }

    /// Execute one call.
    ///
    /// Awaited work is cancelled by dropping this future when the turn stops.
    /// Use [`Context::cancellation`] for child tasks or resources that can
    /// outlive this future, and [`Context::progress`] for correlated status.
    async fn execute(
        &self,
        input: Self::Input,
        context: Context,
    ) -> Result<Self::Output, Self::Error>;
}

/// A type-erased typed tool plus its stable protocol descriptor.
///
/// Constructed through [`Definition::tool`] in normal use. The public accessor
/// exists so capability packages can test and document their exported schema.
#[derive(Clone)]
pub struct Tool {
    spec: ToolSpec,
    handler: Arc<dyn ErasedHandler>,
}

impl Tool {
    fn new<H: Handler>(handler: H) -> Self {
        let spec = ToolSpec {
            name: handler.name().to_string(),
            display_name: handler.display_name().map(str::to_string),
            description: handler.description().to_string(),
            input_schema: schema_for::<H::Input>(),
            output_schema: schema_for::<H::Output>(),
            hints: handler.hints(),
        };
        Self {
            spec,
            handler: Arc::new(HandlerAdapter(handler)),
        }
    }

    /// The stable tool protocol descriptor.
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }
}

impl fmt::Debug for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tool").field("spec", &self.spec).finish()
    }
}

#[async_trait]
impl CoreTool for Tool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn display_name(&self) -> Option<&str> {
        self.spec.display_name.as_deref()
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters_schema(&self) -> Value {
        self.spec.input_schema.clone()
    }

    fn hints(&self) -> ToolHints {
        self.spec.hints.to_core()
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
        let context = Context::from_core(self.spec.name.clone(), context.clone());
        self.handler.call(arguments, context).await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[async_trait]
trait ErasedHandler: Send + Sync {
    async fn call(&self, input: Value, context: Context) -> ToolExecutionResult;
}

struct HandlerAdapter<H>(H);

#[async_trait]
impl<H: Handler> ErasedHandler for HandlerAdapter<H> {
    async fn call(&self, input: Value, context: Context) -> ToolExecutionResult {
        let input = match serde_json::from_value::<H::Input>(input) {
            Ok(input) => input,
            Err(error) => {
                return Error::user(
                    "invalid_arguments",
                    format!("tool arguments did not match the declared schema: {error}"),
                )
                .into_execution_result();
            }
        };
        match self.0.execute(input, context).await {
            Ok(output) => match serde_json::to_value(output) {
                Ok(output) => ToolExecutionResult::success(output),
                Err(error) => Error::internal(
                    "result_serialization",
                    format!("failed to serialize capability result: {error}"),
                )
                .into_execution_result(),
            },
            Err(error) => error.into().into_execution_result(),
        }
    }
}

fn schema_for<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// Stable metadata and JSON schemas for one capability tool.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    name: String,
    display_name: Option<String>,
    description: String,
    input_schema: Value,
    output_schema: Value,
    hints: Hints,
}

impl ToolSpec {
    /// Stable model-facing name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Optional human-readable display name.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
    /// Model-facing tool description.
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Generated JSON Schema for [`Handler::Input`].
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }
    /// Generated JSON Schema for [`Handler::Output`].
    pub fn output_schema(&self) -> &Value {
        &self.output_schema
    }
    /// Semantic and host-owned annotations.
    pub fn hints(&self) -> &Hints {
        &self.hints
    }
}

/// Semantic and host-owned annotations for a capability tool.
///
/// Boolean values are `Option<bool>` so unspecified remains distinct from
/// false. Hints inform scheduling and clients; they do not grant authority.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hints {
    /// The tool does not modify state.
    pub readonly: Option<bool>,
    /// The tool may irreversibly delete or destroy state.
    pub destructive: Option<bool>,
    /// Repeating a call with the same arguments is safe.
    pub idempotent: Option<bool>,
    /// The tool interacts with systems outside the local process.
    pub open_world: Option<bool>,
    /// The tool may commonly take more than a few seconds.
    pub long_running: Option<bool>,
    /// Calls sharing this non-empty key execute sequentially.
    pub concurrency_class: Option<String>,
    /// Host-owned annotations. Never include credentials or sensitive data.
    pub metadata: Option<Value>,
}

impl Hints {
    /// Mark whether the tool is read-only.
    pub fn readonly(mut self, value: bool) -> Self {
        self.readonly = Some(value);
        self
    }
    /// Mark whether the tool can destroy state.
    pub fn destructive(mut self, value: bool) -> Self {
        self.destructive = Some(value);
        self
    }
    /// Mark whether identical calls are safe to repeat.
    pub fn idempotent(mut self, value: bool) -> Self {
        self.idempotent = Some(value);
        self
    }
    /// Mark whether the tool reaches external systems.
    pub fn open_world(mut self, value: bool) -> Self {
        self.open_world = Some(value);
        self
    }
    /// Mark whether the tool is commonly long-running.
    pub fn long_running(mut self, value: bool) -> Self {
        self.long_running = Some(value);
        self
    }
    /// Serialize calls that share this scheduling class.
    pub fn concurrency_class(mut self, value: impl Into<String>) -> Self {
        self.concurrency_class = Some(value.into());
        self
    }
    /// Attach host-owned JSON annotations.
    pub fn metadata(mut self, value: impl Into<Value>) -> Self {
        self.metadata = Some(value.into());
        self
    }

    fn to_core(&self) -> ToolHints {
        ToolHints {
            readonly: self.readonly,
            destructive: self.destructive,
            idempotent: self.idempotent,
            open_world: self.open_world,
            long_running: self.long_running,
            concurrency_class: self.concurrency_class.clone(),
            metadata: self.metadata.clone(),
            ..ToolHints::default()
        }
    }
}

/// Runtime context available to an advanced capability tool.
///
/// This is a narrow projection: identity, locale, progress, and cancellation.
/// Backend stores, credentials, tenant objects, registries, and host extensions
/// intentionally remain private implementation details.
#[derive(Clone)]
pub struct Context {
    tool_name: String,
    session_id: String,
    workspace_id: String,
    locale: Option<String>,
    cancellation: CallCancellation,
    inner: ToolContext,
}

impl Context {
    fn from_core(tool_name: String, inner: ToolContext) -> Self {
        Self {
            tool_name,
            session_id: inner.session_id.to_string(),
            workspace_id: inner.workspace_id.to_string(),
            locale: inner.locale.clone(),
            cancellation: CallCancellation {
                inner: inner.cancellation.clone().unwrap_or_default(),
            },
            inner,
        }
    }

    /// Opaque session identifier for correlation and application-side scoping.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Opaque identifier of the workspace attached to this session.
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// Resolved BCP 47 locale, when the host supplied one.
    pub fn locale(&self) -> Option<&str> {
        self.locale.as_deref()
    }

    /// Call-scoped cancellation signal for work that may outlive `execute`.
    pub fn cancellation(&self) -> &CallCancellation {
        &self.cancellation
    }

    /// Emit a best-effort, correlated `tool.progress` event.
    ///
    /// Delivery failure never fails the tool. Subscribe with
    /// [`Session::events`](crate::Session::events) and match
    /// [`SessionEventKind::ToolProgress`](crate::SessionEventKind::ToolProgress).
    pub async fn progress(&self, message: impl AsRef<str>) {
        self.inner
            .emit_progress(&self.tool_name, message.as_ref())
            .await;
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("tool_name", &self.tool_name)
            .field("session_id", &self.session_id)
            .field("workspace_id", &self.workspace_id)
            .field("locale", &self.locale)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish()
    }
}

/// Cancellation signal tied to one tool call's lifetime.
///
/// The engine cancels it when the call returns, fails, or is dropped because
/// the turn was cancelled. Ordinary awaited work needs no special handling:
/// its future is dropped with the call. Clone this signal into child tasks,
/// processes, or watchers that could otherwise outlive `execute`.
#[derive(Clone)]
pub struct CallCancellation {
    inner: tokio_util::sync::CancellationToken,
}

impl CallCancellation {
    /// Whether the tool call has ended or been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Resolve when the tool call ends or is cancelled.
    pub async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

impl fmt::Debug for CallCancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Whether a capability error is safe to show to the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorVisibility {
    /// Expected domain failure; code, message, and details are model-visible.
    User,
    /// Unexpected implementation failure; details are hidden from the model.
    Internal,
}

/// A structured capability error with stable code, message, and JSON details.
///
/// Use [`Error::user`] for expected failures the model can act on and
/// [`Error::internal`] for diagnostic details that are unsafe to show to the
/// model. Internal messages are logged by the engine and replaced with a
/// generic model-visible error, so they must not contain credentials or other
/// secrets.
#[derive(Debug)]
pub struct Error {
    visibility: ErrorVisibility,
    code: String,
    message: String,
    details: Option<Value>,
}

impl Error {
    /// Create an expected, model-visible domain error.
    pub fn user(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            visibility: ErrorVisibility::User,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Create an internal error whose details must not reach the model.
    pub fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            visibility: ErrorVisibility::Internal,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Attach structured JSON details.
    pub fn details(mut self, details: impl Into<Value>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Stable machine-readable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human-readable error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Structured details, when present.
    pub fn details_value(&self) -> Option<&Value> {
        self.details.as_ref()
    }

    /// Whether the error is model-visible or internal.
    pub fn visibility(&self) -> ErrorVisibility {
        self.visibility
    }

    fn into_execution_result(self) -> ToolExecutionResult {
        match self.visibility {
            ErrorVisibility::User => {
                let payload = json!({
                    "code": self.code,
                    "message": self.message,
                    "details": self.details,
                });
                ToolExecutionResult::tool_error(payload.to_string())
            }
            ErrorVisibility::Internal => ToolExecutionResult::internal_error_msg(format!(
                "capability error [{}]: {}{}",
                self.code,
                self.message,
                self.details
                    .map(|details| format!("; details={details}"))
                    .unwrap_or_default()
            )),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver};
    use everruns_core::tools::Tool as _;
    use everruns_core::{ModelSpec, Provider, SessionId, ToolCall, ToolContext};
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
        Model::with_provider(
            ModelSpec::on("llmsim", "llmsim-model"),
            Provider::new("llmsim", LlmSimDriver::new(sim)),
        )
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
        let tool = lookup_capability().tools()[0].clone();
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
        let context = ToolContext::new(SessionId::new());
        let result = capability.tools()[0]
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
        let result = Error::internal("upstream_failed", "private endpoint returned token=secret")
            .details(json!({ "trace": "private-trace" }))
            .into_execution_result()
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
            .advanced_capability(lookup_capability())
            .build()
            .expect("valid agent");
        let mut session = agent.session();
        let mut events = session.events();

        let turn = session.run("Weather?").await.expect("turn runs");
        assert!(turn.success, "turn should recover: {:?}", turn.error);
        assert_eq!(turn.tool_calls, 1);

        let mut saw_progress = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), events.recv()).await {
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
            .advanced_capability(capability)
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
            .advanced_capability(capability)
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
            .advanced_capability(capability)
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
