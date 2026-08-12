// Organization initialization: built-in harnesses and default plugin marketplace
//
// Decision: Built-in harnesses are readonly, system-managed, provisioned per-org
// Decision: Seed agents are NOT auto-seeded; they live as examples (GET /v1/agent-examples)
//   and are adopted on demand (POST /v1/agent-examples/{slug}/use). This prevents duplicates.
// Decision: Reconciliation ensures all orgs stay up-to-date with built-in harness definitions
// Decision: Built-in harness identity is the `name` string; UUIDs are assigned by the DB at
//   provisioning time. No UUIDs are hardcoded anywhere (no per-org split-brain).
// Decision: Org settings keep separate default and base harness pointers
//
// Default marketplace seeding (see knowledge/integrations/plugins.md):
// Decision: The default marketplace ("everruns", github source everruns/everruns) is seeded
//   once at org creation and in the 058_backfill_default_marketplace.sql backfill migration.
//   It is NEVER re-seeded on read or reconciliation; a user who deletes it loses it permanently.
//   This ensures "default" means seeded, not privileged.

use crate::storage::{
    StorageBackend,
    models::{CreateHarnessRow, CreatePluginMarketplaceRow, UpdateOrganizationSettings},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use everruns_core::{HarnessId, PluginMarketplaceId};
use everruns_durable::UpdateField;
use everruns_platform::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole,
};
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Org-initialization extension point (EVE-811)
// ============================================================================

/// Context handed to an [`OrgInitializer`] after a new organization and its
/// built-in resources have been provisioned.
///
/// Carries what an embedder needs to provision per-org resources — a managed
/// provider, a default budget, an external tenant record — for the freshly
/// created org. The storage backend is exposed so initializers can write rows
/// directly; credentials or gateway tokens the host holds are supplied by the
/// initializer itself, not by OSS.
pub struct OrgInitContext<'a> {
    /// Storage backend, for writing the initializer's per-org rows.
    pub db: &'a StorageBackend,
    /// The organization that was just created.
    pub org_id: i64,
    /// The user who created the org, when creation was user-driven. `None` for
    /// system-created orgs (e.g. the default org seeded at startup).
    pub created_by: Option<uuid::Uuid>,
}

/// Post-create org-initialization hook for embedder-provisioned resources.
///
/// `HostComposition` decides *what* a platform is made of; `OrgInitializer`
/// runs *when an organization is created*. OSS invokes every registered
/// initializer from the shared org-init routine, after built-in harnesses and
/// the default marketplace are provisioned, so an embedder that must set up
/// per-org resources does not have to intercept each org-creation path itself.
///
/// Registered via
/// [`ServerAppBuilder::org_initializer`](crate::ServerAppBuilder::org_initializer);
/// zero or more may be registered. When none are registered, default OSS
/// behavior is unchanged. See `knowledge/foundations/embedding.md`.
#[async_trait]
pub trait OrgInitializer: Send + Sync {
    /// Provision resources for the newly created org. Returning `Err` is handled
    /// per [`required`](OrgInitializer::required): a required initializer aborts
    /// creation, an optional one is logged and skipped.
    async fn on_org_created(&self, ctx: OrgInitContext<'_>) -> Result<()>;

    /// Whether a failure aborts org creation (default `true`).
    ///
    /// Required initializers make provisioning part of org setup: if the
    /// resource cannot be created, the org is not created either. Optional
    /// initializers (`false`) are best-effort — a failure is logged and org
    /// creation proceeds.
    fn required(&self) -> bool {
        true
    }

    /// Stable name used in logs and diagnostics.
    fn name(&self) -> &str {
        "org-initializer"
    }
}

/// A required [`OrgInitializer`] failed, aborting org creation.
#[derive(Debug)]
pub struct OrgInitializerError {
    /// Name of the initializer that failed.
    pub initializer: String,
    /// The underlying error.
    pub source: anyhow::Error,
}

impl std::fmt::Display for OrgInitializerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "required org initializer `{}` failed: {}",
            self.initializer, self.source
        )
    }
}

impl std::error::Error for OrgInitializerError {}

/// Run every registered [`OrgInitializer`] for a freshly created org.
///
/// Initializers run in registration order. A required initializer that fails
/// short-circuits and returns [`OrgInitializerError`]; an optional one that
/// fails is logged and skipped. Returns `Ok(())` when no initializers are
/// registered — the default OSS case.
pub async fn run_org_initializers(
    initializers: &[Arc<dyn OrgInitializer>],
    db: &StorageBackend,
    org_id: i64,
    created_by: Option<uuid::Uuid>,
) -> std::result::Result<(), OrgInitializerError> {
    for initializer in initializers {
        let ctx = OrgInitContext {
            db,
            org_id,
            created_by,
        };
        match initializer.on_org_created(ctx).await {
            Ok(()) => {
                tracing::info!(
                    org_id,
                    initializer = initializer.name(),
                    "Org initializer completed"
                );
            }
            Err(e) if initializer.required() => {
                return Err(OrgInitializerError {
                    initializer: initializer.name().to_string(),
                    source: e,
                });
            }
            Err(e) => {
                tracing::warn!(
                    org_id,
                    initializer = initializer.name(),
                    error = %e,
                    "Optional org initializer failed (non-fatal)"
                );
            }
        }
    }
    Ok(())
}

// ============================================================================
// Default plugin marketplace seeding
// ============================================================================

/// Name of the default marketplace seeded for every org.
pub const DEFAULT_MARKETPLACE_NAME: &str = "everruns";

/// Seed the default plugin marketplace for a newly created organization.
///
/// Inserts a `plugin_marketplaces` row: name `everruns`, source_type `github`,
/// source `{"repo": "everruns/everruns"}`, status `active`, catalog NULL (unsynced
/// until first sync).
///
/// This is **best-effort-durable but non-fatal**: if the insert fails (e.g. name
/// conflict because the user already created a marketplace named "everruns"), org
/// creation still succeeds — a warning is logged.
///
/// Seeding happens only at org creation (and in the one-time backfill migration);
/// it is never re-seeded lazily on read. A user who deletes the default marketplace
/// does not get it back — deletion is permanent.
pub async fn seed_default_plugin_marketplace(db: &StorageBackend, org_id: i64) {
    let source = serde_json::json!({"repo": "everruns/everruns"});
    let input = CreatePluginMarketplaceRow {
        public_id: PluginMarketplaceId::new().to_string(),
        name: DEFAULT_MARKETPLACE_NAME.to_string(),
        source_type: "github".to_string(),
        source,
    };
    match db.create_plugin_marketplace(org_id, input).await {
        Ok(_) => {
            tracing::info!(
                org_id,
                name = DEFAULT_MARKETPLACE_NAME,
                "Seeded default plugin marketplace"
            );
        }
        Err(e) => {
            // Non-fatal by design: org creation must succeed even if seeding
            // fails. Possible causes include a name conflict (the org already
            // has a marketplace named "everruns") or a storage error; the
            // logged error carries the specifics.
            tracing::warn!(
                org_id,
                name = DEFAULT_MARKETPLACE_NAME,
                error = %e,
                "Failed to seed default plugin marketplace (non-fatal)"
            );
        }
    }
}

pub(crate) fn default_harness_definitions() -> Vec<BuiltInHarnessDefinition> {
    crate::platform::oss_built_in_harnesses()
}

/// Resolve the `base` built-in harness ID for an org at runtime.
///
/// Callers that need a fallback harness (session creation without explicit
/// harness, defensive construction from a DB row with `NULL harness_id`) use
/// this instead of a compile-time constant.
pub async fn base_harness_id(db: &StorageBackend, org_id: i64) -> Result<HarnessId> {
    db.get_harness_by_name(org_id, "base")
        .await?
        .filter(|h| h.is_built_in)
        .map(|h| h.id)
        .with_context(|| format!("base harness not provisioned for org {org_id}"))
}

/// Resolve the `generic` built-in harness ID for an org at runtime.
pub async fn generic_harness_id(db: &StorageBackend, org_id: i64) -> Result<HarnessId> {
    db.get_harness_by_name(org_id, "generic")
        .await?
        .filter(|h| h.is_built_in)
        .map(|h| h.id)
        .with_context(|| format!("generic harness not provisioned for org {org_id}"))
}

/// Initialize built-in harnesses for a specific organization.
///
/// Built-in harnesses are identified by name. IDs are assigned by the DB on
/// first provisioning and never hardcoded. All orgs (including the default
/// org) follow the same path.
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
    let mut result = InitResult::default();

    // Release legacy built-ins that were demoted to example harnesses.
    // We keep the rows (so existing sessions/agents that reference them keep
    // working) but flip `is_built_in` to false so they become editable, regular
    // org-owned harnesses. New orgs are unaffected because no row exists.
    release_legacy_built_ins(db, org_id).await?;

    for harness in harnesses {
        let parent_harness_id = resolve_built_in_parent_id(db, org_id, harnesses, harness)
            .await
            .with_context(|| format!("resolve parent for built-in harness {}", harness.name))?;
        let input = CreateHarnessRow {
            name: harness.name.to_string(),
            display_name: Some(harness.display_name.to_string()),
            description: Some(harness.description.to_string()),
            system_prompt: Some(harness.system_prompt.to_string()),
            parent_harness_id,
            default_model_id: None,
            tags: harness.tags.clone(),
            initial_files: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            is_built_in: true,
            network_access: None,
            embedder_metadata: serde_json::json!({}),
        };

        // Look up by name — every org (including the default org) has the same
        // identity model: the `name` is stable, the UUID is DB-assigned.
        let existing_row = db
            .get_harness_by_name(org_id, &harness.name)
            .await?
            .filter(|h| h.is_built_in);

        if let Some(existing_row) = existing_row {
            match db
                .create_harness_with_id(org_id, existing_row.id, input)
                .await?
            {
                Some(_) => {
                    sync_harness_capabilities(db, existing_row.id.uuid(), &harness.capabilities)
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

    sync_org_harness_settings_with_definitions(db, org_id, harnesses).await?;

    Ok(result)
}

/// Demote any rows for the legacy default built-ins (`coding-container`,
/// `coding-daytona`, `data-analyst`) to regular org-owned harnesses. Idempotent
/// — only flips rows that are still flagged `is_built_in = true`.
async fn release_legacy_built_ins(db: &StorageBackend, org_id: i64) -> Result<()> {
    for name in crate::harnesses::LEGACY_BUILT_IN_NAMES {
        let flipped = db.release_built_in_harness(org_id, name).await?;
        if flipped {
            tracing::info!(
                org_id,
                name,
                "Released legacy built-in harness; row preserved as org-owned"
            );
        }
    }
    Ok(())
}

async fn resolve_built_in_parent_id(
    db: &StorageBackend,
    org_id: i64,
    harnesses: &[BuiltInHarnessDefinition],
    harness: &BuiltInHarnessDefinition,
) -> Result<Option<HarnessId>> {
    let Some(parent_name) = harness.parent_name.as_deref() else {
        return Ok(None);
    };

    let parent = harnesses
        .iter()
        .find(|candidate| candidate.name == parent_name)
        .with_context(|| format!("unknown built-in parent {parent_name}"))?;

    let parent_row = db
        .get_harness_by_name(org_id, &parent.name)
        .await?
        .filter(|candidate| candidate.is_built_in)
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
    let desired_ids: Vec<&str> = desired.iter().map(|c| c.capability_id()).collect();

    if current_ids == desired_ids {
        return Ok(false);
    }

    let cap_tuples: Vec<(String, i32, serde_json::Value)> = desired
        .iter()
        .enumerate()
        .map(|(idx, cap)| {
            (
                cap.capability_id().to_string(),
                idx as i32,
                cap.config_value().clone(),
            )
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

        let generic_id = provisioned_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("generic harness")
            .id;
        let base_id = provisioned_harnesses
            .iter()
            .find(|h| h.name == "base")
            .expect("base harness")
            .id;
        let settings = db
            .get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings.default_harness_id, Some(generic_id));
        assert_eq!(settings.base_harness_id, Some(base_id));

        let chat = provisioned_harnesses
            .iter()
            .find(|h| h.name == "platform-chat")
            .expect("chat harness");
        assert_eq!(chat.parent_harness_id, Some(base_id));

        let chat_caps = db.get_harness_capabilities(chat.id.uuid()).await.unwrap();
        let chat_cap_ids = chat_caps
            .iter()
            .map(|cap| cap.capability_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            chat_cap_ids,
            vec![
                "platform",
                "btw",
                "loop_detection",
                "error_disclosure",
                "compaction"
            ]
        );
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

        let generic_id = h_org2.iter().find(|h| h.name == "generic").unwrap().id;
        let base_id = h_org2.iter().find(|h| h.name == "base").unwrap().id;
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
            .find(|h| h.name == "platform-chat")
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
    async fn test_capabilities_synced() {
        let db = make_db();
        seed_default_org(&db).await;

        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        let provisioned = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        let built_in_harnesses = harnesses();

        let generic_def = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .unwrap();
        let generic_id = provisioned
            .iter()
            .find(|h| h.name == "generic")
            .expect("generic harness")
            .id;
        let caps = db
            .get_harness_capabilities(generic_id.uuid())
            .await
            .unwrap();
        let cap_ids: Vec<&str> = caps.iter().map(|c| c.capability_id.as_str()).collect();
        let expected_ids: Vec<&str> = generic_def
            .capabilities
            .iter()
            .map(|c| c.capability_id())
            .collect();
        assert_eq!(
            cap_ids, expected_ids,
            "Generic harness capabilities should match definition"
        );

        let base_id = provisioned
            .iter()
            .find(|h| h.name == "base")
            .expect("base harness")
            .id;
        let base_caps = db.get_harness_capabilities(base_id.uuid()).await.unwrap();
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

    /// Verify that calling initialize_org_harnesses on a fresh default org
    /// (without full seed_all) creates harnesses and sets org settings.
    /// This simulates the safety-net path in auth registration handlers.
    #[tokio::test]
    async fn test_initialize_without_prior_seed() {
        let db = make_db();
        // Only create the org row — skip full seeding (models, providers, etc.)
        seed_default_org(&db).await;

        // Before init, no harnesses exist
        let before = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        assert!(before.is_empty());

        // Initialize harnesses (same call the registration handlers make)
        let result = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert!(result.created > 0, "should create built-in harnesses");

        // Harnesses exist
        let after = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        assert_eq!(after.len(), harnesses().len());

        // Org settings have default and base harness IDs
        let settings = db
            .get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .expect("org settings should exist");
        assert!(
            settings.default_harness_id.is_some(),
            "default_harness_id should be set"
        );
        assert!(
            settings.base_harness_id.is_some(),
            "base_harness_id should be set"
        );

        // Second call is idempotent
        let result2 = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
        assert_eq!(result2.created, 0, "should be idempotent");
        assert_eq!(result2.unchanged, harnesses().len());
    }

    #[tokio::test]
    async fn test_legacy_built_ins_are_released_on_reconcile() {
        // Simulates an org upgraded from a previous version where
        // `data-analyst` was a default built-in. After reconciliation, the row
        // must remain (so existing references keep working) but with
        // `is_built_in = false` so the user can edit/manage it.
        let db = make_db();
        seed_default_org(&db).await;

        // Provision today's built-ins first so generic/base exist for parents.
        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        // Inject a stale `data-analyst` built-in row directly.
        let row = db
            .create_harness(
                DEFAULT_ORG_ID,
                crate::storage::models::CreateHarnessRow {
                    name: "data-analyst".to_string(),
                    display_name: Some("Data Analyst".to_string()),
                    description: Some("legacy".to_string()),
                    system_prompt: Some("legacy prompt".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: true,
                },
            )
            .await
            .unwrap();
        assert!(row.is_built_in);

        // Run reconciliation again — should release the legacy built-in.
        initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();

        let after = db
            .get_harness_by_name(DEFAULT_ORG_ID, "data-analyst")
            .await
            .unwrap()
            .expect("data-analyst row must still exist after reconcile");
        assert!(
            !after.is_built_in,
            "legacy data-analyst row must no longer be flagged as built-in"
        );
        assert_eq!(after.id, row.id, "row identity preserved");
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
