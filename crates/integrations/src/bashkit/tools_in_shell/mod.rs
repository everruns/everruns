//! Tools in shell: the agent's tools as one `tools` command inside the Bashkit
//! shell, so a single script can call several tools, pipe one result into the
//! next with `jq`, and return only what the model needs.
//!
//! See the Tools in Shell capability documentation for the user-facing contract.
//!
//! Decisions:
//!
//! - **Gating.** The shell cannot see which capabilities an agent enabled, only
//!   its tool registry. So this capability contributes a marker tool named
//!   `tools`, which its own hook always hides from the model, and the shell
//!   installs the builtin when that marker is registered. No capability, no
//!   marker, no command.
//! - **One predicate.** [`goes_behind_tools`] classifies operations the shell
//!   adapter can reach. CLI mode hides those native definitions; hybrid mode
//!   keeps them on both surfaces. Owner-required direct tools remain native.
//! - **Spelling.** MCP tools are grouped by server (`tools github list-pulls`);
//!   every other tool is a top-level command (`tools web-fetch`). Registry
//!   tools carry no capability attribution to group the rest by.
//! - **Per-call records, not tool events.** A call from a script is not
//!   emitted as its own `tool.started` / `tool.completed`: the shell call is
//!   the one tool call the conversation records. Each call is recorded as a
//!   `tool.nested_call` event under that shell call instead (see `timeline`).
//! - **Tool search.** CLI mode uses the catalog as its discovery surface. Hybrid
//!   mode keeps native search available alongside the shell adapter.

mod builtin;
mod catalog;
mod input;
mod plan;
mod preflight;
mod ratings;
mod run;
mod scripts;
mod timeline;

pub use preflight::preflight;
pub use ratings::UnlabeledToolRatingsOff;
pub use run::finish as finish_run;

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use bashkit::BashBuilder;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
    SystemPromptContext, ToolDefinitionHook,
};
use everruns_contracts::runtime::mcp_deferred::deferred_mcp_server_prefix;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_shell_exec,
};
use everruns_contracts::runtime::tool_surface::{
    NATIVE_ONLY_TOOL_NAMES, NativeToolDisclosure, ToolSurface, ToolSurfaceMode, ToolSurfaceRoute,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::{
    DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy,
};
use serde_json::{Value, json};

pub use builtin::ToolsBuiltin;

pub const TOOLS_IN_SHELL_CAPABILITY_ID: &str = "tools_in_shell";

/// The shell command, and the name of the hidden marker tool that turns it on.
pub const TOOLS_COMMAND: &str = "tools";

/// Tool search capabilities this one replaces: the `tools` catalog and
/// `tools search` already let the model find a tool without its schema.
const SUPERSEDED: &[&str] = &[
    "tool_search",
    "auto_tool_search",
    "openai_tool_search",
    "claude_tool_search",
];

/// Static behavior contract only. Inventory and descriptions stay in the
/// runtime catalog so this prompt remains stable as MCP tools change.
const SYSTEM_PROMPT: &str = "Most tools are not in your direct tool list: call them from the \
`bash` tool with the `tools` command (input is one JSON object, output is JSON). For a task, \
search first with `tools search <words>`; it finds operations by name, description, and server \
description, then returns runnable command lines. This is the supported CLI discovery command; \
there is no `tools list` command. `tools --help` and `tools <server> --help` are bounded browsing \
aids and may omit entries on large catalogs. A call that needs approval stops \
the script and reports what ran; after the answer, write a new script for what is left and never \
rerun the stopped one. Prefer one script that chains several calls and prints only what you need \
over a series of separate tool calls.";

/// Capability plugins this module contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: None,
    factory: || Box::new(ToolsInShellCapability),
}];

/// Whether a tool is eligible for the `tools` command adapter.
///
/// A tool stays direct when it pauses or shapes the turn, or when its result
/// only makes sense to the model directly. Approval-gated and destructive
/// tools go behind: the turn's approval gate judges each call from a script as
/// it would a direct one, and a call it holds stops the script with a report
/// (see `run`). A deferred MCP server's placeholder (`mcp_<server>`) goes
/// behind too: the shell loads the server itself. This is independent of
/// whether a native schema can be deferred.
pub(crate) fn goes_behind_tools(
    name: &str,
    is_client_side: bool,
    policy: &ToolPolicy,
    hints: &ToolHints,
) -> bool {
    if !is_client_side && deferred_mcp_server_prefix(name).is_some() {
        return true;
    }
    !(NATIVE_ONLY_TOOL_NAMES.contains(&name)
        || is_client_side
        || *policy == ToolPolicy::ClientSide
        || hints.stays_direct == Some(true))
}

pub(crate) fn definition_goes_behind_tools(def: &ToolDefinition) -> bool {
    goes_behind_tools(
        def.name(),
        matches!(def, ToolDefinition::ClientSide(_)),
        def.policy(),
        def.hints(),
    )
}

/// Add the `tools` builtin when this session enabled the capability.
pub(crate) fn install(builder: BashBuilder, context: &ToolContext) -> BashBuilder {
    if !installs(context) {
        return builder;
    }
    builder.builtin(
        TOOLS_COMMAND.to_string(),
        Box::new(builtin::SharedTools(ToolsBuiltin::new(context))),
    )
}

/// Whether the shell in `context` gets the `tools` command.
fn installs(context: &ToolContext) -> bool {
    let enabled = context
        .tool_registry
        .as_ref()
        .is_some_and(|registry| registry.get(TOOLS_COMMAND).is_some());
    enabled && context.nested_tool_policy.is_some()
}

/// Capability that puts the agent's tools behind one shell command.
pub struct ToolsInShellCapability;

#[async_trait]
impl Capability for ToolsInShellCapability {
    fn id(&self) -> &str {
        TOOLS_IN_SHELL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tools in Shell"
    }

    fn description(&self) -> &str {
        r#"Call the agent's tools from the bash shell with one `tools` command.

Most tools, MCP servers included, leave the model's direct tool list and become
commands such as `tools github list-pulls '{"repo":"a/b"}'`. One script can call
several of them, filter the results with `jq`, and return only what matters, in
one turn instead of a chain of tool calls.

> [!NOTE]
> Requires the bash shell. Client-side and turn-shaping tools stay directly
> callable. A call that needs approval stops the script and asks. Replaces
> tool search."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        // Scripts drive real tool calls in loops; admin-gated like its shell.
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    async fn system_prompt_contribution_with_config(
        &self,
        ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        if surface_mode(config) == ToolSurfaceMode::Hybrid {
            Some(format!(
                "<capability id=\"{TOOLS_IN_SHELL_CAPABILITY_ID}\">\n\
                 Native tool calls and the shell `tools` command can reach the same operations. \
                 Use native calls for one-off structured work; use `tools` inside `bash` to batch \
                 calls, filter results with `jq`, and return only what you need. Search shell \
                 operations first with `tools search <words>`; use bounded `tools --help` output to \
                 browse, and `tools <server> <tool> --help` for one input schema. Native \
                 `tool_search` is a separate discovery route when exposed. \
                 A call that needs approval stops the script; after the answer, write a new script \
                 for what is left and never rerun the stopped one.\n</capability>"
            ))
        } else {
            self.system_prompt_contribution(ctx).await
        }
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["bashkit_shell"]
    }

    fn supersedes(&self) -> Vec<&'static str> {
        SUPERSEDED.to_vec()
    }

    fn supersedes_with_config(&self, config: &Value) -> Vec<&'static str> {
        if surface_mode(config) == ToolSurfaceMode::Hybrid {
            vec![]
        } else {
            SUPERSEDED.to_vec()
        }
    }

    fn tool_surface_mode_with_config(&self, config: &Value) -> ToolSurfaceMode {
        surface_mode(config)
    }

    fn tool_surface_routes_with_config(
        &self,
        config: &Value,
        definitions: &[ToolDefinition],
    ) -> std::collections::BTreeMap<String, ToolSurfaceRoute> {
        let mode = surface_mode(config);
        let keep = keep_visible(config);
        definitions
            .iter()
            .map(|definition| {
                let eligible = definition_goes_behind_tools(definition);
                let explicitly_kept = keep.contains(definition.name()) && eligible;
                let surface = if explicitly_kept || (mode == ToolSurfaceMode::Hybrid && eligible) {
                    ToolSurface::Both
                } else if mode == ToolSurfaceMode::Cli && eligible {
                    ToolSurface::Cli
                } else {
                    ToolSurface::Native
                };
                let native_disclosure = NativeToolDisclosure::Eager;
                (
                    definition.name().to_string(),
                    ToolSurfaceRoute {
                        surface,
                        native_disclosure,
                    },
                )
            })
            .collect()
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolsMarkerTool)]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HideBehindToolsHook::default())]
    }

    fn tool_definition_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HideBehindToolsHook {
            keep_visible: keep_visible(config),
            mode: surface_mode(config),
        })]
    }

    fn tool_definition_hooks_with_context(
        &self,
        ctx: &SystemPromptContext,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HideBehindToolsHook {
            keep_visible: keep_visible(config),
            mode: ctx.tool_surface_plan.mode,
        })]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "title": "Tool surface",
                    "enum": ["cli", "hybrid"],
                    "default": "cli",
                    "description": "CLI keeps eligible operations in the shell. Hybrid also exposes eligible operations as native tools, with schemas loaded through native search when needed."
                },
                "keep_visible": {
                    "type": "array",
                    "title": "Keep visible",
                    "items": {
                        "type": "string",
                        "title": "Tool name",
                        "description": "Name of a tool to keep directly callable."
                    },
                    "description": "Tool names the model can still call directly. They stay callable from the shell too."
                },
                "manage_scripts": {
                    "type": "boolean",
                    "title": "Save scripts",
                    "default": false,
                    "description": "Let the agent save scripts with `tools scripts save`. Saved scripts can always be run."
                },
                "rate_unlabeled_tools": {
                    "type": "boolean",
                    "title": "Rate unlabeled tools",
                    "default": true,
                    "description": "Ask the decision service whether a tool that says nothing about its risk changes things, and ask for approval before it if so. Off: such tools run without asking in the normal approval mode."
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let object = config
            .as_object()
            .ok_or_else(|| "config must be a JSON object".to_string())?;
        if let Some(key) = object.keys().find(|key| {
            !matches!(
                key.as_str(),
                "keep_visible" | "manage_scripts" | "rate_unlabeled_tools" | "mode"
            )
        }) {
            return Err(format!("unknown config key: {key}"));
        }
        for flag in ["manage_scripts", "rate_unlabeled_tools"] {
            if object.get(flag).is_some_and(|v| !v.is_boolean()) {
                return Err(format!("{flag} must be true or false"));
            }
        }
        if let Some(mode) = object.get("mode")
            && !matches!(mode.as_str(), Some("cli" | "hybrid"))
        {
            return Err("mode must be `cli` or `hybrid`".to_string());
        }
        if let Some(keep) = object.get("keep_visible") {
            let list = keep
                .as_array()
                .ok_or_else(|| "keep_visible must be an array of tool names".to_string())?;
            if list.iter().any(|v| !v.is_string()) {
                return Err("keep_visible entries must be strings".to_string());
            }
        }
        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls which tools stay directly callable, whether the agent may save scripts, and whether tools without risk hints are rated.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Інструменти в оболонці"),
                description: Some(
                    "Агент викликає свої інструменти з оболонки bash однією командою tools: \
                     один скрипт робить кілька викликів, фільтрує результати через jq і \
                     повертає лише потрібне.",
                ),
                config_description: Some(
                    "Визначає, які інструменти залишаються доступними для прямого виклику, чи може агент зберігати скрипти і чи оцінюються інструменти без позначок ризику.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "keep_visible": {
                            "title": "Залишити видимими",
                            "description": "Назви інструментів, які модель може викликати напряму. Їх так само можна викликати з оболонки.",
                            "items": {
                                "title": "Назва інструмента",
                                "description": "Назва інструмента, що залишається доступним для прямого виклику."
                            }
                        },
                        "manage_scripts": {
                            "title": "Збереження скриптів",
                            "description": "Дозволити агенту зберігати скрипти командою `tools scripts save`. Збережені скрипти можна запускати завжди."
                        },
                        "rate_unlabeled_tools": {
                            "title": "Оцінювати інструменти без позначок",
                            "description": "Питати сервіс рішень, чи змінює щось інструмент, який нічого не каже про свій ризик, і якщо так, просити схвалення перед ним. Вимкнено: такі інструменти виконуються без запиту у звичайному режимі схвалення."
                        }
                    }
                })),
            },
        ]
    }
}

/// Whether the capability config lets the agent save scripts. The host reads
/// it when it binds the agent's saved scripts to a turn.
pub fn manage_scripts(config: &Value) -> bool {
    config
        .get("manage_scripts")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether the capability config lets the decision service rate tools that
/// declare no risk hints; on unless the agent turned it off. The host reads it
/// when it builds the turn's tool context and marks an off turn with
/// [`UnlabeledToolRatingsOff`].
pub fn rate_unlabeled_tools(config: &Value) -> bool {
    config
        .get("rate_unlabeled_tools")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn surface_mode(config: &Value) -> ToolSurfaceMode {
    match config.get("mode").and_then(Value::as_str) {
        Some("hybrid") => ToolSurfaceMode::Hybrid,
        _ => ToolSurfaceMode::Cli,
    }
}

fn keep_visible(config: &Value) -> HashSet<String> {
    config
        .get("keep_visible")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Hides the tools the shell can reach while preserving a stable bash schema.
struct HideBehindToolsHook {
    keep_visible: HashSet<String>,
    mode: ToolSurfaceMode,
}

impl Default for HideBehindToolsHook {
    fn default() -> Self {
        Self {
            keep_visible: HashSet::new(),
            mode: ToolSurfaceMode::Cli,
        }
    }
}

impl ToolDefinitionHook for HideBehindToolsHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        let mut kept = Vec::new();
        for def in tools {
            if def.name() == TOOLS_COMMAND {
                continue;
            }
            if self.mode == ToolSurfaceMode::Cli
                && !self.keep_visible.contains(def.name())
                && definition_goes_behind_tools(&def)
            {
                continue;
            }
            kept.push(def);
        }
        kept
    }

    fn preserves_execution_scope(&self) -> bool {
        true
    }

    fn applies_with_native_tool_search(&self) -> bool {
        // A routing policy, not a schema-deferral optimization.
        true
    }
}

/// Registered so the shell knows to install `tools`; never offered to the
/// model, which reaches it as the shell command.
struct ToolsMarkerTool;

#[async_trait]
impl Tool for ToolsMarkerTool {
    fn name(&self) -> &str {
        TOOLS_COMMAND
    }

    fn description(&self) -> &str {
        "Run the `tools` command inside the bash tool to call the agent's tools."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "`tools` is a shell command: run it inside the bash tool, for example `tools --help`.",
        )
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_stays_direct(true)
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_shell_exec(
            &tool_call.arguments,
            TOOLS_COMMAND,
            phase,
            locale,
            ctx,
        ))
    }
}

#[cfg(test)]
mod tests;

/// Pre-compile the embedded CPython payload once per process so the first
/// `python3` shell call doesn't pay WASM compilation latency (~19ms first
/// call, ~4.4ms after, per upstream measurements). Idempotent: call at startup
/// from every binary that executes bash tools. Never fails startup — a
/// warm-up failure only means the first `python3` call compiles on demand.
pub fn ensure_cpython_warm() {
    static WARMED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    WARMED.get_or_init(|| {
        if let Err(err) = bashkit::CPython::warm_up() {
            tracing::warn!(error = %err, "cpython warm-up failed");
        }
    });
}
