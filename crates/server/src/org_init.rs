// Organization initialization: built-in harnesses and seed agents
//
// Decision: Built-in harnesses are readonly, system-managed, provisioned per-org
// Decision: Seed agents are provisioned per-org during org creation (same as harnesses)
// Decision: New orgs get built-in harnesses and seed agents during creation
// Decision: Reconciliation ensures all orgs stay up-to-date with built-in definitions
// Decision: Default org uses fixed seed UUIDs for backward compat; other orgs get fresh UUIDs
// Decision: Org settings keep separate default and base harness pointers

use crate::api::common::Pagination;
use crate::seed::{SEED_AGENTS, sync_agent_capabilities};
use crate::storage::{
    StorageBackend,
    models::{CreateAgentRow, CreateHarnessRow, UpdateOrganizationSettings},
};
use anyhow::{Context, Result};
use everruns_core::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, DeploymentGrade,
    PlatformDefinition,
};
use everruns_durable::UpdateField;
use uuid::Uuid;
/// Well-known seed IDs for default org harnesses (backward compat)
mod harness_ids {
    use uuid::Uuid;
    pub const BASE: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000601);
    pub const GENERIC: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000602);
    pub const CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);
}

/// Well-known UUID for the Generic harness (org default harness for new orgs)
pub const GENERIC_HARNESS_ID: Uuid = harness_ids::GENERIC;

/// Well-known UUID for the Base harness (session fallback when no harness is provided)
pub const BASE_HARNESS_ID: Uuid = harness_ids::BASE;

/// Well-known UUID for the Chat harness (used by global chat endpoint)
pub const CHAT_HARNESS_ID: Uuid = harness_ids::CHAT;

pub(crate) fn default_harness_definitions() -> Vec<BuiltInHarnessDefinition> {
    crate::platform::oss_built_in_harnesses()
}

/// Initialize built-in harnesses for a specific organization.
///
/// For `DEFAULT_ORG_ID`, uses fixed seed UUIDs for backward compatibility.
/// For other orgs, generates fresh UUIDs.
///
/// Uses upsert semantics: creates if missing, updates if definition changed.
pub async fn initialize_org_harnesses(db: &StorageBackend, org_id: i64) -> Result<InitResult> {
    initialize_org_harnesses_with_definitions(db, org_id, &default_harness_definitions()).await
}

/// Initialize built-in harnesses using an explicit set of harness definitions.
pub async fn initialize_org_harnesses_with_definitions(
    db: &StorageBackend,
    org_id: i64,
    harnesses: &[BuiltInHarnessDefinition],
) -> Result<InitResult> {
    use everruns_core::DEFAULT_ORG_ID;

    let mut result = InitResult::default();
    let is_default_org = org_id == DEFAULT_ORG_ID;

    for harness in harnesses {
        let parent_harness_id =
            resolve_built_in_parent_id(db, org_id, is_default_org, harnesses, harness)
                .await
                .with_context(|| format!("resolve parent for built-in harness {}", harness.name))?;
        let input = CreateHarnessRow {
            name: harness.name.to_string(),
            description: Some(harness.description.to_string()),
            system_prompt: harness.system_prompt.to_string(),
            parent_harness_id,
            default_model_id: None,
            tags: harness.tags.clone(),
            initial_files: serde_json::json!([]),
            is_built_in: true,
        };

        if is_default_org && let Some(seed_id) = harness.seed_id {
            // Default org: use fixed seed UUIDs for backward compat
            match db
                .create_harness_with_id(org_id, seed_id.into(), input)
                .await?
            {
                Some(row) => {
                    sync_harness_capabilities(db, row.id.uuid(), &harness.capabilities).await?;
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
                        sync_harness_capabilities(db, seed_id, &harness.capabilities).await?;
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
            let existing = db
                .list_harnesses(org_id, Some(&harness.name), false)
                .await?;
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
                        sync_harness_capabilities(
                            db,
                            existing_row.id.uuid(),
                            &harness.capabilities,
                        )
                        .await?;
                        tracing::info!(name = harness.name, org_id, "Updated built-in harness");
                        result.updated += 1;
                    }
                    None => {
                        let caps_changed = sync_harness_capabilities(
                            db,
                            existing_row.id.uuid(),
                            &harness.capabilities,
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
                sync_harness_capabilities(db, row.id.uuid(), &harness.capabilities).await?;
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

    sync_org_harness_settings_with_definitions(db, org_id, harnesses).await?;

    Ok(result)
}

async fn resolve_built_in_parent_id(
    db: &StorageBackend,
    org_id: i64,
    is_default_org: bool,
    harnesses: &[BuiltInHarnessDefinition],
    harness: &BuiltInHarnessDefinition,
) -> Result<Option<everruns_core::HarnessId>> {
    let Some(parent_key) = harness.parent_key.as_deref() else {
        return Ok(None);
    };

    let parent = harnesses
        .iter()
        .find(|candidate| candidate.key == parent_key)
        .with_context(|| format!("unknown built-in parent {parent_key}"))?;

    if is_default_org {
        let parent_seed = parent
            .seed_id
            .with_context(|| format!("missing seed id for built-in parent {parent_key}"))?;
        return Ok(Some(parent_seed.into()));
    }

    let parent_row = db
        .list_harnesses(org_id, Some(&parent.name), true)
        .await?
        .into_iter()
        .find(|candidate| candidate.is_built_in && candidate.name == parent.name)
        .with_context(|| format!("missing built-in parent {} for org {org_id}", parent.name))?;
    Ok(Some(parent_row.id))
}

/// Reconcile built-in harnesses across all organizations.
///
/// Ensures every org has up-to-date built-in harnesses. Called during seeding
/// and can be triggered for upgrades when built-in definitions change.
pub async fn reconcile_built_in_harnesses(db: &StorageBackend) -> Result<InitResult> {
    reconcile_built_in_harnesses_with_definitions(db, &default_harness_definitions()).await
}

/// Reconcile built-in harnesses with an explicit set of harness definitions.
pub async fn reconcile_built_in_harnesses_with_definitions(
    db: &StorageBackend,
    harnesses: &[BuiltInHarnessDefinition],
) -> Result<InitResult> {
    let orgs = db.list_organizations().await?;
    let mut total = InitResult::default();

    for org in &orgs {
        let org_result =
            initialize_org_harnesses_with_definitions(db, org.org_id, harnesses).await?;
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

/// Ensure org settings point at built-in default/base harnesses without overriding user changes.
pub async fn sync_org_harness_settings(db: &StorageBackend, org_id: i64) -> Result<()> {
    sync_org_harness_settings_with_definitions(db, org_id, &default_harness_definitions()).await
}

/// Ensure org settings point at platform-provided default/base harnesses without overriding user changes.
pub async fn sync_org_harness_settings_with_definitions(
    db: &StorageBackend,
    org_id: i64,
    harness_definitions: &[BuiltInHarnessDefinition],
) -> Result<()> {
    let settings = db.get_organization_settings(org_id).await?;
    let needs_default = settings
        .as_ref()
        .and_then(|s| s.default_harness_id)
        .is_none();
    let needs_base = settings.as_ref().and_then(|s| s.base_harness_id).is_none();

    if !needs_default && !needs_base {
        return Ok(());
    }

    let harnesses = db.list_harnesses(org_id, None, false).await?;
    let default_harness_name = harness_definitions
        .iter()
        .find(|h| h.has_role(BuiltInHarnessRole::Default))
        .map(|h| h.name.as_str());
    let base_harness_name = harness_definitions
        .iter()
        .find(|h| h.has_role(BuiltInHarnessRole::Base))
        .map(|h| h.name.as_str());
    let default_harness_id = harnesses
        .iter()
        .find(|h| default_harness_name.is_some_and(|name| h.is_built_in && h.name == name))
        .map(|h| h.id);
    let base_harness_id = harnesses
        .iter()
        .find(|h| base_harness_name.is_some_and(|name| h.is_built_in && h.name == name))
        .map(|h| h.id);

    let default_harness_id = if needs_default {
        let name = default_harness_name.context("missing default harness name during org init")?;
        UpdateField::Set(
            default_harness_id
                .context(format!("missing built-in {name} harness during org init"))?,
        )
    } else {
        UpdateField::Unchanged
    };
    let base_harness_id = if needs_base {
        let name = base_harness_name.context("missing base harness name during org init")?;
        UpdateField::Set(
            base_harness_id.context(format!("missing built-in {name} harness during org init"))?,
        )
    } else {
        UpdateField::Unchanged
    };

    db.patch_organization_settings(
        org_id,
        UpdateOrganizationSettings {
            default_harness_id,
            base_harness_id,
            ..Default::default()
        },
    )
    .await?;

    Ok(())
}

/// Sync capabilities for a harness, only writing if the set actually changed.
/// Returns true if capabilities were updated.
async fn sync_harness_capabilities(
    db: &StorageBackend,
    harness_id: Uuid,
    desired: &[BuiltInCapabilityDefinition],
) -> Result<bool> {
    let current = db.get_harness_capabilities(harness_id).await?;
    let current_ids: Vec<&str> = current.iter().map(|c| c.capability_id.as_str()).collect();
    let desired_ids: Vec<&str> = desired.iter().map(|c| c.id.as_str()).collect();

    if current_ids == desired_ids {
        return Ok(false);
    }

    let cap_tuples: Vec<(String, i32, serde_json::Value)> = desired
        .iter()
        .enumerate()
        .map(|(idx, cap)| (cap.id.clone(), idx as i32, cap.config.clone()))
        .collect();
    db.set_harness_capabilities(harness_id, cap_tuples).await?;
    Ok(true)
}

// ============================================
// Seed Agent Initialization (per-org)
// ============================================

/// Initialize seed agents for a specific organization.
///
/// For `DEFAULT_ORG_ID`, uses fixed seed UUIDs for backward compatibility.
/// For other orgs, creates agents with fresh UUIDs (skips if already exists by name).
///
/// Filters out dev-only agents when deployment grade disallows experimental features,
/// and skips agents whose capabilities are not registered in the platform definition.
pub async fn initialize_org_agents(
    db: &StorageBackend,
    org_id: i64,
    grade: DeploymentGrade,
    platform_definition: &PlatformDefinition,
) -> Result<InitResult> {
    use everruns_core::DEFAULT_ORG_ID;

    let mut result = InitResult::default();
    let is_default_org = org_id == DEFAULT_ORG_ID;
    let include_dev_only = grade.experimental_features_enabled();

    for seed in SEED_AGENTS {
        if seed.dev_only && !include_dev_only {
            tracing::debug!(
                name = seed.name,
                org_id,
                "Skipping dev-only agent (deployment_grade={})",
                grade
            );
            continue;
        }

        let missing_capabilities: Vec<&str> = seed
            .capabilities
            .iter()
            .map(|cap| cap.id)
            .filter(|cap_id| !platform_definition.capability_registry().has(cap_id))
            .collect();
        if !missing_capabilities.is_empty() {
            tracing::debug!(
                name = seed.name,
                org_id,
                missing = ?missing_capabilities,
                "Skipping seed agent — missing capabilities in platform definition"
            );
            continue;
        }

        let input = CreateAgentRow {
            public_id: everruns_core::AgentId::from_uuid(seed.id).to_string(),
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            tags: seed.tags.iter().map(|s| s.to_string()).collect(),
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
        };

        if is_default_org {
            // Default org: use fixed seed UUIDs for backward compat
            match db
                .create_agent_with_id(org_id, seed.id.into(), input)
                .await?
            {
                Some(row) => {
                    sync_agent_capabilities(db, row.id.uuid(), seed.capabilities).await?;
                    if row.created_at == row.updated_at {
                        tracing::info!(name = seed.name, org_id, "Created seed agent");
                        result.created += 1;
                    } else {
                        tracing::info!(name = seed.name, org_id, "Updated seed agent");
                        result.updated += 1;
                    }
                }
                None => {
                    let caps_changed =
                        sync_agent_capabilities(db, seed.id, seed.capabilities).await?;
                    if caps_changed {
                        tracing::info!(name = seed.name, org_id, "Updated seed agent capabilities");
                        result.updated += 1;
                    } else {
                        tracing::debug!(name = seed.name, org_id, "Seed agent up to date");
                        result.unchanged += 1;
                    }
                }
            }
        } else {
            // Non-default org: check if agent already exists by name
            let (existing, _) = db
                .list_agents(org_id, Some(seed.name), false, Pagination::new(0, 1))
                .await?;
            let already_exists = existing.iter().any(|a| a.name == seed.name);

            if already_exists {
                result.unchanged += 1;
            } else {
                let row = db.create_agent(org_id, input).await?;
                sync_agent_capabilities(db, row.id.uuid(), seed.capabilities).await?;
                tracing::info!(
                    name = seed.name,
                    org_id,
                    id = %row.id,
                    "Created seed agent for org"
                );
                result.created += 1;
            }
        }
    }

    Ok(result)
}

/// Reconcile seed agents across all organizations.
///
/// Ensures every org has up-to-date seed agents. Called during seeding
/// and can be triggered for upgrades when seed definitions change.
pub async fn reconcile_built_in_agents(
    db: &StorageBackend,
    grade: DeploymentGrade,
    platform_definition: &PlatformDefinition,
) -> Result<InitResult> {
    let orgs = db.list_organizations().await?;
    let mut total = InitResult::default();

    for org in &orgs {
        let org_result = initialize_org_agents(db, org.org_id, grade, platform_definition).await?;
        tracing::debug!(
            org_id = org.org_id,
            created = org_result.created,
            updated = org_result.updated,
            unchanged = org_result.unchanged,
            "Reconciled seed agents for org"
        );
        total.merge(org_result);
    }

    tracing::info!(
        org_count = orgs.len(),
        created = total.created,
        updated = total.updated,
        unchanged = total.unchanged,
        "Seed agent reconciliation complete"
    );

    Ok(total)
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
    use crate::storage::models::UpdateOrganizationSettings;
    use everruns_core::DEFAULT_ORG_ID;

    fn make_db() -> StorageBackend {
        StorageBackend::in_memory()
    }

    fn harnesses() -> Vec<BuiltInHarnessDefinition> {
        default_harness_definitions()
    }

    #[test]
    fn test_built_in_harness_names_unique() {
        let built_in_harnesses = harnesses();
        let names: Vec<&str> = built_in_harnesses.iter().map(|h| h.name.as_str()).collect();
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
        let built_in_harnesses = harnesses();
        let ids: Vec<Uuid> = built_in_harnesses
            .iter()
            .map(|h| {
                h.seed_id
                    .expect("OSS built-in harness should have a seed id")
            })
            .collect();
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
        assert_eq!(result.created, harnesses().len());

        // All harnesses should be listed
        let provisioned_harnesses = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        assert_eq!(provisioned_harnesses.len(), harnesses().len());

        // All should be marked as built-in
        for h in &provisioned_harnesses {
            assert!(h.is_built_in, "Harness {} should be built-in", h.name);
        }

        let settings = db
            .get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            settings.default_harness_id.unwrap().uuid(),
            GENERIC_HARNESS_ID
        );
        assert_eq!(settings.base_harness_id.unwrap().uuid(), BASE_HARNESS_ID);

        let chat = provisioned_harnesses
            .iter()
            .find(|h| h.name == "Platform Chat")
            .expect("chat harness");
        let generic = provisioned_harnesses
            .iter()
            .find(|h| h.name == "Generic")
            .expect("generic harness");
        assert_eq!(chat.parent_harness_id, Some(generic.id));

        let chat_caps = db.get_harness_capabilities(chat.id.uuid()).await.unwrap();
        let chat_cap_ids = chat_caps
            .iter()
            .map(|cap| cap.capability_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(chat_cap_ids, vec!["platform_management"]);
    }

    #[tokio::test]
    async fn test_initialize_idempotent() {
        let db = make_db();
        seed_default_org(&db).await;

        let r1 = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(r1.created, harnesses().len());

        let r2 = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(r2.created, 0);
        assert_eq!(r2.unchanged, harnesses().len());
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
        assert_eq!(result.created, harnesses().len());

        // Verify harnesses exist for org 2
        let h_org2 = db.list_harnesses(org2.org_id, None, false).await.unwrap();
        assert_eq!(h_org2.len(), harnesses().len());
        for h in &h_org2 {
            assert!(h.is_built_in);
        }

        let generic_id = h_org2.iter().find(|h| h.name == "Generic").unwrap().id;
        let base_id = h_org2.iter().find(|h| h.name == "Base").unwrap().id;
        let settings = db
            .get_organization_settings(org2.org_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings.default_harness_id, Some(generic_id));
        assert_eq!(settings.base_harness_id, Some(base_id));
    }

    #[tokio::test]
    async fn test_initialize_does_not_override_existing_org_harness_settings() {
        let db = make_db();
        seed_default_org(&db).await;

        let org2 = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000002".to_string(),
                name: "Test Org 2".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        initialize_org_harnesses(&db, org2.org_id).await.unwrap();

        let harnesses = db.list_harnesses(org2.org_id, None, false).await.unwrap();
        let chat_id = harnesses
            .iter()
            .find(|h| h.name == "Platform Chat")
            .unwrap()
            .id;

        db.patch_organization_settings(
            org2.org_id,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(chat_id),
                base_harness_id: UpdateField::Set(chat_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        initialize_org_harnesses(&db, org2.org_id).await.unwrap();

        let settings = db
            .get_organization_settings(org2.org_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings.default_harness_id, Some(chat_id));
        assert_eq!(settings.base_harness_id, Some(chat_id));
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
        assert_eq!(result.created, harnesses().len() * 2);

        // Second reconcile should be no-op
        let result2 = reconcile_built_in_harnesses(&db).await.unwrap();
        assert_eq!(result2.created, 0);
        assert_eq!(result2.unchanged, harnesses().len() * 2);

        let _ = org2; // silence unused
    }

    #[tokio::test]
    async fn test_default_org_uses_seed_ids() {
        let db = make_db();
        seed_default_org(&db).await;

        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        // Verify default org harnesses use the fixed seed UUIDs
        for harness_def in harnesses() {
            let seed_id = harness_def
                .seed_id
                .expect("OSS built-in harness should have a seed id");
            let row = db
                .get_harness(DEFAULT_ORG_ID, seed_id.into())
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
        let h_org2 = db.list_harnesses(org2.org_id, None, false).await.unwrap();
        let built_in_harnesses = harnesses();
        let seed_ids: Vec<Uuid> = built_in_harnesses
            .iter()
            .map(|h| {
                h.seed_id
                    .expect("OSS built-in harness should have a seed id")
            })
            .collect();
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
        let built_in_harnesses = harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "Generic")
            .unwrap();
        let caps = db
            .get_harness_capabilities(
                generic
                    .seed_id
                    .expect("OSS built-in harness should have a seed id"),
            )
            .await
            .unwrap();
        let cap_ids: Vec<&str> = caps.iter().map(|c| c.capability_id.as_str()).collect();
        let expected_ids: Vec<&str> = generic.capabilities.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            cap_ids, expected_ids,
            "Generic harness capabilities should match definition"
        );

        // Base harness should have no capabilities
        let base = built_in_harnesses
            .iter()
            .find(|h| h.name == "Base")
            .unwrap();
        let base_caps = db
            .get_harness_capabilities(
                base.seed_id
                    .expect("OSS built-in harness should have a seed id"),
            )
            .await
            .unwrap();
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

    // --- Agent initialization tests ---

    fn platform() -> PlatformDefinition {
        crate::platform::oss_platform_definition()
    }

    #[tokio::test]
    async fn test_initialize_default_org_creates_agents() {
        let db = make_db();
        seed_default_org(&db).await;
        // Harnesses must exist first (agents may reference them indirectly)
        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        let pd = platform();
        let result = initialize_org_agents(&db, DEFAULT_ORG_ID, DeploymentGrade::Dev, &pd)
            .await
            .unwrap();
        assert!(result.created > 0, "Should create at least one agent");

        let (agents, _) = db
            .list_agents(DEFAULT_ORG_ID, None, false, Pagination::new(0, 100))
            .await
            .unwrap();
        assert!(!agents.is_empty(), "Agents should exist after init");
    }

    #[tokio::test]
    async fn test_initialize_agents_idempotent() {
        let db = make_db();
        seed_default_org(&db).await;
        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        let pd = platform();
        let r1 = initialize_org_agents(&db, DEFAULT_ORG_ID, DeploymentGrade::Dev, &pd)
            .await
            .unwrap();
        assert!(r1.created > 0);

        let r2 = initialize_org_agents(&db, DEFAULT_ORG_ID, DeploymentGrade::Dev, &pd)
            .await
            .unwrap();
        assert_eq!(r2.created, 0, "Second run should create nothing");
        assert!(r2.unchanged > 0);
    }

    #[tokio::test]
    async fn test_initialize_agents_new_org() {
        let db = make_db();
        seed_default_org(&db).await;

        let org2 = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000099".to_string(),
                name: "Agent Test Org".to_string(),
                created_by: None,
            })
            .await
            .unwrap();
        initialize_org_harnesses(&db, org2.org_id).await.unwrap();

        let pd = platform();
        let result = initialize_org_agents(&db, org2.org_id, DeploymentGrade::Dev, &pd)
            .await
            .unwrap();
        assert!(result.created > 0, "New org should get seed agents");

        // Second call should be idempotent
        let r2 = initialize_org_agents(&db, org2.org_id, DeploymentGrade::Dev, &pd)
            .await
            .unwrap();
        assert_eq!(r2.created, 0);
    }

    #[tokio::test]
    async fn test_seed_agents_respect_deployment_grade() {
        // Verify the filtering logic itself: dev-only agents exist in SEED_AGENTS
        let dev_only_count = SEED_AGENTS.iter().filter(|a| a.dev_only).count();
        assert!(
            dev_only_count > 0,
            "SEED_AGENTS should contain at least one dev-only agent"
        );

        // Non-dev-only agents count
        let non_dev_count = SEED_AGENTS.iter().filter(|a| !a.dev_only).count();
        assert!(
            non_dev_count > 0,
            "SEED_AGENTS should contain at least one non-dev-only agent"
        );
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
