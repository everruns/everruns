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
        provider_name: "OpenRouter".into(),
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
        json!({"reason": "no_longer_listed", "model": "Muse Spark 1.3", "provider": "OpenRouter"});
    let (title, body) = copy_for(AGENT_MODEL_UNAVAILABLE, "open", &evidence, Some("Scout"));
    assert_eq!(title, "Agent's model is unavailable");
    assert!(
        body.starts_with("Scout is set to use Muse Spark 1.3 (OpenRouter), which is no longer listed by OpenRouter."),
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
