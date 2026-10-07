//! Harness inheritance chain resolution.
//!
//! Split out of `direct_worker_adapters` (EVE-1024): walking a parent chain and
//! folding overlays is an algorithm over harness rows, not an adapter concern,
//! and that file is on the file-size ratchet.

use crate::records::{Harness, HarnessStatus, merge_harness};
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::HarnessId;
use uuid::Uuid;

use crate::direct_worker_adapters::store_error;
use crate::storage::StorageBackend;

/// Resolve `harness_id` to the harness an execution actually sees.
///
/// Walks `parent_harness_id` to the root, then folds the chain from the root
/// down so a child's fields win over its parents'. A repeated id is a cycle and
/// fails rather than looping; a missing parent mid-chain is an error, while a
/// missing root is simply "no such harness".
pub async fn resolve_effective_harness(
    db: &StorageBackend,
    org_id: i64,
    harness_id: Uuid,
) -> Result<Option<Harness>> {
    let mut visited = std::collections::HashSet::new();
    let mut chain = Vec::new();
    let mut cursor = Some(HarnessId::from_uuid(harness_id));

    while let Some(current_harness_id) = cursor {
        if !visited.insert(current_harness_id) {
            return Err(store_error("Harness inheritance cycle detected"));
        }

        let row = db
            .get_harness(org_id, current_harness_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get harness: {}", e);
                store_error("Failed to get harness")
            })?;
        let Some(row) = row else {
            if chain.is_empty() {
                return Ok(None);
            }
            return Err(store_error("Parent harness not found"));
        };

        let capabilities = db
            .get_harness_capabilities(current_harness_id.uuid())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|cap| AgentCapabilityConfig::with_config(cap.capability_id, cap.config))
            .collect();
        let capabilities =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                db,
                org_id,
                capabilities,
            )
            .await?;

        cursor = row.parent_harness_id;
        chain.push(Harness {
            id: row.id,
            name: row.name,
            display_name: row.display_name,
            icon: None,
            description: row.description,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: row.system_prompt,
            parent_harness_id: row.parent_harness_id,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
            initial_files: serde_json::from_value(row.initial_files).unwrap_or_default(),
            network_access: row
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            // No per-harness config column (EVE-598).
            parallel_tool_calls: None,
            embedder_metadata: serde_json::from_value(row.embedder_metadata).unwrap_or_default(),
            is_built_in: row.is_built_in,
            status: match row.status.as_str() {
                "active" => HarnessStatus::Active,
                "archived" => HarnessStatus::Archived,
                "deleted" => HarnessStatus::Deleted,
                _ => HarnessStatus::Active,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        });
    }

    let Some(mut effective) = chain.pop() else {
        return Ok(None);
    };
    while let Some(layer) = chain.pop() {
        effective = merge_harness(&effective, &layer);
    }

    Ok(Some(effective))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{CreateDeclarativeCapabilityRow, CreateHarnessRow};

    #[tokio::test]
    async fn malformed_capability_stops_harness_resolution_instead_of_erasing_guardrails() {
        let db = StorageBackend::test_database();
        let org_id = everruns_core::DEFAULT_ORG_ID;
        db.create_declarative_capability(
            org_id,
            CreateDeclarativeCapabilityRow {
                public_id: "cap_01933b5a000070008000000000000001".to_string(),
                name: "broken".to_string(),
                display_name: None,
                description: "Broken stored definition".to_string(),
                definition: serde_json::json!({ "name": "broken" }),
            },
        )
        .await
        .unwrap();
        let harness = db
            .create_harness(
                org_id,
                CreateHarnessRow {
                    name: "secured".to_string(),
                    display_name: None,
                    icon: None,
                    description: None,
                    intro_markdown: None,
                    short_description: None,
                    starters: serde_json::json!([]),
                    system_prompt: None,
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: false,
                },
            )
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![
                ("guardrails".to_string(), 0, serde_json::json!({})),
                ("declarative:broken".to_string(), 1, serde_json::json!({})),
            ],
        )
        .await
        .unwrap();

        let error = resolve_effective_harness(&db, org_id, harness.id.uuid())
            .await
            .expect_err("a malformed capability must stop harness resolution");

        assert!(error.to_string().contains("broken"));
    }
}
