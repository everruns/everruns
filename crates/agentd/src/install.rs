//! agentd as a backend you push to (agentd design, "Sessions, labels and
//! agents"): a controller pushes agents and secrets in, creates sessions
//! with labels, and reads one stream out (`install_stream.rs`). agentd
//! knows no orgs, people or billing.
//!
//! Decisions:
//! - An install maps onto what the data plane already has, so nothing
//!   below the gateway learns a second vocabulary. The install is one org
//!   in the directory shard (`install`); a group (the sessions sharing one
//!   value of the install's group label) is one more. A session's tenant is
//!   its group, so spend, suspension and data keys work per group exactly as
//!   they do per org on the hosted service.
//! - Agents and secrets pushed to the install serve every group; a group's
//!   own secrets come first (`lookup_orgs`). Agents are install-wide: the
//!   group label carries no meaning beyond limits, spend and keys.
//! - A push is one config update in the directory shard's Raft, built from
//!   the install's applied version, so concurrent pushes through different
//!   gateways serialize there and a lost race simply builds again.
//! - Secrets are sealed with their group's data key by the gateway that
//!   takes them; only executors open them (`providers.rs`, `mcp.rs`). A
//!   secret's value may also come from the executor's environment or a
//!   resolver (`secret_sources.rs`); its record holds where, not what.
//! - One controller per install: the first push records its caller id;
//!   another caller's push is refused (`group.rs`).
//! - A session created with labels keeps them for life; they ride every
//!   stream record about it. Only the group label means anything here, and
//!   the one label agentd sets itself: `agentd.agent`, the pushed agent a
//!   session was created from (`agentd.` names are agentd's own).
//! - Model keys can be scoped narrower than a group: to one agent
//!   (`/v1/agents/{agent}/secrets/{name}`) or one session
//!   (`/v1/sessions/{session}/secrets/{name}`), so callers need not share
//!   one key. A turn looks for its provider's binding in the session's
//!   scope, then the agent's, then the group's, then the install's
//!   (`providers.rs`). A scoped secret is stored as `name@scope` with its
//!   binding `llm:provider@scope`, in the session's group (session scope)
//!   or the install (agent scope), so it seals and moves like any other.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use everruns_contracts::dataplane_seal as seal;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::{AdminCommand, OrgConfig};
use crate::dp::{
    ConfigAck, ConfigChange, ConfigPosition, ConfigUpdate, Connection, OrgEnable, OrgState,
    OrgStateCommand, TenantPolicy,
};
use crate::gateway::Gateway;
use crate::group::{GroupOp, GroupView};
use crate::state::{Command, Response};

type ApiError = (StatusCode, String);

/// The install's own org in the directory shard.
pub const INSTALL: &str = "install";

/// The label agentd sets on a session created from a pushed agent.
pub const AGENT_LABEL: &str = "agentd.agent";

/// A secret's or binding's scope: one agent's sessions.
pub fn agent_scope(agent: &str) -> String {
    format!("agent.{agent}")
}

/// A secret's or binding's scope: one session.
pub fn session_scope(session: &str) -> String {
    format!("session.{session}")
}

/// `id` stored under `scope`.
pub fn scoped(id: &str, scope: &str) -> String {
    format!("{id}@{scope}")
}

/// This process's install, when it runs one (`agentd serve`, or a node,
/// gateway or executor started with `--install`): executors read it to find
/// install-wide secrets for a group's sessions.
static CURRENT: OnceLock<String> = OnceLock::new();

/// Mark this process as part of install `id`.
pub fn set_current(id: &str) {
    let _ = CURRENT.set(id.to_owned());
}

/// The orgs whose secrets and connections a session of `tenant` sees, in
/// order: its group's, then the install's.
pub fn lookup_orgs(tenant: &str) -> Vec<String> {
    match CURRENT.get() {
        Some(install) if install != tenant => vec![tenant.to_owned(), install.clone()],
        _ => vec![tenant.to_owned()],
    }
}

pub struct Install {
    /// The install's org id.
    pub id: String,
    /// The label whose value names a session's group, if any.
    pub group_label: Option<String>,
    /// The data plane's key: it wraps every group's data key.
    dp_key: seal::Key,
    /// Groups known enabled, so a create does not read the directory.
    enabled: Mutex<HashSet<String>>,
    /// The fleet's object store, which holds the stream's archive.
    pub store: Option<crate::store::Store>,
    /// Who may call the API (`install_auth.rs`).
    pub auth: crate::install_auth::Auth,
}

impl Install {
    pub fn new(
        id: &str,
        group_label: Option<String>,
        dp_key: seal::Key,
        store: Option<crate::store::Store>,
        auth: crate::install_auth::Auth,
    ) -> Arc<Self> {
        set_current(id);
        Arc::new(Self {
            id: id.to_owned(),
            group_label,
            dp_key,
            enabled: Mutex::default(),
            store,
            auth,
        })
    }

    /// The group (tenant) of a session with `labels`.
    pub fn group_of(&self, labels: &BTreeMap<String, String>) -> Result<String, ApiError> {
        let Some(label) = &self.group_label else {
            return Ok(self.id.clone());
        };
        let value = labels.get(label).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("this install groups sessions by the label {label:?}: give it"),
            )
        })?;
        group_id(value)
    }

    /// Enable `org` in the directory shard once: a data key of its own,
    /// wrapped with the data plane's key, and a policy that copies every
    /// session event onto the stream.
    pub async fn ensure_group(&self, gateway: &Gateway, org: &str) -> Result<(), ApiError> {
        if self.enabled.lock().contains(org) {
            return Ok(());
        }
        if full(gateway, org)
            .await?
            .is_some_and(|c| c.data_key.is_some())
        {
            self.enabled.lock().insert(org.to_owned());
            return Ok(());
        }
        let data_key = seal::random_key();
        let data_key_id = format!("dk-{org}-1");
        let wrapped = seal::wrap_data_key(
            &self.dp_key,
            org,
            &data_key_id,
            &data_key,
            seal::random_nonce(),
        )
        .map_err(internal)?;
        // Decision: one command id per group, so two gateways enabling it
        // at once apply one key; the loser's is dropped as a repeat.
        let command = AdminCommand::Enable(OrgEnable {
            command_id: format!("enable-{org}"),
            wrapped_data_key: wrapped,
            data_key_id,
            policy: TenantPolicy {
                events: vec!["*".into()],
                ..TenantPolicy::default()
            },
        });
        admin(gateway, org, command).await?;
        // A new group's first sessions tap every event from their first
        // turn, not from each shard's next tenants poll (`tenants.rs`
        // POLL): hand every shard the directory's taps now. Repeating what
        // the poll would propose, it is harmless when the poll wins.
        let directory = gateway.directory_shard()?;
        let tenants: crate::tenants::Tenants =
            gateway.call(directory, "/shard/tenants", None).await?;
        for shard in gateway.topology()?.shards.keys().copied() {
            gateway
                .propose(
                    shard,
                    &Command::Taps {
                        taps: tenants.taps.clone(),
                    },
                )
                .await?;
        }
        self.enabled.lock().insert(org.to_owned());
        Ok(())
    }

    /// `org`'s data key, unwrapped in memory, with its id.
    async fn data_key(
        &self,
        gateway: &Gateway,
        org: &str,
    ) -> Result<(seal::Key, String), ApiError> {
        self.ensure_group(gateway, org).await?;
        let directory = gateway.directory_shard()?;
        let wrapped: Option<crate::control::WrappedKey> = gateway
            .call(directory, &format!("/shard/orgs/{org}/data-key"), None)
            .await?;
        let wrapped = wrapped.ok_or_else(|| internal(format!("group {org} has no data key")))?;
        let key = seal::unwrap_data_key(&self.dp_key, org, &wrapped.data_key_id, &wrapped.wrapped)
            .map_err(internal)?;
        Ok((key, wrapped.data_key_id))
    }

    /// Apply `changes` to `org`'s config as one update on top of what is
    /// applied there, building again when another push got in first.
    pub async fn push(
        &self,
        gateway: &Gateway,
        org: &str,
        changes: Vec<Value>,
    ) -> Result<u64, ApiError> {
        self.ensure_group(gateway, org).await?;
        let directory = gateway.directory_shard()?;
        for _ in 0..8 {
            let position: ConfigPosition = gateway
                .call(directory, &format!("/shard/orgs/{org}/config"), None)
                .await?;
            let update = ConfigUpdate {
                epoch: position.epoch.max(1),
                version: position.applied_version + 1,
                base_version: position.applied_version,
                changes: changes.clone(),
            };
            let command = Command::Config {
                org: org.to_owned(),
                update,
                now_ms: crate::node::now_ms(),
            };
            let ack: ConfigAck = value_of(gateway.propose(directory, &command).await?)?;
            match ack {
                ConfigAck::Ok { applied_version } => return Ok(applied_version),
                // Another push applied this version first: build again.
                ConfigAck::AlreadyApplied { .. } | ConfigAck::NeedFullSnapshot { .. } => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                ConfigAck::StaleEpoch { epoch } => {
                    return Err((
                        StatusCode::CONFLICT,
                        format!("the install's config is at epoch {epoch}"),
                    ));
                }
            }
        }
        Err((
            StatusCode::CONFLICT,
            "too many pushes at once; push again".into(),
        ))
    }

    /// Refuse a push from anyone but the install's controller.
    pub async fn claim(&self, gateway: &Gateway, caller: &str) -> Result<(), ApiError> {
        let directory = gateway.directory_shard()?;
        let command = Command::Group {
            org: self.id.clone(),
            change: GroupOp::Claim {
                controller: caller.to_owned(),
            },
            now_ms: crate::node::now_ms(),
        };
        match gateway.propose(directory, &command).await? {
            Response::Ok | Response::Duplicate => Ok(()),
            Response::Rejected { reason } => Err((StatusCode::FORBIDDEN, reason)),
            other => Err(internal(format!("{other:?}"))),
        }
    }
}

/// A group id for label value `value`: the value, when it is a safe id.
pub fn group_id(value: &str) -> Result<String, ApiError> {
    if valid_name(value) && value != INSTALL {
        Ok(value.to_owned())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!("{value:?} is not a group: letters, digits, `-`, `_`, `.`, at most 128"),
        ))
    }
}

/// Names of agents, secrets and groups.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn value_of<T: serde::de::DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    match response {
        Response::Value { value } => serde_json::from_value(value).map_err(internal),
        Response::Rejected { reason } => Err((StatusCode::CONFLICT, reason)),
        other => Err(internal(format!("{other:?}"))),
    }
}

async fn full(gateway: &Gateway, org: &str) -> Result<Option<OrgConfig>, ApiError> {
    let directory = gateway.directory_shard()?;
    gateway
        .call(directory, &format!("/shard/orgs/{org}/full"), None)
        .await
}

async fn admin(gateway: &Gateway, org: &str, command: AdminCommand) -> Result<(), ApiError> {
    let directory = gateway.directory_shard()?;
    let command = Command::Admin {
        org: org.to_owned(),
        command,
        now_ms: crate::node::now_ms(),
    };
    match gateway.propose(directory, &command).await? {
        Response::Value { .. } | Response::Ok | Response::Duplicate => Ok(()),
        other => Err(internal(format!("{other:?}"))),
    }
}

/// Who is calling, as the auth layer found (`install_auth.rs`); `dev` with
/// `serve --dev`.
#[derive(Debug, Clone)]
pub struct Caller(pub String);

fn installed(gateway: &Gateway) -> Result<Arc<Install>, ApiError> {
    gateway
        .install()
        .ok_or((StatusCode::NOT_FOUND, "this gateway runs no install".into()))
}

/// Before every push: the caller must be the install's controller.
async fn pushing(
    gateway: &Gateway,
    caller: &Option<axum::Extension<Caller>>,
) -> Result<Arc<Install>, ApiError> {
    let install = installed(gateway)?;
    let caller = caller
        .as_ref()
        .map_or("anonymous", |axum::Extension(c)| c.0.as_str());
    install.claim(gateway, caller).await?;
    Ok(install)
}

/// The push API, mounted on a gateway that runs an install.
pub fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route("/v1/agents", get(list_agents))
        .route(
            "/v1/agents/{agent}",
            put(put_agent).get(get_agent).delete(delete_agent),
        )
        .route("/v1/secrets", get(list_secrets))
        .route(
            "/v1/secrets/{name}",
            put(put_install_secret).delete(delete_install_secret),
        )
        .route("/v1/groups/{group}", put(put_group).get(get_group))
        .route(
            "/v1/groups/{group}/secrets/{name}",
            put(put_group_secret).delete(delete_group_secret),
        )
        .route(
            "/v1/agents/{agent}/secrets/{name}",
            put(put_agent_secret).delete(delete_agent_secret),
        )
        .route(
            "/v1/sessions/{session}/secrets/{name}",
            put(put_session_secret).delete(delete_session_secret),
        )
}

// --------------------------------------------------------------- agents

#[derive(Serialize, Deserialize)]
pub struct AgentEntry {
    pub id: String,
    pub revision: u64,
    pub name: String,
}

async fn put_agent(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path(agent): Path<String>,
    Json(package): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !valid_name(&agent) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{agent:?} is not an agent id"),
        ));
    }
    // A malformed definition is refused at push, not at its first turn.
    crate::host::package(&package).map_err(|e| (StatusCode::BAD_REQUEST, format!("agent: {e}")))?;
    let install = pushing(&gateway, &caller).await?;
    let current = gateway.agent_revision(&install.id, &agent).await.ok();
    if current.as_ref().is_some_and(|c| c.package == package) {
        let revision = current.map_or(0, |c| c.revision);
        return Ok(Json(
            json!({ "id": agent, "revision": revision, "changed": false }),
        ));
    }
    let revision = current.map_or(0, |c| c.revision) + 1;
    let change =
        json!({ "kind": "Agent", "agent_id": agent, "revision": revision, "package": package });
    install.push(&gateway, &install.id, vec![change]).await?;
    Ok(Json(
        json!({ "id": agent, "revision": revision, "changed": true }),
    ))
}

async fn get_agent(
    State(gateway): State<Arc<Gateway>>,
    Path(agent): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let install = installed(&gateway)?;
    let found = gateway.agent_revision(&install.id, &agent).await?;
    Ok(Json(json!({
        "id": found.agent_id,
        "revision": found.revision,
        "definition": found.package,
    })))
}

async fn list_agents(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let install = installed(&gateway)?;
    let agents: Vec<AgentEntry> = full(&gateway, &install.id)
        .await?
        .map(|c| {
            c.agents
                .into_values()
                .map(|a| AgentEntry {
                    name: a.package["name"].as_str().unwrap_or_default().to_owned(),
                    id: a.agent_id,
                    revision: a.revision,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(json!({ "data": agents })))
}

async fn delete_agent(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path(agent): Path<String>,
) -> Result<StatusCode, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    // Sessions already running it keep the revision they copied.
    let removal = ConfigChange::Removal {
        of: "agent".into(),
        id: agent,
    };
    install
        .push(&gateway, &install.id, vec![removal.to_value()])
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- secrets

/// `PUT /v1/secrets/{name}`: one secret, its source and its binding.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretBody {
    /// The value itself (source `stored`).
    #[serde(default)]
    pub value: Option<String>,
    /// Or the name of an environment variable the executor reads (source
    /// `env`; only where the install allows it, `--env-secrets`).
    #[serde(default)]
    pub env: Option<String>,
    /// Or a resolver the executor asks for the value (source `resolve`).
    #[serde(default)]
    pub resolve: Option<crate::secret_sources::Resolve>,
    /// What the secret is for.
    #[serde(default)]
    pub bind: Option<Bind>,
}

/// Where a secret may be used.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Bind {
    /// A model provider's key: sessions whose agents name provider `model`
    /// call it with this secret.
    #[serde(default)]
    pub model: Option<String>,
    /// The provider's driver (`openai`, `anthropic`, `openrouter`, ...);
    /// the provider's name when unset.
    #[serde(default)]
    pub provider_type: Option<String>,
    /// The provider's API, when not the driver's default.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The chat API, when the provider selects one.
    #[serde(default)]
    pub api: Option<String>,
}

impl Install {
    /// Seal `body` as `org`'s secret `name`, with its binding, and push it.
    pub async fn put_secret(
        &self,
        gateway: &Gateway,
        org: &str,
        name: &str,
        body: SecretBody,
    ) -> Result<Value, ApiError> {
        self.put_scoped_secret(gateway, org, name, None, body).await
    }

    /// Store `name` in `org`, under `scope` when given (`scoped`).
    pub async fn put_scoped_secret(
        &self,
        gateway: &Gateway,
        org: &str,
        name: &str,
        scope: Option<&str>,
        body: SecretBody,
    ) -> Result<Value, ApiError> {
        if !valid_name(name) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("{name:?} is not a secret name"),
            ));
        }
        let shown = name;
        let stored = scope.map(|scope| scoped(name, scope));
        let name = stored.as_deref().unwrap_or(name);
        let plain =
            crate::secret_sources::record(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let (data_key, data_key_id) = self.data_key(gateway, org).await?;
        let revision = full(gateway, org)
            .await?
            .and_then(|c| c.secrets.get(name).map(|s| s.revision))
            .unwrap_or(0)
            + 1;
        let sealed = seal::seal(
            &data_key,
            &data_key_id,
            org,
            name,
            revision,
            &plain,
            seal::random_nonce(),
        )
        .map_err(internal)?;
        let mut changes = vec![ConfigChange::Secret(sealed).to_value()];
        if let Some(Bind {
            model: Some(provider),
            provider_type,
            base_url,
            api,
        }) = body.bind
        {
            let key = everruns_contracts::runtime_provider::ProviderKey::new(&provider);
            let settings = crate::providers::Settings {
                provider_key: key.as_str().to_owned(),
                provider_type: provider_type.unwrap_or_else(|| key.as_str().to_owned()),
                base_url,
                api,
                request_options: Default::default(),
            };
            let base = crate::providers::connection_id(&provider);
            let connection = scope.map_or_else(|| base.clone(), |scope| scoped(&base, scope));
            changes.push(
                ConfigChange::Connection(Connection {
                    connection_id: connection,
                    kind: crate::providers::KIND.into(),
                    settings: serde_json::to_value(&settings).map_err(internal)?,
                    token_secret_id: name.to_owned(),
                    refresh_owner: Default::default(),
                })
                .to_value(),
            );
        }
        self.push(gateway, org, changes).await?;
        Ok(json!({ "name": shown, "scope": scope, "revision": revision }))
    }

    async fn delete_secret(
        &self,
        gateway: &Gateway,
        org: &str,
        name: &str,
    ) -> Result<(), ApiError> {
        let config = full(gateway, org).await?.unwrap_or_default();
        let mut changes = vec![
            ConfigChange::Removal {
                of: "secret".into(),
                id: name.to_owned(),
            }
            .to_value(),
        ];
        // A model binding goes with its secret.
        for connection in config
            .connections
            .values()
            .filter(|c| c.token_secret_id == name)
        {
            changes.push(
                ConfigChange::Removal {
                    of: "connection".into(),
                    id: connection.connection_id.clone(),
                }
                .to_value(),
            );
        }
        self.push(gateway, org, changes).await.map(|_| ())
    }
}

async fn put_install_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path(name): Path<String>,
    Json(body): Json<SecretBody>,
) -> Result<Json<Value>, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = install.id.clone();
    install
        .put_secret(&gateway, &org, &name, body)
        .await
        .map(Json)
}

async fn delete_install_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = install.id.clone();
    install.delete_secret(&gateway, &org, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_group_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((group, name)): Path<(String, String)>,
    Json(body): Json<SecretBody>,
) -> Result<Json<Value>, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = group_id(&group)?;
    install
        .put_secret(&gateway, &org, &name, body)
        .await
        .map(Json)
}

async fn delete_group_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((group, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = group_id(&group)?;
    install.delete_secret(&gateway, &org, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_agent_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((agent, name)): Path<(String, String)>,
    Json(body): Json<SecretBody>,
) -> Result<Json<Value>, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    if !valid_name(&agent) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{agent:?} is not an agent id"),
        ));
    }
    let org = install.id.clone();
    let scope = agent_scope(&agent);
    install
        .put_scoped_secret(&gateway, &org, &name, Some(&scope), body)
        .await
        .map(Json)
}

async fn delete_agent_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((agent, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = install.id.clone();
    install
        .delete_secret(&gateway, &org, &scoped(&name, &agent_scope(&agent)))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A session's group, which keeps its session-scoped secrets.
async fn session_group(
    gateway: &Gateway,
    session: &str,
) -> Result<(String, everruns_contracts::typed_id::SessionId), ApiError> {
    let id: everruns_contracts::typed_id::SessionId = session.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("{session:?} is not a session id"),
        )
    })?;
    let tenant = gateway
        .session_tenant(id)
        .await?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no session {session}")))?;
    Ok((tenant, id))
}

async fn put_session_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((session, name)): Path<(String, String)>,
    Json(body): Json<SecretBody>,
) -> Result<Json<Value>, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let (org, id) = session_group(&gateway, &session).await?;
    let scope = session_scope(&id.to_string());
    install
        .put_scoped_secret(&gateway, &org, &name, Some(&scope), body)
        .await
        .map(Json)
}

async fn delete_session_secret(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path((session, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let (org, id) = session_group(&gateway, &session).await?;
    install
        .delete_secret(
            &gateway,
            &org,
            &scoped(&name, &session_scope(&id.to_string())),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The install's secrets by name and revision; values never leave.
async fn list_secrets(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let install = installed(&gateway)?;
    let config = full(&gateway, &install.id).await?.unwrap_or_default();
    let data: Vec<Value> = config
        .secrets
        .values()
        .map(|s| {
            let bound: Vec<&str> = config
                .connections
                .values()
                .filter(|c| c.token_secret_id == s.secret_id)
                .map(|c| c.connection_id.as_str())
                .collect();
            json!({ "name": s.secret_id, "revision": s.revision, "bound": bound })
        })
        .collect();
    Ok(Json(json!({ "data": data })))
}

// --------------------------------------------------------------- groups

/// `PUT /v1/groups/{value}`: the group's limits, replaced whole.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GroupBody {
    /// Turns the group runs at once (per shard).
    #[serde(default)]
    pub limit: Option<u32>,
    /// No new turn starts while suspended; running ones finish.
    #[serde(default)]
    pub suspended: bool,
    /// The most the group may spend on models, ever, in US dollars.
    #[serde(default)]
    pub spend_usd: Option<f64>,
}

async fn put_group(
    State(gateway): State<Arc<Gateway>>,
    caller: Option<axum::Extension<Caller>>,
    Path(group): Path<String>,
    Json(body): Json<GroupBody>,
) -> Result<Json<Value>, ApiError> {
    let install = pushing(&gateway, &caller).await?;
    let org = if group == INSTALL {
        install.id.clone()
    } else {
        group_id(&group)?
    };
    install.ensure_group(&gateway, &org).await?;
    let directory = gateway.directory_shard()?;
    let set = Command::Group {
        org: org.clone(),
        change: GroupOp::Set {
            max_turns: body.limit,
            spend_limit_micros: body.spend_usd.map(|usd| (usd * 1_000_000.0).round() as i64),
        },
        now_ms: crate::node::now_ms(),
    };
    let view: GroupView = value_of(gateway.propose(directory, &set).await?)?;
    let state = OrgStateCommand {
        command_id: format!("state-{}", uuid::Uuid::new_v4()),
        state: if body.suspended {
            OrgState::Suspended
        } else {
            OrgState::Active
        },
        reason: body.suspended.then(|| "suspended by the controller".into()),
    };
    admin(&gateway, &org, AdminCommand::State(state)).await?;
    Ok(Json(group_json(&group, &view, body.suspended)))
}

async fn get_group(
    State(gateway): State<Arc<Gateway>>,
    Path(group): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let install = installed(&gateway)?;
    let org = if group == INSTALL {
        install.id.clone()
    } else {
        group_id(&group)?
    };
    let config = full(&gateway, &org)
        .await?
        .ok_or((StatusCode::NOT_FOUND, format!("no group {group}")))?;
    let view = config.group.as_ref().map(|g| g.view()).unwrap_or_default();
    Ok(Json(group_json(
        &group,
        &view,
        config.state == Some(OrgState::Suspended),
    )))
}

fn group_json(group: &str, view: &GroupView, suspended: bool) -> Value {
    let usd = |micros: i64| micros as f64 / 1_000_000.0;
    json!({
        "group": group,
        "limit": view.max_turns,
        "suspended": suspended,
        "spend_usd": view.spend_limit_micros.map(usd),
        "spent_usd": usd(view.spent_micros),
        "reserved_usd": usd(view.reserved_micros),
    })
}

// ------------------------------------------------------------- sessions

/// `POST /v1/sessions` on an install.
#[derive(Debug, Default, Deserialize)]
pub struct CreateBody {
    /// A pushed agent, at its current revision.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Or an agent definition of its own.
    #[serde(default)]
    pub agent: Option<Value>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub id: Option<everruns_contracts::typed_id::SessionId>,
}

impl Install {
    pub async fn create(&self, gateway: &Gateway, body: CreateBody) -> Result<Value, ApiError> {
        for (key, value) in &body.labels {
            if key.starts_with("agentd.") {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("label {key:?}: `agentd.` labels are set by agentd"),
                ));
            }
            if !valid_name(key) || value.len() > 256 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "label {key:?}: names are letters, digits, `-`, `_`, `.`; values at most 256 bytes"
                    ),
                ));
            }
        }
        let group = self.group_of(&body.labels)?;
        self.ensure_group(gateway, &group).await?;
        let mut labels = body.labels;
        let definition = match (body.agent_id, body.agent) {
            (Some(agent), None) => {
                let package = gateway.agent_revision(&self.id, &agent).await?.package;
                labels.insert(AGENT_LABEL.to_owned(), agent);
                package
            }
            (None, Some(definition)) => definition,
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "give `agent_id` (a pushed agent) or `agent` (a definition)".into(),
                ));
            }
        };
        gateway
            .create_running(
                definition,
                None,
                body.id,
                Some(group),
                None,
                crate::gateway::Joining {
                    labels,
                    ..Default::default()
                },
            )
            .await
    }
}

/// `GET /v1/sessions?label=k=v&limit=n`: sessions across every shard whose
/// labels hold every `label` given.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub label: Vec<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One session in a list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listed {
    pub id: String,
    pub agent: String,
    pub labels: BTreeMap<String, String>,
    pub created_ms: i64,
    pub running: bool,
}

pub async fn list_sessions(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, ApiError> {
    let labels = label_filter(&query)?;
    let limit = query
        .iter()
        .find(|(k, _)| k == "limit")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(1000);
    let filter = serde_json::to_value(&labels).map_err(internal)?;
    let shards: Vec<_> = gateway.topology()?.shards.keys().copied().collect();
    let mut found: Vec<Listed> = Vec::new();
    for shard in shards {
        let page: Vec<Listed> = gateway
            .call(shard, "/shard/sessions/labelled", Some(&filter))
            .await?;
        found.extend(page);
    }
    found.sort_by_key(|s| std::cmp::Reverse(s.created_ms));
    found.truncate(limit);
    Ok(Json(json!({ "data": found })))
}

/// `label=k=v` pairs from a query.
pub fn label_filter(query: &[(String, String)]) -> Result<BTreeMap<String, String>, ApiError> {
    query
        .iter()
        .filter(|(k, _)| k == "label")
        .map(|(_, v)| {
            v.split_once(['=', ':'])
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    format!("label {v:?}: give name=value"),
                ))
        })
        .collect()
}

/// The shard side: this shard's sessions whose labels hold `filter`.
pub async fn shard_labelled(
    State(node): State<Arc<crate::node::Node>>,
    Json(filter): Json<BTreeMap<String, String>>,
) -> Result<Json<Vec<Listed>>, ApiError> {
    if !node.is_leader() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "not the leader".into()));
    }
    let now = crate::node::now_ms();
    let state = node.shared.state.read();
    let out = state
        .sessions
        .iter()
        .filter(|(_, s)| s.placement == crate::state::Placement::Live)
        .filter(|(_, s)| filter.iter().all(|(k, v)| s.labels.get(k) == Some(v)))
        .map(|(id, s)| Listed {
            id: id.to_string(),
            agent: s.agent.clone(),
            labels: s.labels.clone(),
            created_ms: s.created_ms,
            running: s.authority.executing(now),
        })
        .collect();
    Ok(Json(out))
}
