//! Code-defined capability authoring: typed tool handlers, schema metadata,
//! and structured execution errors.
//!
//! This module is the neutral service-provider interface for reusable
//! capability packages that need several typed tools, capability-level
//! instructions and metadata, execution context, progress events, or
//! call-scoped cancellation. It deliberately depends on no engine, host, or
//! async-runtime crate: hosts adapt [`Context`] onto their own runtime via
//! the [`ProgressSink`] and [`CancellationSignal`] seams, and third-party
//! capability crates can depend on `everruns-capability` alone.
//!
//! Application authors normally consume these types re-exported as
//! `everruns::capability`.
//!
//! # Example
//!
//! ```
//! use everruns_capability::definition::{
//!     self as capability, Context, Definition, Error, Handler,
//! };
//!
//! #[derive(capability::Deserialize, capability::JsonSchema)]
//! #[serde(crate = "everruns_capability::serde")]
//! #[schemars(crate = "everruns_capability::schemars")]
//! struct LookupInput {
//!     city: String,
//! }
//!
//! #[derive(capability::Serialize, capability::JsonSchema)]
//! #[serde(crate = "everruns_capability::serde")]
//! #[schemars(crate = "everruns_capability::schemars")]
//! struct LookupOutput {
//!     city: String,
//!     forecast: String,
//! }
//!
//! struct Lookup;
//!
//! #[capability::async_trait]
//! impl Handler for Lookup {
//!     type Input = LookupInput;
//!     type Output = LookupOutput;
//!     type Error = Error;
//!
//!     fn name(&self) -> &str { "lookup_weather" }
//!     fn description(&self) -> &str { "Look up the weather for a city." }
//!
//!     async fn execute(
//!         &self,
//!         input: Self::Input,
//!         context: Context,
//!     ) -> Result<Self::Output, Self::Error> {
//!         context.progress("Looking up the forecast").await;
//!         Ok(LookupOutput { city: input.city, forecast: "sunny".into() })
//!     }
//! }
//!
//! let weather = Definition::new("weather", "Weather", "Typed weather lookup tools.")
//!     .instructions("Use weather data only when a user asks about a location.")
//!     .tool(Lookup);
//!
//! weather.validate().unwrap();
//! assert_eq!(weather.tools()[0].spec().name(), "lookup_weather");
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::CapabilityError;
use crate::id::validate_capability_id;

pub use async_trait::async_trait;
pub use schemars::{self, JsonSchema};
pub use serde::{self, Deserialize, Serialize};
pub use serde_json;

/// A boxed future used by the host-adaptation seams in this module.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A reusable code-defined capability.
///
/// A definition is an immutable value: clone it to install the same
/// capability on several agents. The consuming host registers it privately
/// when the agent's session starts; capability authors never manipulate an
/// engine registry.
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
    /// schemas are validated by the consuming boundary (e.g. the Framework's
    /// `AgentBuilder::build`).
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

    /// Validate the definition's identity and structural rules.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        validate_capability_id(&self.id)?;
        let invalid = |reason: &str| CapabilityError::InvalidDefinition {
            id: self.id.clone(),
            reason: reason.to_string(),
        };
        if self.name.trim().is_empty() {
            return Err(invalid("capability name must not be blank"));
        }
        if self.description.trim().is_empty() {
            return Err(invalid("capability description must not be blank"));
        }
        if self
            .instructions
            .as_ref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err(invalid("capability instructions must not be blank"));
        }
        if self.tools.is_empty() {
            return Err(invalid("capability must define at least one tool"));
        }
        if let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.spec.description.trim().is_empty())
        {
            return Err(CapabilityError::InvalidDefinition {
                id: self.id.clone(),
                reason: format!("tool {:?} description must not be blank", tool.spec.name),
            });
        }
        Ok(())
    }
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

/// Implemented by one typed tool inside a code-defined capability.
///
/// Input values are deserialized after the model calls the tool. Output
/// values are serialized as JSON without a `String` intermediary. Both types
/// also provide schemas, allowing hosts and documentation to inspect the full
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
/// Constructed through [`Definition::tool`] in normal use. The public
/// accessors exist so capability packages can test and document their
/// exported schema, and so hosts can execute calls via [`Tool::invoke`].
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

    /// Execute one call: deserialize arguments, run the handler, serialize
    /// the typed output.
    ///
    /// Invalid arguments surface as a model-visible
    /// `invalid_arguments` [`Error`]; output serialization failures surface
    /// as an internal `result_serialization` [`Error`].
    pub async fn invoke(&self, arguments: Value, context: Context) -> Result<Value, Error> {
        self.handler.call(arguments, context).await
    }
}

impl fmt::Debug for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tool").field("spec", &self.spec).finish()
    }
}

#[async_trait]
trait ErasedHandler: Send + Sync {
    async fn call(&self, input: Value, context: Context) -> Result<Value, Error>;
}

struct HandlerAdapter<H>(H);

#[async_trait]
impl<H: Handler> ErasedHandler for HandlerAdapter<H> {
    async fn call(&self, input: Value, context: Context) -> Result<Value, Error> {
        let input = serde_json::from_value::<H::Input>(input).map_err(|error| {
            Error::user(
                "invalid_arguments",
                format!("tool arguments did not match the declared schema: {error}"),
            )
        })?;
        let output = self.0.execute(input, context).await.map_err(Into::into)?;
        serde_json::to_value(output).map_err(|error| {
            Error::internal(
                "result_serialization",
                format!("failed to serialize capability result: {error}"),
            )
        })
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
}

/// Host seam: deliver a best-effort, correlated progress event.
///
/// Implemented by the consuming runtime (e.g. `everruns` adapts this onto its
/// session event stream). Delivery failure must never fail the tool.
#[async_trait]
pub trait ProgressSink: Send + Sync {
    /// Deliver one progress message for the named tool.
    async fn emit(&self, tool_name: &str, message: &str);
}

/// Host seam: a call-scoped cancellation signal.
///
/// Implemented by the consuming runtime over its own cancellation primitive.
pub trait CancellationSignal: Send + Sync {
    /// Whether the call has ended or been cancelled.
    fn is_cancelled(&self) -> bool;

    /// Resolve when the call ends or is cancelled.
    fn cancelled<'a>(&'a self) -> BoxFuture<'a, ()>;
}

struct NoopProgressSink;

#[async_trait]
impl ProgressSink for NoopProgressSink {
    async fn emit(&self, _tool_name: &str, _message: &str) {}
}

struct NeverCancelled;

impl CancellationSignal for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }

    fn cancelled<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(std::future::pending())
    }
}

/// Runtime context available to a code-defined capability tool.
///
/// This is a narrow projection: identity, locale, progress, and cancellation.
/// Backend stores, credentials, tenant objects, registries, and host
/// extensions intentionally remain host implementation details. Hosts build
/// one per call via [`Context::new`] and the `with_*` seams; tests can do the
/// same without any runtime.
#[derive(Clone)]
pub struct Context {
    tool_name: String,
    session_id: String,
    workspace_id: String,
    locale: Option<String>,
    progress: Arc<dyn ProgressSink>,
    cancellation: CallCancellation,
}

impl Context {
    /// Create a context with no-op progress and a never-cancelled signal.
    pub fn new(
        tool_name: impl Into<String>,
        session_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            session_id: session_id.into(),
            workspace_id: workspace_id.into(),
            locale: None,
            progress: Arc::new(NoopProgressSink),
            cancellation: CallCancellation {
                inner: Arc::new(NeverCancelled),
            },
        }
    }

    /// Set the resolved BCP 47 locale.
    pub fn with_locale(mut self, locale: Option<String>) -> Self {
        self.locale = locale;
        self
    }

    /// Attach the host's progress delivery seam.
    pub fn with_progress_sink(mut self, sink: Arc<dyn ProgressSink>) -> Self {
        self.progress = sink;
        self
    }

    /// Attach the host's call-scoped cancellation signal.
    pub fn with_cancellation_signal(mut self, signal: Arc<dyn CancellationSignal>) -> Self {
        self.cancellation = CallCancellation { inner: signal };
        self
    }

    /// The tool name this context was created for.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
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
    /// Delivery failure never fails the tool.
    pub async fn progress(&self, message: impl AsRef<str>) {
        self.progress.emit(&self.tool_name, message.as_ref()).await;
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
/// The host cancels it when the call returns, fails, or is dropped because
/// the turn was cancelled. Ordinary awaited work needs no special handling:
/// its future is dropped with the call. Clone this signal into child tasks,
/// processes, or watchers that could otherwise outlive `execute`.
#[derive(Clone)]
pub struct CallCancellation {
    inner: Arc<dyn CancellationSignal>,
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
/// model. Internal messages are logged by the host and replaced with a
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

    /// Decompose the error for host serialization.
    pub fn into_parts(self) -> (ErrorVisibility, String, String, Option<Value>) {
        (self.visibility, self.code, self.message, self.details)
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
    use super::*;
    use serde_json::json;

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

    #[test]
    fn exposes_input_output_schemas_and_hints() {
        let capability = lookup_capability();
        let spec = capability.tools()[0].spec();

        assert_eq!(spec.name(), "lookup_weather");
        assert_eq!(spec.input_schema()["type"], "object");
        assert_eq!(spec.input_schema()["required"], json!(["city"]));
        assert_eq!(spec.input_schema()["properties"]["city"]["type"], "string");
        assert_eq!(spec.output_schema()["type"], "object");
        assert_eq!(spec.output_schema()["properties"]["city"]["type"], "string");
        assert_eq!(
            spec.output_schema()["properties"]["temperatures"]["type"],
            "array"
        );
        assert_eq!(
            spec.output_schema()["properties"]["temperatures"]["items"]["type"],
            "integer"
        );
        assert_eq!(spec.hints().readonly, Some(true));
        assert_eq!(spec.hints().idempotent, Some(true));
        assert_eq!(
            capability.metadata_value(),
            Some(&json!({ "owner": "example" }))
        );
        capability.validate().unwrap();
    }

    #[test]
    fn validate_rejects_structural_problems() {
        let err = Definition::new("weather", "Weather", "d")
            .validate()
            .unwrap_err();
        assert!(err.reason().contains("at least one tool"));

        let err = Definition::new("2fast", "N", "d")
            .tool(Lookup)
            .validate()
            .unwrap_err();
        assert!(err.reason().contains("start with a letter"));

        let err = Definition::new("weather", " ", "d")
            .tool(Lookup)
            .validate()
            .unwrap_err();
        assert!(err.reason().contains("name must not be blank"));
        for (definition, expected) in [
            (
                Definition::new("weather", "Weather", " ").tool(Lookup),
                "description must not be blank",
            ),
            (
                lookup_capability().instructions(" "),
                "instructions must not be blank",
            ),
        ] {
            assert!(
                definition
                    .validate()
                    .unwrap_err()
                    .reason()
                    .contains(expected)
            );
        }
        let mut malformed = lookup_capability();
        malformed.tools[0].spec.description = " ".into();
        assert!(
            malformed
                .validate()
                .unwrap_err()
                .reason()
                .contains("tool \"lookup_weather\" description must not be blank")
        );
    }

    // These local futures must finish in one poll; unexpected I/O fails promptly.
    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    #[derive(Default)]
    struct RecordingProgress(std::sync::Mutex<Vec<(String, String)>>);
    #[async_trait]
    impl ProgressSink for RecordingProgress {
        async fn emit(&self, tool: &str, message: &str) {
            self.0.lock().unwrap().push((tool.into(), message.into()));
        }
    }

    #[test]
    fn invoke_serializes_typed_output() {
        let tool = lookup_capability().tools()[0].clone();
        let progress = Arc::new(RecordingProgress::default());
        let context = Context::new("lookup_weather", "session", "workspace")
            .with_progress_sink(progress.clone());
        let value = ready(tool.invoke(json!({ "city": "Kyiv" }), context)).unwrap();
        assert_eq!(value, json!({ "city": "Kyiv", "temperatures": [18, 21] }));
        assert_eq!(
            *progress.0.lock().unwrap(),
            [("lookup_weather".into(), "forecast ready".into())]
        );
    }

    #[test]
    fn invoke_rejects_invalid_arguments_as_user_error() {
        let tool = lookup_capability().tools()[0].clone();
        let context = Context::new("lookup_weather", "session", "workspace");
        let error = ready(tool.invoke(json!({ "city": 42 }), context)).unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
        assert_eq!(error.visibility(), ErrorVisibility::User);
    }

    #[test]
    fn context_defaults_are_inert() {
        let context = Context::new("t", "s", "w");
        assert!(!context.cancellation().is_cancelled());
        ready(context.progress("no-op"));
        let mut cancelled = std::pin::pin!(context.cancellation().cancelled());
        let mut task = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(cancelled.as_mut().poll(&mut task).is_pending());
    }
}
