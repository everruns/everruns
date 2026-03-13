// Organization initialization and built-in harness management
//
// Decision: Built-in harnesses are readonly, system-managed, provisioned per-org
// Decision: New orgs get built-in harnesses during creation
// Decision: Reconciliation ensures all orgs stay up-to-date with built-in definitions
// Decision: Default org uses fixed seed UUIDs for backward compat; other orgs get fresh UUIDs

use crate::storage::{StorageBackend, models::CreateHarnessRow};
use anyhow::Result;
use uuid::Uuid;

/// Capability entry with optional per-capability config for built-in harnesses.
pub struct BuiltInCapability {
    pub id: &'static str,
    pub config: Option<fn() -> serde_json::Value>,
}

impl BuiltInCapability {
    const fn new(id: &'static str) -> Self {
        Self { id, config: None }
    }

    const fn with_config(id: &'static str, config: fn() -> serde_json::Value) -> Self {
        Self {
            id,
            config: Some(config),
        }
    }
}

impl std::fmt::Display for BuiltInCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

/// Built-in harness definition (static, system-managed)
pub struct BuiltInHarness {
    /// Fixed UUID used only for the default org (backward compat)
    pub seed_id: Uuid,
    pub name: &'static str,
    pub description: &'static str,
    pub system_prompt: &'static str,
    pub tags: &'static [&'static str],
    pub capabilities: &'static [BuiltInCapability],
}

/// Well-known seed IDs for default org harnesses (backward compat)
mod harness_ids {
    use uuid::Uuid;
    pub const BASE: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000601);
    pub const GENERIC: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000602);
    pub const CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);
}

/// Well-known UUID for the Generic harness (default for sessions without explicit harness)
pub const GENERIC_HARNESS_ID: Uuid = harness_ids::GENERIC;

/// Well-known UUID for the Chat harness (used by global chat endpoint)
pub const CHAT_HARNESS_ID: Uuid = harness_ids::CHAT;

/// Built-in harnesses provisioned for every organization
pub const BUILT_IN_HARNESSES: &[BuiltInHarness] = &[
    BuiltInHarness {
        seed_id: harness_ids::BASE,
        name: "Base",
        description: "Empty harness with no capabilities. Provides a blank canvas for custom configurations.",
        system_prompt: "You are a helpful assistant.",
        tags: &["base", "built-in"],
        capabilities: &[] as &[BuiltInCapability],
    },
    BuiltInHarness {
        seed_id: harness_ids::GENERIC,
        name: "Generic",
        description: "General-purpose harness with file system, bash, web fetch, secrets, session management, long-context support, and agent skills. Recommended default for most use cases.",
        system_prompt: "You are a helpful assistant.",
        tags: &["generic", "default", "built-in"],
        capabilities: &[
            BuiltInCapability::new("session_file_system"),
            BuiltInCapability::new("virtual_bash"),
            BuiltInCapability::with_config(
                "web_fetch",
                || serde_json::json!({"enable_file_download": true}),
            ),
            BuiltInCapability::new("session_storage"),
            BuiltInCapability::new("session"),
            BuiltInCapability::new("agent_instructions"),
            BuiltInCapability::new("skills"),
            BuiltInCapability::new("infinity_context"),
            BuiltInCapability::new("openai_tool_search"),
        ],
    },
    BuiltInHarness {
        seed_id: harness_ids::CHAT,
        name: "Platform Chat",
        description: "Conversational harness for the global chat interface with platform management capabilities.",
        system_prompt: "You are a helpful assistant on the Everruns platform.\n\nCapabilities are the primary way to extend agent functionality. Use `list_capabilities` to discover available capabilities (built-in, MCP servers, and skills), then assign them when creating agents or harnesses.\n\nWhen creating agents, always use `list_capabilities` first to find relevant capability IDs to include.\n\n## Running agents\n\nWhen asked to \"run an agent\" or \"run X with agent Y\", follow these steps:\n1. Create a session for the agent (use `manage_sessions` with operation \"create\")\n2. Send the user's message/task to the session (use `session_interact` with operation \"send_message\")\n3. Wait for the turn to complete (use `session_interact` with operation \"wait_for_idle\")\n4. Retrieve and relay the results (use `session_interact` with operation \"get_messages\")\n\n## Harness creation\n\nAvoid creating new harnesses unless the user explicitly needs a custom one. For most tasks, use the built-in \"Generic\" harness (find it via `manage_harnesses` with operation \"list\") which already includes file system, bash, storage, long-context support, session, agent instructions, and skills capabilities.\n\n## Confirmation guidelines\n\n- **Always confirm** before creating a harness or agent — these are reusable org-wide entities.\n- **Sessions**: Use common sense. Routine requests (\"run agent X on this task\") can proceed without confirmation. Unusual or high-impact requests (destructive operations, large-scale actions, unclear intent) should be confirmed first.",
        tags: &["chat", "built-in"],
        capabilities: &[
            BuiltInCapability::new("session_file_system"),
            BuiltInCapability::new("virtual_bash"),
            BuiltInCapability::with_config(
                "web_fetch",
                || serde_json::json!({"enable_file_download": true}),
            ),
            BuiltInCapability::new("session_storage"),
            BuiltInCapability::new("session"),
            BuiltInCapability::new("agent_instructions"),
            BuiltInCapability::new("skills"),
            BuiltInCapability::new("infinity_context"),
            BuiltInCapability::new("platform_management"),
            BuiltInCapability::new("openai_tool_search"),
        ],
    },
];

/// Initialize built-in harnesses for a specific organization.
///
/// For `DEFAULT_ORG_ID`, uses fixed seed UUIDs for backward compatibility.
/// For other orgs, generates fresh UUIDs.
///
/// Uses upsert semantics: creates if missing, updates if definition changed.
pub async fn initialize_org_harnesses(db: &StorageBackend, org_id: i64) -> Result<InitResult> {
    use everruns_core::DEFAULT_ORG_ID;

    let mut result = InitResult::default();
    let is_default_org = org_id == DEFAULT_ORG_ID;

    for harness in BUILT_IN_HARNESSES {
        let input = CreateHarnessRow {
            name: harness.name.to_string(),
            description: Some(harness.description.to_string()),
            system_prompt: harness.system_prompt.to_string(),
            default_model_id: None,
            tags: harness.tags.iter().map(|s| s.to_string()).collect(),
            is_built_in: true,
        };

        if is_default_org {
            // Default org: use fixed seed UUIDs for backward compat
            match db
                .create_harness_with_id(org_id, harness.seed_id.into(), input)
                .await?
            {
                Some(row) => {
                    sync_harness_capabilities(db, row.id.uuid(), harness.capabilities).await?;
                    if row.created_at == row.updated_at {
                        tracing::info!(name = harness.name, org_id, "Created built-in harness");
                        result.created += 1;
                    } else {
                        tracing::info!(name = harness.name, org_id, "Updated built-in harness");
                        result.updated += 1;
                    }
                }
                None => {
                    let caps_changed =
                        sync_harness_capabilities(db, harness.seed_id, harness.capabilities)
                            .await?;
                    if caps_changed {
                        tracing::info!(
                            name = harness.name,
                            org_id,
                            "Updated built-in harness capabilities"
                        );
                        result.updated += 1;
                    } else {
                        tracing::debug!(name = harness.name, org_id, "Built-in harness up to date");
                        result.unchanged += 1;
                    }
                }
            }
        } else {
            // Non-default org: check if built-in harness already exists by name
            let existing = db.list_harnesses(org_id, Some(harness.name)).await?;
            let already_exists = existing
                .iter()
                .any(|h| h.name == harness.name && h.is_built_in);

            if already_exists {
                // Already provisioned — update definition if changed
                let existing_row = existing
                    .iter()
                    .find(|h| h.name == harness.name && h.is_built_in)
                    .unwrap();

                match db
                    .create_harness_with_id(org_id, existing_row.id, input)
                    .await?
                {
                    Some(_) => {
                        sync_harness_capabilities(db, existing_row.id.uuid(), harness.capabilities)
                            .await?;
                        tracing::info!(name = harness.name, org_id, "Updated built-in harness");
                        result.updated += 1;
                    }
                    None => {
                        let caps_changed = sync_harness_capabilities(
                            db,
                            existing_row.id.uuid(),
                            harness.capabilities,
                        )
                        .await?;
                        if caps_changed {
                            result.updated += 1;
                        } else {
                            result.unchanged += 1;
                        }
                    }
                }
            } else {
                // Fresh org — create with new UUID
                let row = db.create_harness(org_id, input).await?;
                sync_harness_capabilities(db, row.id.uuid(), harness.capabilities).await?;
                tracing::info!(
                    name = harness.name,
                    org_id,
                    id = %row.id,
                    "Created built-in harness"
                );
                result.created += 1;
            }
        }
    }

    Ok(result)
}

/// Reconcile built-in harnesses across all organizations.
///
/// Ensures every org has up-to-date built-in harnesses. Called during seeding
/// and can be triggered for upgrades when built-in definitions change.
pub async fn reconcile_built_in_harnesses(db: &StorageBackend) -> Result<InitResult> {
    let orgs = db.list_organizations().await?;
    let mut total = InitResult::default();

    for org in &orgs {
        let org_result = initialize_org_harnesses(db, org.org_id).await?;
        tracing::debug!(
            org_id = org.org_id,
            created = org_result.created,
            updated = org_result.updated,
            unchanged = org_result.unchanged,
            "Reconciled built-in harnesses for org"
        );
        total.merge(org_result);
    }

    tracing::info!(
        org_count = orgs.len(),
        created = total.created,
        updated = total.updated,
        unchanged = total.unchanged,
        "Built-in harness reconciliation complete"
    );

    Ok(total)
}

/// Sync capabilities for a harness, only writing if the set actually changed.
/// Returns true if capabilities were updated.
async fn sync_harness_capabilities(
    db: &StorageBackend,
    harness_id: Uuid,
    desired: &[BuiltInCapability],
) -> Result<bool> {
    let current = db.get_harness_capabilities(harness_id).await?;
    let current_ids: Vec<&str> = current.iter().map(|c| c.capability_id.as_str()).collect();
    let desired_ids: Vec<&str> = desired.iter().map(|c| c.id).collect();

    if current_ids == desired_ids {
        return Ok(false);
    }

    let cap_tuples: Vec<(String, i32, serde_json::Value)> = desired
        .iter()
        .enumerate()
        .map(|(idx, cap)| {
            let config = cap.config.map_or_else(|| serde_json::json!({}), |f| f());
            (cap.id.to_string(), idx as i32, config)
        })
        .collect();
    db.set_harness_capabilities(harness_id, cap_tuples).await?;
    Ok(true)
}

/// Result of org initialization
#[derive(Debug, Default)]
pub struct InitResult {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
}

impl InitResult {
    pub fn merge(&mut self, other: InitResult) {
        self.created += other.created;
        self.updated += other.updated;
        self.unchanged += other.unchanged;
    }

    pub fn has_changes(&self) -> bool {
        self.created > 0 || self.updated > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use everruns_core::DEFAULT_ORG_ID;

    fn make_db() -> StorageBackend {
        StorageBackend::in_memory()
    }

    #[test]
    fn test_built_in_harness_names_unique() {
        let names: Vec<&str> = BUILT_IN_HARNESSES.iter().map(|h| h.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            names.len(),
            unique.len(),
            "Duplicate built-in harness names"
        );
    }

    #[test]
    fn test_built_in_harness_seed_ids_unique() {
        let ids: Vec<Uuid> = BUILT_IN_HARNESSES.iter().map(|h| h.seed_id).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            ids.len(),
            unique.len(),
            "Duplicate built-in harness seed IDs"
        );
    }

    #[tokio::test]
    async fn test_initialize_default_org_creates_harnesses() {
        let db = make_db();
        seed_default_org(&db).await;

        let result = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(result.created, BUILT_IN_HARNESSES.len());

        // All harnesses should be listed
        let harnesses = db.list_harnesses(DEFAULT_ORG_ID, None).await.unwrap();
        assert_eq!(harnesses.len(), BUILT_IN_HARNESSES.len());

        // All should be marked as built-in
        for h in &harnesses {
            assert!(h.is_built_in, "Harness {} should be built-in", h.name);
        }
    }

    #[tokio::test]
    async fn test_initialize_idempotent() {
        let db = make_db();
        seed_default_org(&db).await;

        let r1 = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(r1.created, BUILT_IN_HARNESSES.len());

        let r2 = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(r2.created, 0);
        assert_eq!(r2.unchanged, BUILT_IN_HARNESSES.len());
    }

    #[tokio::test]
    async fn test_initialize_new_org() {
        let db = make_db();
        seed_default_org(&db).await;

        // Create a second org
        let org2 = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
                name: "Test Org 2".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        let result = initialize_org_harnesses(&db, org2.org_id).await.unwrap();
        assert_eq!(result.created, BUILT_IN_HARNESSES.len());

        // Verify harnesses exist for org 2
        let h_org2 = db.list_harnesses(org2.org_id, None).await.unwrap();
        assert_eq!(h_org2.len(), BUILT_IN_HARNESSES.len());
        for h in &h_org2 {
            assert!(h.is_built_in);
        }
    }

    #[tokio::test]
    async fn test_reconcile_across_orgs() {
        let db = make_db();
        seed_default_org(&db).await;

        // Create second org
        let org2 = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
                name: "Test Org 2".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        let result = reconcile_built_in_harnesses(&db).await.unwrap();
        // Should create harnesses for both orgs
        assert_eq!(result.created, BUILT_IN_HARNESSES.len() * 2);

        // Second reconcile should be no-op
        let result2 = reconcile_built_in_harnesses(&db).await.unwrap();
        assert_eq!(result2.created, 0);
        assert_eq!(result2.unchanged, BUILT_IN_HARNESSES.len() * 2);

        let _ = org2; // silence unused
    }

    #[tokio::test]
    async fn test_default_org_uses_seed_ids() {
        let db = make_db();
        seed_default_org(&db).await;

        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        // Verify default org harnesses use the fixed seed UUIDs
        for harness_def in BUILT_IN_HARNESSES {
            let row = db
                .get_harness(DEFAULT_ORG_ID, harness_def.seed_id.into())
                .await
                .unwrap();
            assert!(
                row.is_some(),
                "Default org should have harness with seed ID for {}",
                harness_def.name
            );
            assert_eq!(row.unwrap().name, harness_def.name);
        }
    }

    #[tokio::test]
    async fn test_new_org_gets_different_ids() {
        let db = make_db();
        seed_default_org(&db).await;

        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        let org2 = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
                name: "Test Org 2".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        initialize_org_harnesses(&db, org2.org_id).await.unwrap();

        // Non-default org should NOT use seed UUIDs
        let h_org2 = db.list_harnesses(org2.org_id, None).await.unwrap();
        let seed_ids: Vec<Uuid> = BUILT_IN_HARNESSES.iter().map(|h| h.seed_id).collect();
        for h in &h_org2 {
            assert!(
                !seed_ids.contains(&h.id.uuid()),
                "Non-default org harness {} should not use seed UUID",
                h.name
            );
        }
    }

    #[tokio::test]
    async fn test_capabilities_synced() {
        let db = make_db();
        seed_default_org(&db).await;

        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        // Check that Generic harness has the expected capabilities
        let generic = BUILT_IN_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .unwrap();
        let caps = db.get_harness_capabilities(generic.seed_id).await.unwrap();
        let cap_ids: Vec<&str> = caps.iter().map(|c| c.capability_id.as_str()).collect();
        let expected_ids: Vec<&str> = generic.capabilities.iter().map(|c| c.id).collect();
        assert_eq!(
            cap_ids, expected_ids,
            "Generic harness capabilities should match definition"
        );

        // Base harness should have no capabilities
        let base = BUILT_IN_HARNESSES
            .iter()
            .find(|h| h.name == "Base")
            .unwrap();
        let base_caps = db.get_harness_capabilities(base.seed_id).await.unwrap();
        assert!(
            base_caps.is_empty(),
            "Base harness should have no capabilities"
        );
    }

    #[tokio::test]
    async fn test_init_result_has_changes() {
        let result = InitResult {
            created: 1,
            updated: 0,
            unchanged: 0,
        };
        assert!(result.has_changes());

        let result2 = InitResult {
            created: 0,
            updated: 1,
            unchanged: 0,
        };
        assert!(result2.has_changes());

        let result3 = InitResult {
            created: 0,
            updated: 0,
            unchanged: 3,
        };
        assert!(!result3.has_changes());
    }

    async fn seed_default_org(db: &StorageBackend) {
        use crate::storage::models::CreateOrganizationRow;
        use everruns_core::DEFAULT_ORG_PUBLIC_ID;

        let _ = db
            .create_organization_with_id(
                DEFAULT_ORG_ID,
                CreateOrganizationRow {
                    public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                    name: "Default Organization".to_string(),
                    created_by: None,
                },
            )
            .await;
    }
}
