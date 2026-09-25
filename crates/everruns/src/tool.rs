//! Async functions and closures as agent tools (EVE-828).
//!
//! A library user adds a custom tool by handing the agent an async
//! function or closure — no capability registry, tool record, or backend
//! store. [`FunctionTool::new`] wraps a handler that receives the call
//! arguments as a [`serde_json::Value`] and returns any [`IntoToolResult`]
//! (a `serde_json::Value`, a `String`, or an explicit [`ToolResponse`]).
//!
//! [`AgentBuilder::tool`](crate::AgentBuilder::tool) accepts function tools via
//! [`IntoTool`]. Capability references use the separate, scalable
//! [`AgentBuilder::capability`](crate::AgentBuilder::capability) entrypoint. The
//! builder validates tool names and JSON schemas and rejects duplicates before
//! an [`Agent`](crate::Agent) is produced.
//!
//! Under the hood a `FunctionTool` is adapted to the core tool-execution
//! contract ([`everruns_core::tools::Tool`]) and registered as a single-tool,
//! closure-backed [`Capability`](everruns_core::capabilities::Capability) on the
//! in-process runtime, so the model can call it and the result flows back like
//! any built-in tool. Handler errors are treated as internal errors and redacted
//! before reaching the model; a handler never panics the turn.
//!
//! # Example
//!
//! ```
//! use everruns::{Agent, FunctionTool, Model};
//! use serde_json::json;
//!
//! let weather = FunctionTool::new(
//!     "get_weather",
//!     "Look up the weather for a city.",
//!     json!({
//!         "type": "object",
//!         "properties": { "city": { "type": "string" } },
//!         "required": ["city"],
//!     }),
//!     |args: serde_json::Value| async move {
//!         let city = args["city"].as_str().unwrap_or("unknown");
//!         Ok::<_, String>(json!({ "city": city, "forecast": "sunny" }))
//!     },
//! );
//!
//! let agent = Agent::builder()
//!     .instructions("You are a helpful weather assistant.")
//!     .model(Model::simulated("It is sunny."))
//!     .tool(weather)
//!     .build()?;
//! # let _ = agent;
//! # Ok::<(), everruns::BuildError>(())
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::capabilities::Capability;
use everruns_core::tools::{Tool as CoreTool, ToolExecutionResult};
use serde_json::Value;

/// A ready-to-return structured tool result.
///
/// The explicit adapter for handlers that need the full tool-result shape
/// rather than a bare `serde_json::Value` or `String`: return
/// `ToolResponse::error(..)` to surface a model-visible tool error from an
/// `Ok(..)` handler, or `ToolResponse::json(..)` / `ToolResponse::text(..)`
/// for a success. It maps onto the engine's existing structured tool output
/// without a library user importing any core type.
pub struct ToolResponse(ToolExecutionResult);

impl ToolResponse {
    /// A successful result carrying a JSON value.
    pub fn json(value: impl Into<Value>) -> Self {
        Self(ToolExecutionResult::success(value))
    }

    /// A successful result carrying plain text.
    pub fn text(text: impl Into<String>) -> Self {
        Self(ToolExecutionResult::success(Value::String(text.into())))
    }

    /// A model-visible tool error (the turn continues; the model sees the message).
    pub fn error(message: impl Into<String>) -> Self {
        Self(ToolExecutionResult::tool_error(message))
    }
}

impl fmt::Debug for ToolResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ToolResponse").field(&self.0).finish()
    }
}

/// Converts a handler's success value into the engine's tool-result shape.
///
/// Implemented for the ergonomic return types a handler may produce:
/// [`serde_json::Value`], [`String`], and the explicit [`ToolResponse`]
/// adapter. A handler returns `Result<T, E>` where `T: IntoToolResult`.
pub trait IntoToolResult {
    /// Consume `self` and produce the engine tool result.
    fn into_tool_result(self) -> ToolExecutionResult;
}

impl IntoToolResult for Value {
    fn into_tool_result(self) -> ToolExecutionResult {
        ToolExecutionResult::Success(self)
    }
}

impl IntoToolResult for String {
    fn into_tool_result(self) -> ToolExecutionResult {
        ToolExecutionResult::Success(Value::String(self))
    }
}

impl IntoToolResult for ToolResponse {
    fn into_tool_result(self) -> ToolExecutionResult {
        self.0
    }
}

/// The boxed, type-erased async handler behind a [`FunctionTool`].
type HandlerFn = Arc<
    dyn Fn(ToolCallContext, Value) -> Pin<Box<dyn Future<Output = ToolExecutionResult> + Send>>
        + Send
        + Sync,
>;

/// Decides from a call's arguments whether it needs approval.
pub(crate) type ApprovalPredicate = Arc<dyn Fn(&Value) -> bool + Send + Sync>;

/// What a [`FunctionTool`] handler knows about the call it is serving.
///
/// Handlers built with [`FunctionTool::with_context`] (or a
/// `#[everruns::tool]` function whose first parameter is a `ToolCallContext`)
/// receive one per call. It identifies the session, turn, and tool call so a
/// host serving many sessions can correlate work, and it reports progress
/// that observers see as [`SessionEventKind::ToolProgress`](crate::SessionEventKind::ToolProgress)
/// events on [`Session::events`](crate::Session::events).
///
/// Cheap to clone; clones describe the same call.
///
/// Stability: alpha — may change without a major bump; see
/// [`stability`](crate::stability).
#[derive(Clone)]
pub struct ToolCallContext {
    inner: Arc<ToolCallContextInner>,
}

struct ToolCallContextInner {
    tool_name: String,
    tool_call_id: String,
    turn_id: Option<String>,
    context: everruns_core::tool_context::ToolContext,
}

impl ToolCallContext {
    fn from_core(tool_name: &str, context: &everruns_core::tool_context::ToolContext) -> Self {
        Self {
            inner: Arc::new(ToolCallContextInner {
                tool_name: tool_name.to_string(),
                tool_call_id: context.tool_call_id.clone().unwrap_or_default(),
                turn_id: context
                    .event_context
                    .as_ref()
                    .and_then(|event| event.turn_id)
                    .map(|turn_id| turn_id.to_string()),
                context: context.clone(),
            }),
        }
    }

    /// The session this call belongs to.
    pub fn session_id(&self) -> crate::SessionId {
        self.inner.context.session_id
    }

    /// The turn that issued this call, when the runtime reported one.
    pub fn turn_id(&self) -> Option<String> {
        self.inner.turn_id.clone()
    }

    /// The model-assigned id of this tool call.
    ///
    /// Empty only when the tool is invoked outside an engine turn.
    pub fn tool_call_id(&self) -> &str {
        &self.inner.tool_call_id
    }

    /// The name of the tool being called.
    pub fn tool_name(&self) -> &str {
        &self.inner.tool_name
    }

    /// Report human-readable progress for this call.
    ///
    /// Best effort: the event is emitted to the session's event stream and
    /// never fails the call. Outside an engine turn it is a no-op.
    pub async fn progress(&self, message: impl Into<String>) {
        let message = message.into();
        self.inner
            .context
            .emit_progress(&self.inner.tool_name, &message)
            .await;
    }
}

impl fmt::Debug for ToolCallContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCallContext")
            .field("session_id", &self.session_id())
            .field("turn_id", &self.inner.turn_id)
            .field("tool_call_id", &self.inner.tool_call_id)
            .field("tool_name", &self.inner.tool_name)
            .finish()
    }
}

/// A custom agent tool backed by an async function or closure.
///
/// Construct one with [`FunctionTool::new`] and hand it to
/// [`AgentBuilder::tool`](crate::AgentBuilder::tool). The handler is invoked
/// with the model's call arguments deserialized into a [`serde_json::Value`]
/// and returns any [`IntoToolResult`]. A returned `Err` becomes a redacted
/// internal error — the turn is never panicked by a handler. Return
/// [`ToolResponse::error`] from the `Ok` path for errors that are safe to show
/// to the model.
///
/// The `Err` channel is redacted because a handler's error text is written
/// against the host — a path, a query, a connection string — and the model is
/// an untrusted reader of anything it is shown. The full error is still logged
/// with the tool name and call id, so nothing is lost for the developer.
/// `#[everruns::tool]` is unaffected: it maps a function's own `Err` to a
/// model-visible tool error before it reaches this channel, and reserves this
/// channel for failures the model did not cause.
///
/// A `FunctionTool` is cheap to clone (the handler is reference-counted) and is
/// `Send + Sync`, so the same tool can execute calls concurrently.
#[derive(Clone)]
pub struct FunctionTool {
    name: String,
    description: String,
    schema: Value,
    handler: HandlerFn,
    approval: Option<ApprovalPredicate>,
}

impl FunctionTool {
    /// Wrap an async handler as a callable tool.
    ///
    /// - `name` is the identifier the model calls (validated at
    ///   [`Agent::build`](crate::AgentBuilder::build)).
    /// - `description` tells the model when to use the tool.
    /// - `json_schema` is the JSON Schema for the tool's arguments (validated
    ///   at build time).
    /// - `handler` is an async `Fn(serde_json::Value) -> Result<T, E>` where
    ///   `T: IntoToolResult` (a `Value`, a `String`, or a [`ToolResponse`]) and
    ///   `E: Display`. On `Err`, the error's `Display` text is retained for
    ///   internal diagnostics but redacted from the model-facing tool result.
    pub fn new<F, Fut, T, E>(
        name: impl Into<String>,
        description: impl Into<String>,
        json_schema: Value,
        handler: F,
    ) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: IntoToolResult + 'static,
        E: fmt::Display + 'static,
    {
        let handler: HandlerFn = Arc::new(move |_context, args| {
            let fut = handler(args);
            Box::pin(async move { into_execution_result(fut.await) })
        });
        Self {
            name: name.into(),
            description: description.into(),
            schema: json_schema,
            handler,
            approval: None,
        }
    }

    /// Wrap an async handler that also receives the call's [`ToolCallContext`].
    ///
    /// Identical to [`new`](Self::new) except the handler takes the context
    /// first: use it to report [`progress`](ToolCallContext::progress) or to
    /// correlate work with the session, turn, and tool call.
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{FunctionTool, ToolCallContext};
    /// use serde_json::{Value, json};
    ///
    /// let export = FunctionTool::with_context(
    ///     "export",
    ///     "Export the report.",
    ///     json!({ "type": "object", "properties": {} }),
    ///     |ctx: ToolCallContext, _args: Value| async move {
    ///         ctx.progress("rendering").await;
    ///         Ok::<_, String>(json!({ "session": ctx.session_id().to_string() }))
    ///     },
    /// );
    /// assert_eq!(export.name(), "export");
    /// ```
    pub fn with_context<F, Fut, T, E>(
        name: impl Into<String>,
        description: impl Into<String>,
        json_schema: Value,
        handler: F,
    ) -> Self
    where
        F: Fn(ToolCallContext, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: IntoToolResult + 'static,
        E: fmt::Display + 'static,
    {
        let handler: HandlerFn = Arc::new(move |context, args| {
            let fut = handler(context, args);
            Box::pin(async move { into_execution_result(fut.await) })
        });
        Self {
            name: name.into(),
            description: description.into(),
            schema: json_schema,
            handler,
            approval: None,
        }
    }

    /// Require approval for calls whose arguments satisfy `predicate`.
    ///
    /// Before such a call runs, the approver set with
    /// [`AgentBuilder::approver`](crate::AgentBuilder::approver) is asked; a
    /// rejection reaches the model as a tool error and the handler never runs.
    /// `predicate` returning `false` runs the call without asking. An agent
    /// with an approval-gated tool and no approver fails to
    /// [`build`](crate::AgentBuilder::build) — the gate never fails open.
    ///
    /// Calling this again replaces the earlier rule.
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    pub fn needs_approval(
        mut self,
        predicate: impl Fn(&Value) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.approval = Some(Arc::new(predicate));
        self
    }

    /// Require approval before every call of this tool.
    ///
    /// Shorthand for [`needs_approval`](Self::needs_approval) with a predicate
    /// that always returns `true`.
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    pub fn always_needs_approval(self) -> Self {
        self.needs_approval(|_| true)
    }

    /// Whether any call of this tool may need approval.
    pub(crate) fn approval(&self) -> Option<&ApprovalPredicate> {
        self.approval.as_ref()
    }

    /// The tool's name (the identifier the model calls).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tool's argument JSON schema.
    pub(crate) fn schema(&self) -> &Value {
        &self.schema
    }

    /// Wrap this tool as a single-tool capability for runtime registration.
    pub(crate) fn into_capability(self) -> FunctionCapability {
        FunctionCapability { tool: self }
    }
}

impl fmt::Debug for FunctionTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("schema", &self.schema)
            .field("needs_approval", &self.approval.is_some())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CoreTool for FunctionTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Outside an engine turn there is no session: give context-aware
        // handlers a detached context whose progress reports go nowhere.
        let context = everruns_core::tool_context::ToolContext::new(crate::SessionId::new());
        self.execute_with_context(arguments, &context).await
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &everruns_core::tool_context::ToolContext,
    ) -> ToolExecutionResult {
        (self.handler)(ToolCallContext::from_core(&self.name, context), arguments).await
    }
}

fn into_execution_result<T: IntoToolResult, E: fmt::Display>(
    result: Result<T, E>,
) -> ToolExecutionResult {
    match result {
        Ok(value) => value.into_tool_result(),
        Err(err) => ToolExecutionResult::internal_error_msg(err.to_string()),
    }
}

/// A closure-backed [`Capability`] exposing exactly one [`FunctionTool`].
///
/// Its capability id equals the tool name. The Framework creates the matching
/// private activation when `.tool(...)` is built; applications never configure
/// this implementation as a public capability reference.
pub(crate) struct FunctionCapability {
    tool: FunctionTool,
}

impl Capability for FunctionCapability {
    fn id(&self) -> &str {
        &self.tool.name
    }

    fn name(&self) -> &str {
        &self.tool.name
    }

    fn description(&self) -> &str {
        &self.tool.description
    }

    fn tools(&self) -> Vec<Box<dyn CoreTool>> {
        vec![Box::new(self.tool.clone())]
    }
}

/// A closure-backed function tool accepted by [`AgentBuilder::tool`](crate::AgentBuilder::tool).
///
/// Capability references are deliberately not represented here; configure
/// those through [`AgentBuilder::capability`](crate::AgentBuilder::capability).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Tool {
    /// A custom tool backed by an async function or closure.
    Function(FunctionTool),
}

impl Tool {
    /// The model-facing name of this tool.
    pub(crate) fn name(&self) -> &str {
        match self {
            Tool::Function(tool) => &tool.name,
        }
    }

    pub(crate) fn into_function(self) -> FunctionTool {
        match self {
            Self::Function(tool) => tool,
        }
    }
}

/// Conversion into a [`Tool`] for [`AgentBuilder::tool`](crate::AgentBuilder::tool).
///
/// Implemented for [`FunctionTool`] and the [`Tool`] wrapper emitted by public
/// tool adapters. Capability IDs implement
/// [`IntoCapability`](crate::IntoCapability), not `IntoTool`.
pub trait IntoTool {
    /// Convert `self` into a [`Tool`].
    fn into_tool(self) -> Tool;
}

impl IntoTool for Tool {
    fn into_tool(self) -> Tool {
        self
    }
}

impl IntoTool for FunctionTool {
    fn into_tool(self) -> Tool {
        Tool::Function(self)
    }
}

/// Validate a tool name for use as a model-facing tool identifier.
///
/// Accepts `[A-Za-z0-9_-]{1,64}` starting with a letter or underscore — the
/// common intersection of provider tool-name rules. Returns a human-readable
/// reason on rejection.
pub(crate) fn validate_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("tool name must not be empty".to_string());
    }
    if name.len() > 64 {
        return Err(format!(
            "tool name must be at most 64 characters (got {})",
            name.len()
        ));
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "tool name must start with a letter or underscore (got {first:?})"
        ));
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "tool name may only contain letters, digits, '_' or '-' (got {bad:?})"
        ));
    }
    Ok(())
}

/// Validate a tool's JSON argument schema.
///
/// The schema must be a JSON object, and when it declares a top-level `type`
/// that type must be `"object"` (the model always calls a tool with an object
/// of arguments).
pub(crate) fn validate_tool_schema(schema: &Value) -> Result<(), String> {
    let Some(object) = schema.as_object() else {
        return Err("JSON schema must be a JSON object".to_string());
    };
    if let Some(type_value) = object.get("type") {
        match type_value.as_str() {
            Some("object") => {}
            Some(other) => {
                return Err(format!(
                    "JSON schema top-level \"type\" must be \"object\" (got {other:?})"
                ));
            }
            None => {
                return Err("JSON schema \"type\" must be a string".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // `CoreTool` (the core `Tool` trait) is in scope via `super::*`, so tool
    // handlers can be driven directly through `execute`.
    use super::*;
    use serde_json::json;

    fn obj_schema() -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    #[tokio::test]
    async fn handler_receives_args_and_returns_json() {
        let tool = FunctionTool::new(
            "echo",
            "Echo the input back.",
            obj_schema(),
            |args: Value| async move { Ok::<_, String>(json!({ "echoed": args })) },
        );
        let result = tool.execute(json!({ "a": 1, "b": "two" })).await;
        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["echoed"]["a"], json!(1));
                assert_eq!(value["echoed"]["b"], json!("two"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn string_result_maps_to_success() {
        let tool = FunctionTool::new(
            "shout",
            "Return text.",
            obj_schema(),
            |_args: Value| async move { Ok::<_, String>("hello".to_string()) },
        );
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::Success(Value::String(s)) => assert_eq!(s, "hello"),
            other => panic!("expected string success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_error_maps_to_internal_error_without_panicking() {
        let tool = FunctionTool::new(
            "boom",
            "Always fails.",
            obj_schema(),
            |_args: Value| async move { Err::<Value, String>("kaboom".to_string()) },
        );
        let result = tool.execute(json!({})).await;
        assert!(matches!(result, ToolExecutionResult::InternalError(_)));

        let visible = result
            .into_tool_result("call_boom", "boom")
            .error
            .expect("model-visible error");
        assert_eq!(
            visible,
            "An internal error occurred while executing the tool"
        );
        assert!(!visible.contains("kaboom"));
    }

    #[tokio::test]
    async fn tool_response_adapter_can_return_error_from_ok() {
        let tool = FunctionTool::new(
            "structured",
            "Returns a structured tool error from an Ok path.",
            obj_schema(),
            |_args: Value| async move { Ok::<_, String>(ToolResponse::error("not found")) },
        );
        match tool.execute(json!({})).await {
            ToolExecutionResult::ToolError(message) => assert_eq!(message, "not found"),
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn function_tool_executes_concurrently() {
        let tool = Arc::new(FunctionTool::new(
            "double",
            "Double a number.",
            obj_schema(),
            |args: Value| async move {
                let n = args["n"].as_i64().unwrap_or(0);
                Ok::<_, String>(json!({ "result": n * 2 }))
            },
        ));

        let mut handles = Vec::new();
        for n in 0..16i64 {
            let tool = tool.clone();
            handles.push(tokio::spawn(async move {
                let result = tool.execute(json!({ "n": n })).await;
                match result {
                    ToolExecutionResult::Success(value) => value["result"].as_i64().unwrap(),
                    other => panic!("expected success, got {other:?}"),
                }
            }));
        }
        for (n, handle) in handles.into_iter().enumerate() {
            assert_eq!(handle.await.unwrap(), n as i64 * 2);
        }
    }

    #[tokio::test]
    async fn with_context_handler_sees_the_core_call_context() {
        let tool = FunctionTool::with_context(
            "whoami",
            "Report the call.",
            obj_schema(),
            |ctx: ToolCallContext, _args: Value| async move {
                Ok::<_, String>(json!({
                    "session": ctx.session_id().to_string(),
                    "call": ctx.tool_call_id(),
                    "tool": ctx.tool_name(),
                    "turn": ctx.turn_id(),
                }))
            },
        );
        let session_id = crate::SessionId::new();
        let mut context = everruns_core::tool_context::ToolContext::new(session_id);
        context.tool_call_id = Some("call_7".to_string());
        match tool.execute_with_context(json!({}), &context).await {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["session"], json!(session_id.to_string()));
                assert_eq!(value["call"], json!("call_7"));
                assert_eq!(value["tool"], json!("whoami"));
                assert_eq!(value["turn"], Value::Null);
            }
            other => panic!("expected success, got {other:?}"),
        }
        // Progress outside a turn (no emitter) is a silent no-op.
        let detached = ToolCallContext::from_core("whoami", &context);
        detached.progress("ignored").await;
    }

    #[tokio::test]
    async fn plain_handlers_still_run_through_execute_with_context() {
        let tool = FunctionTool::new("plain", "Plain.", obj_schema(), |args: Value| async move {
            Ok::<_, String>(args)
        });
        let context = everruns_core::tool_context::ToolContext::new(crate::SessionId::new());
        match tool.execute_with_context(json!({ "a": 1 }), &context).await {
            ToolExecutionResult::Success(value) => assert_eq!(value, json!({ "a": 1 })),
            other => panic!("expected success, got {other:?}"),
        }
        assert!(tool.approval().is_none());
    }

    #[test]
    fn approval_builders_set_and_replace_the_rule() {
        let tool = FunctionTool::new("gate", "Gate.", obj_schema(), |_: Value| async move {
            Ok::<_, String>(json!({}))
        });
        let ruled = tool
            .clone()
            .needs_approval(|args| args["risky"] == json!(true));
        let predicate = ruled.approval().expect("rule set");
        assert!(predicate(&json!({ "risky": true })));
        assert!(!predicate(&json!({ "risky": false })));
        let always = ruled.always_needs_approval();
        assert!(always.approval().expect("replaced")(&json!({})));
        assert!(format!("{always:?}").contains("needs_approval: true"));
    }

    #[test]
    fn into_tool_maps_function_tool() {
        let tool = FunctionTool::new("noop", "no-op", obj_schema(), |_: Value| async move {
            Ok::<_, String>(json!({}))
        });
        let Tool::Function(function) = tool.into_tool();
        assert_eq!(function.name(), "noop");
    }

    #[test]
    fn validate_tool_name_accepts_and_rejects() {
        assert!(validate_tool_name("get_weather").is_ok());
        assert!(validate_tool_name("Add-2").is_ok());
        assert!(validate_tool_name("_x").is_ok());
        assert!(validate_tool_name("").is_err());
        assert!(validate_tool_name("2fast").is_err());
        assert!(validate_tool_name("has space").is_err());
        assert!(validate_tool_name("dot.name").is_err());
        assert!(validate_tool_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn validate_tool_schema_accepts_and_rejects() {
        assert!(validate_tool_schema(&json!({ "type": "object" })).is_ok());
        assert!(validate_tool_schema(&json!({ "properties": {} })).is_ok());
        assert!(validate_tool_schema(&json!({ "type": "array" })).is_err());
        assert!(validate_tool_schema(&json!("nope")).is_err());
        assert!(validate_tool_schema(&json!(42)).is_err());
    }
}
