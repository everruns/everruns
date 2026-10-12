// Slack on the channel host against a real database: verifying and reading
// events, binding a conversation once under the keys Slack sessions always
// had, pane controls, the relevance policy, the per-turn plan, and the
// backfill that binds conversations started before the host.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelError, DeliveryContext, DeliveryResult, DeliveryTarget,
    OutboundChannelMessage,
};
use everruns_contracts::runtime::channel_driver::{ChannelRequest, ChannelResponse};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::channel::SessionBinding;
use everruns_core::channel_runtime::{
    ChannelStore, DeliveryOptions, ResolvedChannel, TurnDeliveryPlanner,
};
use everruns_core::host::{TurnBackend, TurnRequest, TurnTicket};
use serde_json::{Value, json};
use uuid::Uuid;

use super::approvals::SLACK_APPROVAL_HINT;
use super::events::SlackState;
use super::events::tests_support::{make_signature, test_config};
use super::host::SlackTurnPlanner;
use crate::channels::host::PgChannelStore;
use crate::domains::agent_channels::ingress::{IngressChannel, IngressContext, row_to_ingress};
use crate::domains::agent_channels::record::slack_channel::{
    SlackChannelConfig, SlackResponsePolicy,
};
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{CreateAgentChannelRow, StorageBackend};

/// Starts nothing: a started turn never ends.
#[derive(Default)]
struct IdleRunner {
    started: Mutex<Vec<SessionId>>,
}

#[async_trait]
impl TurnBackend for IdleRunner {
    async fn start_turn(
        &self,
        request: TurnRequest,
    ) -> everruns_contracts::error::Result<TurnTicket> {
        self.started.lock().unwrap().push(request.session_id);
        Ok(TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

/// Never reaches Slack.
struct Silent;

#[async_trait]
impl ChannelDeliveryAdapter for Silent {
    fn platform(&self) -> &str {
        "slack"
    }

    async fn deliver(
        &self,
        _message: &OutboundChannelMessage,
        _context: &DeliveryContext,
    ) -> DeliveryResult {
        DeliveryResult::Ok
    }

    async fn send_ack(
        &self,
        _thread_ref: &str,
        _text: &str,
        _context: &DeliveryContext,
    ) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

const SECRET: &str = "secret";

/// A channel installed in workspace `T1`: its senders' identities are
/// scoped to that workspace.
fn slack_config(binding: SessionBinding) -> SlackChannelConfig {
    SlackChannelConfig {
        team_id: Some("T1".into()),
        ..test_config(binding)
    }
}

struct Slack {
    db: Arc<StorageBackend>,
    state: SlackState,
    context: IngressContext,
    channel: IngressChannel,
    runner: Arc<IdleRunner>,
}

impl Slack {
    async fn new(config: SlackChannelConfig) -> Self {
        let db = Arc::new(StorageBackend::test_database());
        let harness = Uuid::now_v7();
        db.create_test_harness(DEFAULT_ORG_ID, harness).await;
        let agent = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
        sqlx::query("UPDATE agents SET harness_id = $1 WHERE id = $2")
            .bind(harness)
            .bind(agent)
            .execute(db.database().pool())
            .await
            .expect("move agent to the test harness");
        let row = db
            .create_agent_channel(
                DEFAULT_ORG_ID,
                CreateAgentChannelRow {
                    agent_id: agent,
                    public_id: format!("appchan_{}", Uuid::now_v7().simple()),
                    channel_type: "slack".into(),
                    channel_config: serde_json::to_value(&config).unwrap(),
                    channel_config_encrypted: None,
                    auth: None,
                    auth_encrypted: None,
                    enabled: true,
                    status: "live".into(),
                    virtual_user_id: None,
                    owner_principal_id: PrincipalId::from_seed(1).uuid(),
                    resolved_owner_user_id: None,
                },
            )
            .await
            .expect("create channel");
        let (context, channel) = row_to_ingress(None, row).expect("ingress");
        let runner = Arc::new(IdleRunner::default());
        let mut state = SlackState::new(
            db.clone(),
            None,
            runner.clone(),
            EventDelivery::in_memory(),
            "https://example.com/api".into(),
        );
        state.delivery_adapter = Arc::new(Silent);
        // Display names come from Slack; the tests' senders are known.
        for user in ["U1", "U2"] {
            state
                .user_name_cache
                .insert(user.to_string(), Some(format!("Person {user}")));
        }
        let state = state.with_channel_host();
        Self {
            db,
            state,
            context,
            channel,
            runner,
        }
    }

    fn resolved(&self) -> ResolvedChannel {
        self.state
            .channels
            .as_ref()
            .unwrap()
            .resolver
            .channel(&self.context, &self.channel)
            .expect("a Slack channel runs on the host")
    }

    fn signed(&self, body: &Value, secret: &str) -> ChannelRequest {
        let body = body.to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        ChannelRequest {
            path: String::new(),
            headers: [
                ("x-slack-request-timestamp".to_string(), timestamp.clone()),
                (
                    "x-slack-signature".to_string(),
                    make_signature(secret, &timestamp, &body),
                ),
            ]
            .into_iter()
            .collect(),
            body: body.into_bytes(),
        }
    }

    async fn send(&self, body: Value) -> Result<ChannelResponse, ChannelError> {
        let request = self.signed(&body, SECRET);
        self.state
            .channels
            .as_ref()
            .unwrap()
            .host
            .try_handle(&self.resolved(), &request)
            .await
    }

    async fn bound(&self, key: &str) -> Option<String> {
        PgChannelStore::new(self.db.clone())
            .session_for(&self.channel.internal_id.to_string(), key)
            .await
            .unwrap()
    }

    async fn bindings(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_bindings WHERE channel_id = $1")
            .bind(self.channel.internal_id)
            .fetch_one(self.db.database().pool())
            .await
            .unwrap()
    }

    async fn title(&self, session: &str) -> Option<String> {
        self.db
            .get_session(DEFAULT_ORG_ID, session.parse().unwrap())
            .await
            .unwrap()
            .unwrap()
            .title
    }
}

fn message(channel: &str, ts: &str, thread_ts: Option<&str>) -> Value {
    json!({
        "type": "event_callback",
        "team_id": "T1",
        "event": {
            "type": "message",
            "text": "Plan my trip",
            "user": "U1",
            "channel": channel,
            "ts": ts,
            "thread_ts": thread_ts,
        }
    })
}

fn pane_message(ts: &str) -> Value {
    let mut body = message("D1", ts, None);
    body["event"]["channel_type"] = json!("im");
    body
}

fn pane_event(event_type: &str, ts: &str, extra: Value) -> Value {
    let mut thread = json!({ "channel_id": "D1", "thread_ts": ts });
    if let (Some(thread), Some(extra)) = (thread.as_object_mut(), extra.as_object()) {
        thread.extend(extra.clone());
    }
    json!({
        "type": "event_callback",
        "team_id": "T1",
        "event": { "type": event_type, "user": "U1", "assistant_thread": thread }
    })
}

fn session_of(response: &ChannelResponse) -> String {
    response.body["session_id"]
        .as_str()
        .expect("the message started a session")
        .to_string()
}

#[tokio::test]
async fn a_thread_message_starts_one_session_and_a_retry_starts_nothing() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let response = slack.send(message("C1", "1.0", None)).await.unwrap();
    let session = session_of(&response);
    assert_eq!(response.body["created"], true);
    // The key is the routing tag Slack sessions always carried.
    assert_eq!(slack.bound("slack:thread:1.0").await, Some(session.clone()));

    let retry = slack.send(message("C1", "1.0", None)).await.unwrap();
    assert_eq!(retry.body["duplicate"], true);
    assert_eq!(slack.runner.started.lock().unwrap().len(), 1);

    // The session can draw approval cards, and the sender joined it.
    let row = slack
        .db
        .get_session(DEFAULT_ORG_ID, session.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.hints.unwrap()[SLACK_APPROVAL_HINT], true);
    let thread = slack
        .db
        .get_session_key_value(row.id.uuid(), everruns_core::channel::THREAD_CONTEXT_KV_KEY)
        .await
        .unwrap()
        .and_then(|row| everruns_core::channel::decode_thread_context(&row.value))
        .expect("the sender is on the thread");
    assert_eq!(thread.participant_count(), 1);
}

#[tokio::test]
async fn a_reply_in_the_thread_continues_its_session() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let first = session_of(&slack.send(message("C1", "1.0", None)).await.unwrap());
    let reply = slack.send(message("C1", "2.0", Some("1.0"))).await.unwrap();
    assert_eq!(session_of(&reply), first);
    assert_eq!(reply.body["created"], false);
    assert_eq!(slack.bindings().await, 1);
}

#[tokio::test]
async fn the_pane_is_one_thread_whatever_the_channel_binds() {
    let mut config = slack_config(SessionBinding::Conversation);
    config.agent_surface_enabled = true;
    let slack = Slack::new(config).await;
    let pane = session_of(&slack.send(pane_message("1.0")).await.unwrap());
    assert_eq!(slack.bound("slack:thread:1.0").await, Some(pane));
    let channel = session_of(&slack.send(message("C1", "2.0", None)).await.unwrap());
    assert_eq!(slack.bound("slack:channel:C1").await, Some(channel));
}

#[tokio::test]
async fn a_rename_in_the_pane_becomes_the_session_title() {
    let mut config = slack_config(SessionBinding::Thread);
    config.agent_surface_enabled = true;
    let slack = Slack::new(config).await;
    let session = session_of(&slack.send(pane_message("1.0")).await.unwrap());

    slack
        .send(pane_event(
            "agent_session_title_changed",
            "1.0",
            json!({ "title": "  Trip to Lisbon  " }),
        ))
        .await
        .unwrap();
    assert_eq!(
        slack.title(&session).await.as_deref(),
        Some("Trip to Lisbon")
    );

    // A blank rename, and one for a thread this channel never saw, change
    // nothing and start nothing.
    slack
        .send(pane_event(
            "agent_session_title_changed",
            "1.0",
            json!({ "title": " " }),
        ))
        .await
        .unwrap();
    slack
        .send(pane_event(
            "agent_session_title_changed",
            "9.0",
            json!({ "title": "Elsewhere" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        slack.title(&session).await.as_deref(),
        Some("Trip to Lisbon")
    );
    assert_eq!(slack.bindings().await, 1);
}

#[tokio::test]
async fn where_the_user_looks_is_kept_without_an_event() {
    let mut config = slack_config(SessionBinding::Thread);
    config.agent_surface_enabled = true;
    let slack = Slack::new(config).await;
    let session: SessionId = session_of(&slack.send(pane_message("1.0")).await.unwrap())
        .parse()
        .unwrap();

    slack
        .send(pane_event(
            "app_context_changed",
            "1.0",
            json!({ "context": { "channel_id": "C_ELSEWHERE", "team_id": "T1" } }),
        ))
        .await
        .unwrap();
    let thread = slack
        .db
        .get_session_key_value(
            session.uuid(),
            everruns_core::channel::THREAD_CONTEXT_KV_KEY,
        )
        .await
        .unwrap()
        .and_then(|row| everruns_core::channel::decode_thread_context(&row.value))
        .expect("the view is kept");
    let view = thread.current_view.clone().expect("the reported place");
    assert_eq!(view.channel_id.as_deref(), Some("C_ELSEWHERE"));
    assert_eq!(thread.participant_count(), 1, "participants survive");
}

#[tokio::test]
async fn bots_and_other_workspaces_start_nothing() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;

    // THREAT[TM-SLACK-002]: never answer a bot, ours included.
    let mut bot = message("C1", "1.0", None);
    bot["event"]["bot_id"] = json!("B1");
    let mut elsewhere = message("C1", "2.0", None);
    elsewhere["team_id"] = json!("T_OTHER");
    let mut join = message("C1", "3.0", None);
    join["event"]["subtype"] = json!("channel_join");
    for body in [bot, elsewhere, join] {
        let response = slack.send(body).await.unwrap();
        assert!(response.body.get("session_id").is_none());
    }
    assert_eq!(slack.bindings().await, 0);
    assert!(slack.runner.started.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_binding_slack_does_not_offer_starts_nothing() {
    // Stored before writes rejected it: one shared session would hand one
    // person's conversation to everyone in the channel.
    let slack = Slack::new(slack_config(SessionBinding::Shared)).await;
    let response = slack.send(message("C1", "1.0", None)).await.unwrap();
    assert!(response.body.get("session_id").is_none());
    assert_eq!(slack.bindings().await, 0);
}

#[tokio::test]
async fn a_forged_request_is_refused() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let request = slack.signed(&message("C1", "1.0", None), "not-the-secret");
    let error = slack
        .state
        .channels
        .as_ref()
        .unwrap()
        .host
        .try_handle(&slack.resolved(), &request)
        .await
        .unwrap_err();
    assert!(matches!(error, ChannelError::Unauthorized(_)));
    assert_eq!(slack.bindings().await, 0);
}

#[tokio::test]
async fn the_url_check_gets_its_challenge_back() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let response = slack
        .send(json!({ "type": "url_verification", "challenge": "c-123" }))
        .await
        .unwrap();
    assert_eq!(response.body["challenge"], "c-123");
}

#[tokio::test]
async fn mentions_only_lets_an_unmentioned_message_pass_by() {
    let mut config = slack_config(SessionBinding::Thread);
    config.response_policy = SlackResponsePolicy::MentionsOnly;
    let slack = Slack::new(config).await;
    let response = slack.send(message("C1", "1.0", None)).await.unwrap();
    assert!(response.body.get("session_id").is_none());
    assert_eq!(slack.bindings().await, 0);

    let mut mention = message("C1", "2.0", None);
    mention["event"]["type"] = json!("app_mention");
    session_of(&slack.send(mention).await.unwrap());
    assert_eq!(slack.bindings().await, 1);
}

#[tokio::test]
async fn a_pane_turn_streams_and_a_channel_turn_does_not() {
    let mut config = slack_config(SessionBinding::Thread);
    config.agent_surface_enabled = true;
    let slack = Slack::new(config.clone()).await;
    let session = session_of(&slack.send(pane_message("1.0")).await.unwrap());
    let planner = SlackTurnPlanner::new(slack.state.clone(), DEFAULT_ORG_ID, config);

    let mut pane = DeliveryTarget::new("D1", "1.0");
    pane.extra.insert("slack_surface".into(), "pane".into());
    let (_, options) = planner
        .plan(&session, &pane, DeliveryOptions::default())
        .await;
    assert!(options.stream && options.agent_surface);

    let mut channel = DeliveryTarget::new("C1", "2.0");
    channel
        .extra
        .insert("slack_surface".into(), "channel".into());
    let (_, options) = planner
        .plan(&session, &channel, DeliveryOptions::default())
        .await;
    assert!(!options.stream && !options.agent_surface);
    assert!(options.replies);
}

/// Sessions the tag lookup created before the host, tagged as it tagged them.
async fn tagged_session(slack: &Slack, owner: PrincipalId, tags: &[String]) -> SessionId {
    crate::domains::sessions::SessionService::new(slack.db.clone())
        .create_from_app(
            &everruns_core::Caller::internal(DEFAULT_ORG_ID),
            slack.context.harness_id.uuid(),
            Some(slack.context.agent_internal_id),
            slack.context.agent_id,
            None,
            Some(slack.channel.internal_id),
            None,
            owner,
            None,
            crate::domains::sessions::record::SessionSource::Slack,
            crate::domains::sessions::types::CreateSessionRequest {
                tags: tags.to_vec(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn existing_slack_conversations_keep_their_session() {
    let slack = Slack::new(slack_config(SessionBinding::Thread)).await;
    let endpoint = format!("slack:endpoint:{}", slack.channel.public_id);
    let owner = slack.context.owner_principal_id;
    let thread = tagged_session(
        &slack,
        owner,
        &[endpoint.clone(), "slack:thread:1.0".into()],
    )
    .await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    tagged_session(
        &slack,
        owner,
        &[endpoint.clone(), "slack:thread:1.0".into()],
    )
    .await;
    let user = tagged_session(&slack, owner, &[endpoint.clone(), "slack:user:U1".into()]).await;
    // Another owner's session, and one without this channel's endpoint tag,
    // were never this channel's conversations.
    let stranger = crate::domains::users::PrincipalService::new(slack.db.clone())
        .ensure_external_actor_principal(
            DEFAULT_ORG_ID,
            &everruns_core::ExternalActor {
                actor_id: "U_STRANGER".into(),
                actor_name: None,
                source: "slack".into(),
                metadata: Some([("team_id".to_string(), "T1".to_string())].into()),
            },
        )
        .await
        .unwrap();
    tagged_session(&slack, stranger.id, &[endpoint, "slack:channel:C1".into()]).await;
    tagged_session(&slack, owner, &["slack:channel:C2".into()]).await;

    sqlx::raw_sql(include_str!(
        "../../../migrations/legacy/217_slack_channel_bindings.sql"
    ))
    .execute(slack.db.database().pool())
    .await
    .unwrap();

    assert_eq!(
        slack.bound("slack:thread:1.0").await,
        Some(thread.to_string())
    );
    assert_eq!(slack.bound("slack:user:U1").await, Some(user.to_string()));
    assert_eq!(slack.bound("slack:channel:C1").await, None);
    assert_eq!(slack.bound("slack:channel:C2").await, None);

    // The host continues the bound thread instead of starting over.
    let reply = slack.send(message("C1", "2.0", Some("1.0"))).await.unwrap();
    assert_eq!(session_of(&reply), thread.to_string());
}

#[tokio::test]
async fn a_top_level_dm_is_answered_in_the_dm_and_a_thread_keeps_its_thread() {
    let slack = Slack::new(slack_config(SessionBinding::Requester)).await;
    let inline = |body: Value| {
        let slack = &slack;
        async move {
            let session = session_of(&slack.send(body).await.unwrap());
            let input = slack
                .db
                .latest_input_message_event_since(session.parse().unwrap(), 0)
                .await
                .unwrap()
                .expect("the message was recorded");
            input.data["message"]["metadata"]["slack_reply_inline"] == true
        }
    };
    assert!(inline(pane_message("1.0")).await, "top-level DM");
    let mut threaded = pane_message("2.0");
    threaded["event"]["thread_ts"] = json!("1.0");
    assert!(!inline(threaded).await, "a reply inside a DM thread");
    assert!(
        !inline(message("C1", "3.0", None)).await,
        "a channel message"
    );
}
