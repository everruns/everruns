use super::*;
use crate::storage::{CreateAppRow, CreateHarnessRow, CreateSessionRow};
use crate::storage::{CreateLegacyAliasChannelRow, UpdateChannelByIdRow};
use everruns_contracts::slack_action::SlackActionInvoker;
use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    pub(super) async fn seed_app_with_channel(
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
                    virtual_user_id: None,
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
            .create_legacy_alias_channel(
                app.id,
                CreateLegacyAliasChannelRow {
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
            .update_channel_by_id(
                channel.id,
                UpdateChannelByIdRow {
                    status: Some("live".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("publish channel");

        (app.id, channel.id, public_id)
    }

    pub(super) async fn seed_session(
        &self,
        org_id: i64,
        app_id: Option<Uuid>,
        channel_id: Option<Uuid>,
        tags: Vec<String>,
    ) -> SessionId {
        self.seed_session_for_agent(org_id, app_id, channel_id, tags, None)
            .await
    }

    pub(super) async fn seed_session_for_agent(
        &self,
        org_id: i64,
        app_id: Option<Uuid>,
        channel_id: Option<Uuid>,
        tags: Vec<String>,
        agent_id: Option<AgentId>,
    ) -> SessionId {
        let harness_id = self.seed_harness().await;
        let session = self
            .db
            .create_session(CreateSessionRow {
                playground_user_id: None,
                source: crate::domains::sessions::record::SessionSource::Api,
                workspace_id: None,
                org_id,
                app_id,
                channel_id,
                trigger_id: None,
                harness_id: Some(harness_id),
                agent_id,
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
        .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
    let server = MockServer::start().await;
    let fixture = Fixture::new();
    let (app_id, channel_id, _) = fixture
        .seed_app_with_channel(ORG, "slack", "xoxb-secret")
        .await;
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
        .seed_session(ORG, Some(app_id), Some(channel_id), vec![])
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
            vec![format!("slack:channel:{channel_public_id}")],
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

mod health_tests;
mod reaction_tests;

#[tokio::test]
async fn an_alternate_channel_is_rejected_without_a_slack_request() {
    let server = MockServer::start().await;
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

#[tokio::test]
async fn an_upload_without_the_session_thread_is_rejected_before_slack() {
    let server = MockServer::start().await;
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
            thread_ts: Some("1.2".to_string()),
            filename: "report.md".to_string(),
            content: b"hello".to_vec(),
            initial_comment: None,
        })
        .await
        .expect_err("a 400 on the upload URL must not read as success");

    assert!(matches!(error, SlackActionError::Rejected(_)));
}
