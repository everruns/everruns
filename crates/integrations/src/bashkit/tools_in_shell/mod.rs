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
//! - **One predicate.** [`goes_behind_tools`] decides both what the hook hides
//!   from the model and what the command can reach, so a hidden tool is always
//!   reachable and a tool that must stay direct is never callable from a
//!   script.
//! - **Spelling.** MCP tools are grouped by server (`tools github list-pulls`);
//!   every other tool is a top-level command (`tools web-fetch`). Registry
//!   tools carry no capability attribution to group the rest by.
//! - **Per-call records, not tool events.** A call from a script is not
//!   emitted as its own `tool.started` / `tool.completed`: the shell call is
//!   the one tool call the conversation records. Each call is recorded as a
//!   `tool.nested_call` event under that shell call instead (see `timeline`).
//! - **Tool search.** The catalog is the discovery surface, so this capability
//!   supersedes the tool search capabilities (see [`Capability::supersedes`]).

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
pub use run::finish as finish_run;

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use bashkit::BashBuilder;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
    ToolDefinitionHook,
};
use everruns_contracts::runtime::mcp_deferred::deferred_mcp_server_prefix;
use everruns_contracts::runtime::mcp_server::parse_mcp_tool_name;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_shell_exec,
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

/// Tools that are themselves the way in (`bash`, `lua`, the marker), the
/// discovery surface this replaces, or drive a screen the model must watch.
/// `spawn_agent` is named here because three capabilities (subagents, handoff,
/// A2A delegation) each contribute a tool by that name; every other tool's
/// owner marks it with [`ToolHints::stays_direct`].
const ALWAYS_DIRECT: &[&str] = &[
    "bash",
    "lua",
    TOOLS_COMMAND,
    "tool_search",
    "computer",
    "browser",
    "spawn_agent",
];

/// Behaviour contract only; the inventory lives in the `bash` description and
/// in `tools --help`.
const SYSTEM_PROMPT: &str = "Most tools are not in your direct tool list: call them from the \
`bash` tool with the `tools` command (`tools --help` lists them, input is one JSON object, output \
is JSON). Prefer one script that chains several calls and prints only what you need over a series \
of separate tool calls. A call that needs approval stops the script and reports what ran; after \
the answer, write a new script for what is left and never rerun the stopped one.";

/// Capability plugins this module contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: None,
    factory: || Box::new(ToolsInShellCapability),
}];

/// Whether a tool is reached through `tools` rather than called directly.
///
/// A tool stays direct when it pauses or shapes the turn, or when its result
/// only makes sense to the model directly. Approval-gated and destructive
/// tools go behind: the turn's approval gate judges each call from a script as
/// it would a direct one, and a call it holds stops the script with a report
/// (see `run`). A deferred MCP server's placeholder (`mcp_<server>`) goes
/// behind too: the shell loads the server itself.
pub(crate) fn goes_behind_tools(
    name: &str,
    is_client_side: bool,
    policy: &ToolPolicy,
    deferrable: &DeferrablePolicy,
    hints: &ToolHints,
) -> bool {
    if !is_client_side && deferred_mcp_server_prefix(name).is_some() {
        return true;
    }
    !(ALWAYS_DIRECT.contains(&name)
        || is_client_side
        || *policy == ToolPolicy::ClientSide
        || *deferrable == DeferrablePolicy::Never
        || hints.stays_direct == Some(true))
}

fn definition_goes_behind_tools(def: &ToolDefinition) -> bool {
    goes_behind_tools(
        def.name(),
        matches!(def, ToolDefinition::ClientSide(_)),
        def.policy(),
        def.deferrable(),
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

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["bashkit_shell"]
    }

    fn supersedes(&self) -> Vec<&'static str> {
        SUPERSEDED.to_vec()
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
        })]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
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
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "keep_visible" | "manage_scripts"))
        {
            return Err(format!("unknown config key: {key}"));
        }
        if object
            .get("manage_scripts")
            .is_some_and(|v| !v.is_boolean())
        {
            return Err("manage_scripts must be true or false".to_string());
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
                    "Controls which tools stay directly callable and whether the agent may save scripts.",
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
                    "Визначає, які інструменти залишаються доступними для прямого виклику і чи може агент зберігати скрипти.",
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

/// Hides the tools the shell can reach and names them on the `bash` tool, the
/// description the model reads right before it writes a script.
#[derive(Default)]
struct HideBehindToolsHook {
    keep_visible: HashSet<String>,
}

impl ToolDefinitionHook for HideBehindToolsHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        let mut kept = Vec::new();
        let mut hidden: Vec<(String, Option<String>)> = Vec::new();
        for def in tools {
            if def.name() == TOOLS_COMMAND {
                continue;
            }
            if !self.keep_visible.contains(def.name()) && definition_goes_behind_tools(&def) {
                let server = parse_mcp_tool_name(def.name()).map(|(server, _)| server);
                hidden.push((def.name().to_string(), server));
            } else {
                kept.push(def);
            }
        }
        if !hidden.is_empty()
            && let Some(bash) = kept.iter_mut().find(|d| d.name() == "bash")
        {
            let summary = catalog::Catalog::summary_line(
                hidden
                    .iter()
                    .map(|(name, server)| (name.as_str(), server.as_deref())),
            );
            let block = format!(
                "\n\nMore tools are available ONLY inside this shell through the `tools` command: \
                 {summary}. Run `tools --help` to list them and `tools <name> --help` for one \
                 tool's input; pass input as one JSON object."
            );
            match bash {
                ToolDefinition::Builtin(b) => b.description.push_str(&block),
                ToolDefinition::ClientSide(c) => c.description.push_str(&block),
            }
        }
        kept
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
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_shell_exec(
            &tool_call.arguments,
            TOOLS_COMMAND,
            phase,
            locale,
        ))
    }
}

#[cfg(test)]
mod tests;
