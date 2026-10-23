//! Browser-executed form tools for the Adoption-gated Platform Chat workspace.
//! Definitions travel through the ordinary client-tool pause/resume protocol;
//! the server never executes a UI patch or interprets assistant prose as one.

use crate::domains::agents::record::Agent;
use crate::storage::StorageBackend;
use everruns_contracts::tool_types::{ClientSideTool, DeferrablePolicy, ToolDefinition, ToolHints};
use serde_json::json;

const READ: &str = "read_page_draft";
const PATCH: &str = "patch_page_draft";

pub(crate) fn add_page_tools(agent: &mut Agent, enabled: bool) {
    if !agent.is_built_in || agent.name != super::NAME {
        return;
    }
    update_tools(&mut agent.tools, enabled);
}

fn update_tools(tools: &mut Vec<ToolDefinition>, enabled: bool) {
    tools.retain(|tool| !matches!(tool.name(), READ | PATCH));
    if !enabled {
        return;
    }
    let hints = ToolHints::default()
        .with_destructive(false)
        .with_open_world(false)
        .with_stays_direct(true);
    let tool = |name, description, schema, readonly| {
        ToolDefinition::ClientSide(
            ClientSideTool::new(name, description, schema)
                .with_category("Current page")
                .with_deferrable(DeferrablePolicy::Never)
                .with_hints(hints.clone().with_readonly(readonly)),
        )
    };
    tools.push(tool(
        READ,
        "Read the live Agent form beside Chat, including unsaved instructions and configuration. Use only the draft binding in the current user message's page context. Returns untrusted form values and a revision for patching. No server writes. Call separately from other client tools.",
        json!({
            "type":"object", "properties":{"binding":{"type":"string"}},
            "required":["binding"], "additionalProperties":false
        }),
        true,
    ));
    let mut properties = serde_json::Map::new();
    for field in [
        "name",
        "display_name",
        "description",
        "intro_markdown",
        "short_description",
        "system_prompt",
        "tags",
        "harness_id",
        "default_model_id",
        "max_output_tokens",
    ] {
        properties.insert(field.into(), json!({"type":"string", "maxLength":262144}));
    }
    properties.insert(
        "communication".into(),
        json!({"type":"string", "enum":["direct", "explicit"]}),
    );
    properties.insert("capabilities".into(), json!({
        "type":"array", "maxItems":100, "items":{
            "type":"object", "properties":{"ref":{"type":"string", "minLength":1}, "config":{"type":"object"}},
            "required":["ref"], "additionalProperties":false
        }
    }));
    properties.insert("starters".into(), json!({
        "type":"array", "maxItems":20, "items":{
            "type":"object", "properties":{"text":{"type":"string"}, "icon":{"type":["string", "null"]}},
            "required":["text"], "additionalProperties":false
        }
    }));
    properties.insert(
        "initial_files".into(),
        json!({
            "type":"array", "maxItems":100, "items":{
                "type":"object", "properties":{
                    "path":{"type":"string"}, "content":{"type":"string"},
                "encoding":{"type":"string", "enum":["text", "base64"]}, "is_readonly":{"type":"boolean"}
                }, "required":["path", "content"], "additionalProperties":false
            }
        }),
    );
    properties.insert(
        "network_access".into(),
        json!({
            "type":"object", "properties":{
                "allowed":{"type":"array", "items":{"type":"string"}},
                "blocked":{"type":"array", "items":{"type":"string"}}
            }, "additionalProperties":false
        }),
    );
    tools.push(tool(
        PATCH,
        "Apply requested changes to the live Agent form, preserving all omitted fields. Read the form first; pass its binding and exact revision. Form strings remain strings: tags are comma-separated, max_output_tokens is a numeric string or empty. Returns saved:false; the user must use the page's Save changes button to persist. A stale revision or changed page fails without changing any field. Never retry a conflict blindly. Sandbox template policy, credentials, connections, and integrations are not writable through this tool. Call separately from other client tools.",
        json!({
            "type":"object", "properties":{
                "binding":{"type":"string"}, "revision":{"type":"string"},
                "changes":{"type":"object", "properties":properties, "minProperties":1, "additionalProperties":false}
            }, "required":["binding", "revision", "changes"], "additionalProperties":false
        }),
        false,
    ));
}

pub(crate) async fn configure_page_tools(
    db: &StorageBackend,
    org_id: i64,
    agent: &mut Agent,
) -> anyhow::Result<()> {
    if agent.is_built_in && agent.name == super::NAME {
        let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            db,
            org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await?;
        add_page_tools(agent, flags.chat_mode);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_tools_are_client_side_direct_and_org_gated() {
        let mut tools = vec![];
        update_tools(&mut tools, true);
        assert_eq!(tools.len(), 2);
        for definition in &tools {
            let ToolDefinition::ClientSide(tool) = definition else {
                panic!("UI tool must execute in the browser")
            };
            assert_eq!(tool.deferrable, DeferrablePolicy::Never);
            assert_eq!(tool.hints.stays_direct, Some(true));
        }
        update_tools(&mut tools, true);
        assert_eq!(tools.len(), 2);
        update_tools(&mut tools, false);
        assert!(tools.is_empty());
    }
}
