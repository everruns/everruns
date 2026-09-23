//! Control-plane implementation of the Slack action seam (EVE-1024).
//!
//! The native `slack` capability names an action; this module resolves which
//! Slack channel the session belongs to, fetches that channel's `bot_token`,
//! and makes the call. The token stays in this process — see
//! [`everruns_platform::slack_action`] for why the action travels instead.
//!
//! # Resolution
//!
//! `sessions.channel_id` is the authoritative answer to "which door did this
//! session come through" (EVE-1004), so it is preferred over the
//! `slack:endpoint:{id}` routing tag, which is mutable. The tag is the fallback
//! for app-channel sessions that predate the FK backfill.
//!
//! Resolving through the channel rather than the app matters: since EVE-1008
//! the channel owns Slack bot identity, so an app with two Slack channels has
//! two different bots, and `App::slack_channel()` would return whichever comes
//! first.
//!
//! Org scoping is structural rather than a check bolted on: the session read is
//! `get_session(org_id, session_id)` and the app read is
//! `get_by_internal_id(.., org_id, ..)`, so a session id from another tenant
//! resolves to nothing rather than to another tenant's bot.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_platform::slack_action::{
    SlackAction, SlackActionError, SlackActionInvoker, SlackActionOutcome,
};
use everruns_platform::{App, AppChannel, ChannelType};
use everruns_provider::typed_id::SessionId;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::slack_api::{SLACK_API_BASE, slack_api_call};
use crate::slack_api_error::{SlackApiError, parse_retry_after};
use crate::storage::{EncryptionService, StorageBackend};

/// Routing-tag prefix stamped on Slack-originated sessions by
/// `slack_events::build_session_tags`. The `endpoint` spelling is a persisted
/// routing key kept for compatibility across the Endpoint -> Channel rename.
const SLACK_AGENT_CHANNEL_TAG_PREFIX: &str = "slack:endpoint:";

/// Ceiling on an upload's byte count.
///
/// THREAT[TM-DOS-031]: the capability caps the model-chosen argument before it
/// crosses the seam, but this side must not trust that a caller did — a second
/// implementation of the seam, or a replayed request, would not. Kept equal to
/// the capability's cap so a legitimate call never hits only one of them.
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

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
        }
    }

    #[cfg(test)]
    fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Resolve the Slack channel that created this session, and its bot token.
    async fn resolve_bot_token(&self) -> Result<String, SlackActionError> {
        let org = self.org_id;
        let session_id = self.session_id;
        let session = self
            .db
            .get_session(org, session_id)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .ok_or(SlackActionError::NoSlackSession)?;

        let Some(app_internal_id) = session.app_id else {
            // No owning app bundle: an API-, schedule-, or user-created
            // session. There is nothing to act as.
            return Err(SlackActionError::NoSlackSession);
        };

        // Decide which channel before loading the app, so the tag fallback and
        // the FK agree on a single target.
        let channel_selector = match session.channel_id {
            Some(internal_id) => ChannelSelector::Internal(internal_id),
            None => session
                .tags
                .iter()
                .find_map(|tag| tag.strip_prefix(SLACK_AGENT_CHANNEL_TAG_PREFIX))
                .map(|public_id| ChannelSelector::Public(public_id.to_string()))
                .ok_or(SlackActionError::NoSlackSession)?,
        };

        let app = crate::domains::apps::queries::get_by_internal_id(
            &self.db,
            self.encryption.as_ref(),
            org,
            app_internal_id,
        )
        .await
        .map_err(|e| SlackActionError::Transient(e.to_string()))?
        .ok_or(SlackActionError::ChannelUnavailable)?;

        let channel = select_slack_channel(&app, &channel_selector)
            .ok_or(SlackActionError::NoSlackSession)?;

        if !channel.status.is_live() {
            // A retired or drafted channel must not keep acting. Reported as
            // unavailable rather than as "not from Slack": the session did come
            // from Slack, the door is just shut.
            return Err(SlackActionError::ChannelUnavailable);
        }

        let config = channel
            .slack_config()
            .ok_or(SlackActionError::ChannelUnavailable)?;
        if config.bot_token.trim().is_empty() {
            return Err(SlackActionError::NotConfigured);
        }
        Ok(config.bot_token)
    }
}

/// Which channel on the app a session belongs to.
enum ChannelSelector {
    /// From `sessions.channel_id`, the authoritative FK (EVE-1004).
    Internal(uuid::Uuid),
    /// From the `slack:endpoint:{id}` routing tag, for pre-backfill sessions.
    Public(String),
}

/// Pick the Slack channel `selector` names, or `None` when it names none.
///
/// Requires `ChannelType::Slack` even when the id matches: an id that resolves
/// to a non-Slack channel means the session came through another channel, and
/// falling through to a sibling Slack channel would be exactly the
/// wrong-bot bug that resolving by channel exists to prevent.
fn select_slack_channel<'a>(app: &'a App, selector: &ChannelSelector) -> Option<&'a AppChannel> {
    let channel = app.channels.iter().find(|channel| match selector {
        ChannelSelector::Internal(internal_id) => channel.internal_id == *internal_id,
        ChannelSelector::Public(public_id) => &channel.public_id.to_string() == public_id,
    })?;
    (channel.channel_type == ChannelType::Slack).then_some(channel)
}

impl From<SlackApiError> for SlackActionError {
    fn from(error: SlackApiError) -> Self {
        match error {
            SlackApiError::RateLimited { retry_after } => SlackActionError::RateLimited {
                retry_after_secs: retry_after.map(|d| d.as_secs()),
            },
            SlackApiError::Permanent(message) => SlackActionError::Rejected(message),
            SlackApiError::Transient(message) => SlackActionError::Transient(message),
        }
    }
}

#[async_trait]
impl SlackActionInvoker for DbSlackActionInvoker {
    async fn invoke(&self, action: SlackAction) -> Result<SlackActionOutcome, SlackActionError> {
        let kind = action.kind();
        let bot_token = self.resolve_bot_token().await?;

        let outcome = match action {
            SlackAction::AddReaction {
                channel,
                timestamp,
                name,
            } => add_reaction(&self.api_base, &bot_token, &channel, &timestamp, &name).await?,
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            } => update_message(&self.api_base, &bot_token, &channel, &timestamp, &text).await?,
            SlackAction::LookupUser { user_id } => {
                lookup_user(&self.api_base, &bot_token, &user_id).await?
            }
            SlackAction::UploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            } => {
                upload_file(
                    &self.api_base,
                    &bot_token,
                    &channel,
                    thread_ts.as_deref(),
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

async fn add_reaction(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    timestamp: &str,
    name: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    let payload = json!({ "channel": channel, "timestamp": timestamp, "name": name });
    match slack_api_call(api_base, bot_token, "reactions.add", payload).await {
        Ok(_) => Ok(SlackActionOutcome::ReactionAdded {
            already_reacted: false,
        }),
        // The agent's intent — that emoji is on that message — is satisfied
        // either way, so a duplicate is a success that says so rather than an
        // error the model has to interpret.
        Err(error) if error.code() == Some("already_reacted") => {
            Ok(SlackActionOutcome::ReactionAdded {
                already_reacted: true,
            })
        }
        Err(error) => Err(error.into()),
    }
}

async fn update_message(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    timestamp: &str,
    text: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    // Mirrors the delivery path's rendering choice: `markdown` blocks, so agent
    // output reaches Slack as the Markdown it actually is rather than being
    // reinterpreted as the much smaller `mrkdwn` dialect. `text` is still sent
    // as the notification/fallback string.
    let payload = json!({
        "channel": channel,
        "ts": timestamp,
        "text": text,
        "blocks": [{ "type": "markdown", "text": text }],
    });
    slack_api_call(api_base, bot_token, "chat.update", payload).await?;
    Ok(SlackActionOutcome::MessageUpdated {
        channel: channel.to_string(),
        timestamp: timestamp.to_string(),
    })
}

async fn lookup_user(
    api_base: &str,
    bot_token: &str,
    user_id: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    let body = slack_api_call(
        api_base,
        bot_token,
        "users.info",
        json!({ "user": user_id }),
    )
    .await?;
    let user = body.get("user").ok_or_else(|| {
        SlackActionError::Transient("Slack users.info returned no user".to_string())
    })?;
    let profile = user.get("profile");

    // Only the fields an agent needs to address someone. `users.info` also
    // carries email, phone, and title; forwarding those would widen what the
    // model sees for no gain.
    Ok(SlackActionOutcome::User {
        user_id: user_id.to_string(),
        display_name: profile
            .and_then(|p| p.get("display_name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        real_name: profile
            .and_then(|p| p.get("real_name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        is_bot: user.get("is_bot").and_then(Value::as_bool).unwrap_or(false),
        tz: user
            .get("tz")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

/// Share a file, via the three-step external upload flow.
///
/// `files.upload` is retired, so this is `files.getUploadURLExternal` for a
/// one-time URL, a raw PUT/POST of the bytes to it, then
/// `files.completeUploadExternal` to place the file in the conversation. Only
/// the first and last speak the `ok`/`error` envelope; the middle step is a
/// plain HTTP upload whose failures are classified here.
async fn upload_file(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: Option<&str>,
    filename: &str,
    content: Vec<u8>,
    initial_comment: Option<&str>,
) -> Result<SlackActionOutcome, SlackActionError> {
    if content.is_empty() {
        return Err(SlackActionError::InvalidArgument(
            "file content must not be empty".to_string(),
        ));
    }
    if content.len() > MAX_UPLOAD_BYTES {
        return Err(SlackActionError::InvalidArgument(format!(
            "file content is {} bytes, over the {MAX_UPLOAD_BYTES} byte limit",
            content.len()
        )));
    }

    let reserve = slack_api_call(
        api_base,
        bot_token,
        "files.getUploadURLExternal",
        json!({ "filename": filename, "length": content.len() }),
    )
    .await?;

    let upload_url = reserve
        .get("upload_url")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SlackActionError::Transient(
                "Slack files.getUploadURLExternal returned no upload_url".to_string(),
            )
        })?;
    let file_id = reserve
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SlackActionError::Transient(
                "Slack files.getUploadURLExternal returned no file_id".to_string(),
            )
        })?
        .to_string();

    let client = reqwest::Client::new();
    let response = client
        .post(upload_url)
        .body(content)
        .send()
        .await
        .map_err(|e| SlackActionError::Transient(e.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SlackActionError::RateLimited {
                retry_after_secs: retry_after.map(|d| d.as_secs()),
            });
        }
        // 4xx on the one-time URL means the reservation no longer applies;
        // retrying the same URL cannot help, so it is reported as a rejection
        // rather than something to back off on.
        return Err(if status.is_client_error() {
            SlackActionError::Rejected(format!("Slack rejected the file upload ({status})"))
        } else {
            SlackActionError::Transient(format!("Slack file upload failed ({status})"))
        });
    }

    let mut file_entry = json!({ "id": file_id });
    if let Some(comment) = initial_comment {
        // Slack takes the comment per file on `completeUploadExternal`, not on
        // the request as a whole.
        file_entry["title"] = json!(filename);
        let mut payload = json!({
            "files": [file_entry],
            "channel_id": channel,
            "initial_comment": comment,
        });
        if let Some(thread_ts) = thread_ts {
            payload["thread_ts"] = json!(thread_ts);
        }
        return complete_upload(api_base, bot_token, payload, file_id).await;
    }

    let mut payload = json!({ "files": [file_entry], "channel_id": channel });
    if let Some(thread_ts) = thread_ts {
        payload["thread_ts"] = json!(thread_ts);
    }
    complete_upload(api_base, bot_token, payload, file_id).await
}

async fn complete_upload(
    api_base: &str,
    bot_token: &str,
    payload: Value,
    file_id: String,
) -> Result<SlackActionOutcome, SlackActionError> {
    let body = slack_api_call(api_base, bot_token, "files.completeUploadExternal", payload).await?;
    let permalink = body
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.get("permalink"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if permalink.is_none() {
        // Not fatal: the file is shared, we just cannot link to it.
        warn!(file_id = %file_id, "Slack completeUploadExternal returned no permalink");
    }
    Ok(SlackActionOutcome::FileUploaded { file_id, permalink })
}

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
    org_id: i64,
    session_id: SessionId,
) -> Arc<dyn SlackActionInvoker> {
    Arc::new(DbSlackActionInvoker::new(
        db.clone(),
        encryption.cloned(),
        org_id,
        session_id,
    ))
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

    let invoker = in_process_invoker(db, encryption, req.org_id, session_id);
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
    use crate::storage::models::{
        CreateAppChannelRow, CreateAppRow, CreateHarnessRow, CreateSessionRow, UpdateAppChannel,
    };
    use everruns_platform::slack_action::SlackActionInvoker;
    use everruns_provider::typed_id::{AgentId, HarnessId, PrincipalId};
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ORG: i64 = 1;

    struct Fixture {
        db: Arc<StorageBackend>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                db: Arc::new(StorageBackend::in_memory()),
            }
        }

        /// A channel must be owned by an agent, so every app needs one.
        async fn seed_agent(&self, org_id: i64, harness_id: HarnessId) -> AgentId {
            use crate::storage::models::CreateAgentRow;
            let id = AgentId::new();
            self.db
                .create_agent_with_id(
                    org_id,
                    id,
                    CreateAgentRow {
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
                        is_built_in: false,
                    },
                )
                .await
                .expect("create agent")
                .expect("agent should be created");
            id
        }

        async fn seed_harness(&self) -> HarnessId {
            self.db
                .create_harness(
                    ORG,
                    CreateHarnessRow {
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

        /// Seed an app with one channel of `channel_type`, live and configured.
        async fn seed_app_with_channel(
            &self,
            org_id: i64,
            channel_type: &str,
            bot_token: &str,
        ) -> (Uuid, Uuid, String) {
            let harness_id = self.seed_harness().await;
            let agent_id = self.seed_agent(org_id, harness_id).await;
            let app = self
                .db
                .create_app(
                    org_id,
                    CreateAppRow {
                        public_id: format!("app_{}", Uuid::now_v7().simple()),
                        name: "slack app".to_string(),
                        description: None,
                        harness_id: harness_id.uuid(),
                        agent_id: Some(agent_id.uuid()),
                        agent_version_policy: "pinned".to_string(),
                        agent_version_id: None,
                        agent_identity_id: None,
                        owner_principal_id: PrincipalId::from_seed(1),
                        resolved_owner_user_id: None,
                        channel_type: None,
                        channel_config: json!({}),
                        channel_config_encrypted: None,
                    },
                )
                .await
                .expect("create app");

            let public_id = format!("appchan_{}", Uuid::now_v7().simple());
            let channel = self
                .db
                .create_app_channel(
                    app.id,
                    CreateAppChannelRow {
                        public_id: public_id.clone(),
                        channel_type: channel_type.to_string(),
                        channel_config: json!({
                            "signing_secret": "s",
                            "bot_token": bot_token,
                        }),
                        channel_config_encrypted: None,
                        auth: None,
                        auth_encrypted: None,
                        durable_schedule_id: None,
                        enabled: true,
                    },
                )
                .await
                .expect("create channel");

            // A new channel starts `draft`; publish it so it accepts traffic.
            self.db
                .update_app_channel(
                    channel.id,
                    UpdateAppChannel {
                        status: Some("live".to_string()),
                        ..Default::default()
                    },
                )
                .await
                .expect("publish channel");

            (app.id, channel.id, public_id)
        }

        async fn seed_session(
            &self,
            org_id: i64,
            app_id: Option<Uuid>,
            channel_id: Option<Uuid>,
            tags: Vec<String>,
        ) -> SessionId {
            let harness_id = self.seed_harness().await;
            self.db
                .create_session(CreateSessionRow {
                    source: everruns_platform::SessionSource::Api,
                    workspace_id: None,
                    org_id,
                    app_id,
                    channel_id,
                    harness_id: Some(harness_id),
                    agent_id: None,
                    agent_version_id: None,
                    agent_config_hash: None,
                    agent_identity_id: None,
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
                .expect("create session")
                .id
        }

        fn invoker(&self, org_id: i64, session_id: SessionId) -> DbSlackActionInvoker {
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "schedule", "xoxb-nope")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let (app_id, channel_id, _) = fixture.seed_app_with_channel(ORG, "slack", "").await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        fixture
            .db
            .update_app_channel(
                channel_id,
                UpdateAppChannel {
                    status: Some("disabled".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("disable channel");
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .invoke(add_reaction_action())
            .await
            .expect_err("a disabled channel must not keep acting");

        assert!(matches!(error, SlackActionError::ChannelUnavailable));
    }

    /// Pre-backfill sessions carry no `channel_id`, so the routing tag is the
    /// fallback (EVE-1004).
    #[tokio::test]
    async fn the_routing_tag_resolves_a_session_without_the_channel_fk() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (app_id, _, channel_public_id) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(
                ORG,
                Some(app_id),
                None,
                vec![format!("slack:endpoint:{channel_public_id}")],
            )
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(
                format!("{}/", server.uri())
                    .trim_end_matches('/')
                    .to_string(),
            )
            .invoke(add_reaction_action())
            .await
            .expect("the routing tag must resolve the channel");

        assert!(matches!(
            outcome,
            SlackActionOutcome::ReactionAdded {
                already_reacted: false
            }
        ));
    }

    /// An app session with neither the FK nor the tag names no channel, and
    /// must not pick whichever Slack channel the app happens to carry.
    #[tokio::test]
    async fn an_app_session_with_no_channel_reference_fails_closed() {
        let fixture = Fixture::new();
        let (app_id, _, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture.seed_session(ORG, Some(app_id), None, vec![]).await;

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
        let server = MockServer::start().await;
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-channel-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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

    /// A duplicate reaction satisfies the agent's intent, so it is reported as
    /// a success that says so rather than an error the model has to interpret.
    #[tokio::test]
    async fn a_duplicate_reaction_is_a_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": false, "error": "already_reacted" })),
            )
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let server = MockServer::start().await;
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let server = MockServer::start().await;
        Mock::given(method("POST"))
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": false, "error": "channel_not_found" })),
            )
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
            .await;

        // No mock server: reaching Slack at all would fail the test with a
        // transient error instead of the expected invalid argument.
        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base("http://127.0.0.1:1".to_string())
            .invoke(SlackAction::UploadFile {
                channel: "C1".to_string(),
                thread_ts: None,
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
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/files.getUploadURLExternal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "upload_url": format!("{}/upload-here", server.uri()),
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
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "files": [{ "id": "F1", "permalink": "https://slack.example/F1" }],
            })))
            .mount(&server)
            .await;

        let fixture = Fixture::new();
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
            .await;

        let outcome = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(SlackAction::UploadFile {
                channel: "C1".to_string(),
                thread_ts: Some("1.2".to_string()),
                filename: "report.md".to_string(),
                content: b"hello".to_vec(),
                initial_comment: Some("here it is".to_string()),
            })
            .await
            .expect("the upload flow must complete");

        let SlackActionOutcome::FileUploaded { file_id, permalink } = outcome else {
            panic!("expected a file outcome, got {outcome:?}");
        };
        assert_eq!(file_id, "F1");
        assert_eq!(permalink.as_deref(), Some("https://slack.example/F1"));
    }

    /// A 4xx on the one-time upload URL cannot be retried, so it must not be
    /// classified as backpressure.
    #[tokio::test]
    async fn a_rejected_upload_url_is_not_treated_as_transient() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/files.getUploadURLExternal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "upload_url": format!("{}/upload-here", server.uri()),
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
        let (app_id, channel_id, _) = fixture
            .seed_app_with_channel(ORG, "slack", "xoxb-secret")
            .await;
        let session_id = fixture
            .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
            .await;

        let error = fixture
            .invoker(ORG, session_id)
            .with_api_base(server.uri())
            .invoke(SlackAction::UploadFile {
                channel: "C1".to_string(),
                thread_ts: None,
                filename: "report.md".to_string(),
                content: b"hello".to_vec(),
                initial_comment: None,
            })
            .await
            .expect_err("a 400 on the upload URL must not read as success");

        assert!(matches!(error, SlackActionError::Rejected(_)));
    }
}
