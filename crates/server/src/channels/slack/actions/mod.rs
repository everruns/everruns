//! Control-plane implementation of the Slack action seam (EVE-1024).
//!
//! The native `slack` capability names an action; this module resolves which
//! Slack channel the session belongs to, fetches that channel's `bot_token`,
//! and makes the call. The token stays in this process — see
//! [`everruns_contracts::slack_action`] for why the action travels instead.
//!
//! # Resolution
//!
//! `sessions.channel_id` is the authoritative channel binding. Mutable tags
//! cannot grant access to a bot. Resolution checks the session organization,
//! owning agent, channel type and liveness before decrypting credentials.

use std::sync::Arc;

use crate::domains::agent_channels::record::{AgentChannel, ChannelType};
use async_trait::async_trait;
use everruns_contracts::slack_action::{
    SlackAction, SlackActionError, SlackActionInvoker, SlackActionOutcome,
};
use everruns_contracts::typed_id::SessionId;
#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use tracing::{debug, warn};

use crate::storage::{EncryptionService, StorageBackend};
pub(super) use everruns_integrations::slack_channel::actions::action_error;
use everruns_integrations::slack_channel::actions::{
    add_reaction_scoped, lookup_user_scoped, share_message_files_scoped, update_message_scoped,
    upload_file_scoped,
};
use everruns_integrations::slack_channel::web_api::SLACK_API_BASE;

mod post_message;
pub(crate) use post_message::slack_target as signed_slack_target;

/// Performs Slack actions as one session's own channel bot.
///
/// Bound to one org and one session at construction, like
/// `platform_store(org_id, session_id)`: the tenant and session are not
/// arguments a caller can vary.
pub struct DbSlackActionInvoker {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    org_id: i64,
    session_id: SessionId,
    /// Slack Web API base. Overridden in tests to point at a local mock.
    api_base: String,
    egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
}

impl DbSlackActionInvoker {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        org_id: i64,
        session_id: SessionId,
    ) -> Self {
        Self {
            db,
            encryption,
            org_id,
            session_id,
            api_base: SLACK_API_BASE.to_string(),
            egress: None,
        }
    }

    fn with_egress(
        mut self,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Self {
        self.egress = egress
            .map(|service| crate::domains::messages::intake::org_egress(service, self.org_id));
        self
    }

    #[cfg(test)]
    fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Resolve the Slack endpoint and trusted conversation that created this session.
    async fn resolve_context(
        &self,
        input_message_id: Option<&str>,
    ) -> Result<SlackActionContext, SlackActionError> {
        let org = self.org_id;
        let session_id = self.session_id;
        let session = self
            .db
            .get_session(org, session_id)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .ok_or(SlackActionError::NoSlackSession)?;

        let (app_public_id, bound_channel) = self.resolve_action_channel(&session).await?;

        if !bound_channel.enabled || !bound_channel.status.is_live() {
            // A retired or drafted endpoint must not keep acting. Reported as
            // unavailable rather than as "not from Slack": the session did come
            // from Slack, the door is just shut.
            return Err(SlackActionError::ChannelUnavailable);
        }

        let config = bound_channel
            .slack_config()
            .ok_or(SlackActionError::ChannelUnavailable)?;
        if config.bot_token.trim().is_empty() {
            return Err(SlackActionError::NotConfigured);
        }

        if let Some(input_message_id) = input_message_id {
            return self
                .resolve_post_context(input_message_id, app_public_id, &bound_channel, config)
                .await;
        }

        let thread_context = self
            .db
            .get_session_key_value(
                session_id.uuid(),
                everruns_core::channel::THREAD_CONTEXT_KV_KEY,
            )
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .and_then(|row| everruns_core::channel::decode_thread_context(&row.value))
            .filter(|context| context.platform == "slack")
            .ok_or_else(|| {
                SlackActionError::InvalidArgument(
                    "the Slack conversation for this session is unavailable".to_string(),
                )
            })?;
        let channel = thread_context
            .platform_metadata
            .get("channel_id")
            .filter(|channel| !channel.trim().is_empty())
            .cloned()
            .ok_or_else(|| {
                SlackActionError::InvalidArgument(
                    "the Slack channel for this session is unavailable".to_string(),
                )
            })?;

        // THREAT[TM-SLACK-005]: action arguments are model-controlled. Bind them to
        // both trusted ingress metadata and any administrator-configured scope.
        if config.channel_id.as_deref().is_some_and(|id| id != channel) {
            return Err(SlackActionError::ChannelUnavailable);
        }

        Ok(SlackActionContext {
            installation: bound_channel.public_id.to_string(),
            team: config.team_id,
            bot_token: config.bot_token,
            channel,
            thread_ts: thread_context.thread_ref,
            channel_id: bound_channel.internal_id,
            channel_revision: bound_channel.updated_at,
        })
    }
}

struct SlackActionContext {
    installation: String,
    team: Option<String>,
    channel_id: uuid::Uuid,
    channel_revision: chrono::DateTime<chrono::Utc>,
    bot_token: String,
    channel: String,
    thread_ts: String,
}

impl SlackActionContext {
    fn delivery_context(
        &self,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> everruns_contracts::runtime::channel::DeliveryContext {
        let mut context = everruns_contracts::runtime::channel::DeliveryTarget::new(
            &self.channel,
            &self.thread_ts,
        )
        .context(&self.bot_token);
        context.extra.insert(
            everruns_integrations::slack_channel::bridge::INSTALLATION_ID.into(),
            self.installation.clone(),
        );
        if let Some(team) = &self.team {
            context.extra.insert(
                everruns_integrations::slack_channel::bridge::TEAM_ID.into(),
                team.clone(),
            );
        }
        context.egress = egress;
        context
    }

    fn authorize_channel(&self, channel: &str) -> Result<(), SlackActionError> {
        if channel != self.channel {
            return Err(SlackActionError::InvalidArgument(
                "Slack actions may only target this session's channel".to_string(),
            ));
        }
        Ok(())
    }

    fn authorize_thread(&self, thread_ts: Option<&str>) -> Result<(), SlackActionError> {
        if thread_ts != Some(self.thread_ts.as_str()) {
            return Err(SlackActionError::InvalidArgument(
                "Slack file uploads must target this session's thread".to_string(),
            ));
        }
        Ok(())
    }
}

/// A Slack API failure as the action tool reports it.
///
/// A function rather than `From`: both types live in other crates now.

#[async_trait]
impl SlackActionInvoker for DbSlackActionInvoker {
    async fn invoke(&self, action: SlackAction) -> Result<SlackActionOutcome, SlackActionError> {
        let kind = action.kind();
        let input_id = match &action {
            SlackAction::PostMessage {
                input_message_id, ..
            } => Some(input_message_id.as_str()),
            _ => None,
        };
        let context = self.resolve_context(input_id).await?;
        let provider_context = context.delivery_context(self.egress.clone());

        let outcome = match action {
            SlackAction::PostMessage {
                input_message_id,
                tool_call_id,
                text,
                files,
            } => {
                let outcome = post_message::post_message(
                    &self.api_base,
                    self.session_id,
                    &context,
                    &provider_context,
                    input_message_id,
                    tool_call_id,
                    text,
                )
                .await?;
                if !files.is_empty() {
                    // Same trusted conversation the text just went to.
                    share_message_files_scoped(&self.api_base, &provider_context, files)
                        .await
                        .map_err(|error| {
                            let confirmed = match &outcome {
                                SlackActionOutcome::MessagePosted { channel, timestamp } => {
                                    vec![format!("{channel}:{timestamp}")]
                                }
                                _ => vec![],
                            };
                            everruns_integrations::slack_channel::actions::with_confirmed(
                                error, confirmed,
                            )
                        })?;
                }
                outcome
            }
            SlackAction::AddReaction {
                channel,
                timestamp,
                name,
            } => {
                context.authorize_channel(&channel)?;
                match add_reaction_scoped(&self.api_base, &provider_context, &timestamp, &name)
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        if error.code() == Some("missing_scope") {
                            let observation = crate::storage::ObserveHealthIssue {
                                org_id: self.org_id,
                                channel_id: context.channel_id,
                                channel_revision: context.channel_revision,
                                status: "open".into(),
                                missing_scopes: vec!["reactions:write".into()],
                                error_code: Some("missing_scope".into()),
                                checked_at: chrono::Utc::now(),
                            };
                            if let Err(storage_error) =
                                self.db.observe_health_issue(observation).await
                            {
                                tracing::warn!(%storage_error,"Could not persist Slack permission issue");
                            }
                        }
                        if error.code() == Some("missing_scope") {
                            return Err(SlackActionError::Rejected(
                                "missing_scope: reconnect the Slack app and approve reactions:write to add reactions".into(),
                            ));
                        }
                        return Err(action_error(error));
                    }
                }
            }
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            } => {
                context.authorize_channel(&channel)?;
                update_message_scoped(&self.api_base, &provider_context, &timestamp, &text).await?
            }
            SlackAction::LookupUser { user_id } => {
                lookup_user_scoped(&self.api_base, &provider_context, &user_id).await?
            }
            SlackAction::UploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            } => {
                // THREAT[TM-SLACK-005]: no channel means the session's own
                // conversation, read from trusted ingress; a named one must
                // match it.
                let (channel, thread_ts) = match channel {
                    None => (
                        context.channel.clone(),
                        Some(context.thread_ts.clone()).filter(|ts| !ts.is_empty()),
                    ),
                    Some(channel) => {
                        context.authorize_channel(&channel)?;
                        context.authorize_thread(thread_ts.as_deref())?;
                        (channel, thread_ts)
                    }
                };
                let mut upload_context = provider_context.clone();
                upload_context.channel_id = channel;
                upload_context.thread_ref = thread_ts.unwrap_or_default();
                upload_file_scoped(
                    &self.api_base,
                    &upload_context,
                    &filename,
                    content,
                    initial_comment.as_deref(),
                )
                .await?
            }
        };

        debug!(
            session_id = %self.session_id,
            action = kind,
            "Performed Slack action as the channel bot"
        );
        Ok(outcome)
    }
}

// ============================================================================
// Individual actions
// ============================================================================

// ============================================================================
// Entry points
// ============================================================================

/// Build the in-process invoker for one session.
///
/// Lives here rather than in `direct_worker_adapters` so the adapter states
/// only that it has a route, and the construction stays next to the resolution
/// rules it depends on.
pub fn in_process_invoker(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<EncryptionService>>,
    egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    org_id: i64,
    session_id: SessionId,
) -> Arc<dyn SlackActionInvoker> {
    Arc::new(
        DbSlackActionInvoker::new(db.clone(), encryption.cloned(), org_id, session_id)
            .with_egress(egress),
    )
}

/// Serve one `InvokeSlackAction` RPC (EVE-1024).
///
/// The invoker resolves which Slack channel created the session and reads that
/// channel's `bot_token`, both org-scoped, so a session id from another tenant
/// resolves to nothing. The token is used here and never returned: the response
/// carries the action's outcome or a typed error.
///
/// A failed action is a successful RPC. The capability distinguishes "not a
/// Slack session" (a tool error the model should act on) from a transient fault
/// (an internal error), and a `Status` would flatten both into one transport
/// failure.
pub async fn serve_rpc(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<EncryptionService>>,
    egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    request: tonic::Request<everruns_internal_protocol::proto::InvokeSlackActionRequest>,
) -> Result<
    tonic::Response<everruns_internal_protocol::proto::InvokeSlackActionResponse>,
    tonic::Status,
> {
    use everruns_internal_protocol::proto;

    let req = request.into_inner();
    let session_id = SessionId::parse(&req.session_id)
        .map_err(|e| tonic::Status::invalid_argument(format!("invalid session_id: {e}")))?;
    let action: SlackAction = req
        .action
        .ok_or_else(|| tonic::Status::invalid_argument("missing action"))?
        .into();

    let invoker = in_process_invoker(db, encryption, egress, req.org_id, session_id);
    let result = match invoker.invoke(action).await {
        Ok(outcome) => outcome.into(),
        Err(error) => {
            if !matches!(error, SlackActionError::NoSlackSession) {
                // A non-Slack session is an ordinary configuration outcome and
                // would otherwise log one line per turn for any agent that has
                // the capability enabled broadly.
                warn!(
                    session_id = %session_id,
                    org_id = req.org_id,
                    error = %error,
                    "Slack action failed"
                );
            }
            let wire: proto::SlackActionError = error.into();
            proto::invoke_slack_action_response::Result::Error(wire)
        }
    };

    Ok(tonic::Response::new(proto::InvokeSlackActionResponse {
        result: Some(result),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateAgentChannelRow, UpdateChannelByIdRow};
    use crate::storage::{CreateHarnessRow, CreateSessionRow};
    use everruns_contracts::slack_action::SlackActionInvoker;
    use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId};
    use uuid::Uuid;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, ResponseTemplate};

    const ORG: i64 = 1;

    pub(super) struct Fixture {
        pub(super) db: Arc<StorageBackend>,
    }

    impl Fixture {
        pub(super) fn new() -> Self {
            Self {
                db: Arc::new(StorageBackend::test_database()),
            }
        }

        /// An endpoint must be owned by an agent, so every app needs one.
        pub(super) async fn seed_agent(&self, org_id: i64, harness_id: HarnessId) -> AgentId {
            use crate::storage::CreateAgentRow;
            let id = AgentId::new();
            self.db
                .create_agent_with_id(
                    org_id,
                    id,
                    CreateAgentRow {
                        completion_check: Default::default(),
                        localizations: Default::default(),
                        public_id: id.to_string(),
                        name: format!("a-{}", Uuid::now_v7()),
                        display_name: None,
                        description: None,
                        intro_markdown: None,
                        short_description: None,
                        starters: json!([]),
                        system_prompt: String::new(),
                        default_model_id: None,
                        harness_id,
                        tags: vec![],
                        initial_files: json!([]),
                        tools: json!([]),
                        mcp_servers: json!({}),
                        max_iterations: None,
                        network_access: None,
                        parallel_tool_calls: None,
                        max_output_tokens: None,
                        communication: Default::default(),
                        environments: None,
                        is_built_in: false,
                    },
                )
                .await
                .expect("create agent")
                .expect("agent should be created");
            id
        }

        pub(super) async fn seed_harness(&self) -> HarnessId {
            self.db
                .create_harness(
                    ORG,
                    CreateHarnessRow {
                        localizations: Default::default(),
                        name: format!("h-{}", Uuid::now_v7()),
                        display_name: None,
                        icon: None,
                        description: None,
                        intro_markdown: None,
                        short_description: None,
                        starters: json!([]),
                        system_prompt: Some("test".to_string()),
                        parent_harness_id: None,
                        default_model_id: None,
                        tags: vec![],
                        initial_files: json!([]),
                        mcp_servers: json!({}),
                        network_access: None,
                        embedder_metadata: json!({}),
                        is_built_in: false,
                    },
                )
                .await
                .expect("create harness")
                .id
        }

        /// Seed an app with one endpoint of `channel_type`, live and configured.
        pub(super) async fn seed_channel(
            &self,
            org_id: i64,
            channel_type: &str,
            bot_token: &str,
        ) -> (Uuid, Uuid, String) {
            let harness_id = self.seed_harness().await;
            let agent_id = self.seed_agent(org_id, harness_id).await;
            let public_id = format!("appchan_{}", Uuid::now_v7().simple());
            let channel = self
                .db
                .create_agent_channel(
                    org_id,
                    CreateAgentChannelRow {
                        agent_id: agent_id.uuid(),
                        status: "live".into(),
                        virtual_user_id: None,
                        owner_principal_id: PrincipalId::from_seed(1).uuid(),
                        resolved_owner_user_id: None,
                        public_id: public_id.clone(),
                        channel_type: channel_type.to_string(),
                        channel_config: json!({
                            "signing_secret": "s",
                            "bot_token": bot_token,
                        }),
                        channel_config_encrypted: None,
                        auth: None,
                        auth_encrypted: None,
                        enabled: true,
                    },
                )
                .await
                .expect("create channel");

            // A new channel starts `draft`; publish it so it accepts traffic.
            self.db
                .update_channel_by_id(
                    channel.channel_id,
                    UpdateChannelByIdRow {
                        status: Some("live".to_string()),
                        ..Default::default()
                    },
                )
                .await
                .expect("publish channel");

            (agent_id.uuid(), channel.channel_id, public_id)
        }

        pub(super) async fn seed_session(
            &self,
            org_id: i64,
            owner_agent_id: Option<Uuid>,
            channel_id: Option<Uuid>,
            tags: Vec<String>,
        ) -> SessionId {
            self.seed_session_for_agent(org_id, owner_agent_id, channel_id, tags, None)
                .await
        }

        pub(super) async fn seed_session_for_agent(
            &self,
            org_id: i64,
            owner_agent_id: Option<Uuid>,
            channel_id: Option<Uuid>,
            tags: Vec<String>,
            agent_id: Option<AgentId>,
        ) -> SessionId {
            let harness_id = self.seed_harness().await;
            let session = self
                .db
                .create_session(CreateSessionRow {
                    mcp_creator: None,
                    playground_user_id: None,
                    source: crate::domains::sessions::record::SessionSource::Api,
                    workspace_id: None,
                    org_id,
                    channel_id,
                    trigger_id: None,
                    harness_id: Some(harness_id),
                    agent_id: agent_id.or(owner_agent_id.map(AgentId::from_uuid)),
                    agent_revision: None,
                    virtual_user_id: None,
                    owner_principal_id: PrincipalId::from_seed(1),
                    resolved_owner_user_id: None,
                    title: Some("slack-actions-test".to_string()),
                    locale: None,
                    tags,
                    model_id: None,
                    capabilities: json!([]),
                    tools: json!([]),
                    mcp_servers: json!({}),
                    system_prompt: None,
                    initial_files: json!([]),
                    hints: None,
                    network_access: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    blueprint_id: None,
                    blueprint_config: None,
                    parent_session_id: None,
                    budget_root_session_id: None,
                })
                .await
                .expect("create session");
            let mut context = everruns_core::channel::ThreadContext::new("1.2", "slack");
            context
                .platform_metadata
                .insert("channel_id".to_string(), "C1".to_string());
            self.db
                .upsert_session_key_value(crate::storage::UpsertSessionKeyValue {
                    session_id: session.id,
                    key: everruns_core::channel::THREAD_CONTEXT_KV_KEY.to_string(),
                    value: everruns_core::channel::encode_thread_context(&context)
                        .expect("encode context"),
                })
                .await
                .expect("store thread context");
            session.id
        }

        pub(super) fn invoker(&self, org_id: i64, session_id: SessionId) -> DbSlackActionInvoker {
            DbSlackActionInvoker::new(self.db.clone(), None, org_id, session_id)
        }
    }

    fn add_reaction_action() -> SlackAction {
        SlackAction::AddReaction {
            channel: "C1".to_string(),
            timestamp: "1.2".to_string(),
            name: "eyes".to_string(),
        }
    }

    mod channel_authority_tests;

    /// The case the issue calls out: an agent with the capability enabled,
    /// running from the API or a schedule, must fail closed.
    #[tokio::test]
    async fn a_session_with_no_app_has_no_channel_to_act_as() {
        let fixture = Fixture::new();
        let session_id = fixture.seed_session(ORG, None, None, vec![]).await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("a non-app session must not resolve a channel");

        assert!(matches!(error, SlackActionError::NoSlackSession));
    }

    #[tokio::test]
    async fn an_unknown_session_fails_closed() {
        let fixture = Fixture::new();

        let error = fixture
            .invoker(ORG, SessionId::new())
            .invoke(add_reaction_action())
            .await
            .expect_err("an unknown session must not resolve a channel");

        assert!(matches!(error, SlackActionError::NoSlackSession));
    }

    /// An app session whose channel is not a Slack channel must not fall
    /// through to a Slack sibling.
    #[tokio::test]
    async fn a_non_slack_channel_does_not_resolve() {
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "schedule", "xoxb-nope").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("a schedule channel is not a Slack channel");

        assert!(matches!(error, SlackActionError::NoSlackSession));
    }

    /// Org scoping: the session read is org-scoped, so another tenant's org id
    /// resolves to nothing rather than to this tenant's bot.
    #[tokio::test]
    async fn another_orgs_id_does_not_reach_this_sessions_channel() {
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG + 1, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("a foreign org must not resolve this session");

        assert!(matches!(error, SlackActionError::NoSlackSession));
    }

    #[tokio::test]
    async fn a_channel_without_a_bot_token_is_not_configured() {
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) = fixture.seed_channel(ORG, "slack", "").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("an unconfigured channel has no token to act with");

        assert!(matches!(error, SlackActionError::NotConfigured));
    }

    #[tokio::test]
    async fn a_disabled_channel_stops_acting() {
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        fixture
            .db
            .update_channel_by_id(
                channel_id,
                UpdateChannelByIdRow {
                    status: Some("disabled".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("disable channel");
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("a disabled channel must not keep acting");

        assert!(matches!(error, SlackActionError::ChannelUnavailable));
    }

    #[tokio::test]
    async fn endpoint_channel_scope_must_match_the_ingress_channel() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        fixture
            .db
            .update_channel_by_id(
                channel_id,
                UpdateChannelByIdRow {
                    channel_config: Some(json!({
                        "signing_secret": "s",
                        "bot_token": "xoxb-secret",
                        "channel_id": "C_CONFIGURED",
                    })),
                    ..Default::default()
                },
            )
            .await
            .expect("scope channel");
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(add_reaction_action())
            .await
            .expect_err("a stale or inconsistent ingress channel must fail closed");

        assert!(matches!(error, SlackActionError::ChannelUnavailable));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// Mutable transport tags cannot manufacture channel authority. The
    /// migration backfills only unambiguous reserved provenance.
    #[tokio::test]
    async fn routing_tag_without_channel_fk_cannot_authorize_actions() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        let fixture = Fixture::new();
        let (owner_agent_id, _, channel_public_id) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(
                ORG,
                Some(owner_agent_id),
                None,
                vec![format!("slack:channel:{channel_public_id}")],
            )
            .await;
        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(add_reaction_action())
            .await;
        assert!(matches!(outcome, Err(SlackActionError::NoSlackSession)));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// An app session with neither the FK nor the tag names no channel, and
    /// must not pick whichever Slack channel the app happens to carry.
    #[tokio::test]
    async fn an_app_session_with_no_channel_reference_fails_closed() {
        let fixture = Fixture::new();
        let (owner_agent_id, _, _) = fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), None, vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("no channel reference must not fall back to a sibling");

        assert!(matches!(error, SlackActionError::NoSlackSession));
    }

    /// The whole point of EVE-1024's "Done when": react to the triggering
    /// message using the channel's own bot token and no extra credential.
    #[tokio::test]
    async fn a_slack_session_reacts_with_the_channels_own_token() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer xoxb-channel-secret",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) = fixture
            .seed_channel(ORG, "slack", "xoxb-channel-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(add_reaction_action())
            .await
            .expect("the channel's own token must be used");

        assert!(matches!(
            outcome,
            SlackActionOutcome::ReactionAdded {
                already_reacted: false
            }
        ));
    }

    mod health_tests;
    mod reaction_tests;

    #[tokio::test]
    async fn an_alternate_channel_is_rejected_without_a_slack_request() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(SlackAction::AddReaction {
                channel: "C_PRIVILEGED".to_string(),
                timestamp: "9.9".to_string(),
                name: "eyes".to_string(),
            })
            .await
            .expect_err("a model-chosen alternate channel must be rejected");

        assert!(matches!(error, SlackActionError::InvalidArgument(_)));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A duplicate reaction satisfies the agent's intent, so it is reported as
    /// a success that says so rather than an error the model has to interpret.
    #[tokio::test]
    async fn a_duplicate_reaction_is_a_success() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": false, "error": "already_reacted" })),
            )
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(add_reaction_action())
            .await
            .expect("already_reacted must not surface as a failure");

        assert!(matches!(
            outcome,
            SlackActionOutcome::ReactionAdded {
                already_reacted: true
            }
        ));
    }

    /// Slack's own backpressure advice must survive to the model, not be
    /// flattened into a generic failure (EVE-968's reasoning).
    #[tokio::test]
    async fn a_rate_limit_keeps_slacks_retry_advice() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/chat.update"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "37")
                    .set_body_json(json!({ "ok": false, "error": "ratelimited" })),
            )
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(SlackAction::UpdateMessage {
                channel: "C1".to_string(),
                timestamp: "1.2".to_string(),
                text: "done".to_string(),
            })
            .await
            .expect_err("a 429 must not read as success");

        assert!(matches!(
            error,
            SlackActionError::RateLimited {
                retry_after_secs: Some(37)
            }
        ));
    }

    /// `users.info` carries email, phone, and title. An agent needs a name, so
    /// only the addressing fields are forwarded.
    #[tokio::test]
    async fn lookup_user_forwards_only_the_addressing_fields() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "user": {
                    "id": "U1",
                    "is_bot": false,
                    "tz": "Europe/Kyiv",
                    "profile": {
                        "display_name": "ada",
                        "real_name": "Ada Lovelace",
                        "email": "ada@example.com",
                        "phone": "+100000000",
                        "title": "Engineer",
                    }
                }
            })))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(SlackAction::LookupUser {
                user_id: "U1".to_string(),
            })
            .await
            .expect("lookup must succeed");

        let SlackActionOutcome::User {
            user_id,
            display_name,
            real_name,
            is_bot,
            tz,
        } = outcome
        else {
            panic!("expected a user outcome, got {outcome:?}");
        };
        assert_eq!(user_id, "U1");
        assert_eq!(display_name.as_deref(), Some("ada"));
        assert_eq!(real_name.as_deref(), Some("Ada Lovelace"));
        assert!(!is_bot);
        assert_eq!(tz.as_deref(), Some("Europe/Kyiv"));
        // The outcome type has no field that could carry the rest, which is the
        // point: there is nowhere for email or phone to leak to.
    }

    /// A permanent Slack refusal reaches the model as a tool error it can act
    /// on, not an internal fault.
    #[tokio::test]
    async fn a_permanent_slack_refusal_is_a_tool_error() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": false, "error": "channel_not_found" })),
            )
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(add_reaction_action())
            .await
            .expect_err("channel_not_found must not read as success");

        assert!(matches!(error, SlackActionError::Rejected(_)));
        assert!(error.is_tool_error());
    }

    #[tokio::test]
    async fn an_empty_upload_is_rejected_before_any_request() {
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        // No mock server: reaching Slack at all would fail the test with a
        // transient error instead of the expected invalid argument.
        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base("http://127.0.0.1:1".to_string())
            .invoke(SlackAction::UploadFile {
                channel: Some("C1".to_string()),
                thread_ts: Some("1.2".to_string()),
                filename: "empty.txt".to_string(),
                content: Vec::new(),
                initial_comment: None,
            })
            .await
            .expect_err("an empty upload must be rejected");

        assert!(matches!(error, SlackActionError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn upload_file_walks_the_external_upload_flow() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/files.getUploadURLExternal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "upload_url": "https://files.slack.com/upload-here",
                "file_id": "F1",
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/upload-here"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/files.completeUploadExternal"))
            .and(body_partial_json(
                json!({ "blocks": [{ "type": "markdown" }] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "files": [{ "id": "F1", "permalink": "https://slack.example/F1" }],
            })))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .with_egress(Some(
                crate::channels::slack::events::tests_support::upload_egress(&server.uri()),
            ))
            .invoke(SlackAction::UploadFile {
                channel: Some("C1".to_string()),
                thread_ts: Some("1.2".to_string()),
                filename: "report.md".to_string(),
                content: b"hello".to_vec(),
                initial_comment: Some("**here** it is".to_string()),
            })
            .await
            .expect("the upload flow must complete");

        let SlackActionOutcome::FileUploaded { file_id, permalink } = outcome else {
            panic!("expected a file outcome, got {outcome:?}");
        };
        assert_eq!(file_id, "F1");
        assert_eq!(permalink.as_deref(), Some("https://slack.example/F1"));
    }

    #[tokio::test]
    async fn an_upload_without_the_session_thread_is_rejected_before_slack() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .with_egress(Some(
                crate::channels::slack::events::tests_support::upload_egress(&server.uri()),
            ))
            .invoke(SlackAction::UploadFile {
                channel: Some("C1".to_string()),
                thread_ts: None,
                filename: "report.md".to_string(),
                content: b"hello".to_vec(),
                initial_comment: Some("post me".to_string()),
            })
            .await
            .expect_err("a channel-level upload must be rejected");

        assert!(matches!(error, SlackActionError::InvalidArgument(_)));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A 4xx on the one-time upload URL cannot be retried, so it must not be
    /// classified as backpressure.
    #[tokio::test]
    async fn a_rejected_upload_url_is_not_treated_as_transient() {
        let server = crate::channels::slack::events::tests_support::mock_provider().await;
        Mock::given(method("POST"))
            .and(path("/files.getUploadURLExternal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "upload_url": "https://files.slack.com/upload-here",
                "file_id": "F1",
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/upload-here"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (owner_agent_id, channel_id, _) =
            fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
        let session_id = fixture
            .seed_session(ORG, Some(owner_agent_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .with_egress(Some(
                crate::channels::slack::events::tests_support::upload_egress(&server.uri()),
            ))
            .invoke(SlackAction::UploadFile {
                channel: Some("C1".to_string()),
                thread_ts: Some("1.2".to_string()),
                filename: "report.md".to_string(),
                content: b"hello".to_vec(),
                initial_comment: None,
            })
            .await
            .expect_err("a 400 on the upload URL must not read as success");

        assert!(matches!(error, SlackActionError::Rejected(_)));
    }
}

#[cfg(test)]
mod post_message_tests;
