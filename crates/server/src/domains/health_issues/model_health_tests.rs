use super::model_health::*;
use crate::storage::StorageBackend;
use crate::storage::background_work::ConfiguredModelRow;
use everruns_core::DEFAULT_ORG_ID;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

fn row() -> ConfiguredModelRow {
    ConfiguredModelRow {
        org_id: DEFAULT_ORG_ID,
        agent_id: Some(Uuid::now_v7()),
        model_name: "Muse Spark 1.3".into(),
        model_id: "meta/muse-spark-1.3".into(),
        provider_id: Uuid::now_v7(),
        provider_name: "OpenRouter".into(),
        last_synced_at: None,
        enabled: true,
        no_longer_listed: false,
        capabilities: json!(["chat"]),
        provider_metadata: None,
        provider_status: "active".into(),
    }
}

#[test]
fn a_model_is_usable_only_when_the_pickers_would_offer_it() {
    assert_eq!(problem(&row()), None);
    let mut r = row();
    r.enabled = false;
    assert_eq!(problem(&r), Some("disabled"));
    let mut r = row();
    r.no_longer_listed = true;
    assert_eq!(problem(&r), Some("no_longer_listed"));
    let mut r = row();
    r.capabilities = json!(["embeddings"]);
    assert_eq!(problem(&r), Some("not_chat"));
    // A turned-off provider outranks the model's own state.
    let mut r = row();
    r.provider_status = "disabled".into();
    r.enabled = false;
    assert_eq!(problem(&r), Some("provider_inactive"));
}

#[test]
fn the_copy_names_the_agent_model_and_fix() {
    let evidence =
        json!({"reason": "disabled", "model": "Muse Spark 1.3", "provider": "OpenRouter"});
    let (title, body) = copy_for(AGENT_MODEL_UNAVAILABLE, "open", &evidence, Some("Scout"));
    assert_eq!(title, "Agent's model is unavailable");
    assert!(
        body.starts_with("Scout is set to use Muse Spark 1.3 (OpenRouter), which is turned off on the Models page."),
        "{body}"
    );
    assert!(body.contains("Slack"), "{body}");
    let (title, body) = copy_for(ORG_DEFAULT_MODEL_UNAVAILABLE, "open", &evidence, None);
    assert_eq!(title, "Default model is unavailable");
    assert!(
        body.contains("Agents without a model of their own"),
        "{body}"
    );
    let (title, _) = copy_for(AGENT_MODEL_UNAVAILABLE, "resolved", &evidence, None);
    assert_eq!(title, "Agent's model is usable again");
    assert!(super::background_work::owns(AGENT_MODEL_UNAVAILABLE));
    assert!(super::background_work::owns(ORG_DEFAULT_MODEL_UNAVAILABLE));
}

/// A model missing from the provider's list is reported as what was seen
/// (which list, which id), never as "no longer listed", and the fix offered
/// is the sync that tells an old list from a real removal.
#[test]
fn a_missing_model_names_the_list_and_the_id_it_was_looked_up_by() {
    let evidence = json!({
        "reason": "no_longer_listed",
        "model": "GPT-6.1 Sol",
        "model_id": "gpt-6.1-sol",
        "provider": "OpenRouter",
        "synced_at": "2026-10-31T00:13:24Z",
    });
    let (title, body) = copy_for(
        AGENT_MODEL_UNAVAILABLE,
        "open",
        &evidence,
        Some("Company Info"),
    );
    assert_eq!(title, "Agent's model not found at its provider");
    assert_eq!(
        body,
        "Company Info uses GPT-6.1 Sol (OpenRouter), but OpenRouter's model list from Oct 31 does not include gpt-6.1-sol. Sync OpenRouter's models to check again: if OpenRouter still offers it, this clears. If not, choose another model in the agent's settings, since its chats and channels can fail to answer."
    );
    assert!(!body.contains("no longer listed"), "{body}");
    // Issues recorded before the id and date were kept still read.
    let old =
        json!({"reason": "no_longer_listed", "model": "GPT-6.1 Sol", "provider": "OpenRouter"});
    let (title, body) = copy_for(ORG_DEFAULT_MODEL_UNAVAILABLE, "open", &old, None);
    assert_eq!(title, "Default model not found at its provider");
    assert!(
        body.contains("OpenRouter's last model list does not include it."),
        "{body}"
    );
    assert!(body.contains("choose another default model"), "{body}");
}

struct Fixture {
    db: Arc<StorageBackend>,
    agent_public_id: String,
    model: Uuid,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let pool = db.database().pool();
    let provider: Uuid = sqlx::query_scalar(
        "INSERT INTO providers (org_id, name, provider_type, api_key_set, status, last_synced_at)
         VALUES ($1, $2, 'openrouter', TRUE, 'active', NOW()) RETURNING id",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(format!("Model health {}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();
    let model: Uuid = sqlx::query_scalar(
        "INSERT INTO models (org_id, provider_id, model_id, display_name, capabilities, enabled, source, last_seen_at)
         VALUES ($1, $2, 'meta/muse-spark-1.3', 'Muse Spark 1.3', '[\"chat\"]', TRUE, 'discovered', NOW())
         RETURNING id",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(provider)
    .fetch_one(pool)
    .await
    .unwrap();
    let agent = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
    let agent_public_id: String = sqlx::query_scalar(
        "UPDATE agents SET default_model_id = $1 WHERE id = $2 RETURNING public_id",
    )
    .bind(model)
    .bind(agent)
    .fetch_one(pool)
    .await
    .unwrap();
    Fixture {
        db,
        agent_public_id,
        model,
    }
}

impl Fixture {
    async fn agent_issue_open(&self) -> bool {
        self.db
            .open_subject_health_issues(AGENT_MODEL_UNAVAILABLE)
            .await
            .unwrap()
            .iter()
            .any(|row| row.subject_id.as_deref() == Some(self.agent_public_id.as_str()))
    }

    async fn set(&self, sql: &'static str) {
        sqlx::query(sql)
            .bind(self.model)
            .execute(self.db.database().pool())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn an_agents_unusable_model_opens_an_issue_that_resolves_once_fixed() {
    let f = fixture().await;
    sweep(&f.db).await.unwrap();
    assert!(!f.agent_issue_open().await);

    // The provider's last sync no longer listed the model.
    f.set("UPDATE models SET last_seen_at = NOW() - INTERVAL '1 day' WHERE id = $1")
        .await;
    sweep(&f.db).await.unwrap();
    assert!(f.agent_issue_open().await);

    // A later sync lists it again: rechecking the org resolves the issue.
    f.set("UPDATE models SET last_seen_at = NOW() + INTERVAL '1 minute' WHERE id = $1")
        .await;
    check_org(&f.db, DEFAULT_ORG_ID).await;
    assert!(!f.agent_issue_open().await);

    // Turning the model off opens it again.
    f.set("UPDATE models SET enabled = FALSE WHERE id = $1")
        .await;
    check_org(&f.db, DEFAULT_ORG_ID).await;
    assert!(f.agent_issue_open().await);
}

/// A model missing from its provider's list offers that provider's model
/// sync, and says why it cannot run when the provider has no key.
#[tokio::test]
async fn a_missing_model_offers_its_providers_model_sync() {
    use super::fixers::{MODELS_SYNC, fixers_for};
    use crate::domains::common::Ctx;
    use crate::domains::models::ModelSyncService;
    use crate::kernel_imports::{Caller, OrgRole};

    let f = fixture().await;
    f.set("UPDATE models SET last_seen_at = NOW() - INTERVAL '1 day' WHERE id = $1")
        .await;
    sweep(&f.db).await.unwrap();
    let row =
        f.db.open_subject_health_issues(AGENT_MODEL_UNAVAILABLE)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.subject_id.as_deref() == Some(f.agent_public_id.as_str()))
            .expect("issue open");
    let evidence = row.evidence.clone().unwrap();
    assert_eq!(evidence["model_id"], "meta/muse-spark-1.3");
    assert!(evidence["synced_at"].is_string(), "{evidence}");

    let caller = Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: format!("org_{DEFAULT_ORG_ID:032}"),
        user_id: None,
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    };
    let mut ctx = Ctx::new(
        caller,
        f.db.clone(),
        Arc::new(crate::services::CapabilityService::new(f.db.clone(), None)),
        None,
        Arc::new(everruns_core::DefaultPermissionResolver),
    );
    ctx.model_sync_service = Some(Arc::new(ModelSyncService::new(
        f.db.clone(),
        ctx.driver_registry.clone(),
        None,
    )));
    let fixers = fixers_for(&ctx, &row).await.unwrap();
    let sync = fixers
        .iter()
        .find(|fixer| fixer.kind == MODELS_SYNC)
        .expect("sync offered");
    assert!(
        sync.label.starts_with("Sync Model health"),
        "{}",
        sync.label
    );
    assert_eq!(sync.unavailable_reason, None);

    // Without a key the list cannot be read: the fix says so.
    sqlx::query("UPDATE providers SET api_key_set = FALSE")
        .execute(f.db.database().pool())
        .await
        .unwrap();
    let fixers = fixers_for(&ctx, &row).await.unwrap();
    let reason = fixers[0].unavailable_reason.as_deref().unwrap_or_default();
    assert!(reason.contains("has no API key"), "{reason}");
}
