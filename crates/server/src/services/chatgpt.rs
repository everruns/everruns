//! Personal ChatGPT connections. Runtime ownership and token rotation are control-plane effects.
use crate::kernel_imports::Caller;
use crate::storage::{
    EncryptionService, StorageBackend,
    models::{ProviderRow, UpdateProvider},
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use everruns_contracts::typed_id::ProviderId;
use everruns_drivers::chatgpt::{
    CodexAuth,
    auth::{RotatingAuth, TokenRoute, TokenStore},
    oauth,
};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, OwnedMutexGuard};

pub fn visible(settings: &Value, caller: &Caller) -> bool {
    match settings
        .pointer("/chatgpt/owner_user_id")
        .and_then(Value::as_str)
    {
        Some(owner) => caller.user_id.is_some_and(|u| u.to_string() == owner),
        None => settings.get("chatgpt").is_none(),
    }
}
pub async fn require_enabled(db: &StorageBackend, org: i64) -> Result<()> {
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        db,
        org,
        &crate::records::FeatureFlagPolicy::current(),
    )
    .await?;
    anyhow::ensure!(
        flags.chatgpt_plan,
        "ChatGPT plan connections are disabled for this organization."
    );
    Ok(())
}
pub async fn initial_settings(db: Arc<StorageBackend>, caller: &Caller) -> Result<Value> {
    require_enabled(&db, caller.org_id).await?;
    let user = caller
        .user_id
        .ok_or_else(|| anyhow!("ChatGPT sign-in requires a user account"))?;
    let principal = crate::services::PrincipalService::new(db)
        .default_runtime_owner_principal(caller, None)
        .await?;
    // Bind the grant to the user's current personal runtime identity, never to
    // its lineage's human owner. Other virtual users and Playground cannot borrow it.
    Ok(
        json!({"owner_user_id":user.to_string(),"owner_principal_id":principal.id.to_string(),"host_id":format!("urn:uuid:{}",principal.id.uuid()),"status":"disconnected"}),
    )
}
#[derive(Clone)]
pub struct DbTokenStore {
    pub db: Arc<StorageBackend>,
    pub encryption: Arc<EncryptionService>,
    pub org: i64,
    pub provider: ProviderId,
}
type RotationLocks = Mutex<std::collections::HashMap<(i64, ProviderId), Arc<Mutex<()>>>>;
static ROTATION_LOCKS: OnceLock<RotationLocks> = OnceLock::new();
async fn memory_lock(org: i64, id: ProviderId) -> OwnedMutexGuard<()> {
    let lock = ROTATION_LOCKS
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .await
        .entry((org, id))
        .or_default()
        .clone();
    lock.lock_owned().await
}
/// Field order is drop order: end the transaction (releasing the database
/// lock) before handing the in-process lock to the next local waiter.
struct PostgresLease {
    _transaction: sqlx::Transaction<'static, sqlx::Postgres>,
    _local: OwnedMutexGuard<()>,
}
#[async_trait]
impl TokenStore for DbTokenStore {
    async fn lock(&self) -> Result<Box<dyn Send>> {
        {
            let db = self.db.database();
            // The lease spans a load and save through the shared pool and,
            // on expiry, an OAuth refresh. Queue same-replica callers on
            // the in-process mutex, then poll the cross-replica lock so no
            // waiter parks a pool connection the holder needs.
            let local = memory_lock(self.org, self.provider).await;
            let transaction = db
                .advisory_xact_lock_polling(
                    "chatgpt_token",
                    &self.provider.uuid().to_string(),
                    crate::storage::repositories::ADVISORY_LOCK_WAIT,
                )
                .await?;
            Ok(Box::new(PostgresLease {
                _transaction: transaction,
                _local: local,
            }))
        }
    }
    async fn load(&self) -> Result<Option<CodexAuth>> {
        let Some(row) = self.db.get_provider(self.org, self.provider.uuid()).await? else {
            return Ok(None);
        };
        let Some(encrypted) = row.api_key_encrypted else {
            return Ok(None);
        };
        let plaintext = self.encryption.decrypt_to_string(&encrypted)?;
        Ok(Some(serde_json::from_str(&plaintext).context(
            "Invalid encrypted ChatGPT credential document",
        )?))
    }
    async fn save(&self, auth: CodexAuth) -> Result<()> {
        let secret = self
            .encryption
            .encrypt_string(&serde_json::to_string(&auth)?)?;
        let row = self
            .db
            .update_provider(
                self.org,
                self.provider.uuid(),
                UpdateProvider {
                    api_key_encrypted: Some(secret),
                    ..empty_update()
                },
            )
            .await?;
        anyhow::ensure!(
            row.is_some(),
            "The ChatGPT provider was removed during login"
        );
        Ok(())
    }
}
pub fn empty_update() -> UpdateProvider {
    UpdateProvider {
        name: None,
        provider_type: None,
        base_url: None,
        api_key_encrypted: None,
        status: None,
        settings: None,
    }
}
pub fn store(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    org: i64,
    id: ProviderId,
) -> Result<DbTokenStore> {
    Ok(DbTokenStore {
        db,
        encryption: encryption
            .ok_or_else(|| anyhow!("Encryption must be configured for ChatGPT sign-in"))?,
        org,
        provider: id,
    })
}
pub async fn access_token(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    row: &ProviderRow,
) -> Result<String> {
    require_enabled(&db, row.org_id).await?;
    let auth = RotatingAuth::new(
        Arc::new(store(db, encryption, row.org_id, row.id)?),
        TokenRoute::ChatGptPlan,
    )
    .token()
    .await?;
    Ok(auth.access_token)
}
// THREAT[TM-AUTHZ-024]: Personal grants require the exact session runtime principal.
pub async fn check_session(
    db: &StorageBackend,
    row: &ProviderRow,
    session: Option<uuid::Uuid>,
) -> Result<()> {
    let Some(owner) = row
        .settings
        .pointer("/chatgpt/owner_principal_id")
        .and_then(Value::as_str)
    else {
        if row.provider_type == "chatgpt" {
            bail!("ChatGPT provider has no runtime owner")
        };
        return Ok(());
    };
    let session = session
        .ok_or_else(|| anyhow!("A personal provider requires a session runtime identity"))?;
    let session = db
        .get_session(row.org_id, session.into())
        .await?
        .ok_or_else(|| anyhow!("Session not found"))?;
    anyhow::ensure!(
        session.owner_principal_id.to_string() == owner && session.playground_user_id.is_none(),
        "This ChatGPT account belongs to another runtime identity. Select your own account or an organization provider."
    );
    Ok(())
}
pub async fn disconnect(store: &DbTokenStore) -> Result<()> {
    let _lease = store.lock().await?;
    let row = store
        .db
        .get_provider(store.org, store.provider.uuid())
        .await?
        .ok_or_else(|| anyhow!("Provider not found"))?;
    let mut settings = row.settings;
    // Invalidate listeners on other replicas before any asynchronous revocation.
    settings["chatgpt"]["generation"] = json!(uuid::Uuid::new_v4());
    store
        .db
        .update_provider(
            store.org,
            store.provider.uuid(),
            UpdateProvider {
                settings: Some(settings),
                ..empty_update()
            },
        )
        .await?;
    if let Some(auth) = store.load().await? {
        let client = auth
            .client_id
            .as_deref()
            .ok_or_else(|| anyhow!("Missing revocation client"))?;
        let refresh = auth
            .refresh_token
            .as_deref()
            .ok_or_else(|| anyhow!("Missing revocation token"))?;
        oauth::revoke_refresh_token(client, refresh).await?;
    }
    // Clear only after revocation is confirmed. Keep the registration for reconnect.
    store
        .db
        .clear_provider_credential(store.org, store.provider.uuid())
        .await?;
    let row = store
        .db
        .get_provider(store.org, store.provider.uuid())
        .await?
        .ok_or_else(|| anyhow!("Provider not found"))?;
    let mut settings = row.settings;
    settings["chatgpt"]["status"] = "disconnected".into();
    settings["chatgpt"]["error"] = Value::Null;
    store
        .db
        .update_provider(
            store.org,
            store.provider.uuid(),
            UpdateProvider {
                settings: Some(settings),
                ..empty_update()
            },
        )
        .await?;
    Ok(())
}

/// Process-local pending ChatGPT sign-in tasks, keyed by (org, provider).
/// Lives here rather than in the HTTP layer so the provider domain can cancel
/// a pending sign-in when a provider is deleted.
pub(crate) static ATTEMPTS: OnceLock<
    Mutex<std::collections::HashMap<(i64, ProviderId), tokio::task::AbortHandle>>,
> = OnceLock::new();

pub(crate) async fn cancel_attempt(org: i64, id: ProviderId) {
    if let Some(task) = ATTEMPTS
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .await
        .remove(&(org, id))
    {
        task.abort();
    }
}

#[cfg(test)]
#[path = "chatgpt_tests.rs"]
mod tests;
