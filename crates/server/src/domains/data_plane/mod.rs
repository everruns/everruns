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
use everruns_contracts::session_commands::{Kind, SessionCommand};
use serde_json::Value;

use crate::storage::{DataPlaneRow, DpOrgRow, EncryptionService, StorageBackend};

pub mod agent_scripts;
pub mod allowances;
pub mod attachments;
pub mod catalog;
pub mod default_plane;
pub mod event_tap;
pub mod feed;
pub mod host_calls;
pub mod mcp;
pub mod memory;
pub mod moves;
pub mod org_settings;
pub mod outbox;
pub mod providers;
pub mod sandboxes;
pub mod session_plane;
pub mod sessions;
pub mod subagents;
pub mod task_webhooks;
pub mod tokens;
pub mod user_mcp;
pub mod warehouse;

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
    /// Delivers v2 tasks' webhooks (`task_webhooks.rs`); none drops them.
    pub egress: Option<Arc<dyn everruns_core::EgressService>>,
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
            egress: None,
        }
    }

    /// With `egress` for the webhooks the feed asks for.
    pub fn with_egress(mut self, egress: Arc<dyn everruns_core::EgressService>) -> Self {
        self.egress = Some(egress);
        self
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
            egress: None,
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

/// A session the control plane creates on an org's data plane.
pub struct NewSession<'a> {
    /// The control plane's id for it: its record and the data plane's
    /// session are one session.
    pub id: everruns_contracts::typed_id::SessionId,
    /// The org's agent it runs, at its current revision.
    pub agent_key: &'a str,
    /// Or the session's own declaration of that agent (its own settings on
    /// top).
    pub definition: Option<&'a Value>,
    /// The session's own starter files.
    pub initial_files: &'a [everruns_core::session_file::InitialFile],
    /// The managed sandbox pinned for it, as a capability entry.
    pub sandbox: Option<&'a Value>,
    /// The control plane's Memory mounts it has (`/memory/user`, ...),
    /// whose files stay in the control plane (`data_plane::memory`).
    pub memory: &'a [String],
    /// A moved v1 session being imported: closed to messages until it is
    /// activated (`moves.rs`).
    pub importing: bool,
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
    /// `POST /dp/v1/orgs/{org}/sessions`: create the org's session
    /// ([`NewSession`]). Creating again is a no-op.
    async fn create_session(&self, org: &str, session: &NewSession<'_>) -> anyhow::Result<()>;
    /// `POST /dp/v1/orgs/{org}/sessions/{id}/{command}`: a declared session
    /// command the control plane runs on one of the org's sessions itself
    /// (`messages`: an agent trigger's run, a channel's message, where a
    /// retry with the same message id admits once; `cancel-turn`; `seed`, a
    /// channel's prior conversation; see [`everruns_contracts::session_commands`]).
    /// The cell's answer.
    async fn session_command(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        command: &'static SessionCommand,
        body: &Value,
    ) -> anyhow::Result<Value>;
    /// `GET /dp/v1/orgs/{org}/sessions/{id}/{read}?{query}`: a declared
    /// session read of one of the org's sessions (`trace`, `events`, ...;
    /// step 11.6).
    async fn session_read(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        read: &'static SessionCommand,
        query: &str,
    ) -> anyhow::Result<Value>;
    /// `GET /dp/v1/orgs/{org}/sessions/{id}/sse`: the session's committed
    /// events after `after_sequence`, then live, as event envelopes. The
    /// stream ends when the data plane cycles the connection; the caller
    /// reconnects from the last sequence it saw.
    async fn events(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        after_sequence: Option<i64>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, Value>>;
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

    async fn create_session(&self, _: &str, _: &NewSession<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    async fn session_command(
        &self,
        _: &str,
        session: everruns_contracts::typed_id::SessionId,
        _: &'static SessionCommand,
        _: &Value,
    ) -> anyhow::Result<Value> {
        anyhow::bail!("session {session} is not on a data plane")
    }

    async fn session_read(
        &self,
        _: &str,
        _: everruns_contracts::typed_id::SessionId,
        _: &'static SessionCommand,
        _: &str,
    ) -> anyhow::Result<Value> {
        anyhow::bail!("the org has no data plane")
    }

    async fn events(
        &self,
        _: &str,
        session: everruns_contracts::typed_id::SessionId,
        _: Option<i64>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, Value>> {
        anyhow::bail!("session {session} is not on a data plane")
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

    async fn create_session(&self, org: &str, session: &NewSession<'_>) -> anyhow::Result<()> {
        let url = format!("{}{}/orgs/{org}/sessions", self.row.url, dp::PREFIX);
        let mut body = serde_json::json!({ "id": session.id, "agent_ref": session.agent_key });
        if let Some(definition) = session.definition {
            body["definition"] = definition.clone();
        }
        if !session.initial_files.is_empty() {
            body["initial_files"] = serde_json::to_value(session.initial_files)?;
        }
        if let Some(sandbox) = session.sandbox {
            body["sandbox"] = sandbox.clone();
        }
        if !session.memory.is_empty() {
            body["memory"] = serde_json::to_value(session.memory)?;
            // Where the data plane reads and writes them, with a session
            // token, once this control plane serves Memory as files.
            if let Some(url) = crate::injected::hosted::memory::served_url() {
                body["memory_server"] = Value::String(url);
            }
        }
        if session.importing {
            body["importing"] = serde_json::json!(true);
        }
        self.request(reqwest::Method::POST, url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn session_command(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        command: &'static SessionCommand,
        body: &Value,
    ) -> anyhow::Result<Value> {
        debug_assert_eq!(command.kind, Kind::Command);
        let url = format!(
            "{}{}/orgs/{org}/sessions/{session}/{}",
            self.row.url,
            dp::PREFIX,
            command.name
        );
        Ok(self
            .request(reqwest::Method::POST, url)
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn session_read(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        read: &'static SessionCommand,
        query: &str,
    ) -> anyhow::Result<Value> {
        debug_assert_eq!(read.kind, Kind::Read);
        let mut url = format!(
            "{}{}/orgs/{org}/sessions/{session}/{}",
            self.row.url,
            dp::PREFIX,
            read.name
        );
        if !query.is_empty() {
            url.push('?');
            url.push_str(query);
        }
        Ok(self
            .request(reqwest::Method::GET, url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn events(
        &self,
        org: &str,
        session: everruns_contracts::typed_id::SessionId,
        after_sequence: Option<i64>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, Value>> {
        let mut url = format!(
            "{}{}/orgs/{org}/sessions/{session}/sse",
            self.row.url,
            dp::PREFIX
        );
        if let Some(after) = after_sequence {
            url.push_str(&format!("?after_sequence={after}"));
        }
        // No overall timeout: the stream stays open until the data plane
        // cycles it.
        let mut request = self.http.get(url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?.error_for_status()?;
        Ok(sse_events(response.bytes_stream()))
    }
}

/// The event envelopes of an SSE body: each `data:` line of an event
/// other than the stream's own `connected`, `disconnecting` and heartbeats.
fn sse_events<S, B, E>(body: S) -> futures::stream::BoxStream<'static, Value>
where
    S: futures::Stream<Item = Result<B, E>> + Send + 'static,
    B: AsRef<[u8]>,
{
    use futures::StreamExt;
    let lines = futures::stream::unfold(
        (
            Box::pin(body),
            Vec::<u8>::new(),
            std::collections::VecDeque::<String>::new(),
        ),
        |(mut body, mut buffer, mut ready)| async move {
            loop {
                if let Some(line) = ready.pop_front() {
                    return Some((line, (body, buffer, ready)));
                }
                match body.next().await {
                    Some(Ok(chunk)) => {
                        buffer.extend_from_slice(chunk.as_ref());
                        while let Some(at) = buffer.iter().position(|b| *b == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=at).collect();
                            let line = String::from_utf8_lossy(&line);
                            ready.push_back(line.trim_end_matches(['\r', '\n']).to_owned());
                        }
                    }
                    _ => return None,
                }
            }
        },
    );
    lines
        .filter_map(|line| async move {
            let data = line.strip_prefix("data:")?.trim_start();
            let value: Value = serde_json::from_str(data).ok()?;
            // Envelopes carry their type; the stream's own frames do not
            // name an event of the session.
            value["type"]
                .as_str()
                .is_some_and(|t| t.contains('.'))
                .then_some(value)
        })
        .boxed()
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
    event_tap::sync(&cp.db, org_id).await?;
    providers::sync(cp, org_id).await?;
    // Until its settings arrive the cell asks the control plane, so a
    // setting that does not sync never keeps the org off its data plane.
    if let Err(error) = org_settings::sync(cp, org_id).await {
        tracing::warn!(org_id, %error, "the data plane's org settings were not synced");
    }
    for key in cp.db.engine_agent_keys_for_edit(org_id, None).await? {
        if let Some((revision, _)) = cp.db.engine_agent_current(&key.agent_key).await? {
            agent_published(cp, &key.agent_key, org_id, &revision).await?;
        }
    }
    // Every agent, for tools that hand work to another one (parity gap 9).
    if let Err(error) = catalog::sync(cp, org_id).await {
        tracing::warn!(org_id, %error, "the data plane's agent catalog was not synced");
    }
    Ok(org)
}

/// The step 4 publisher published `revision` of `agent_key`: a v2 org's
/// data plane gets it.
pub async fn agent_published(
    cp: &ControlPlane,
    agent_key: &str,
    org_id: i64,
    revision: &str,
) -> anyhow::Result<()> {
    let db = &cp.db;
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
    let mut package = declaration.agent;
    mcp::prepare(cp, org_id, agent_key, &mut package).await?;
    sandboxes::strip(&mut package);
    crate::injected::attach(&mut package, &crate::injected::served());
    let change = ConfigChange::Agent(dp::AgentDefinition {
        agent_id: agent_key.to_owned(),
        revision: n,
        format: 1,
        package,
        card: catalog::card(cp, org_id, agent_key).await?,
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
    db: &StorageBackend,
    org_id: i64,
    policy: &TenantPolicy,
) -> anyhow::Result<()> {
    let current = db.dp_item(org_id, "policy", "policy").await?;
    let change = ConfigChange::Policy(policy.clone()).to_value();
    // An equal policy is not a change.
    if current
        .as_ref()
        .is_some_and(|item| item.body.as_ref() == Some(&change))
    {
        return Ok(());
    }
    let revision = current.map_or(1, |item| item.revision + 1);
    db.dp_put_item(org_id, "policy", "policy", revision, Some(&change))
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

/// What a restored control plane's first snapshot set aside on `org_id`'s
/// data plane (`dp::ConfigLeftovers`); with `keep`, resolve it: `true`
/// writes it back into this database's config, so the next delivery sends
/// it again, `false` drops it. Either way the data plane then forgets it.
///
/// Decision: kept items go through the outbox like any change, so the
/// control plane stays the one source of config. A kept agent is not in
/// the platform's agent list (the restored database lost it); publishing
/// that agent again starts from the platform's own revisions.
pub async fn reconcile(
    cp: &ControlPlane,
    org_id: i64,
    keep: Option<bool>,
) -> anyhow::Result<Option<dp::ConfigLeftovers>> {
    let plane = for_org(cp, org_id).await?;
    let path = format!(
        "{}/orgs/{}/config/leftovers",
        dp::PREFIX,
        org_public(org_id)
    );
    let found: Option<dp::ConfigLeftovers> = serde_json::from_value(
        plane
            .admin(reqwest::Method::GET, &path, &Value::Null)
            .await?,
    )?;
    let (Some(left), Some(keep)) = (&found, keep) else {
        return Ok(found);
    };
    if keep {
        // Only what is still newer than this database's own revision.
        let newer = |current: Option<crate::storage::DpConfigItemRow>, revision: u64| {
            current.is_none_or(|item| (item.revision as u64) < revision)
        };
        for agent in &left.agents {
            let current = cp.db.dp_item(org_id, "agent", &agent.agent_id).await?;
            if newer(current, agent.revision) {
                let change = ConfigChange::Agent(agent.clone()).to_value();
                cp.db
                    .dp_put_item(
                        org_id,
                        "agent",
                        &agent.agent_id,
                        agent.revision as i64,
                        Some(&change),
                    )
                    .await?;
            }
        }
        for secret in &left.secrets {
            let current = cp.db.dp_item(org_id, "secret", &secret.secret_id).await?;
            if newer(current, secret.revision) {
                let change = ConfigChange::Secret(secret.clone()).to_value();
                cp.db
                    .dp_put_item(
                        org_id,
                        "secret",
                        &secret.secret_id,
                        secret.revision as i64,
                        Some(&change),
                    )
                    .await?;
            }
        }
        for connection in &left.connections {
            if cp
                .db
                .dp_item(org_id, "connection", &connection.connection_id)
                .await?
                .is_none()
            {
                put_connection(&cp.db, org_id, connection.clone()).await?;
            }
        }
    }
    let clear = dp::ClearLeftovers {
        command_id: format!(
            "leftovers-{}-{}",
            left.epoch,
            if keep { "kept" } else { "dropped" }
        ),
    };
    plane
        .admin(reqwest::Method::POST, &path, &serde_json::to_value(&clear)?)
        .await?;
    Ok(found)
}
