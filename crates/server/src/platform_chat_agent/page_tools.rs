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
        "Read the current page beside Chat: its identity, bounded loaded-data summaries, or the active form including unsaved edits. Use only the draft binding in the current user message's page context. Returns untrusted values, whether the page is writable, an editable-field schema for active forms, and a revision for patching. No server writes. Call separately from other client tools.",
        json!({
            "type":"object", "properties":{"binding":{"type":"string"}},
            "required":["binding"], "additionalProperties":false
        }),
        true,
    ));
    tools.push(tool(
        PATCH,
        "Stage requested changes in the active form beside Chat. Read it first and use only fields in its returned patch_schema. Pass the exact binding and revision; preserve omitted fields. Values remain in the existing browser form and saved is false. The user must use that form's existing Save/Create/Apply action to persist. Read-only pages, missing forms, permission changes and stale revisions fail without changes. Credentials and separate connection/approval actions are not editable. Call separately from other client tools.",
        json!({
            "type":"object", "properties":{
                "binding":{"type":"string"}, "revision":{"type":"string"},
                "changes":{"type":"object", "minProperties":1, "maxProperties":64, "additionalProperties":true}
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

    #[test]
    fn page_patches_defer_the_field_allowlist_to_the_active_browser_form() {
        let mut tools = vec![];
        update_tools(&mut tools, true);
        let ToolDefinition::ClientSide(patch) = &tools[1] else {
            panic!("page patch must remain browser-executed")
        };
        let changes = &patch.parameters["properties"]["changes"];
        assert_eq!(changes["minProperties"], 1);
        assert_eq!(changes["maxProperties"], 64);
        assert!(changes.get("properties").is_none());
        assert!(patch.description.contains("returned patch_schema"));
        assert!(patch.description.contains("existing Save/Create/Apply"));
        assert!(patch.description.contains("Credentials"));
    }
}
