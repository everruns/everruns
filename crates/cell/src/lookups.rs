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
//! - A lookup that cannot reach the control plane fails the tool call, as
//!   on a v1 worker whose control plane is down: a credential or a policy
//!   never falls back to a default.
//! - The org's egress grants (open egress, allowlist extension) are asked
//!   the same way, by the process's egress boundary ([`serve_org_egress`]):
//!   a session's turns run as its tenant's org (`hosting.rs`), so its
//!   tools' requests carry that org. A failed lookup grants nothing, as on
//!   v1 (fail closed).

use std::sync::{Arc, Once, Weak};

use async_trait::async_trait;
use everruns_capabilities::capabilities::{PoppyCall, PoppyInvoker, PoppyInvokerExt};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::host_calls::{HostCallAnswer, SLACK_ACTION, by_command};
use everruns_contracts::knowledge::KnowledgeRuntimeExt;
use everruns_contracts::runtime::organization::org_public_id_from_internal;
use everruns_contracts::runtime::web_search::WebSearchOrgSettings;
use everruns_contracts::runtime::{OrgEgressAllowlist, OrgEgressGrant};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::session_sqldb::SessionSqlDbStoreExt;
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
    CommandConnectionResolver, CommandOrgEgressAllowlist, InternalCommandTransport, is_bad_request,
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
}

#[async_trait]
impl InternalCommandTransport for OrgLookups {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        if by_command(name).is_none() {
            return Ok(Err(proto::CommandError {
                kind: proto::command_error::Kind::Forbidden as i32,
                message: format!("{name} is not available on the data plane"),
            }));
        }
        let material = self
            .material
            .upgrade()
            .ok_or_else(|| AgentLoopError::store("the executor is stopping"))?;
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
    Some(OrgLookups {
        material: weak.clone(),
        org,
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
            lookups: Arc::new(CommandConnectionResolver::new(lookups)),
        }),
        None => keys,
    }
}

/// `backends` for one activation of `session`: its connection resolver
/// ([`connections`]) and, when the control plane is reached, the org's
/// knowledge for the `knowledge` tools (`KnowledgeRuntimeExt`), the
/// session's Poppy client (`PoppyInvokerExt`), its Slack actions
/// (`SlackActionInvokerExt`) and its SQL databases (`SessionSqlDbStoreExt`),
/// as a v1 worker installs them.
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
        let embedder = backends.tool_context_extensions_factory.take();
        backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
            let mut extensions = embedder
                .as_ref()
                .map(|factory| factory(org, session))
                .unwrap_or_default();
            extensions.insert(knowledge.clone());
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
            // The `sql_database` tools' store: the session's databases in
            // the control plane, where the console reads them.
            extensions.insert(Arc::new(SessionSqlDbStoreExt(Arc::new(
                crate::sqldb::CellSqlDb::new(lookups.clone(), session),
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

/// `org_id`'s egress grants from `material`'s control plane; nothing
/// granted without one (a standalone cell has no org settings).
async fn org_grant(
    material: Option<Weak<dyn OrgMaterial>>,
    org_id: &OrgId,
) -> std::result::Result<OrgEgressGrant, String> {
    let Some(material) = material else {
        return Ok(OrgEgressGrant::default());
    };
    CommandOrgEgressAllowlist::new(move |org| OrgLookups {
        material: material.clone(),
        org: org_public_id_from_internal(org),
    })
    .grant(org_id)
    .await
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

/// Sandbox keys from the cell, every other lookup from the control plane.
struct CellConnections {
    keys: Arc<dyn UserConnectionResolver>,
    lookups: Arc<dyn UserConnectionResolver>,
}

#[async_trait]
impl UserConnectionResolver for CellConnections {
    fn for_execution(&self, input_message_id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        let lookups = self.lookups.for_execution(input_message_id)?;
        Some(Arc::new(Self {
            keys: self.keys.clone(),
            lookups,
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
        self.lookups.get_web_search_settings(session_id).await
    }

    async fn get_tool_approval_policy(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionToolApprovalPolicy>> {
        self.lookups.get_tool_approval_policy(session_id).await
    }

    async fn get_web_search_account_key(
        &self,
        session_id: SessionId,
        account_id: Uuid,
    ) -> Result<Option<String>> {
        self.lookups
            .get_web_search_account_key(session_id, account_id)
            .await
    }
}

#[cfg(test)]
#[path = "lookups_tests.rs"]
mod tests;
