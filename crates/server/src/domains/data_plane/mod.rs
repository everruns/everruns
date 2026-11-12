//! The control plane's side of `/dp/v1` (data plane v2 step 7,
//! knowledge/execution/data-plane-v2/cp-dp-protocol.md): this server is the
//! control plane of the orgs it puts on a data plane.
//!
//! Decisions:
//! - An org is on a data plane (v2) when it has a `dp_orgs` row; every other
//!   org runs on the platform's own workers (v1) and nothing here touches
//!   it. [`for_org`] answers which, and the v1 answer is a no-op.
//! - The outbox is state, not a log: `dp_config_items` holds each v2 org's
//!   config, each item stamped with the org version that last changed it.
//!   A delivery sends everything changed since the version the data plane
//!   applied, so a burst of edits coalesces into one update, and a full
//!   snapshot is every live item. `pg_notify` wakes the dispatcher at once
//!   (config reaches the data plane in well under a second); a 1 s poll
//!   backs it up.
//! - Agents reach the data plane under their Engine agent keys (step 4):
//!   opaque, so the data plane never learns a platform id.
//! - Secrets are sealed here with the org's data key (`dataplane_seal`, the
//!   same code the executors open them with). The data key is stored under
//!   the server's secrets key and goes to the data plane only wrapped with
//!   the data plane's key (`EVERRUNS_DP_KEY`, a local key until step 10
//!   moves it to KMS).
//! - The link carries a bearer token both ways (`EVERRUNS_DP_TOKEN`, the
//!   cells' `CELL_DP_TOKEN`) until step 10 replaces it with mTLS.
//! - Money: the grant ledger (`allowances`) reserves spend against the
//!   org's usage policies when it grants and releases it only through a
//!   closed settlement, so a data plane can never spend more than a policy
//!   allows (see `dp_allowance` for the arithmetic).

use std::sync::Arc;

use everruns_contracts::dataplane::{self as dp, ConfigChange, TenantPolicy};
use everruns_contracts::dataplane_seal as seal;
use serde_json::Value;

use crate::storage::{DataPlaneRow, DpOrgRow, EncryptionService, StorageBackend};

pub mod allowances;
pub mod feed;
pub mod outbox;
pub mod providers;
pub mod sessions;
pub mod tokens;

#[cfg(test)]
mod tests;

/// The link's settings, from the environment.
#[derive(Clone, Default)]
pub struct Link {
    /// Bearer token both ways (`EVERRUNS_DP_TOKEN`).
    pub token: Option<String>,
    /// The data plane's wrapping key (`EVERRUNS_DP_KEY`, base64, 32 bytes).
    pub dp_key: Option<seal::Key>,
}

impl Link {
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("EVERRUNS_DP_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let dp_key = match std::env::var("EVERRUNS_DP_KEY") {
            Ok(key) if !key.is_empty() => Some(
                seal::key_from_base64(&key).map_err(|e| anyhow::anyhow!("EVERRUNS_DP_KEY: {e}"))?,
            ),
            _ => None,
        };
        Ok(Self { token, dp_key })
    }
}

/// What the control plane needs to drive data planes.
#[derive(Clone)]
pub struct ControlPlane {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub link: Link,
    pub http: reqwest::Client,
}

impl ControlPlane {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        link: Link,
    ) -> Self {
        Self {
            db,
            encryption,
            link,
            http: reqwest::Client::new(),
        }
    }

    /// For a request path (a session create, a provider edit): the link and
    /// secrets key from the environment, read once, and one HTTP client
    /// shared by every caller.
    pub fn from_env(db: Arc<StorageBackend>) -> Self {
        static SHARED: std::sync::OnceLock<(
            Option<Arc<EncryptionService>>,
            Link,
            reqwest::Client,
        )> = std::sync::OnceLock::new();
        let (encryption, link, http) = SHARED.get_or_init(|| {
            (
                EncryptionService::from_env().ok().map(Arc::new),
                Link::from_env().unwrap_or_default(),
                reqwest::Client::new(),
            )
        });
        Self {
            db,
            encryption: encryption.clone(),
            link: link.clone(),
            http: http.clone(),
        }
    }

    /// The server's secrets key: a v2 org's data key and the org token
    /// signing key are stored under it, so key rotation covers them.
    pub fn encryption(&self) -> anyhow::Result<&EncryptionService> {
        self.encryption.as_deref().ok_or_else(|| {
            anyhow::anyhow!("SECRETS_ENCRYPTION_KEY is not set: data planes need it")
        })
    }

    /// The org's data key, opened.
    pub fn data_key(&self, org: &DpOrgRow) -> anyhow::Result<seal::Key> {
        self.encryption()?
            .decrypt(&org.data_key_encrypted)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("the org's data key is not 32 bytes"))
    }
}

/// The org id on the wire: its public id.
pub fn org_public(org_id: i64) -> String {
    everruns_core::org_public_id_from_internal(org_id)
}

/// The internal id of a public org id that came over the link.
pub fn org_internal(public: &str) -> Option<i64> {
    let id = public.parse::<everruns_contracts::typed_id::OrgId>().ok()?;
    let internal = everruns_core::organization::org_internal_id_from_public(id);
    (org_public(internal) == public).then_some(internal)
}

/// A data plane as the control plane talks to it. The v1 answer
/// ([`NoDataPlane`]) takes every call and does nothing.
#[async_trait::async_trait]
pub trait DataPlane: Send + Sync {
    /// Which data plane, if any.
    fn id(&self) -> Option<&str>;
    /// `PUT /dp/v1/orgs/{org}/config/{epoch}/{version}`.
    async fn push_config(
        &self,
        org: &str,
        update: &dp::ConfigUpdate,
    ) -> anyhow::Result<dp::ConfigAck>;
    /// An org admin call on a node (`/dp/v1/orgs/...`, feed replay).
    async fn admin(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
    ) -> anyhow::Result<Value>;
    /// `POST /dp/v1/orgs/{org}/sessions`: create the org's session `id`
    /// running its agent `agent_key`. Creating again is a no-op.
    async fn create_session(
        &self,
        org: &str,
        id: everruns_contracts::typed_id::SessionId,
        agent_key: &str,
    ) -> anyhow::Result<()>;
}

/// v1: the org runs on the platform's own workers.
pub struct NoDataPlane;

#[async_trait::async_trait]
impl DataPlane for NoDataPlane {
    fn id(&self) -> Option<&str> {
        None
    }

    async fn push_config(
        &self,
        _: &str,
        update: &dp::ConfigUpdate,
    ) -> anyhow::Result<dp::ConfigAck> {
        Ok(dp::ConfigAck::Ok {
            applied_version: update.version,
        })
    }

    async fn admin(&self, _: reqwest::Method, _: &str, _: &Value) -> anyhow::Result<Value> {
        Ok(Value::Null)
    }

    async fn create_session(
        &self,
        _: &str,
        _: everruns_contracts::typed_id::SessionId,
        _: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A data plane over HTTPS.
pub struct HttpDataPlane {
    pub row: DataPlaneRow,
    token: Option<String>,
    http: reqwest::Client,
}

impl HttpDataPlane {
    pub fn new(row: DataPlaneRow, link: &Link, http: reqwest::Client) -> Self {
        Self {
            row,
            token: link.token.clone(),
            http,
        }
    }

    fn request(&self, method: reqwest::Method, url: String) -> reqwest::RequestBuilder {
        let request = self
            .http
            .request(method, url)
            .timeout(std::time::Duration::from_secs(10));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

#[async_trait::async_trait]
impl DataPlane for HttpDataPlane {
    fn id(&self) -> Option<&str> {
        Some(&self.row.data_plane_id)
    }

    async fn push_config(
        &self,
        org: &str,
        update: &dp::ConfigUpdate,
    ) -> anyhow::Result<dp::ConfigAck> {
        let url = format!(
            "{}{}/orgs/{org}/config/{}/{}",
            self.row.url,
            dp::PREFIX,
            update.epoch,
            update.version
        );
        Ok(self
            .request(reqwest::Method::PUT, url)
            .json(update)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn admin(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
    ) -> anyhow::Result<Value> {
        let url = format!("{}{path}", self.row.admin_url);
        let mut request = self.request(method.clone(), url);
        if method != reqwest::Method::GET {
            request = request.json(body);
        }
        Ok(request.send().await?.error_for_status()?.json().await?)
    }

    async fn create_session(
        &self,
        org: &str,
        id: everruns_contracts::typed_id::SessionId,
        agent_key: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{}{}/orgs/{org}/sessions", self.row.url, dp::PREFIX);
        self.request(reqwest::Method::POST, url)
            .json(&serde_json::json!({ "id": id, "agent_ref": agent_key }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// The data plane `org_id` runs on: [`NoDataPlane`] for a v1 org.
pub async fn for_org(cp: &ControlPlane, org_id: i64) -> anyhow::Result<Box<dyn DataPlane>> {
    let Some(org) = cp.db.dp_org(org_id).await? else {
        return Ok(Box::new(NoDataPlane));
    };
    let row = cp
        .db
        .dp_data_plane(&org.data_plane_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("data plane {} is not registered", org.data_plane_id))?;
    Ok(Box::new(HttpDataPlane::new(row, &cp.link, cp.http.clone())))
}

/// Put `org_id` on `data_plane_id` (v2): mint its data key, seed its config
/// with every published agent and its policy, and let the dispatcher enable
/// it on the data plane. Running it again changes nothing.
pub async fn enable_org(
    cp: &ControlPlane,
    org_id: i64,
    data_plane_id: &str,
) -> anyhow::Result<DpOrgRow> {
    anyhow::ensure!(
        cp.db.dp_data_plane(data_plane_id).await?.is_some(),
        "data plane {data_plane_id} is not registered"
    );
    anyhow::ensure!(
        cp.link.dp_key.is_some(),
        "EVERRUNS_DP_KEY is not set: the org's data key could not be wrapped"
    );
    let key = seal::random_key();
    let stored = cp.encryption()?.encrypt(&key)?;
    let data_key_id = format!("dk_{}", uuid::Uuid::new_v4().simple());
    let org = cp
        .db
        .dp_insert_org(org_id, data_plane_id, &data_key_id, &stored)
        .await?;
    put_policy(cp, org_id, &TenantPolicy::default()).await?;
    providers::sync(cp, org_id).await?;
    for key in cp.db.engine_agent_keys_for_edit(org_id, None).await? {
        if let Some((revision, _)) = cp.db.engine_agent_current(&key.agent_key).await? {
            agent_published(&cp.db, &key.agent_key, org_id, &revision).await?;
        }
    }
    Ok(org)
}

/// The step 4 publisher published `revision` of `agent_key`: a v2 org's
/// data plane gets it.
pub async fn agent_published(
    db: &StorageBackend,
    agent_key: &str,
    org_id: i64,
    revision: &str,
) -> anyhow::Result<()> {
    if db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    let Some(declaration) = db.engine_agent_definition(agent_key, revision).await? else {
        return Ok(());
    };
    let declaration: everruns::hosting::RuntimeAgent = serde_json::from_value(declaration)?;
    // The declaration's agent layer is the package manifest the data plane
    // runs; checked here so a bad one never leaves the control plane.
    declaration
        .package()
        .map_err(|e| anyhow::anyhow!("agent {agent_key} {revision}: {e}"))?;
    let n: u64 = revision.trim_start_matches('r').parse().unwrap_or(1);
    let change = ConfigChange::Agent(dp::AgentDefinition {
        agent_id: agent_key.to_owned(),
        revision: n,
        format: 1,
        package: declaration.agent,
    });
    db.dp_put_item(
        org_id,
        "agent",
        agent_key,
        n as i64,
        Some(&change.to_value()),
    )
    .await?;
    Ok(())
}

/// Seal `plain` as the next revision of `org_id`'s secret `secret_id`.
pub async fn put_secret(
    cp: &ControlPlane,
    org_id: i64,
    secret_id: &str,
    plain: &[u8],
) -> anyhow::Result<u64> {
    let org = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} is not on a data plane"))?;
    let key = cp.data_key(&org)?;
    let revision = cp
        .db
        .dp_item(org_id, "secret", secret_id)
        .await?
        .map_or(1, |item| item.revision + 1) as u64;
    let sealed = seal::seal(
        &key,
        &org.data_key_id,
        &org_public(org_id),
        secret_id,
        revision,
        plain,
        seal::random_nonce(),
    )?;
    cp.db
        .dp_put_item(
            org_id,
            "secret",
            secret_id,
            revision as i64,
            Some(&ConfigChange::Secret(sealed).to_value()),
        )
        .await?;
    Ok(revision)
}

/// Set `org_id`'s connection (its token is a secret of its own).
pub async fn put_connection(
    db: &StorageBackend,
    org_id: i64,
    connection: dp::Connection,
) -> anyhow::Result<()> {
    let id = connection.connection_id.clone();
    let revision = db
        .dp_item(org_id, "connection", &id)
        .await?
        .map_or(1, |item| item.revision + 1);
    db.dp_put_item(
        org_id,
        "connection",
        &id,
        revision,
        Some(&ConfigChange::Connection(connection).to_value()),
    )
    .await?;
    Ok(())
}

/// Set `org_id`'s tenant policy, whole.
pub async fn put_policy(
    cp: &ControlPlane,
    org_id: i64,
    policy: &TenantPolicy,
) -> anyhow::Result<()> {
    let revision = cp
        .db
        .dp_item(org_id, "policy", "policy")
        .await?
        .map_or(1, |item| item.revision + 1);
    let change = ConfigChange::Policy(policy.clone()).to_value();
    // An equal policy is not a change.
    if cp
        .db
        .dp_item(org_id, "policy", "policy")
        .await?
        .is_some_and(|item| item.body.as_ref() == Some(&change))
    {
        return Ok(());
    }
    cp.db
        .dp_put_item(org_id, "policy", "policy", revision, Some(&change))
        .await?;
    Ok(())
}

/// Remove an agent, secret or connection: running turns keep what they
/// started with.
pub async fn remove(db: &StorageBackend, org_id: i64, kind: &str, id: &str) -> anyhow::Result<()> {
    let Some(item) = db.dp_item(org_id, kind, id).await? else {
        return Ok(());
    };
    db.dp_put_item(org_id, kind, id, item.revision + 1, None)
        .await?;
    Ok(())
}

/// Suspend or resume `org_id` on its data plane, at once.
pub async fn set_org_state(
    cp: &ControlPlane,
    org_id: i64,
    state: dp::OrgState,
    reason: Option<String>,
) -> anyhow::Result<()> {
    let plane = for_org(cp, org_id).await?;
    let label = match state {
        dp::OrgState::Active => "active",
        dp::OrgState::Suspended => "suspended",
    };
    let command = dp::OrgStateCommand {
        command_id: format!("state-{}", uuid::Uuid::new_v4().simple()),
        state,
        reason,
    };
    plane
        .admin(
            reqwest::Method::POST,
            &format!("{}/orgs/{}/state", dp::PREFIX, org_public(org_id)),
            &serde_json::to_value(&command)?,
        )
        .await?;
    cp.db.dp_set_org_state(org_id, label).await?;
    Ok(())
}

/// What a restored control plane's data plane holds that this database
/// does not know (an operator resolves it before the new epoch's snapshot).
pub async fn config_diff(cp: &ControlPlane, org_id: i64) -> anyhow::Result<dp::ConfigDiff> {
    let plane = for_org(cp, org_id).await?;
    let mut known = dp::ConfigKnown::default();
    for item in cp.db.dp_items_live(org_id).await? {
        let revision = item.revision as u64;
        match item.kind.as_str() {
            "agent" => known.agents.push((item.item_id, revision)),
            "secret" => known.secrets.push((item.item_id, revision)),
            "connection" => known.connections.push(item.item_id),
            _ => {}
        }
    }
    let diff = plane
        .admin(
            reqwest::Method::POST,
            &format!("{}/orgs/{}/config/unknown", dp::PREFIX, org_public(org_id)),
            &serde_json::to_value(&known)?,
        )
        .await?;
    Ok(serde_json::from_value(diff)?)
}
