// Tool definitions and policies for agent execution
//
// Design Decision: Tools are identified by name (string) for extensibility.
// The BuiltinToolKind enum has been removed to allow adding new tools
// without code changes. Tool execution happens via the ToolRegistry
// which looks up tools by name.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

pub const HUMAN_INTENT_ARGUMENT: &str = "human_intent";

/// An image returned by a tool execution.
///
/// This allows tools (built-in or MCP) to return images that are sent
/// to the LLM as native image content blocks, not stringified JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultImage {
    /// Base64-encoded image data
    pub base64: String,
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
}

const HUMAN_INTENT_DESCRIPTION: &str = "Short user-facing narration of what this tool call will do, written as an action phrase like \"Listing all harnesses\". Do not include hidden reasoning, private chain of thought, secrets, or credential values.";

/// Tool policy determines how tool calls are handled
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    /// Execute immediately without user approval
    #[default]
    Auto,
    /// Require user approval before execution (HITL)
    RequiresApproval,
    /// Client-side tool: pause workflow, send to client for execution
    ClientSide,
}

/// Controls whether a tool's full schema can be deferred (tool_search).
///
/// When tool_search is active and a model supports it, tools marked as
/// `Automatic` or `Always` will have `defer_loading: true` set, meaning
/// only the name+description are sent upfront and full parameter schemas
/// are loaded on-demand by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeferrablePolicy {
    /// Never defer — always send full schema (e.g., high-frequency tools like write_todos)
    Never,
    /// Let the driver decide based on tool count threshold (default)
    #[default]
    Automatic,
    /// Always defer when tool_search is active, regardless of threshold
    Always,
}

impl DeferrablePolicy {
    /// Returns true when the value is the default (`Automatic`).
    pub fn is_default(&self) -> bool {
        matches!(self, DeferrablePolicy::Automatic)
    }
}

/// Tool definition in agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    /// Built-in tool - executed by the worker via ToolRegistry
    Builtin(BuiltinTool),
    /// Client-side tool - executed by the client, not the server
    ClientSide(ClientSideTool),
}

/// Built-in tool configuration
///
/// Note: The `kind` field has been removed. Tools are now identified
/// solely by their `name` field, and execution happens via the ToolRegistry
/// which looks up tools by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BuiltinTool {
    /// Tool name (used by LLM and for registry lookup)
    pub name: String,
    /// Human-readable display name for UI rendering (e.g., "Get Current Time" for `get_current_time`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Tool policy (auto or requires_approval)
    #[serde(default)]
    pub policy: ToolPolicy,
    /// Category for tool_search namespace grouping (from parent capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether this tool's schema can be deferred via tool_search
    #[serde(default, skip_serializing_if = "DeferrablePolicy::is_default")]
    pub deferrable: DeferrablePolicy,
    /// Semantic hints describing the tool's behavioral properties
    #[serde(default, skip_serializing_if = "ToolHints::is_empty")]
    pub hints: ToolHints,
    /// Original full parameter schema saved by `DeferSchemaHook` before stripping.
    /// Serialized only when present so durable reason-to-act scheduling can
    /// preserve deferred schemas for `tool_search` in the act phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_parameters: Option<serde_json::Value>,
}

/// Client-side tool - executed by the client, not the server
/// The server pauses execution and waits for the client to submit results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ClientSideTool {
    /// Tool name (used by LLM and for correlation)
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Category for tool_search namespace grouping (from parent capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether this tool's schema can be deferred via tool_search
    #[serde(default, skip_serializing_if = "DeferrablePolicy::is_default")]
    pub deferrable: DeferrablePolicy,
    /// Semantic hints describing the tool's behavioral properties
    #[serde(default, skip_serializing_if = "ToolHints::is_empty")]
    pub hints: ToolHints,
    /// Original full parameter schema saved by `DeferSchemaHook` before stripping.
    /// Serialized only when present so durable reason-to-act scheduling can
    /// preserve deferred schemas for `tool_search` in the act phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_parameters: Option<serde_json::Value>,
}

impl ToolDefinition {
    /// Get the tool name regardless of variant
    pub fn name(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.name,
            ToolDefinition::ClientSide(c) => &c.name,
        }
    }

    /// Get the tool display name regardless of variant
    pub fn display_name(&self) -> Option<&str> {
        match self {
            ToolDefinition::Builtin(b) => b.display_name.as_deref(),
            ToolDefinition::ClientSide(c) => c.display_name.as_deref(),
        }
    }

    /// Get the tool description regardless of variant
    pub fn description(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.description,
            ToolDefinition::ClientSide(c) => &c.description,
        }
    }

    /// Get the tool parameters schema regardless of variant
    pub fn parameters(&self) -> &serde_json::Value {
        match self {
            ToolDefinition::Builtin(b) => &b.parameters,
            ToolDefinition::ClientSide(c) => &c.parameters,
        }
    }

    /// Get the full (pre-deferral) parameter schema, falling back to `parameters()`.
    ///
    /// When `DeferSchemaHook` strips a tool's schema it saves the original in
    /// `full_parameters`. Callers that need the real schema (e.g. `tool_search`)
    /// should use this method so deferred tools still return useful results.
    pub fn full_parameters(&self) -> &serde_json::Value {
        match self {
            ToolDefinition::Builtin(b) => b.full_parameters.as_ref().unwrap_or(&b.parameters),
            ToolDefinition::ClientSide(c) => c.full_parameters.as_ref().unwrap_or(&c.parameters),
        }
    }

    /// Get the tool policy regardless of variant
    pub fn policy(&self) -> &ToolPolicy {
        match self {
            ToolDefinition::Builtin(b) => &b.policy,
            ToolDefinition::ClientSide(_) => &ToolPolicy::ClientSide,
        }
    }

    /// Get the tool category for namespace grouping
    pub fn category(&self) -> Option<&str> {
        match self {
            ToolDefinition::Builtin(b) => b.category.as_deref(),
            ToolDefinition::ClientSide(c) => c.category.as_deref(),
        }
    }

    /// Get the deferrable policy for tool_search
    pub fn deferrable(&self) -> &DeferrablePolicy {
        match self {
            ToolDefinition::Builtin(b) => &b.deferrable,
            ToolDefinition::ClientSide(c) => &c.deferrable,
        }
    }

    /// Get the tool hints
    pub fn hints(&self) -> &ToolHints {
        match self {
            ToolDefinition::Builtin(b) => &b.hints,
            ToolDefinition::ClientSide(c) => &c.hints,
        }
    }

    /// Scheduling conflict key for this tool, if any (see
    /// `ToolHints::concurrency_class`). `None` means the tool has no mutation
    /// conflicts and may always run concurrently with others.
    pub fn concurrency_class(&self) -> Option<&str> {
        self.hints().concurrency_class.as_deref()
    }

    /// Whether this tool performs CPU-bound/non-yielding in-process work and
    /// should be offloaded to its own task by the act scheduler.
    pub fn is_cpu_bound(&self) -> bool {
        self.hints().cpu_bound.unwrap_or(false)
    }

    /// Effective side-effect class for this tool (defaults to `AtMostOnce`).
    pub fn side_effect_class(&self) -> SideEffectClass {
        self.hints().effective_side_effect_class()
    }

    /// Get reporting attribution for the capability that contributed this tool.
    pub fn capability_attribution(&self) -> Option<(&str, Option<&str>)> {
        self.hints()
            .capability_id
            .as_deref()
            .map(|id| (id, self.hints().capability_name.as_deref()))
    }

    /// Set the category on this tool definition (builder pattern)
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        match &mut self {
            ToolDefinition::Builtin(b) => b.category = Some(category.into()),
            ToolDefinition::ClientSide(c) => c.category = Some(category.into()),
        }
        self
    }

    /// Set the hints on this tool definition (builder pattern)
    pub fn with_hints(mut self, hints: ToolHints) -> Self {
        match &mut self {
            ToolDefinition::Builtin(b) => b.hints = hints,
            ToolDefinition::ClientSide(c) => c.hints = hints,
        }
        self
    }

    /// Set reporting attribution on this tool definition (builder pattern).
    pub fn with_capability_attribution(
        mut self,
        capability_id: impl Into<String>,
        capability_name: Option<impl Into<String>>,
    ) -> Self {
        let capability_id = capability_id.into();
        let capability_name = capability_name.map(Into::into);
        match &mut self {
            ToolDefinition::Builtin(b) => {
                b.hints.capability_id = Some(capability_id);
                b.hints.capability_name = capability_name;
            }
            ToolDefinition::ClientSide(c) => {
                c.hints.capability_id = Some(capability_id);
                c.hints.capability_name = capability_name;
            }
        }
        self
    }

    /// Add the cross-cutting `human_intent` argument to the tool's JSON schema.
    ///
    /// This field is model-authored narration for UI rendering. Tool execution
    /// strips it before invoking the underlying tool implementation.
    pub fn with_human_intent_argument(mut self) -> Self {
        match &mut self {
            ToolDefinition::Builtin(b) => add_human_intent_to_schema(&mut b.parameters),
            ToolDefinition::ClientSide(c) => add_human_intent_to_schema(&mut c.parameters),
        }
        self
    }
}

pub fn add_human_intent_to_tool_definitions(tools: &[ToolDefinition]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .cloned()
        .map(ToolDefinition::with_human_intent_argument)
        .collect()
}

pub fn human_intent(arguments: &Value) -> Option<&str> {
    arguments
        .get(HUMAN_INTENT_ARGUMENT)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn strip_human_intent_argument(arguments: &Value) -> Value {
    let mut stripped = arguments.clone();
    if let Value::Object(ref mut object) = stripped {
        object.remove(HUMAN_INTENT_ARGUMENT);
    }
    stripped
}

fn add_human_intent_to_schema(schema: &mut Value) {
    let Value::Object(schema_obj) = schema else {
        return;
    };

    schema_obj
        .entry("type")
        .or_insert_with(|| Value::String("object".to_string()));

    let properties = schema_obj
        .entry("properties")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(properties_obj) = properties {
        properties_obj.insert(
            HUMAN_INTENT_ARGUMENT.to_string(),
            serde_json::json!({
                "type": "string",
                "description": HUMAN_INTENT_DESCRIPTION,
                "maxLength": 120,
            }),
        );
    }

    // `human_intent` is intentionally optional: models should provide it when
    // useful, but old calls, provider quirks, and client-side calls remain valid.
}

/// How many times a tool call may safely be executed given the same inputs.
///
/// Used by the durable Act activity (EVE-530) to decide what to do when a
/// prior execution attempt left a `running` claim in `durable_tool_results`:
///
/// * `Pure` / `Idempotent` — the running claim is stale; re-execute freely.
/// * `AtMostOnce` — never re-execute from a stale running claim; settle it
///   as `interrupted` and surface an uncertain result to the model instead.
///
/// When unset (`None`), the conservative default is `AtMostOnce`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum SideEffectClass {
    /// No external side effects; always safe to re-execute (e.g. read-only queries).
    Pure,
    /// Idempotent side effects; safe to re-execute with the same arguments
    /// (e.g. create-or-update, PUT-style writes).
    Idempotent,
    /// Exactly-once semantics required; must not be re-executed from a stale
    /// running claim (e.g. charge a card, send an email, open a PR).
    #[default]
    AtMostOnce,
}

/// Semantic hints describing a tool's behavioral properties.
///
/// Follows the MCP tool annotations convention (readOnlyHint, destructiveHint,
/// idempotentHint, openWorldHint) plus everruns-specific hints. All fields are
/// optional booleans — `None` means "unknown/unspecified". Consumers should
/// treat `None` as the conservative default (e.g., assume not readonly, assume
/// not idempotent).
///
/// These hints are informational — they do not enforce policy. Use `ToolPolicy`
/// for execution gating (auto vs requires_approval).
// `Eq` is deliberately absent: `metadata` is an opaque `serde_json::Value`, which
// is only `PartialEq`. Hints are compared for equality (`is_empty`), never hashed
// or used as a map key, so `PartialEq` is sufficient.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolHints {
    /// Tool does not modify any state (read-only queries, lookups).
    /// When true: safe to call speculatively, result can be cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,

    /// Tool may irreversibly destroy or delete data.
    /// Subset of non-readonly — a tool can be non-readonly (writes) without
    /// being destructive (e.g., create/update operations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,

    /// Calling the tool repeatedly with the same arguments produces the same
    /// effect. Safe to retry on transient failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,

    /// Tool interacts with external entities beyond the local system
    /// (network calls, third-party APIs, cloud services).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world: Option<bool>,

    /// Tool requires API keys, credentials, or other secrets to function.
    /// Useful for UI to show connection prompts and for LLMs to anticipate
    /// authentication failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_secrets: Option<bool>,

    /// Tool may take significant time to complete (> ~5s typical).
    /// Useful for clients to show progress indicators and set timeouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_running: Option<bool>,

    /// Tool supports detached background execution via `spawn_background`.
    /// When true, the tool may be executed asynchronously outside the current
    /// foreground tool call and report status back later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_background: Option<bool>,

    /// Scheduling conflict key. Tool calls within the same act batch that share
    /// a non-empty `concurrency_class` are executed sequentially in arrival
    /// order; calls in different classes (or with no class) run concurrently.
    ///
    /// Set this on tools that mutate shared session state so that, e.g., two
    /// file writes or two SQL mutations in one batch do not race. Read-only
    /// tools should leave this `None` so they always parallelize. See
    /// `everruns-engine`'s tool scheduler for how the act phase consumes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_class: Option<String>,

    /// Tool performs significant CPU-bound or otherwise non-yielding work in
    /// process (e.g. an in-process interpreter). When true, the act scheduler
    /// runs the call on its own task (`tokio::spawn`) so a long CPU burst does
    /// not starve the cooperative polling of I/O-bound tools in the same batch.
    ///
    /// Distinct from `long_running`, which describes wall-clock time for
    /// I/O-bound work (those tools yield at await points and need no offload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_bound: Option<bool>,

    /// Tool output should be persisted to session VFS before truncation.
    /// When set, the `tool_output_persistence` capability (EVE-222, EVE-245) writes
    /// stdout to `/outputs/{tool_call_id}.stdout` and stderr to
    /// `/outputs/{tool_call_id}.stderr`, injecting `full_output`, `total_lines`,
    /// and `output_files` into the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_output: Option<bool>,

    /// Capability that contributed this tool definition.
    ///
    /// Reporting uses this attribution only as metadata. It must never contain
    /// tool arguments, results, prompts, or any other sensitive payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,

    /// Human-readable capability name snapshot for reporting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,

    /// Entity noun for operation-based narration (e.g. "agent", "harness").
    /// When set, the narration system reads the `operation` argument and
    /// produces verb-based narration like "Created agent: Neon Cartographer"
    /// instead of the generic "Ran Manage Agents".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration_noun: Option<String>,

    /// Replay-safety class used by the durable Act activity (EVE-530).
    ///
    /// Controls what happens when a worker reclaims a stale `running` claim:
    /// `Pure`/`Idempotent` tools are re-executed; `AtMostOnce` tools are
    /// settled as `interrupted` to prevent double side-effects.
    ///
    /// `None` is treated conservatively as `AtMostOnce`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect_class: Option<SideEffectClass>,

    /// Host-owned annotations that core does not interpret.
    ///
    /// The typed hints above are the vocabulary core itself reasons about. This
    /// is the escape hatch for everything a *host* wants to carry alongside a
    /// tool — risk tiers for an approval UI, presentation hints, an embedder's
    /// routing keys — without adding a field to core for each one. Core reads
    /// nothing here and no driver sends it to a provider; it travels with the
    /// definition so a consumer sees it at the point of decision (e.g. a
    /// `PreToolUseHook` gating on what the tool declared).
    ///
    /// The schema belongs to whoever writes it. Never put credentials or other
    /// sensitive payload here: like the rest of the definition, it is persisted
    /// and surfaced to clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ToolHints {
    /// Returns true when all fields are None (default/empty state).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Builder: attach host-owned metadata (see [`ToolHints::metadata`]).
    pub fn with_metadata(mut self, value: serde_json::Value) -> Self {
        self.metadata = Some(value);
        self
    }

    /// Builder: set readonly hint.
    pub fn with_readonly(mut self, value: bool) -> Self {
        self.readonly = Some(value);
        self
    }

    /// Builder: set destructive hint.
    pub fn with_destructive(mut self, value: bool) -> Self {
        self.destructive = Some(value);
        self
    }

    /// Builder: set idempotent hint.
    pub fn with_idempotent(mut self, value: bool) -> Self {
        self.idempotent = Some(value);
        self
    }

    /// Builder: set open_world hint.
    pub fn with_open_world(mut self, value: bool) -> Self {
        self.open_world = Some(value);
        self
    }

    /// Builder: set reporting attribution.
    pub fn with_capability_attribution(
        mut self,
        capability_id: impl Into<String>,
        capability_name: Option<impl Into<String>>,
    ) -> Self {
        self.capability_id = Some(capability_id.into());
        self.capability_name = capability_name.map(Into::into);
        self
    }

    /// Builder: set requires_secrets hint.
    pub fn with_requires_secrets(mut self, value: bool) -> Self {
        self.requires_secrets = Some(value);
        self
    }

    /// Builder: set long_running hint.
    pub fn with_long_running(mut self, value: bool) -> Self {
        self.long_running = Some(value);
        self
    }

    /// Builder: set supports_background hint.
    pub fn with_supports_background(mut self, value: bool) -> Self {
        self.supports_background = Some(value);
        self
    }

    /// Builder: set the scheduling conflict key (see `concurrency_class`).
    pub fn with_concurrency_class(mut self, class: impl Into<String>) -> Self {
        self.concurrency_class = Some(class.into());
        self
    }

    /// Builder: set the cpu_bound hint (see `cpu_bound`).
    pub fn with_cpu_bound(mut self, value: bool) -> Self {
        self.cpu_bound = Some(value);
        self
    }

    /// Builder: set persist_output hint.
    pub fn with_persist_output(mut self, value: bool) -> Self {
        self.persist_output = Some(value);
        self
    }

    /// Builder: set narration noun for operation-based narration.
    pub fn with_narration_noun(mut self, noun: impl Into<String>) -> Self {
        self.narration_noun = Some(noun.into());
        self
    }

    /// Builder: set the replay-safety class (EVE-530).
    pub fn with_side_effect_class(mut self, class: SideEffectClass) -> Self {
        self.side_effect_class = Some(class);
        self
    }

    /// Returns the effective side-effect class, defaulting to `AtMostOnce`
    /// when unset (conservative default).
    pub fn effective_side_effect_class(&self) -> SideEffectClass {
        self.side_effect_class
            .clone()
            .unwrap_or(SideEffectClass::AtMostOnce)
    }
}

/// Tool call from LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCall {
    /// Unique ID for this tool call
    pub id: String,
    /// Tool name to execute
    pub name: String,
    /// Arguments as JSON
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Arguments safe to pass to the actual tool implementation.
    pub fn execution_arguments(&self) -> serde_json::Value {
        strip_human_intent_argument(&self.arguments)
    }

    /// Convert tool call to OpenAI-compatible format
    ///
    /// Returns format: `{id, type: "function", function: {name, arguments}}`
    /// where arguments is stringified JSON.
    pub fn to_openai_format(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "type": "function",
            "function": {
                "name": self.name,
                "arguments": serde_json::to_string(&self.arguments).unwrap_or_else(|_| "{}".to_string())
            }
        })
    }
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool call ID this result corresponds to
    pub tool_call_id: String,
    /// Result data (success)
    pub result: Option<serde_json::Value>,
    /// Images returned by the tool (sent as native image content to LLM)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ToolResultImage>>,
    /// Error message (failure)
    pub error: Option<String>,
    /// When set, indicates the tool requires a user connection for this provider.
    /// The workflow should pause and prompt the user to configure the connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_required: Option<String>,
    /// Pre-truncation cleaned output for persistence hooks.
    /// Populated by exec tools (after ANSI strip + CR collapse, before truncation).
    /// Consumed by PostToolExecHook (e.g. tool_output_persistence) then cleared.
    /// Never serialized to messages or sent to LLM.
    #[serde(skip)]
    pub raw_output: Option<String>,
}

/// `result.code` marking a tool result that stopped on a URL mode elicitation.
///
/// The MCP executor is the only producer; the engine's `UrlElicitationHook` is
/// the consumer. It lives here, beside [`ToolResult`], because the two crates
/// must agree on the shape and neither depends on the other.
pub const URL_ELICITATION_REQUIRED_CODE: &str = "url_elicitation_required";

/// Name of the synthetic client-side tool call that carries a URL mode
/// elicitation to a human: the engine emits it, the client renders a consent
/// surface for it, and the API that collects the decision recognises it.
pub const CONFIRM_URL_ELICITATION_TOOL: &str = "confirm_url_elicitation";

/// Structured payload of a tool result that stopped on a URL mode elicitation.
///
/// An MCP server answered `tools/call` by asking that a human visit a URL out
/// of band (a secret to type, an authorization to grant, a payment to make).
/// The call did not fail, and no credential is missing — it is waiting on a
/// person. Everything here is safe to show: the URL was validated before any
/// human saw it and is never fetched by the client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UrlElicitationRequired {
    /// Always [`URL_ELICITATION_REQUIRED_CODE`]; discriminates the payload.
    pub code: String,
    /// Model-facing sentence explaining what is being waited on.
    pub error: String,
    /// The URL a human must open. Shown in full — never shortened, and never
    /// turned into a bare "click here" link.
    pub url: String,
    /// Host of `url`, so a consent surface can highlight the domain instead of
    /// re-parsing (clients SHOULD highlight it against subdomain spoofing).
    pub url_host: String,
    /// Whether the host carries a Punycode label. Legitimate, but worth a
    /// warning before a user trusts the domain.
    pub url_is_punycode: bool,
    /// Logical MCP server that asked.
    pub server: String,
    /// MCP tool the elicitation interrupted. Consent is recorded against this
    /// pair, so it must survive into the payload.
    pub tool: String,
    /// The tool as the model knows it (`mcp_<server>_<tool>`), so whatever
    /// resumes the work can name the call to make once a human has consented.
    pub retry_tool: String,
    /// The server's own explanation of why the interaction is needed.
    pub message: String,
    /// True when a human refused. Nothing more to ask; do not prompt again.
    pub declined: bool,
}

impl UrlElicitationRequired {
    /// Recover the payload from a tool result, if that is what it carries.
    pub fn from_tool_result(result: &ToolResult) -> Option<Self> {
        let value = result.result.as_ref()?;
        if value.get("code")?.as_str()? != URL_ELICITATION_REQUIRED_CODE {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}

impl ToolResult {
    /// Construct a minimal error-only ToolResult (used for fingerprinting error paths).
    pub fn error(msg: &str) -> Self {
        Self {
            tool_call_id: String::new(),
            result: None,
            images: None,
            error: Some(msg.to_string()),
            connection_required: None,
            raw_output: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn definition(kind: &str) -> ToolDefinition {
        serde_json::from_value(json!({"type":kind,"name":"tool","description":"Description","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}})).unwrap()
    }

    #[test]
    fn tool_variants_have_complete_literal_wire_defaults_and_effective_policies() {
        for kind in ["builtin", "client_side"] {
            let tool = definition(kind);
            let mut expected = json!({"type":kind,"name":"tool","description":"Description","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}});
            if kind == "builtin" {
                expected["policy"] = json!("auto");
            }
            assert_eq!(serde_json::to_value(&tool).unwrap(), expected);
            assert_eq!(tool.name(), "tool");
            assert_eq!(tool.description(), "Description");
            assert_eq!(tool.display_name(), None);
            assert_eq!(
                tool.parameters(),
                &json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"]})
            );
            assert_eq!(
                tool.policy(),
                if kind == "builtin" {
                    &ToolPolicy::Auto
                } else {
                    &ToolPolicy::ClientSide
                }
            );
            assert_eq!(tool.deferrable(), &DeferrablePolicy::Automatic);
            assert!(tool.hints().is_empty());
            assert_eq!(tool.concurrency_class(), None);
            assert!(!tool.is_cpu_bound());
            assert_eq!(tool.side_effect_class(), SideEffectClass::AtMostOnce);
        }
        let mixed:Vec<ToolDefinition>=serde_json::from_value(json!([
            {"type":"builtin","name":"server","description":"Server","parameters":{},"policy":"requires_approval"},
            {"type":"client_side","name":"client","description":"Client","parameters":{},"policy":"auto"}
        ])).unwrap();
        assert_eq!(
            (
                mixed[0].name(),
                mixed[0].policy(),
                mixed[1].name(),
                mixed[1].policy()
            ),
            (
                "server",
                &ToolPolicy::RequiresApproval,
                "client",
                &ToolPolicy::ClientSide
            )
        );
        assert!(matches!(&mixed[0], ToolDefinition::Builtin(_)));
        assert!(matches!(&mixed[1], ToolDefinition::ClientSide(_)));
        for (policy, wire) in [
            (ToolPolicy::Auto, "auto"),
            (ToolPolicy::RequiresApproval, "requires_approval"),
            (ToolPolicy::ClientSide, "client_side"),
        ] {
            assert_eq!(serde_json::to_value(&policy).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ToolPolicy>(json!(wire)).unwrap(),
                policy
            );
        }
    }

    #[test]
    fn hints_builders_preserve_false_values_metadata_and_scheduling_fields() {
        for kind in ["builtin", "client_side"] {
            let reader = definition(kind).with_hints(ToolHints::default().with_readonly(true));
            assert_eq!(reader.concurrency_class(), None);
            assert!(!reader.is_cpu_bound());
            assert_eq!(reader.side_effect_class(), SideEffectClass::AtMostOnce);
        }

        for value in [false, true] {
            let hints = ToolHints::default()
                .with_readonly(value)
                .with_destructive(value)
                .with_idempotent(value)
                .with_open_world(value)
                .with_requires_secrets(value)
                .with_long_running(value)
                .with_supports_background(value)
                .with_cpu_bound(value)
                .with_persist_output(value)
                .with_concurrency_class("session_workspace")
                .with_narration_noun("file")
                .with_side_effect_class(SideEffectClass::Idempotent)
                .with_capability_attribution("files", Some("Files"))
                .with_metadata(json!({"risk_tier":"high","nested":[1,false,null]}));
            let expected = json!({"readonly":value,"destructive":value,"idempotent":value,"open_world":value,"requires_secrets":value,"long_running":value,"supports_background":value,"cpu_bound":value,"persist_output":value,"concurrency_class":"session_workspace","narration_noun":"file","side_effect_class":"Idempotent","capability_id":"files","capability_name":"Files","metadata":{"risk_tier":"high","nested":[1,false,null]}});
            assert_eq!(serde_json::to_value(&hints).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<ToolHints>(expected).unwrap(),
                hints
            );
            for kind in ["builtin", "client_side"] {
                let tool = definition(kind)
                    .with_hints(hints.clone())
                    .with_category("workspace")
                    .with_capability_attribution("new-files", Some("New Files"));
                assert_eq!(tool.category(), Some("workspace"));
                assert_eq!(tool.concurrency_class(), Some("session_workspace"));
                assert_eq!(tool.is_cpu_bound(), value);
                assert_eq!(tool.side_effect_class(), SideEffectClass::Idempotent);
                assert_eq!(
                    tool.capability_attribution(),
                    Some(("new-files", Some("New Files")))
                );
                let mut expected_hints = serde_json::to_value(&hints).unwrap();
                expected_hints["capability_id"] = json!("new-files");
                expected_hints["capability_name"] = json!("New Files");
                assert_eq!(serde_json::to_value(tool.hints()).unwrap(), expected_hints);
            }
        }
        assert_eq!(
            serde_json::to_value(ToolHints::default()).unwrap(),
            json!({})
        );
        let metadata = ToolHints::default().with_metadata(json!({"any":"thing"}));
        assert!(!metadata.is_empty());
        assert_eq!(
            serde_json::to_value(metadata).unwrap(),
            json!({"metadata":{"any":"thing"}})
        );
        for class in [
            SideEffectClass::Pure,
            SideEffectClass::Idempotent,
            SideEffectClass::AtMostOnce,
        ] {
            assert_eq!(
                ToolHints::default()
                    .with_side_effect_class(class.clone())
                    .effective_side_effect_class(),
                class
            );
        }
    }

    #[test]
    fn tool_display_and_deferred_schemas_survive_both_wire_variants() {
        for kind in ["builtin", "client_side"] {
            for (deferrable, wire) in [
                (DeferrablePolicy::Never, Some("never")),
                (DeferrablePolicy::Automatic, None),
                (DeferrablePolicy::Always, Some("always")),
            ] {
                let mut payload = json!({"type":kind,"name":"tool","display_name":"Display","description":"Description","parameters":{"type":"object"},"full_parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]},"category":"files"});
                if let Some(wire) = wire {
                    payload["deferrable"] = json!(wire);
                }
                let tool: ToolDefinition = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(tool.display_name(), Some("Display"));
                assert_eq!(tool.deferrable(), &deferrable);
                assert_eq!(tool.parameters(), &json!({"type":"object"}));
                assert_eq!(
                    tool.full_parameters(),
                    &json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
                );
                if kind == "builtin" {
                    payload["policy"] = json!("auto");
                }
                assert_eq!(serde_json::to_value(tool).unwrap(), payload);
            }
            let tool = definition(kind);
            assert_eq!(tool.full_parameters(), tool.parameters());
        }
    }

    #[test]
    fn tool_call_wire_and_openai_arguments_preserve_the_complete_payload() {
        for (arguments, text) in [
            (json!({"city":"New York"}), r#"{"city":"New York"}"#),
            (json!({}), "{}"),
            (
                json!({"count":9007199254740993_u64,"text":"line\nquoted\""}),
                r#"{"count":9007199254740993,"text":"line\nquoted\""}"#,
            ),
        ] {
            let call = ToolCall {
                id: "call_123".into(),
                name: "get_weather".into(),
                arguments: arguments.clone(),
            };
            assert_eq!(
                serde_json::to_value(&call).unwrap(),
                json!({"id":"call_123","name":"get_weather","arguments":arguments})
            );
            let parsed: ToolCall = serde_json::from_value(
                json!({"id":"call_123","name":"get_weather","arguments":arguments}),
            )
            .unwrap();
            assert_eq!(parsed.arguments, arguments);
            assert_eq!(
                call.to_openai_format(),
                json!({"id":"call_123","type":"function","function":{"name":"get_weather","arguments":text}})
            );
        }
    }

    #[test]
    fn tool_result_wire_excludes_raw_output_but_preserves_images_errors_and_data() {
        let expected = json!({"tool_call_id":"call_123","result":{"temperature":72},"images":[{"base64":"aGk=","media_type":"image/png"}],"error":"partial failure","connection_required":"provider"});
        let mut injected = expected.clone();
        injected["raw_output"] = json!("private-raw");
        let mut result: ToolResult = serde_json::from_value(injected).unwrap();
        assert!(result.raw_output.is_none());
        result.raw_output = Some("private-raw".into());
        assert_eq!(serde_json::to_value(result).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(ToolResult::error("failed")).unwrap(),
            json!({"tool_call_id":"","result":null,"error":"failed"})
        );
        let success: ToolResult = serde_json::from_value(
            json!({"tool_call_id":"call_123","result":{"temperature":72},"error":null}),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(success).unwrap(),
            json!({"tool_call_id":"call_123","result":{"temperature":72},"error":null})
        );
    }

    #[test]
    fn narration_schema_is_optional_idempotent_and_does_not_mutate_input_definitions() {
        let original = vec![definition("builtin"), definition("client_side")];
        let before = serde_json::to_value(&original).unwrap();
        let augmented = add_human_intent_to_tool_definitions(&original);
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
        let property = json!({"type":"string","description":"Short user-facing narration of what this tool call will do, written as an action phrase like \"Listing all harnesses\". Do not include hidden reasoning, private chain of thought, secrets, or credential values.","maxLength":120});
        for tool in &augmented {
            assert_eq!(
                tool.parameters(),
                &json!({"type":"object","properties":{"input":{"type":"string"},"human_intent":property},"required":["input"]})
            );
        }
        assert_eq!(
            serde_json::to_value(add_human_intent_to_tool_definitions(&augmented)).unwrap(),
            serde_json::to_value(&augmented).unwrap()
        );
        let mut closed = json!({"properties":{"operation":{"type":"string","enum":["list"]}},"required":["operation"],"additionalProperties":false});
        add_human_intent_to_schema(&mut closed);
        assert_eq!(
            closed,
            json!({"type":"object","properties":{"operation":{"type":"string","enum":["list"]},"human_intent":property},"required":["operation"],"additionalProperties":false})
        );
        let mut empty = json!({});
        add_human_intent_to_schema(&mut empty);
        assert_eq!(
            empty,
            json!({"type":"object","properties":{"human_intent":property}})
        );
        for mut scalar in [json!(null), json!(false), json!([])] {
            let before = scalar.clone();
            add_human_intent_to_schema(&mut scalar);
            assert_eq!(scalar, before);
        }
    }

    #[test]
    fn execution_strips_only_top_level_narration_and_trims_display_text() {
        for (value, expected) in [
            (
                json!(" Listing all harnesses "),
                Some("Listing all harnesses"),
            ),
            (json!(" \t"), None),
            (json!(7), None),
            (json!(null), None),
        ] {
            let arguments = json!({"operation":"list","human_intent":value,"nested":{"human_intent":"ordinary data"}});
            let call = ToolCall {
                id: "call".into(),
                name: "manage_harnesses".into(),
                arguments: arguments.clone(),
            };
            assert_eq!(human_intent(&call.arguments), expected);
            assert_eq!(
                call.execution_arguments(),
                json!({"operation":"list","nested":{"human_intent":"ordinary data"}})
            );
            assert_eq!(call.arguments, arguments);
        }
        for value in [json!(null), json!([1, "text"]), json!({"operation":"list"})] {
            assert_eq!(strip_human_intent_argument(&value), value);
            assert_eq!(human_intent(&value), None);
        }
    }

    #[test]
    fn url_elicitation_requires_discriminator_and_complete_payload() {
        let payload = json!({"code":"url_elicitation_required","error":"Waiting","url":"https://consent.example/path","url_host":"consent.example","url_is_punycode":false,"server":"server","tool":"tool","retry_tool":"mcp_server_tool","message":"Connect account","declined":false});
        let mut result = ToolResult::error("unrelated");
        result.result = Some(payload.clone());
        assert_eq!(
            serde_json::to_value(UrlElicitationRequired::from_tool_result(&result).unwrap())
                .unwrap(),
            payload
        );
        let mut declined = payload.clone();
        declined["declined"] = json!(true);
        result.result = Some(declined.clone());
        assert_eq!(
            serde_json::to_value(UrlElicitationRequired::from_tool_result(&result).unwrap())
                .unwrap(),
            declined
        );
        for malformed in [
            None,
            Some(json!(null)),
            Some(json!({"code":"url_elicitation_required"})),
            Some({
                let mut value = payload.clone();
                value["code"] = json!("connection_required");
                value
            }),
            Some({
                let mut value = payload.clone();
                value["declined"] = json!("false");
                value
            }),
        ] {
            result.result = malformed;
            assert!(UrlElicitationRequired::from_tool_result(&result).is_none());
        }
    }
}
