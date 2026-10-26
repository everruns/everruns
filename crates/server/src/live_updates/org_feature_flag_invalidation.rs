// Keeps the in-memory feature flag overrides exact: LISTEN for the
// `org_feature_flags` trigger and clear the org that changed.
//
// Decision: the cache is live only while this loop is listening. Connecting
// (or losing the connection) clears it, so a notification missed during a
// reconnect cannot leave an old value behind; see
// `storage/backend/org_feature_flag_cache.rs`.

use std::time::Duration;

use tracing::{info, warn};

use crate::storage::{ORG_FEATURE_FLAGS_CHANNEL, OrgFeatureFlagCache};

const RETRY_AFTER: Duration = Duration::from_secs(5);

/// Start the listener for the life of the process.
pub fn spawn(database_url: String, cache: OrgFeatureFlagCache) {
    tokio::spawn(async move {
        loop {
            listen(&database_url, &cache).await;
            cache.set_live(false);
            tokio::time::sleep(RETRY_AFTER).await;
        }
    });
}

async fn listen(database_url: &str, cache: &OrgFeatureFlagCache) {
    let mut listener = match sqlx::postgres::PgListener::connect(database_url).await {
        Ok(listener) => listener,
        Err(error) => {
            warn!(error = %error, "Feature flag listener could not connect; flags read uncached");
            return;
        }
    };
    if let Err(error) = listener.listen(ORG_FEATURE_FLAGS_CHANNEL).await {
        warn!(error = %error, "Feature flag listener could not LISTEN; flags read uncached");
        return;
    }
    cache.set_live(true);
    info!("Feature flag overrides cached; cleared on change");
    loop {
        // `try_recv` reports a lost connection as `None`. Whatever was sent
        // while it was down is gone, so the run ends and the cache with it.
        match listener.try_recv().await {
            Ok(Some(notification)) => match notification.payload().parse::<i64>() {
                Ok(org_id) => cache.invalidate(org_id),
                Err(_) => {
                    warn!(payload = %notification.payload(), "Bad feature flag notification");
                    // Unknown org: clear them all.
                    cache.set_live(true);
                }
            },
            Ok(None) => {
                warn!("Feature flag listener lost its connection; flags read uncached");
                return;
            }
            Err(error) => {
                warn!(error = %error, "Feature flag listener failed; flags read uncached");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::storage::StorageBackend;

    // A write that bypasses this backend (another replica, a migration, psql)
    // reaches the cache through the trigger.
    #[tokio::test]
    async fn a_write_elsewhere_clears_the_cache_through_the_trigger() {
        let db = StorageBackend::test_database();
        let url = db.database().test_database().unwrap().url().to_string();
        let cache = db.org_feature_flag_cache().clone();
        super::spawn(url, cache.clone());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !cache.is_live() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("listener connects");

        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), true)]))
            .await
            .unwrap();
        assert!(db.list_org_feature_flags(1).await.unwrap()["skills"]);

        db.database()
            .replace_org_feature_flags(1, &HashMap::from([("skills".into(), false)]))
            .await
            .unwrap();
        let seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if !db.list_org_feature_flags(1).await.unwrap()["skills"] {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(seen.is_ok(), "the other writer's revocation is read back");
    }
}
