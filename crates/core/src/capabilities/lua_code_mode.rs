//! Lua Code-Mode Routing Capability (experimental)
//!
//! Turns the sandboxed `lua` tool into the agent's *primary* action surface by
//! hiding "non-essential" tools from the model's direct tool list. Those tools
//! are not removed from the running session — they stay in the executable
//! registry — they are just no longer offered to the model as direct tool
//! calls. The agent reaches them inside a Lua script through the existing
//! code-mode `tools.<name>(args)` table, so one script can read inputs, call
//! several sibling tools, and shape the result in a single turn instead of a
//! chain of individual round-trips.
//!
//! ## How the non-functional requirements are met
//!
//! 1. **Capability-extensibility only (tool filtering).** The sole mechanism is
//!    the existing [`ToolDefinitionHook`] seam (see `specs/capabilities.md` →
//!    Tool definition hooks). No new agent-loop plumbing: the hook runs after
//!    the runtime agent has merged its final tool list and simply drops the
//!    eligible definitions before they reach the model. The *executable*
//!    `ToolRegistry` is built separately from capability `tools()` and is left
//!    untouched, which is what keeps the hidden tools callable from Lua.
//! 2. **Relies on the existing `lua` capability.** `lua` is declared as a hard
//!    [`dependencies`](Capability::dependencies) entry, and its code mode
//!    (`tools.<name>`) is what makes the hidden tools invokable. The filter
//!    mirrors [`is_code_mode_eligible`] exactly, so the set removed from the
//!    model is precisely the set Lua re-exposes — no tool can become
//!    unreachable.
//! 3. **Capabilities export their functions to Lua.** Every eligible capability
//!    tool is surfaced inside the VM as a `tools.<name>(args)` function by the
//!    `lua` engine. This capability is what makes that export the agent's
//!    default path rather than an occasional optimization.

use std::sync::Arc;

use serde_json::{Value, json};

use super::lua::is_code_mode_eligible;
use super::{Capability, CapabilityStatus, RiskLevel, ToolDefinitionHook};
use crate::tool_types::ToolDefinition;

pub const LUA_CODE_MODE_CAPABILITY_ID: &str = "lua_code_mode";

/// Behavior contract only (no tool inventory — that lives in the tool schemas
/// and the `lua` capability's own prompt). Kept to two dense sentences per the
/// prompt-budget guidance in `specs/capabilities.md`.
const SYSTEM_PROMPT: &str = "Most action tools are intentionally omitted from your direct tool list; \
to use them, call them from inside the `lua` tool via the `tools.<name>(args)` table, where a single \
script can chain several calls and process their results in one turn. Prefer one Lua script over a \
sequence of separate tool calls.";

/// Capability that routes non-essential tool calls through the Lua sandbox.
pub struct LuaCodeModeCapability;

impl Capability for LuaCodeModeCapability {
    fn id(&self) -> &str {
        LUA_CODE_MODE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Lua Code Mode"
    }

    fn description(&self) -> &str {
        r#"Route non-essential tool calls through the Lua sandbox.

Hides auto, non-destructive tools from the model's direct tool list so the agent
orchestrates them inside the `lua` tool via `tools.<name>(args)` — fewer
round-trips, structured results.

> [!NOTE]
> Requires the `lua` capability. Execution, approval-gated, destructive, and
> client-side tools stay directly callable."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        // Inseparable from `lua` (scripted code execution) and reshapes how the
        // agent acts. Admin-gated like its dependency.
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("code")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["lua"]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HideCodeModeToolsHook {
            keep_visible: Vec::new(),
        })]
    }

    fn tool_definition_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HideCodeModeToolsHook {
            keep_visible: parse_keep_visible(config),
        })]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "keep_visible": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names to keep directly callable by the model even when they would otherwise be routed through Lua code mode."
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let obj = config
            .as_object()
            .ok_or_else(|| "config must be a JSON object".to_string())?;
        if let Some(keep) = obj.get("keep_visible") {
            let arr = keep
                .as_array()
                .ok_or_else(|| "keep_visible must be an array of tool names".to_string())?;
            if arr.iter().any(|v| !v.is_string()) {
                return Err("keep_visible entries must be strings".to_string());
            }
        }
        Ok(())
    }
}

/// Read the optional `keep_visible` allowlist from per-agent config.
fn parse_keep_visible(config: &Value) -> Vec<String> {
    config
        .get("keep_visible")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Drops code-mode-eligible tool definitions from the model-facing tool list.
///
/// Tools listed in `keep_visible` are always retained. Everything that is *not*
/// eligible (the `lua`/`bash` execution tools, destructive, approval-gated,
/// client-side, and `cpu_bound` tools) is also retained — those stay direct
/// tool calls.
struct HideCodeModeToolsHook {
    keep_visible: Vec<String>,
}

impl ToolDefinitionHook for HideCodeModeToolsHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        tools
            .into_iter()
            .filter(|def| {
                let name = def.name();
                self.keep_visible.iter().any(|k| k == name)
                    || !is_code_mode_eligible(name, def.policy(), def.hints())
            })
            .collect()
    }

    fn applies_with_native_tool_search(&self) -> bool {
        // This is a hard routing policy (the model must go through Lua), not a
        // schema-deferral optimization, so it applies regardless of whether the
        // model uses hosted tool_search.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, ClientSideTool, DeferrablePolicy, ToolHints, ToolPolicy};

    fn builtin(name: &str, policy: ToolPolicy, hints: ToolHints) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: format!("{name} tool"),
            parameters: json!({ "type": "object" }),
            policy,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints,
            full_parameters: None,
        })
    }

    fn names(defs: &[ToolDefinition]) -> Vec<&str> {
        defs.iter().map(|d| d.name()).collect()
    }

    #[test]
    fn capability_metadata() {
        let cap = LuaCodeModeCapability;
        assert_eq!(cap.id(), "lua_code_mode");
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.dependencies(), vec!["lua"]);
        assert_eq!(cap.category(), Some("Execution"));
        assert!(cap.system_prompt_addition().is_some());
    }

    #[test]
    fn hides_eligible_tools_keeps_lua_and_essential() {
        let hook = HideCodeModeToolsHook {
            keep_visible: Vec::new(),
        };
        let defs = vec![
            builtin("lua", ToolPolicy::Auto, ToolHints::default()),
            builtin("bash", ToolPolicy::Auto, ToolHints::default()),
            builtin("add", ToolPolicy::Auto, ToolHints::default()),
            builtin(
                "delete_file",
                ToolPolicy::Auto,
                ToolHints::default().with_destructive(true),
            ),
            builtin(
                "transfer_funds",
                ToolPolicy::RequiresApproval,
                ToolHints::default(),
            ),
            ToolDefinition::ClientSide(ClientSideTool {
                name: "pick_file".to_string(),
                display_name: None,
                description: "client".to_string(),
                parameters: json!({ "type": "object" }),
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default(),
                full_parameters: None,
            }),
        ];

        let transformed = hook.transform(defs);
        let kept = names(&transformed);
        // `add` is the only eligible tool → hidden. Everything else stays.
        assert!(kept.contains(&"lua"));
        assert!(kept.contains(&"bash"));
        assert!(kept.contains(&"delete_file"));
        assert!(kept.contains(&"transfer_funds"));
        assert!(kept.contains(&"pick_file"));
        assert!(!kept.contains(&"add"), "eligible tool must be hidden");
    }

    #[test]
    fn keep_visible_overrides_hiding() {
        let hook = HideCodeModeToolsHook {
            keep_visible: vec!["add".to_string()],
        };
        let defs = vec![
            builtin("add", ToolPolicy::Auto, ToolHints::default()),
            builtin("multiply", ToolPolicy::Auto, ToolHints::default()),
        ];
        let transformed = hook.transform(defs);
        let kept = names(&transformed);
        assert!(kept.contains(&"add"), "keep_visible must retain the tool");
        assert!(!kept.contains(&"multiply"));
    }

    #[test]
    fn config_aware_hook_reads_keep_visible() {
        let cap = LuaCodeModeCapability;
        let hook = cap
            .tool_definition_hooks_with_config(&json!({ "keep_visible": ["multiply"] }))
            .pop()
            .unwrap();
        let defs = vec![
            builtin("add", ToolPolicy::Auto, ToolHints::default()),
            builtin("multiply", ToolPolicy::Auto, ToolHints::default()),
        ];
        let transformed = hook.transform(defs);
        let kept = names(&transformed);
        assert_eq!(kept, vec!["multiply"]);
    }

    #[test]
    fn validate_config_rejects_bad_keep_visible() {
        let cap = LuaCodeModeCapability;
        assert!(
            cap.validate_config(&json!({ "keep_visible": ["ok"] }))
                .is_ok()
        );
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&Value::Null).is_ok());
        assert!(
            cap.validate_config(&json!({ "keep_visible": "nope" }))
                .is_err()
        );
        assert!(
            cap.validate_config(&json!({ "keep_visible": [1, 2] }))
                .is_err()
        );
    }

    // Prompt-budget ratchet (specs/capabilities.md → prompt size). Lowering is
    // always fine; raising requires an explicit bump + justification.
    #[test]
    fn system_prompt_within_budget() {
        let cap = LuaCodeModeCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(
            prompt.len() <= 500,
            "lua_code_mode prompt grew to {} bytes (budget 500)",
            prompt.len()
        );
    }
}
