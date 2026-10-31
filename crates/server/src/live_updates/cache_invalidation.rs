// Keeps the in-memory feature flag overrides and agent/harness configuration
// rows exact: LISTEN for their triggers and drop what changed.
//
// Decision: the caches are live only while this loop is listening. Connecting
// (or losing the connection) clears them, so a notification missed during a
// reconnect cannot leave an old value behind; see
// `storage/backend/org_feature_flag_cache.rs` and `config_row_cache.rs`.

use std::time::Duration;

use tracing::{info, warn};

use crate::storage::{
    CONFIG_ROWS_CHANNEL, ConfigRowCache, ConfigRowChange, ORG_FEATURE_FLAGS_CHANNEL,
    OrgFeatureFlagCache,
};

const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The caches one listener keeps exact.
#[derive(Clone)]
pub struct Caches {
    pub flags: OrgFeatureFlagCache,
    pub config_rows: ConfigRowCache,
}

impl Caches {
    fn set_live(&self, live: bool) {
        self.flags.set_live(live);
        self.config_rows.set_live(live);
    }
}

/// Start the listener for the life of the process.
pub fn spawn(database_url: String, caches: Caches) {
    tokio::spawn(async move {
        loop {
            listen(&database_url, &caches).await;
            caches.set_live(false);
            tokio::time::sleep(RETRY_AFTER).await;
        }
    });
}

async fn listen(database_url: &str, caches: &Caches) {
    let mut listener = match sqlx::postgres::PgListener::connect(database_url).await {
        Ok(listener) => listener,
        Err(error) => {
            warn!(error = %error, "Cache listener could not connect; reads go to the tables");
            return;
        }
    };
    if let Err(error) = listener
        .listen_all([ORG_FEATURE_FLAGS_CHANNEL, CONFIG_ROWS_CHANNEL])
        .await
    {
        warn!(error = %error, "Cache listener could not LISTEN; reads go to the tables");
        return;
    }
    caches.set_live(true);
    info!("Feature flags and agent/harness rows cached; cleared on change");
    loop {
        // `try_recv` reports a lost connection as `None`. Whatever was sent
        // while it was down is gone, so the run ends and the caches with it.
        match listener.try_recv().await {
            Ok(Some(notification)) => apply(caches, notification.channel(), notification.payload()),
            Ok(None) => {
                warn!("Cache listener lost its connection; reads go to the tables");
                return;
            }
            Err(error) => {
                warn!(error = %error, "Cache listener failed; reads go to the tables");
                return;
            }
        }
    }
}

fn apply(caches: &Caches, channel: &str, payload: &str) {
    if channel == ORG_FEATURE_FLAGS_CHANNEL {
        match payload.parse::<i64>() {
            Ok(org_id) => caches.flags.invalidate(org_id),
            Err(_) => {
                warn!(payload, "Bad feature flag notification");
                // Unknown org: clear them all.
                caches.flags.set_live(true);
            }
        }
    } else if channel == CONFIG_ROWS_CHANNEL {
        match ConfigRowChange::parse(payload) {
            Some(change) => caches.config_rows.invalidate(change),
            None => {
                warn!(payload, "Bad configuration row notification");
                caches.config_rows.set_live(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Caches;
    use crate::storage::{CreateHarnessRow, StorageBackend};

    async fn listening(db: &StorageBackend) {
        let url = db.database().test_database().unwrap().url().to_string();
        let caches = Caches {
            flags: db.org_feature_flag_cache().clone(),
            config_rows: db.config_row_cache().clone(),
        };
        super::spawn(url, caches.clone());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !caches.flags.is_live() || !caches.config_rows.is_live() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("listener connects");
    }

    async fn eventually<F, Fut>(mut check: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !check().await {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok()
    }

    // A write that bypasses this backend (another replica, a migration, psql)
    // reaches the cache through the trigger.
    #[tokio::test]
    async fn a_write_elsewhere_clears_the_cache_through_the_trigger() {
        let db = StorageBackend::test_database();
        listening(&db).await;

        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), true)]))
            .await
            .unwrap();
        assert!(db.list_org_feature_flags(1).await.unwrap()["skills"]);

        db.database()
            .replace_org_feature_flags(1, &HashMap::from([("skills".into(), false)]))
            .await
            .unwrap();
        let seen =
            eventually(|| async { !db.list_org_feature_flags(1).await.unwrap()["skills"] }).await;
        assert!(seen, "the other writer's revocation is read back");
    }

    #[tokio::test]
    async fn a_harness_or_capability_change_is_read_back() {
        let db = StorageBackend::test_database();
        listening(&db).await;
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let harness = db
            .create_harness(org_id, harness_row("cached"))
            .await
            .unwrap();
        let read = db.get_harness(org_id, harness.id).await.unwrap().unwrap();
        assert_eq!(read.system_prompt.as_deref(), Some("before"));
        assert!(
            db.get_harness_capabilities(harness.id.uuid())
                .await
                .unwrap()
                .is_empty()
        );

        // Another org never sees the row, cached or not.
        assert!(
            db.get_harness(org_id + 1, harness.id)
                .await
                .unwrap()
                .is_none()
        );

        sqlx::query("UPDATE harnesses SET system_prompt = 'after' WHERE id = $1")
            .bind(harness.id.uuid())
            .execute(db.pool())
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![("guardrails".to_string(), 0, serde_json::json!({}))],
        )
        .await
        .unwrap();

        let seen = eventually(|| async {
            let row = db.get_harness(org_id, harness.id).await.unwrap().unwrap();
            let caps = db
                .get_harness_capabilities(harness.id.uuid())
                .await
                .unwrap();
            row.system_prompt.as_deref() == Some("after") && caps.len() == 1
        })
        .await;
        assert!(
            seen,
            "the edited harness and its capabilities are read back"
        );
    }

    #[tokio::test]
    async fn a_provider_edit_is_read_back() {
        let db = StorageBackend::test_database();
        listening(&db).await;
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let provider = db
            .create_provider(
                org_id,
                crate::storage::CreateProviderRow {
                    name: format!("cached-{}", uuid::Uuid::now_v7()),
                    provider_type: "llmsim".into(),
                    base_url: Some("http://before".into()),
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        let id = provider.id.uuid();
        let read = db.get_provider_config(org_id, id).await.unwrap().unwrap();
        assert_eq!(read.base_url.as_deref(), Some("http://before"));
        assert!(
            db.get_provider_config(org_id + 1, id)
                .await
                .unwrap()
                .is_none()
        );

        sqlx::query("UPDATE providers SET base_url = 'http://after' WHERE id = $1")
            .bind(id)
            .execute(db.pool())
            .await
            .unwrap();
        let seen = eventually(|| async {
            let row = db.get_provider_config(org_id, id).await.unwrap().unwrap();
            row.base_url.as_deref() == Some("http://after")
        })
        .await;
        assert!(seen, "the edited provider is read back");
    }

    // Usage counters move on every model call; they must not churn the cache,
    // while a configuration edit of the same row does notify.
    #[tokio::test]
    async fn agent_usage_counters_do_not_notify_but_edits_do() {
        let db = StorageBackend::test_database();
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let harness = db
            .create_harness(org_id, harness_row("agent-host"))
            .await
            .unwrap();
        let agent_id = everruns_contracts::typed_id::AgentId::new();
        db.create_agent_with_id(
            org_id,
            agent_id,
            crate::platform_chat_agent::definition(harness.id, agent_id),
        )
        .await
        .unwrap()
        .unwrap();
        let url = db.database().test_database().unwrap().url().to_string();
        let mut listener = sqlx::postgres::PgListener::connect(&url).await.unwrap();
        listener
            .listen(crate::storage::CONFIG_ROWS_CHANNEL)
            .await
            .unwrap();

        db.increment_agent_usage(agent_id.uuid(), 10, 5, 0, 0, 0.0, 0.0, 0.0)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), listener.recv())
                .await
                .is_err(),
            "a usage counter write sends no notification"
        );

        sqlx::query("UPDATE agents SET description = 'edited' WHERE id = $1")
            .bind(agent_id.uuid())
            .execute(db.pool())
            .await
            .unwrap();
        let edit = tokio::time::timeout(std::time::Duration::from_secs(10), listener.recv())
            .await
            .expect("the edit notifies")
            .unwrap();
        assert_eq!(edit.payload(), format!("agent:{}", agent_id.uuid()));
    }

    fn harness_row(name: &str) -> CreateHarnessRow {
        CreateHarnessRow {
            localizations: Default::default(),
            name: name.to_string(),
            display_name: None,
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: Some("before".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            initial_files: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            embedder_metadata: serde_json::json!({}),
            is_built_in: false,
        }
    }
}
