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
//!    the existing [`ToolDefinitionHook`] seam (see `knowledge/execution/capabilities.md` →
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

use super::is_code_mode_eligible;
use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, ToolDefinitionHook};
use crate::tool_types::ToolDefinition;

pub const LUA_CODE_MODE_CAPABILITY_ID: &str = "lua_code_mode";

/// Behavior contract only (no tool inventory — that lives in the tool schemas
/// and the `lua` capability's own prompt). Kept to two dense sentences per the
/// prompt-budget guidance in `knowledge/execution/capabilities.md`.
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
        vec![Arc::new(HideCodeModeToolsHook::default())]
    }

    fn tool_definition_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        let opts = parse_options(config);
        vec![Arc::new(HideCodeModeToolsHook {
            keep_visible: opts.keep_visible,
            full_schemas: opts.full_schemas,
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
                    "description": "Tool names to keep directly callable by the model even when they would otherwise be routed through Lua code mode."
                },
                "full_schemas": {
                    "type": "boolean",
                    "title": "Embed full schemas",
                    "default": false,
                    "description": "Embed each hidden tool's full JSON Schema in the lua tool description (lossless discovery). Off by default: a compact typed signature is shown instead."
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
        // Enforce the same closed contract as `config_schema`
        // (`additionalProperties: false`) on every write path.
        for key in obj.keys() {
            if key != "keep_visible" && key != "full_schemas" {
                return Err(format!("unknown config key: {key}"));
            }
        }
        if let Some(keep) = obj.get("keep_visible") {
            let arr = keep
                .as_array()
                .ok_or_else(|| "keep_visible must be an array of tool names".to_string())?;
            if arr.iter().any(|v| !v.is_string()) {
                return Err("keep_visible entries must be strings".to_string());
            }
        }
        if let Some(full) = obj.get("full_schemas")
            && !full.is_boolean()
        {
            return Err("full_schemas must be a boolean".to_string());
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
                    "Controls which tools stay directly callable and whether hidden \
                     tools' full JSON Schemas are embedded in the lua tool description.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Режим коду Lua"),
                description: Some(
                    "Спрямовує виклики неосновних інструментів через пісочницю Lua: \
                     приховує їх із прямого списку інструментів моделі, тож агент \
                     викликає їх усередині інструмента lua через tools.<name>(args) — \
                     менше окремих звернень, структуровані результати.",
                ),
                config_description: Some(
                    "Визначає, які інструменти залишаються доступними для прямого виклику, та чи вбудовувати повні JSON-схеми прихованих інструментів в опис інструмента lua.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "keep_visible": {
                            "title": "Залишити видимими",
                            "description": "Назви інструментів, які модель може викликати напряму, навіть якщо їх інакше було б спрямовано через режим коду Lua.",
                            "items": {
                                "title": "Назва інструмента",
                                "description": "Назва інструмента, що залишається доступним для прямого виклику."
                            }
                        },
                        "full_schemas": {
                            "title": "Вбудовувати повні схеми",
                            "description": "Вбудовує повну JSON-схему кожного прихованого інструмента в опис інструмента lua (виявлення без втрат). Типово вимкнено: натомість показується компактний типізований підпис."
                        }
                    }
                })),
            },
        ]
    }
}

/// Per-agent options parsed from capability config.
#[derive(Default)]
struct CodeModeOptions {
    keep_visible: Vec<String>,
    full_schemas: bool,
}

fn parse_options(config: &Value) -> CodeModeOptions {
    CodeModeOptions {
        keep_visible: config
            .get("keep_visible")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        full_schemas: config
            .get("full_schemas")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Drops code-mode-eligible tool definitions from the model-facing tool list
/// and grafts a catalog of them onto the `lua` tool description.
///
/// Tools listed in `keep_visible` are always retained. Everything that is *not*
/// eligible (the `lua`/`bash` execution tools, destructive, approval-gated,
/// client-side, and `cpu_bound` tools) is also retained — those stay direct
/// tool calls.
///
/// Because the hidden tools' standalone schemas are removed, the model would
/// otherwise have no way to discover their names/arguments. So the eligible set
/// is summarized into a `tools.<name>{ args }` catalog appended to the `lua`
/// tool's own description — the discovery surface the model reads right before
/// it decides to call `lua`. Each entry shows a typed signature; with
/// `full_schemas` it also embeds the tool's complete JSON Schema (lossless).
#[derive(Default)]
struct HideCodeModeToolsHook {
    keep_visible: Vec<String>,
    full_schemas: bool,
}

impl ToolDefinitionHook for HideCodeModeToolsHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        let mut kept: Vec<ToolDefinition> = Vec::new();
        let mut catalog: Vec<String> = Vec::new();
        for def in tools {
            let name = def.name();
            let hidden = !self.keep_visible.iter().any(|k| k == name)
                && is_code_mode_eligible(name, def.policy(), def.hints());
            if hidden {
                catalog.push(format_catalog_entry(&def, self.full_schemas));
            } else {
                kept.push(def);
            }
        }
        if !catalog.is_empty()
            && let Some(lua) = kept.iter_mut().find(|d| d.name() == "lua")
        {
            graft_catalog_onto_lua(lua, &catalog);
        }
        kept
    }

    fn applies_with_native_tool_search(&self) -> bool {
        // This is a hard routing policy (the model must go through Lua), not a
        // schema-deferral optimization, so it applies regardless of whether the
        // model uses hosted tool_search.
        true
    }
}

/// One catalog entry: `- name(a: number, b?: string) — first sentence`, with an
/// optional indented `schema: { ... }` line when `full_schemas` is set.
fn format_catalog_entry(def: &ToolDefinition, full_schemas: bool) -> String {
    let sig = typed_signature(def.full_parameters());
    let desc = first_sentence(def.description());
    let mut entry = if desc.is_empty() {
        format!("- {}({sig})", def.name())
    } else {
        format!("- {}({sig}) — {desc}", def.name())
    };
    if full_schemas {
        entry.push_str(&format!(
            "\n    schema: {}",
            compact_schema(def.full_parameters())
        ));
    }
    entry
}

/// Typed argument signature from a JSON Schema object: `name: type` for required
/// args first, then `name?: type` for optional ones. The synthetic
/// `human_intent` narration argument (added by `human_intent`) is omitted.
fn typed_signature(schema: &Value) -> String {
    let Some(props) = schema.get("properties").and_then(|v| v.as_object()) else {
        return String::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for r in &required {
        if *r != crate::tool_types::HUMAN_INTENT_ARGUMENT
            && let Some(prop) = props.get(*r)
        {
            parts.push(format!("{r}: {}", json_type(prop)));
        }
    }
    for (key, prop) in props {
        if key != crate::tool_types::HUMAN_INTENT_ARGUMENT && !required.contains(&key.as_str()) {
            parts.push(format!("{key}?: {}", json_type(prop)));
        }
    }
    parts.join(", ")
}

/// Render a JSON Schema property's `type` as a short string (e.g. `number`,
/// `string|null`, `enum`, `object`). Falls back to `any` when unspecified.
fn json_type(prop: &Value) -> String {
    match prop.get("type") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("|"),
        _ if prop.get("enum").is_some() => "enum".to_string(),
        _ => "any".to_string(),
    }
}

/// Minified JSON Schema for full discovery, with the synthetic `human_intent`
/// argument stripped (it is never passed to `tools.*`).
fn compact_schema(schema: &Value) -> String {
    let mut schema = schema.clone();
    if let Some(obj) = schema.as_object_mut() {
        if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
            props.remove(crate::tool_types::HUMAN_INTENT_ARGUMENT);
        }
        if let Some(req) = obj.get_mut("required").and_then(|v| v.as_array_mut()) {
            req.retain(|v| v.as_str() != Some(crate::tool_types::HUMAN_INTENT_ARGUMENT));
        }
    }
    serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
}

/// First sentence (up to `.` or newline), trimmed and length-capped.
fn first_sentence(description: &str) -> String {
    let trimmed = description.trim();
    let end = trimmed
        .find(['.', '\n'])
        .map(|i| i + 1)
        .unwrap_or(trimmed.len());
    let mut s = trimmed[..end].trim().to_string();
    const MAX: usize = 140;
    if s.len() > MAX {
        let boundary = s
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= MAX)
            .last()
            .unwrap_or(0);
        s.truncate(boundary);
        s.push('…');
    }
    s
}

/// Append the catalog to the `lua` tool's description so the model can discover
/// the in-script `tools.*` functions and their arguments.
fn graft_catalog_onto_lua(def: &mut ToolDefinition, catalog: &[String]) {
    let block = format!(
        "\n\nThese tools are available ONLY inside this script via the `tools` table — \
call them as `tools.<name>{{ args }}` (e.g. `local r = tools.add{{ a = 1, b = 2 }}`). \
Argument types follow each name:\n{}",
        catalog.join("\n")
    );
    match def {
        ToolDefinition::Builtin(b) => b.description.push_str(&block),
        ToolDefinition::ClientSide(c) => c.description.push_str(&block),
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

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn hides_eligible_tools_keeps_lua_and_essential() {
        let hook = HideCodeModeToolsHook {
            keep_visible: Vec::new(),
            full_schemas: false,
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
    fn grafts_catalog_onto_lua_description() {
        let hook = HideCodeModeToolsHook {
            keep_visible: Vec::new(),
            full_schemas: false,
        };
        let add = ToolDefinition::Builtin(BuiltinTool {
            name: "add".to_string(),
            display_name: None,
            description: "Add two numbers together and return the result.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
                "required": ["a", "b"],
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        });
        let defs = vec![builtin("lua", ToolPolicy::Auto, ToolHints::default()), add];

        let kept = hook.transform(defs);
        // `add` is gone from the model's tool list...
        assert_eq!(names(&kept), vec!["lua"]);
        // ...but discoverable from the lua tool description with typed arguments.
        let lua_desc = kept[0].description();
        assert!(lua_desc.contains("tools.<name>"), "{lua_desc}");
        assert!(
            lua_desc.contains("- add(a: number, b: number)"),
            "{lua_desc}"
        );
        assert!(
            lua_desc.contains("Add two numbers together"),
            "catalog should carry the description: {lua_desc}",
        );
        // Default mode does not embed the full JSON schema.
        assert!(!lua_desc.contains("schema:"), "{lua_desc}");
    }

    #[test]
    fn full_schemas_embeds_complete_schema() {
        let hook = HideCodeModeToolsHook {
            keep_visible: Vec::new(),
            full_schemas: true,
        };
        let add = ToolDefinition::Builtin(BuiltinTool {
            name: "add".to_string(),
            display_name: None,
            description: "Add.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" },
                    "human_intent": { "type": "string" }
                },
                "required": ["a", "b"],
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        });
        let kept = hook.transform(vec![
            builtin("lua", ToolPolicy::Auto, ToolHints::default()),
            add,
        ]);
        let lua_desc = kept[0].description();
        assert!(lua_desc.contains("schema:"), "{lua_desc}");
        assert!(lua_desc.contains("\"properties\""), "{lua_desc}");
        // The synthetic human_intent arg is stripped from both signature & schema.
        assert!(!lua_desc.contains("human_intent"), "{lua_desc}");
    }

    #[test]
    fn first_sentence_truncates_multibyte_descriptions_on_char_boundary() {
        let desc = format!("{}é", "a".repeat(139));

        let truncated = first_sentence(&desc);

        assert_eq!(truncated, format!("{}…", "a".repeat(139)));
    }

    #[test]
    fn catalog_handles_mcp_like_multibyte_tool_descriptions() {
        let hook = HideCodeModeToolsHook {
            keep_visible: Vec::new(),
            full_schemas: false,
        };
        let mcp_like_tool = ToolDefinition::Builtin(BuiltinTool {
            name: "mcp_server_tool".to_string(),
            display_name: None,
            description: format!("{}é", "a".repeat(139)),
            parameters: json!({ "type": "object" }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default().with_open_world(true),
            full_parameters: None,
        });

        let kept = hook.transform(vec![
            builtin("lua", ToolPolicy::Auto, ToolHints::default()),
            mcp_like_tool,
        ]);

        assert_eq!(names(&kept), vec!["lua"]);
        let lua_desc = kept[0].description();
        assert!(lua_desc.contains("- mcp_server_tool()"), "{lua_desc}");
        assert!(
            lua_desc.contains(&format!("{}…", "a".repeat(139))),
            "{lua_desc}"
        );
        assert!(!lua_desc.contains('é'), "{lua_desc}");
    }

    #[test]
    fn typed_signature_handles_optional_and_union_types() {
        let sig = typed_signature(&json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "limit": { "type": ["integer", "null"] },
                "mode": { "enum": ["a", "b"] }
            },
            "required": ["path"],
        }));
        assert!(sig.starts_with("path: string"), "{sig}");
        assert!(sig.contains("limit?: integer|null"), "{sig}");
        assert!(sig.contains("mode?: enum"), "{sig}");
    }

    #[test]
    fn keep_visible_overrides_hiding() {
        let hook = HideCodeModeToolsHook {
            keep_visible: vec!["add".to_string()],
            full_schemas: false,
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
        assert!(
            cap.validate_config(&json!({ "full_schemas": true }))
                .is_ok()
        );
        assert!(
            cap.validate_config(&json!({ "full_schemas": "yes" }))
                .is_err()
        );
        // Closed contract: unknown keys are rejected (matches config_schema's
        // additionalProperties: false).
        assert!(
            cap.validate_config(&json!({ "nope": 1 })).is_err(),
            "unknown config keys must be rejected"
        );
    }

    #[test]
    fn uk_localization_resolves() {
        let cap = LuaCodeModeCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Режим коду Lua");
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("пісочницю Lua")
        );
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());
    }

    // Prompt-budget ratchet (knowledge/execution/capabilities.md → prompt size). Lowering is
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
