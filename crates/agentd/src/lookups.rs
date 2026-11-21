//! What a tool on the data plane asks the control plane at call time (data
//! plane v2, step 11.6b).
//!
//! A hosted capability's tools look org answers up as they run: a session's
//! connection token, a GitHub token for one use, the agent's service API
//! key, the org's web search settings and account keys, the tool approval
//! policy, the org's knowledge collections, a Poppy call, a Slack action.
//! On a v1 worker they reach the control plane through internal
//! commands; here the same resolver ([`CommandConnectionResolver`]) runs the
//! same commands over the link: executor, directory cell, control plane
//! as host calls (`everruns_contracts::host_calls`,
//! `POST /dp/v1/tenants/{org}/host-calls/{name}`). The answer comes back
//! sealed with the org's data key, so the cell that forwards it holds
//! ciphertext only.
//!
//! Decisions:
//! - Only the commands a host call declares go over; the rest stay with
//!   the data plane: sandbox keys are sealed org secrets the control plane
//!   wrote when it pinned the sandbox (`sandbox.rs`), MCP credentials are
//!   the org's MCP secrets (`mcp.rs`), and a cell leases nothing to clean
//!   up. Anything else is refused here, before it leaves the executor.
//! - A standalone cell has no control plane: its tools get what a host
//!   without org settings gives (no connections, default settings), as they
//!   did before lookups.
//! - The org's own settings (web search settings and account keys, the
//!   tool approval policy, the egress grants) are read from its pushed
//!   config (`dataplane::ORG_SETTING`, the control plane's
//!   `org_settings.rs`), so they keep answering while the control plane is
//!   down; one that has not arrived yet is asked as before. The session
//!   owner's own approval settings are still asked, and without an answer
//!   the turn runs with the org's policy alone, which only asks more.
//! - A lookup that cannot reach the control plane fails the tool call, as
//!   on a v1 worker whose control plane is down: a credential or a policy
//!   never falls back to a default.
//! - The org's egress grants (open egress, allowlist extension) are asked
//!   the same way, by the process's egress boundary ([`serve_org_egress`]):
//!   a session's turns run as its tenant's org (`hosting.rs`), so its
//!   tools' requests carry that org. A failed lookup grants nothing, as on
//!   v1 (fail closed).

use std::collections::BTreeSet;
use std::sync::{Arc, Once, Weak};

use async_trait::async_trait;
use everruns_capabilities::capabilities::{PoppyCall, PoppyInvoker, PoppyInvokerExt};
use everruns_contracts::dataplane::{
    EGRESS_SETTING, ORG_SETTING, TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING, web_search_key_secret,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::host_calls::{
    HostCallAnswer, OPEN_TURN, SLACK_ACTION, TURN_CALLS, by_command,
};
use everruns_contracts::runtime::capabilities::injected::{
    CREDENTIAL_CALLS, CREDENTIAL_TOOL, CREDENTIALS,
};

use crate::mcp::Acting;
use everruns_contracts::knowledge::KnowledgeRuntimeExt;
use everruns_contracts::runtime::organization::org_public_id_from_internal;
use everruns_contracts::runtime::session_spend::SessionSpendReaderExt;
use everruns_contracts::runtime::web_search::WebSearchOrgSettings;
use everruns_contracts::runtime::{OrgEgressAllowlist, OrgEgressGrant};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::slack_action::{
    SlackAction, SlackActionError, SlackActionInvoker, SlackActionInvokerExt, SlackActionOutcome,
};
use everruns_contracts::tool_approval_policy::SessionToolApprovalPolicy;
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_core::connection_services::{
    GitHubAccess, McpResolvedCredential, ServiceApiKeyConnection, UserConnectionResolver,
};
use everruns_core::host::HostBackends;
use everruns_internal_protocol::commands::knowledge::CommandKnowledgeRuntime;
use everruns_internal_protocol::commands::{
    CommandConnectionResolver, CommandOrgEgressAllowlist, CommandSessionSpendReader,
    InternalCommandTransport, is_bad_request,
};
use everruns_internal_protocol::proto;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::providers::OrgMaterial;

/// One org's lookups, as the internal command transport the shared
/// resolver runs over.
#[derive(Clone)]
pub struct OrgLookups {
    material: Weak<dyn OrgMaterial>,
    org: String,
    /// The host calls the session's agent declares (its capabilities'
    /// `host_calls`), when the lookups serve one session; `None` for the
    /// process's egress boundary, which serves every session.
    declared: Option<Arc<BTreeSet<&'static str>>>,
}

#[async_trait]
impl InternalCommandTransport for OrgLookups {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        let Some(call) = by_command(name) else {
            return Ok(Err(proto::CommandError {
                kind: proto::command_error::Kind::Forbidden as i32,
                message: format!("{name} is not available on the data plane"),
            }));
        };
        let material = self
            .material
            .upgrade()
            .ok_or_else(|| AgentLoopError::store("the executor is stopping"))?;
        if !self.declares(call.name) {
            // Counted, not refused (yet): a capability reaches some calls
            // through shared parts (a pinned sandbox's git credential, the
            // egress boundary), so the declarations are checked against the
            // live fleet before a call they miss is refused.
            tracing::warn!(org = %self.org, call = call.name, "a host call the session's capabilities do not declare");
            material.undeclared_host_call(call.name);
        }
        if CREDENTIAL_CALLS.contains(&call.name)
            && let Some(server) = credentials_server(&self.material, &material, &params)
        {
            return match server {
                Ok(server) => credential(&server, call.name, params).await,
                Err(refused) => Ok(Err(refused)),
            };
        }
        match material.lookup(&self.org, name, params).await {
            Ok(HostCallAnswer::Value(value)) => Ok(Ok(value)),
            Ok(HostCallAnswer::Error { kind, message }) => {
                Ok(Err(proto::CommandError { kind, message }))
            }
            Err(error) => {
                tracing::warn!(org = %self.org, command = name, %error, "lookup failed");
                Err(AgentLoopError::store(
                    "the control plane could not be reached for this lookup",
                ))
            }
        }
    }
}

/// The control plane's credentials server for the session a credential
/// lookup names, acting as the person who sent the message it names
/// (`"open"`: the open turn's, none: no one); `None` when the session's
/// create brought no such server, and the lookup is a host call.
fn credentials_server(
    weak: &Weak<dyn OrgMaterial>,
    material: &Arc<dyn OrgMaterial>,
    params: &Value,
) -> Option<std::result::Result<crate::mcp::ControllerServer, proto::CommandError>> {
    use everruns_contracts::typed_id::MessageId;
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
        crate::mcp::ControllerServer::acting(Some(weak), session, acting, CREDENTIALS, &url)
            .ok_or_else(|| bad("this node holds no session tokens for the credential lookup")),
    )
}

/// Credential lookup `call` on the control plane's credentials server.
async fn credential(
    server: &crate::mcp::ControllerServer,
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
    /// Whether the session's agent may make host call `name`: a call every
    /// turn makes, or one its capabilities declare.
    fn declares(&self, name: &str) -> bool {
        TURN_CALLS.contains(&name)
            || self
                .declared
                .as_ref()
                .is_none_or(|declared| declared.contains(name))
    }

    /// The org setting `id` the control plane pushed with the org's config
    /// (`dataplane::ORG_SETTING`), read from the directory cell; `None`
    /// before it arrives or when the directory cell cannot be read, and
    /// the caller then asks the control plane as before.
    async fn setting<T: serde::de::DeserializeOwned>(&self, id: &str) -> Option<T> {
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
}

/// `org`'s lookups for `session`, when `material` reaches a control plane
/// and knows the session's org.
pub(crate) fn org_lookups(
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> Option<OrgLookups> {
    let weak = material?;
    let material = weak.upgrade()?;
    if !material.looks_up() {
        return None;
    }
    let org = material.tenant(session)?;
    let declared = material.declared_host_calls(session).map(Arc::new);
    Some(OrgLookups {
        material: weak.clone(),
        org,
        declared,
    })
}

/// `org`'s lookups outside any one session (the executor's dream sweep),
/// when `material` reaches a control plane.
pub(crate) fn for_org(material: Weak<dyn OrgMaterial>, org: String) -> Option<OrgLookups> {
    if !material.upgrade()?.looks_up() {
        return None;
    }
    Some(OrgLookups {
        material,
        org,
        declared: None,
    })
}

/// The connection resolver of one activation of `session`: `keys` for its
/// sandbox, and the control plane's lookups when `material` reaches one.
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
        Some(lookups) => Arc::new(CellConnections {
            keys,
            lookups: Arc::new(CommandConnectionResolver::new(lookups.clone())),
            pushed: lookups,
        }),
        None => keys,
    }
}

/// `backends` for one activation of `session`: its connection resolver
/// ([`connections`]) and, when the control plane is reached, the org's
/// knowledge for the `knowledge` tools (`KnowledgeRuntimeExt`), the
/// session's Poppy client (`PoppyInvokerExt`) and its Slack actions
/// (`SlackActionInvokerExt`), as a v1 worker installs them. Its SQL
/// databases are the cell's own (`hosting_sqldb.rs`).
///
/// Decision: automatic knowledge context (evidence added to a turn's input)
/// stays off on a cell: the control plane records each retrieval against
/// the turn's input message, which on the data plane lives in the cell, so
/// it answers none. The tools search and browse the same collections.
pub fn attach(
    mut backends: HostBackends,
    keys: Arc<dyn UserConnectionResolver>,
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> HostBackends {
    let lookups = org_lookups(material, session);
    if backends.connection_resolver.is_none() {
        backends.connection_resolver = Some(with_lookups(keys, lookups.clone()));
    }
    if let Some(lookups) = lookups {
        let knowledge =
            KnowledgeRuntimeExt(Arc::new(CommandKnowledgeRuntime::new(lookups.clone())));
        let knowledge = Arc::new(knowledge);
        // `get_session_info`'s spend: the control plane prices it (price
        // tables, the org's rates, sandbox spans), reading the model calls
        // from this cell's log.
        let spend = Arc::new(SessionSpendReaderExt(Arc::new(
            CommandSessionSpendReader::new(lookups.clone()),
        )));
        let embedder = backends.tool_context_extensions_factory.take();
        backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
            let mut extensions = embedder
                .as_ref()
                .map(|factory| factory(org, session))
                .unwrap_or_default();
            extensions.insert(knowledge.clone());
            extensions.insert(spend.clone());
            extensions.insert(Arc::new(PoppyInvokerExt(Arc::new(CellPoppy {
                lookups: lookups.clone(),
                session,
            }))));
            // The `slack` tools' actions, as the session's own endpoint bot.
            // Decision: not the conversation sender a v1 worker installs with
            // them: a post binds to the turn's input event, which on the data
            // plane lives in the cell, so the channel host delivers the
            // turn's messages from the session's stream instead.
            extensions.insert(Arc::new(SlackActionInvokerExt(Arc::new(
                CellSlackActions {
                    lookups: lookups.clone(),
                    session,
                },
            ))));
            extensions
        }));
    }
    backends
}

/// Executors of this process, for [`CellOrgEgress`].
static EXECUTORS: parking_lot::Mutex<Vec<Weak<dyn OrgMaterial>>> =
    parking_lot::Mutex::new(Vec::new());

/// Let this process's egress boundary ask `material`'s control plane for an
/// org's egress grants. The boundary keeps one resolver per process
/// (`install_runtime_org_egress_allowlist`), so it is installed once and
/// asks the first live executor that reaches a control plane: every
/// executor of a process serves the same data plane.
pub fn serve_org_egress(material: Weak<dyn OrgMaterial>) {
    static INSTALLED: Once = Once::new();
    let mut executors = EXECUTORS.lock();
    executors.retain(|executor| executor.strong_count() > 0);
    executors.push(material);
    drop(executors);
    INSTALLED.call_once(|| {
        everruns_core::host::install_runtime_org_egress_allowlist(Arc::new(CellOrgEgress));
    });
}

/// An org's egress grants from the control plane (`serve_org_egress`).
struct CellOrgEgress;

#[async_trait]
impl OrgEgressAllowlist for CellOrgEgress {
    async fn grant(&self, org_id: &OrgId) -> std::result::Result<OrgEgressGrant, String> {
        let reaching = EXECUTORS.lock().iter().find_map(|weak| {
            let material = weak.upgrade()?;
            material.looks_up().then(|| weak.clone())
        });
        org_grant(reaching, org_id).await
    }
}

/// `org_id`'s egress grants: from the org's pushed config
/// (`dataplane::EGRESS_SETTING`), else from `material`'s control plane;
/// nothing granted without one (a standalone cell has no org settings).
async fn org_grant(
    material: Option<Weak<dyn OrgMaterial>>,
    org_id: &OrgId,
) -> std::result::Result<OrgEgressGrant, String> {
    let Some(material) = material else {
        return Ok(OrgEgressGrant::default());
    };
    let lookups = move |org| OrgLookups {
        material: material.clone(),
        org: org_public_id_from_internal(org),
        declared: None,
    };
    let org = everruns_contracts::runtime::organization::org_internal_id_from_public(*org_id);
    if let Some(enforced) = lookups(org).setting::<Enforced>(EGRESS_SETTING).await {
        // Patterns are pushed only for a granted org.
        return Ok(everruns_contracts::runtime::org_egress_grant(
            enforced.open,
            true,
            &enforced.patterns,
        ));
    }
    CommandOrgEgressAllowlist::new(lookups).grant(org_id).await
}

/// The egress grants as pushed (`org_settings.rs` on the control plane).
#[derive(serde::Deserialize)]
struct Enforced {
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    open: bool,
}

/// The Poppy client of one session: the session owner's keys and Session
/// Tokens stay in the control plane (`worker_poppy_call`), only the call and
/// its answer cross. Same answers as the v1 worker's `CommandPoppyInvoker`.
struct CellPoppy {
    lookups: OrgLookups,
    session: SessionId,
}

#[async_trait]
impl PoppyInvoker for CellPoppy {
    async fn invoke(&self, call: PoppyCall) -> std::result::Result<Value, String> {
        let params = json!({ "session_id": self.session.to_string(), "call": call });
        match self
            .lookups
            .execute_internal_command("worker_poppy_call", params)
            .await
        {
            Ok(Ok(value)) => Ok(value),
            // A company's refusal or an unreachable company: for the model.
            Ok(Err(error)) if is_bad_request(&error) => Err(error.message),
            Ok(Err(error)) => {
                tracing::warn!(message = %error.message, "Poppy call failed");
                Err("Poppy is unavailable right now.".to_string())
            }
            Err(error) => {
                tracing::warn!(%error, "Poppy call did not reach the control plane");
                Err("Poppy is unavailable right now.".to_string())
            }
        }
    }
}

/// The Slack actions of one session ([`SLACK_ACTION`]): the control plane
/// acts as the bot of the endpoint the session came through, and the bot
/// token stays there. Same answers as the v1 worker's
/// `GrpcSlackActionInvoker`: a refusal comes back as the action's error, a
/// lookup that fails is transient.
struct CellSlackActions {
    lookups: OrgLookups,
    session: SessionId,
}

#[async_trait]
impl SlackActionInvoker for CellSlackActions {
    async fn invoke(
        &self,
        action: SlackAction,
    ) -> std::result::Result<SlackActionOutcome, SlackActionError> {
        let params = json!({ "session_id": self.session.to_string(), "action": action });
        let answer = match self
            .lookups
            .execute_internal_command(SLACK_ACTION, params)
            .await
        {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => return Err(SlackActionError::Transient(error.message)),
            Err(error) => return Err(SlackActionError::Transient(error.to_string())),
        };
        let transient = |error: serde_json::Error| SlackActionError::Transient(error.to_string());
        if let Some(error) = answer.get("error") {
            return Err(serde_json::from_value(error.clone()).map_err(transient)?);
        }
        serde_json::from_value(answer["outcome"].clone()).map_err(transient)
    }
}

/// Sandbox keys from the cell, the org's settings and search keys from its
/// pushed config (`org_settings.rs` on the control plane), every other
/// lookup from the control plane. A setting not pushed yet is asked as
/// before.
struct CellConnections {
    keys: Arc<dyn UserConnectionResolver>,
    lookups: Arc<dyn UserConnectionResolver>,
    pushed: OrgLookups,
}

#[async_trait]
impl UserConnectionResolver for CellConnections {
    fn for_execution(&self, input_message_id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        let lookups = self.lookups.for_execution(input_message_id)?;
        Some(Arc::new(Self {
            keys: self.keys.clone(),
            lookups,
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
        self.lookups
            .get_connection_token(session_id, provider)
            .await
    }

    async fn get_service_api_key_connection(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<ServiceApiKeyConnection>> {
        self.lookups
            .get_service_api_key_connection(session_id, provider)
            .await
    }

    async fn get_github_token(
        &self,
        session_id: SessionId,
        access: &GitHubAccess,
    ) -> Result<Option<McpResolvedCredential>> {
        self.lookups.get_github_token(session_id, access).await
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
        session_id: SessionId,
    ) -> Result<Option<WebSearchOrgSettings>> {
        if let Some(settings) = self.pushed.setting(WEB_SEARCH_SETTING).await {
            return Ok(Some(settings));
        }
        self.lookups.get_web_search_settings(session_id).await
    }

    /// The org's policy from its pushed config; the session owner's own
    /// settings are a person's and still asked. Without their answer (the
    /// control plane is down) the turn runs with the org's policy and no
    /// personal settings, which only asks more.
    async fn get_tool_approval_policy(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionToolApprovalPolicy>> {
        let Some(org) = self.pushed.setting(TOOL_APPROVAL_SETTING).await else {
            return self.lookups.get_tool_approval_policy(session_id).await;
        };
        let personal = match self.lookups.get_tool_approval_policy(session_id).await {
            Ok(answer) => answer.map(|policy| policy.personal).unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%session_id, %error, "the owner's approval settings were not read");
                Default::default()
            }
        };
        Ok(Some(SessionToolApprovalPolicy { org, personal }))
    }

    async fn get_web_search_account_key(
        &self,
        session_id: SessionId,
        account_id: Uuid,
    ) -> Result<Option<String>> {
        if let Some(key) = self.pushed.secret(&web_search_key_secret(account_id)).await {
            return Ok(Some(String::from_utf8_lossy(&key).into_owned()));
        }
        // Pushed settings and no key: not one of the org's search accounts.
        if self
            .pushed
            .setting::<WebSearchOrgSettings>(WEB_SEARCH_SETTING)
            .await
            .is_some()
        {
            return Ok(None);
        }
        self.lookups
            .get_web_search_account_key(session_id, account_id)
            .await
    }
}

#[cfg(test)]
#[path = "lookups_tests.rs"]
mod tests;
