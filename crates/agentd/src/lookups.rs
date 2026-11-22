//! What a tool on the data plane needs from the org at call time: its
//! pushed settings and the credentials only the control plane holds.
//!
//! Decisions:
//! - The org's own settings (web search settings and account keys, the tool
//!   approval policy and each member's own approval settings, the egress
//!   grants, the helper model catalog) are read from its pushed config
//!   (`dataplane::ORG_SETTING`, the control plane's `org_settings.rs`), so
//!   they keep answering while the control plane is down. One that has not
//!   arrived yet answers as a host without org settings would: no web
//!   search account, no egress grant, the agent's own approval config,
//!   each of which only asks or refuses more.
//! - A credential (a connection token, an MCP grant, a GitHub token, a
//!   service API key) is asked of the control plane's credentials server
//!   (`injected::CREDENTIALS`, sent with the create), as the person who
//!   sent the turn the call belongs to. The shared resolver
//!   ([`CommandConnectionResolver`]) runs over [`OrgLookups`], which
//!   forwards only those four lookups; anything else is refused before it
//!   leaves the executor. Sandbox keys are sealed org secrets the control
//!   plane wrote when it pinned the sandbox (`sandbox.rs`), and MCP OAuth
//!   credentials are the org's MCP secrets (`mcp.rs`).
//! - A standalone shard has no control plane: its sessions carry no
//!   credentials server, so its tools get no connections, as a host
//!   without org settings.
//! - A lookup that cannot reach the credentials server fails the tool call:
//!   a credential never falls back to a default.
//! - `get_session_info`'s spend is priced by the control plane (price
//!   tables, the org's rates, sandbox spans) on its platform commands
//!   server (`injected::PLATFORM_COMMANDS`, operation `spend`), reading the
//!   model calls from this shard's log; it acts for no one.

use std::sync::{Arc, Once, Weak};

use async_trait::async_trait;
use everruns_contracts::dataplane::{
    EGRESS_SETTING, ORG_SETTING, TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING,
    personal_approval_setting, web_search_key_secret,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::capabilities::injected::{
    CONNECTION_TOKEN, CREDENTIAL_TOOL, CREDENTIALS, GITHUB_TOKEN, MCP_CONNECTION_TOKEN,
    PLATFORM_COMMAND_TOOL, PLATFORM_COMMANDS, SERVICE_API_KEY,
};
use everruns_contracts::runtime::organization::org_public_id_from_internal;
use everruns_contracts::runtime::session_spend::{
    SessionSpend, SessionSpendReader, SessionSpendReaderExt,
};
use everruns_contracts::runtime::web_search::WebSearchOrgSettings;
use everruns_contracts::runtime::{OrgEgressAllowlist, OrgEgressGrant};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::tool_approval_policy::{PersonalToolApproval, SessionToolApprovalPolicy};
use everruns_contracts::typed_id::{MessageId, OrgId, SessionId, TurnId};
use everruns_core::connection_services::{
    GitHubAccess, McpResolvedCredential, ServiceApiKeyConnection, UserConnectionResolver,
};
use everruns_core::host::HostBackends;
use everruns_internal_protocol::commands::{CommandConnectionResolver, InternalCommandTransport};
use everruns_internal_protocol::proto;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::mcp::{Acting, ControllerServer};
use crate::providers::OrgMaterial;

/// The `input_message_id` of a lookup for the turn the session has open.
pub const OPEN_TURN: &str = "open";

/// The credentials server's call for v1 worker command `command`, the one
/// the shared resolver sends.
fn credential_call(command: &str) -> Option<&'static str> {
    Some(match command {
        "worker_get_connection_token" => CONNECTION_TOKEN,
        "worker_get_mcp_connection_token" => MCP_CONNECTION_TOKEN,
        "worker_get_github_token" => GITHUB_TOKEN,
        "worker_get_service_api_key_connection" => SERVICE_API_KEY,
        _ => return None,
    })
}

/// One org's lookups: its pushed settings, and its credentials as the
/// internal command transport the shared resolver runs over.
#[derive(Clone)]
pub struct OrgLookups {
    material: Weak<dyn OrgMaterial>,
    org: String,
}

#[async_trait]
impl InternalCommandTransport for OrgLookups {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        let Some(call) = credential_call(name) else {
            return Ok(Err(proto::CommandError {
                kind: proto::command_error::Kind::Forbidden as i32,
                message: format!("{name} is not available on the data plane"),
            }));
        };
        let material = self
            .material
            .upgrade()
            .ok_or_else(|| AgentLoopError::store("the executor is stopping"))?;
        match credentials_server(&self.material, &material, &params) {
            Some(Ok(server)) => credential(&server, call, params).await,
            Some(Err(refused)) => Ok(Err(refused)),
            // No control plane (a standalone shard): no connections.
            None => Ok(Ok(Value::Null)),
        }
    }
}

/// The control plane's credentials server for the session a credential
/// lookup names, acting as the person who sent the message it names
/// ([`OPEN_TURN`]: the open turn's, none: no one); `None` when the
/// session's create brought no such server.
fn credentials_server(
    weak: &Weak<dyn OrgMaterial>,
    material: &Arc<dyn OrgMaterial>,
    params: &Value,
) -> Option<std::result::Result<ControllerServer, proto::CommandError>> {
    let session = params["session_id"]
        .as_str()
        .and_then(|id| SessionId::parse(id).ok())?;
    let url = material.controller_url(session, CREDENTIALS)?;
    let bad = |message: &str| proto::CommandError {
        kind: proto::command_error::Kind::BadRequest as i32,
        message: message.to_owned(),
    };
    let acting = match params["input_message_id"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        None => Acting::NoOne,
        Some(OPEN_TURN) => Acting::Open,
        Some(id) => match MessageId::parse(id)
            .ok()
            .or_else(|| Uuid::parse_str(id).ok().map(MessageId::from_uuid))
        {
            Some(message) => Acting::Message(message),
            None => return Some(Err(bad("a credential lookup needs a valid input message"))),
        },
    };
    Some(
        ControllerServer::acting(Some(weak), session, acting, CREDENTIALS, &url)
            .ok_or_else(|| bad("this node holds no session tokens for the credential lookup")),
    )
}

/// Credential lookup `call` on the control plane's credentials server.
async fn credential(
    server: &ControllerServer,
    call: &str,
    mut params: Value,
) -> Result<std::result::Result<Value, proto::CommandError>> {
    // The server names the session and the person from the token.
    if let Some(params) = params.as_object_mut() {
        for key in ["session_id", "input_message_id", "acting_user"] {
            params.remove(key);
        }
    }
    let text = server
        .call_tool(CREDENTIAL_TOOL, json!({ "call": call, "params": params }))
        .await
        .map_err(|error| {
            tracing::warn!(call, %error, "credential lookup failed");
            AgentLoopError::store("the control plane could not be reached for this lookup")
        })?;
    let answer: Value = serde_json::from_str(&text)
        .map_err(|_| AgentLoopError::store("the credential answer did not parse"))?;
    if let Some(error) = answer.get("error") {
        return Ok(Err(proto::CommandError {
            kind: error["kind"].as_i64().unwrap_or_default() as i32,
            message: error["message"].as_str().unwrap_or_default().to_owned(),
        }));
    }
    Ok(Ok(answer.get("ok").cloned().unwrap_or(Value::Null)))
}

impl OrgLookups {
    /// The org setting `id` the control plane pushed with the org's config
    /// (`dataplane::ORG_SETTING`), read from the directory shard; `None`
    /// before it arrives or when the directory shard cannot be read.
    pub(crate) async fn setting<T: serde::de::DeserializeOwned>(&self, id: &str) -> Option<T> {
        let material = self.material.upgrade()?;
        match material.connection(&self.org, id).await {
            Ok(Some(connection)) if connection.kind == ORG_SETTING => {
                serde_json::from_value(connection.settings)
                    .inspect_err(|error| {
                        tracing::warn!(org = %self.org, setting = id, %error, "a pushed setting did not parse");
                    })
                    .ok()
            }
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(org = %self.org, setting = id, %error, "a pushed setting was not read");
                None
            }
        }
    }

    /// The person who owns `session`, from its shard state.
    fn owner(&self, session: SessionId) -> Option<String> {
        self.material.upgrade()?.owner(session)
    }

    /// The org's sealed secret `id`, opened; `None` when the org has none
    /// or it cannot be read now.
    async fn secret(&self, id: &str) -> Option<Vec<u8>> {
        let material = self.material.upgrade()?;
        match material.open_secret(&self.org, id).await {
            Ok(opened) => opened.map(|opened| opened.plain),
            Err(error) => {
                tracing::debug!(org = %self.org, secret = id, %error, "a pushed secret was not read");
                None
            }
        }
    }

    /// The token the control plane hands out for an MCP server's per-turn
    /// grant (`server_prefix`, `provider`, `acts_as`) in the turn `session`
    /// has open, as the person who sent it. No grant, or no credentials
    /// server: none, and the server's call is unauthorized, which tells the
    /// person to connect.
    pub(crate) async fn open_turn_mcp_token(
        &self,
        session: SessionId,
        server_prefix: &str,
        provider: &str,
        acts_as: &str,
    ) -> Result<Option<String>> {
        let params = json!({
            "session_id": session.to_string(),
            "server_prefix": server_prefix,
            "provider": provider,
            "acts_as": acts_as,
            "input_message_id": OPEN_TURN,
        });
        match self
            .execute_internal_command("worker_get_mcp_connection_token", params)
            .await?
        {
            Ok(Value::String(token)) => Ok(Some(token)),
            Ok(_) => Ok(None),
            Err(error) => {
                tracing::warn!(org = %self.org, server_prefix, message = %error.message, "no MCP grant for the turn");
                Ok(None)
            }
        }
    }
}

/// `session`'s org lookups, when `material` knows the session's org.
pub(crate) fn org_lookups(
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> Option<OrgLookups> {
    let weak = material?;
    let org = weak.upgrade()?.tenant(session)?;
    Some(OrgLookups {
        material: weak.clone(),
        org,
    })
}

/// The connection resolver of one activation of `session`: `keys` for its
/// sandbox, and the org's settings and credentials when `material` knows
/// its org.
pub fn connections(
    keys: Arc<dyn UserConnectionResolver>,
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> Arc<dyn UserConnectionResolver> {
    with_lookups(keys, org_lookups(material, session))
}

fn with_lookups(
    keys: Arc<dyn UserConnectionResolver>,
    lookups: Option<OrgLookups>,
) -> Arc<dyn UserConnectionResolver> {
    match lookups {
        Some(lookups) => Arc::new(ShardConnections {
            keys,
            credentials: Arc::new(CommandConnectionResolver::new(lookups.clone())),
            pushed: lookups,
        }),
        None => keys,
    }
}

/// `backends` for one activation of `session`: its connection resolver
/// ([`connections`]) and, when the session's create brought the control
/// plane's platform commands server, its spend for `get_session_info`.
/// Knowledge, Poppy, Slack actions, Memory and the MCP store are the
/// control plane's injected servers, sent with the create.
pub fn attach(
    mut backends: HostBackends,
    keys: Arc<dyn UserConnectionResolver>,
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> HostBackends {
    let lookups = org_lookups(material, session);
    if backends.connection_resolver.is_none() {
        backends.connection_resolver = Some(with_lookups(keys, lookups));
    }
    let server = material.and_then(|weak| {
        let url = weak.upgrade()?.controller_url(session, PLATFORM_COMMANDS)?;
        ControllerServer::acting(Some(weak), session, Acting::NoOne, PLATFORM_COMMANDS, &url)
    });
    if let Some(server) = server {
        let spend = Arc::new(SessionSpendReaderExt(Arc::new(ShardSpend { server })));
        let embedder = backends.tool_context_extensions_factory.take();
        backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
            let mut extensions = embedder
                .as_ref()
                .map(|factory| factory(org, session))
                .unwrap_or_default();
            extensions.insert(spend.clone());
            extensions
        }));
    }
    backends
}

/// A session's spend, priced by the control plane's platform commands
/// server for the session its token names.
struct ShardSpend {
    server: ControllerServer,
}

#[async_trait]
impl SessionSpendReader for ShardSpend {
    async fn session_spend(
        &self,
        _session: SessionId,
        turn: Option<TurnId>,
    ) -> Result<Option<SessionSpend>> {
        let text = self
            .server
            .call_tool(
                PLATFORM_COMMAND_TOOL,
                json!({
                    "operation": "spend",
                    "arguments": { "turn_id": turn.map(|turn| turn.to_string()) },
                }),
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, "session spend failed");
                AgentLoopError::store("the control plane could not price this session")
            })?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| AgentLoopError::store("the session spend did not parse"))
    }
}

/// Executors of this process, for [`ShardOrgEgress`].
static EXECUTORS: parking_lot::Mutex<Vec<Weak<dyn OrgMaterial>>> =
    parking_lot::Mutex::new(Vec::new());

/// Let this process's egress boundary read an org's egress grants from
/// `material`'s pushed config. The boundary keeps one resolver per process
/// (`install_runtime_org_egress_allowlist`), so it is installed once and
/// reads through the first live executor: every executor of a process
/// serves the same data plane.
pub fn serve_org_egress(material: Weak<dyn OrgMaterial>) {
    static INSTALLED: Once = Once::new();
    let mut executors = EXECUTORS.lock();
    executors.retain(|executor| executor.strong_count() > 0);
    executors.push(material);
    drop(executors);
    INSTALLED.call_once(|| {
        everruns_core::host::install_runtime_org_egress_allowlist(Arc::new(ShardOrgEgress));
    });
}

/// An org's egress grants from its pushed config (`serve_org_egress`).
struct ShardOrgEgress;

#[async_trait]
impl OrgEgressAllowlist for ShardOrgEgress {
    async fn grant(&self, org_id: &OrgId) -> std::result::Result<OrgEgressGrant, String> {
        let live = EXECUTORS
            .lock()
            .iter()
            .find(|weak| weak.strong_count() > 0)
            .cloned();
        Ok(org_grant(live, org_id).await)
    }
}

/// `org_id`'s egress grants from the org's pushed config
/// (`dataplane::EGRESS_SETTING`); nothing granted before it arrives, or
/// without a control plane (fail closed).
async fn org_grant(material: Option<Weak<dyn OrgMaterial>>, org_id: &OrgId) -> OrgEgressGrant {
    let Some(material) = material else {
        return OrgEgressGrant::default();
    };
    let org = everruns_contracts::runtime::organization::org_internal_id_from_public(*org_id);
    let lookups = OrgLookups {
        material,
        org: org_public_id_from_internal(org),
    };
    match lookups.setting::<Enforced>(EGRESS_SETTING).await {
        // Patterns are pushed only for a granted org.
        Some(enforced) => {
            everruns_contracts::runtime::org_egress_grant(enforced.open, true, &enforced.patterns)
        }
        None => OrgEgressGrant::default(),
    }
}

/// The egress grants as pushed (`org_settings.rs` on the control plane).
#[derive(serde::Deserialize)]
struct Enforced {
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    open: bool,
}

/// Sandbox keys from the shard, the org's settings and search keys from its
/// pushed config (`org_settings.rs` on the control plane), credentials from
/// the control plane's credentials server.
struct ShardConnections {
    keys: Arc<dyn UserConnectionResolver>,
    credentials: Arc<dyn UserConnectionResolver>,
    pushed: OrgLookups,
}

#[async_trait]
impl UserConnectionResolver for ShardConnections {
    fn for_execution(&self, input_message_id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        let credentials = self.credentials.for_execution(input_message_id)?;
        Some(Arc::new(Self {
            keys: self.keys.clone(),
            credentials,
            pushed: self.pushed.clone(),
        }))
    }

    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        // MCP grants are the org's MCP secrets on the data plane (`mcp.rs`);
        // the control plane refuses them here too.
        if provider.starts_with("mcp_oauth_") {
            return Ok(None);
        }
        self.credentials
            .get_connection_token(session_id, provider)
            .await
    }

    async fn get_service_api_key_connection(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<ServiceApiKeyConnection>> {
        self.credentials
            .get_service_api_key_connection(session_id, provider)
            .await
    }

    async fn get_github_token(
        &self,
        session_id: SessionId,
        access: &GitHubAccess,
    ) -> Result<Option<McpResolvedCredential>> {
        self.credentials.get_github_token(session_id, access).await
    }

    async fn get_sandbox_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>> {
        self.keys
            .get_sandbox_connection_token(session_id, provider, credential)
            .await
    }

    async fn get_web_search_settings(
        &self,
        _session_id: SessionId,
    ) -> Result<Option<WebSearchOrgSettings>> {
        Ok(self.pushed.setting(WEB_SEARCH_SETTING).await)
    }

    /// The org's policy and the session owner's own settings, both from
    /// the org's pushed config. No policy pushed yet: none, and the turn
    /// runs with the agent's own config, which only asks more.
    async fn get_tool_approval_policy(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionToolApprovalPolicy>> {
        let Some(org) = self.pushed.setting(TOOL_APPROVAL_SETTING).await else {
            return Ok(None);
        };
        let personal = match self.pushed.owner(session_id) {
            Some(owner) => self
                .pushed
                .setting::<PersonalToolApproval>(&personal_approval_setting(&owner))
                .await
                .map(PersonalToolApproval::sanitized)
                .unwrap_or_default(),
            None => PersonalToolApproval::default(),
        };
        Ok(Some(SessionToolApprovalPolicy { org, personal }))
    }

    /// An account key the org pushed with its search settings; none for an
    /// account that is not one of the org's.
    async fn get_web_search_account_key(
        &self,
        _session_id: SessionId,
        account_id: Uuid,
    ) -> Result<Option<String>> {
        Ok(self
            .pushed
            .secret(&web_search_key_secret(account_id))
            .await
            .map(|key| String::from_utf8_lossy(&key).into_owned()))
    }
}

#[cfg(test)]
#[path = "lookups_tests.rs"]
mod tests;
