//! A v2 org's sessions (data plane v2, step 8): the control plane keeps
//! the session's record and the data plane runs it.
//!
//! Decisions:
//! - `POST /v1/sessions` stays the control plane's: it writes the record
//!   (owner, title, tags, budgets) as for any org, then creates the session
//!   on the org's data plane under the same id, so the record and the
//!   running session are one session and the edge finds it by id.
//! - The session runs its agent's Engine key at the current revision. The
//!   key's revision is put in the org's config and delivered before the
//!   create, so the data plane never misses the agent it is asked to run.
//! - A create the data plane refuses fails the request: a record with no
//!   running session would take messages nothing answers. The data plane's
//!   create is idempotent, so a retry is safe.
//! - A session created with its own settings (a prompt, client tools, MCP
//!   servers, network access, iteration controls, capabilities) runs its
//!   own declaration: the agent's layers with the session's settings
//!   folded on top (`agent_definitions::PlatformLayers::session`), sent
//!   with the create rather than published as config, so an org's config
//!   does not grow with its sessions and every node does not hold every
//!   session's agent. A v2 session keeps the definition it was created
//!   with, as every v2 session does (parity gap 4). Its credentials are
//!   bound under the session's own key (`{agent key}:{session}`), never
//!   shared with the agent's.
//! - Its own starter files are taken (step 11.1): the data plane writes
//!   them into the session's workspace with the create, only where absent,
//!   so a retried create never puts back a file the agent changed.
//! - Its pinned managed sandbox goes with the create (step 11.3,
//!   `sandboxes.rs`), so the record is pinned before the create.

use everruns_contracts::session_commands;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId};

use crate::domains::common::CommandError;
use everruns_core::session_file::InitialFile;

use super::{ControlPlane, agent_published, for_org, org_public, outbox};
use crate::domains::agent_definitions::key_for;
use crate::storage::DpSessionSummary;
use everruns_contracts::runtime::config_layer::AgentConfigOverlay;

/// Tries at the data plane before the create fails.
const TRIES: u32 = 3;

/// Whether `org_id` runs on a data plane.
pub async fn on_data_plane(cp: &ControlPlane, org_id: i64) -> anyhow::Result<bool> {
    Ok(cp.db.dp_org(org_id).await?.is_some())
}

/// A session's own settings as a layer over its agent, or `None` when it
/// has none. `capabilities` are the ones its creator named, not the
/// resolved set the record stores; starter files go with the create
/// separately ([`created`]).
pub fn session_layer(
    request: &crate::domains::sessions::types::CreateSessionRequest,
) -> Option<AgentConfigOverlay> {
    let layer = AgentConfigOverlay {
        system_prompt: request
            .system_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty()),
        capabilities: request.capabilities.clone(),
        tools: request.tools.clone(),
        mcp_servers: request.mcp_servers.clone(),
        network_access: request.network_access.clone(),
        max_iterations: request.max_iterations,
        parallel_tool_calls: request.parallel_tool_calls,
        ..AgentConfigOverlay::default()
    };
    let empty = layer.system_prompt.is_none()
        && layer.capabilities.is_empty()
        && layer.tools.is_empty()
        && layer.mcp_servers.is_empty()
        && layer.network_access.is_none()
        && layer.max_iterations.is_none()
        && layer.parallel_tool_calls.is_none();
    (!empty).then_some(layer)
}

/// The session's own declaration: its agent's layers with `layer` on top,
/// its MCP servers bound under the session's own key.
async fn session_definition(
    cp: &ControlPlane,
    org_id: i64,
    agent_key: &str,
    session: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    layer: AgentConfigOverlay,
) -> anyhow::Result<serde_json::Value> {
    let mut layers =
        crate::domains::agent_definitions::resolve(&cp.db, org_id, harness_id, agent_id).await?;
    layers.session = Some(layer);
    let declaration = crate::domains::agent_definitions::compile(&layers)?;
    let mut package = declaration.agent;
    let binding_key = format!("{agent_key}:{session}");
    super::mcp::prepare_layers(cp, org_id, &binding_key, agent_id, &layers, &mut package).await?;
    super::sandboxes::strip(&mut package);
    Ok(package)
}

/// Create the record `session`'s running session on `org_id`'s data plane.
/// A v1 org is left alone.
pub async fn created(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    initial_files: &[InitialFile],
    layer: Option<AgentConfigOverlay>,
) -> anyhow::Result<()> {
    if !on_data_plane(cp, org_id).await? {
        return Ok(());
    }
    let (key, data_plane) = create_on_plane(
        cp,
        org_id,
        session,
        harness_id,
        agent_id,
        initial_files,
        layer,
    )
    .await?;
    // Marks the session as the data plane's: its messages go there, never
    // to the platform's workers. The feed's summaries (version 1 and up)
    // update it.
    let now = chrono::Utc::now();
    cp.db
        .dp_upsert_session_summary(DpSessionSummary {
            session_id: &session.to_string(),
            org_id,
            data_plane_id: &data_plane,
            agent_id: &key,
            session_version: 0,
            title: "",
            status: "started",
            turns: 0,
            cost_micros: 0,
            created_at: now,
            last_activity_at: now,
        })
        .await?;
    Ok(())
}

/// Create `session` on `org_id`'s data plane, its agent delivered first
/// (retry-safe: the same id is the same session); its agent's key and the
/// data plane's id. Whether its messages go there is the caller's
/// ([`created`], `moves.rs`).
pub(crate) async fn create_on_plane(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    initial_files: &[InitialFile],
    layer: Option<AgentConfigOverlay>,
) -> anyhow::Result<(String, String)> {
    let (key, revision) = key_for(&cp.db, org_id, harness_id, agent_id).await?;
    agent_published(cp, &key, org_id, &revision).await?;
    let definition = match layer {
        Some(layer) => {
            Some(session_definition(cp, org_id, &key, session, harness_id, agent_id, layer).await?)
        }
        None => None,
    };
    // Sealed before the config is delivered below, so the data plane holds
    // the sandbox's key when the session first calls it.
    let sandbox = super::sandboxes::pinned(cp, org_id, session).await?;
    let memory = super::memory::mounts(cp, session).await?;
    let plane = for_org(cp, org_id).await?;
    let public = org_public(org_id);
    let mut last = None;
    for attempt in 0..TRIES {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt))).await;
        }
        // Up to date first: a no-op when the dispatcher already delivered.
        let org = cp
            .db
            .dp_org(org_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("org {org_id} left its data plane"))?;
        if org.delivered_version < org.version || !org.enabled {
            let epoch = cp.db.dp_epoch().await?;
            if let Err(error) = outbox::deliver(cp, &org, epoch).await {
                last = Some(error);
                continue;
            }
        }
        match plane
            .create_session(
                &public,
                &super::NewSession {
                    id: session,
                    agent_key: &key,
                    definition: definition.as_ref(),
                    initial_files,
                    sandbox: sandbox.as_ref(),
                    memory: &memory,
                },
            )
            .await
        {
            Ok(()) => {
                let data_plane = plane.id().unwrap_or_default().to_owned();
                return Ok((key, data_plane));
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .unwrap_or_else(|| anyhow::anyhow!("no attempt made"))
        .context("the data plane did not create the session"))
}

/// Whether `session` of `org_id` runs on a data plane (created there by
/// [`created`]). Decision: a message send is a hot path, so whether the
/// org has a data plane at all is cached for a few seconds; only a v2
/// org's sends pay for the session lookup. An org leaving its data plane
/// keeps its sessions there, which the session lookup still answers.
pub async fn runs_on_data_plane(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
) -> anyhow::Result<bool> {
    static ORGS: std::sync::OnceLock<moka::future::Cache<i64, bool>> = std::sync::OnceLock::new();
    let orgs = ORGS.get_or_init(|| {
        moka::future::Cache::builder()
            .max_capacity(100_000)
            .time_to_live(std::time::Duration::from_secs(5))
            .build()
    });
    let v2 = match orgs.get(&org_id).await {
        Some(v2) => v2,
        None => {
            let v2 = cp.db.dp_org_ever(org_id).await?;
            orgs.insert(org_id, v2).await;
            v2
        }
    };
    if !v2 {
        return Ok(false);
    }
    cp.db.dp_session_on_data_plane(&session.to_string()).await
}

/// An agent trigger's run, when its session may be on a data plane.
pub struct TriggerRun<'a> {
    pub org_id: i64,
    pub session: SessionId,
    /// Identifies the dispatch: a retry with the same key admits once.
    pub key: Option<String>,
    pub text: &'a str,
    pub origin: &'a everruns_core::message::MessageOrigin,
    /// The saved script the run runs instead of the model (Shell Code Mode
    /// D9), carried as the message's reserved script-run metadata; the
    /// cell's turn runs it as a worker's does.
    pub script: Option<&'a everruns_contracts::runtime::saved_scripts::ScriptRun>,
}

impl TriggerRun<'_> {
    /// Admit the run into its session on the org's data plane (step 11.4);
    /// `false` for a session the platform's workers run, which takes the
    /// message the ordinary way.
    pub async fn admit(&self, cp: &ControlPlane) -> Result<bool, CommandError> {
        if !runs_on_data_plane(cp, self.org_id, self.session).await? {
            return Ok(false);
        }
        let message = match &self.key {
            Some(key) => MessageId::from_uuid(uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("agent_trigger:{key}").as_bytes(),
            )),
            None => MessageId::new(),
        };
        let mut body =
            serde_json::json!({ "id": message, "text": self.text, "origin": self.origin });
        if let Some(run) = self.script {
            body["metadata"] = serde_json::json!({
                everruns_contracts::runtime::saved_scripts::SCRIPT_RUN_METADATA_KEY:
                    serde_json::to_value(run).map_err(anyhow::Error::from)?,
            });
        }
        for_org(cp, self.org_id)
            .await?
            .session_command(
                &org_public(self.org_id),
                self.session,
                &session_commands::MESSAGES,
                &body,
            )
            .await
            .map_err(|error| error.context("the data plane did not take the message"))?;
        Ok(true)
    }
}
