//! Link-time registrations emitted by the attribute macros and `serve-build`.
//!
//! Each macro submits one `…Registration` through `inventory`; `discover()`
//! iterates them. Everything here is `pub` only so macro expansions can name
//! it (`::serve::__private::…`); none of it is application API.

use std::any::Any;
use std::sync::Arc;

use everruns::ToolResponse;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::connection::McpServer;
use crate::cx::Cx;
use crate::eval::EvalCx;

/// What a tool call resolves to before it reaches `everruns::FunctionTool`.
pub type ToolFuture = BoxFuture<'static, Result<ToolResponse, String>>;

/// `#[agent]`.
pub struct AgentRegistration {
    pub name: &'static str,
    pub doc: &'static str,
    pub sub: bool,
    pub default: bool,
    pub source: &'static str,
    pub build: fn() -> crate::Agent,
}

/// When a tool call needs a person's approval before it runs. The host maps
/// it onto the runtime's gate (`FunctionTool::needs_approval`).
#[derive(Clone, Copy)]
pub enum Approval {
    Never,
    Always,
    /// Decided per call from the parsed arguments.
    When(fn(&Value) -> bool),
}

impl Approval {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Approval::Never => "never",
            Approval::Always => "always",
            Approval::When(_) => "conditional",
        }
    }
}

/// `#[tool]`.
pub struct ToolRegistration {
    pub name: &'static str,
    pub description: &'static str,
    pub source: &'static str,
    pub schema: fn() -> Value,
    pub approval: Approval,
    pub call: fn(Cx, Value) -> ToolFuture,
}

/// `#[channel]`.
pub struct ChannelRegistration {
    pub name: &'static str,
    pub source: &'static str,
    pub build: fn() -> Box<dyn crate::Channel>,
}

/// `#[schedule]`.
pub struct ScheduleRegistration {
    pub name: &'static str,
    pub cron: &'static str,
    pub source: &'static str,
    pub run: fn(Cx) -> BoxFuture<'static, crate::Result>,
}

/// `#[connection]`.
pub struct ConnectionRegistration {
    pub name: &'static str,
    pub source: &'static str,
    pub build: fn() -> crate::Result<ConnectionValue>,
}

/// `#[eval]`.
pub struct EvalRegistration {
    pub name: &'static str,
    pub doc: &'static str,
    pub source: &'static str,
    pub run: for<'a> fn(&'a mut EvalCx) -> BoxFuture<'a, crate::Result>,
}

/// One file embedded from `agent/**` (or `serve.toml`, as `@serve.toml`).
#[derive(Debug)]
pub struct AssetRegistration {
    /// Path relative to `agent/`.
    pub path: &'static str,
    /// Contents at build time.
    pub contents: &'static str,
    /// Absolute path at build time, re-read in `dev` for hot reload.
    pub disk: &'static str,
}

/// The application crate's Cargo name and version, submitted by
/// `serve::assets!()` (which expands inside that crate).
pub struct AppInfoRegistration {
    pub name: &'static str,
    pub version: &'static str,
}

inventory::collect!(AppInfoRegistration);
inventory::collect!(AgentRegistration);
inventory::collect!(ToolRegistration);
inventory::collect!(ChannelRegistration);
inventory::collect!(ScheduleRegistration);
inventory::collect!(ConnectionRegistration);
inventory::collect!(EvalRegistration);
inventory::collect!(AssetRegistration);

/// A built connection: the typed value tools downcast, plus what the manifest
/// and agent wiring need to know about it.
#[derive(Clone)]
pub struct ConnectionValue {
    pub(crate) name: &'static str,
    pub(crate) type_name: &'static str,
    pub(crate) value: Arc<dyn Any + Send + Sync>,
    pub(crate) mcp: Option<McpServer>,
}

impl ConnectionValue {
    pub fn new<T: Any + Send + Sync>(name: &'static str, value: T) -> Self {
        let mcp = (&value as &dyn Any).downcast_ref::<McpServer>().cloned();
        Self {
            name,
            type_name: std::any::type_name::<T>(),
            value: Arc::new(value),
            mcp,
        }
    }
}

/// Parse a tool's JSON arguments into its generated struct.
pub fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
}

/// JSON Schema of a tool's generated argument struct.
pub fn schema_for<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// Model-visible error for arguments that do not match the schema, so the
/// model can correct its own call.
pub fn invalid_arguments(tool: &str, err: serde_json::Error) -> Result<ToolResponse, String> {
    Ok(ToolResponse::error(format!(
        "invalid arguments for tool `{tool}`: {err}"
    )))
}

/// Map a tool's `Result<T, E>` to a tool response. `Err` is shown to the model
/// with its full cause chain (`{:#}`), which is what lets it recover.
pub fn tool_result<T: serde::Serialize, E: std::fmt::Display>(
    out: Result<T, E>,
) -> Result<ToolResponse, String> {
    match out {
        Ok(value) => tool_value(&value),
        Err(err) => Ok(ToolResponse::error(format!("{err:#}"))),
    }
}

/// Map a tool's plain return value to a tool response.
pub fn tool_value<T: serde::Serialize>(value: &T) -> Result<ToolResponse, String> {
    serde_json::to_value(value)
        .map(ToolResponse::json)
        .map_err(|err| format!("tool result is not serializable: {err}"))
}
