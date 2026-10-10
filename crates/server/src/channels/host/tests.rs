// The channel host's server plug-ins against a real database: the store's
// first-writer-wins binding, delivery keys and leases, the port's sessions,
// messages and events, and the host running on both.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{ControlAction, DeliveryTarget};
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::channel_runtime::{
    ChannelConfig, ChannelHost, ChannelResolver, ChannelScope, ChannelSessionPort, ChannelStore,
    NewChannelSession, PendingDelivery, ResolvedChannel, SendOutcome,
};
use everruns_core::host::{TurnBackend, TurnRequest, TurnTicket};
use futures::StreamExt;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::domains::agent_channels::ingress::row_to_ingress;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::SessionSource;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{CreateAgentChannelRow, StorageBackend};

const LEASE: Duration = Duration::from_secs(60);

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

struct Fixture {
    db: Arc<StorageBackend>,
    scope: ServerChannelScope,
    runner: Arc<IdleRunner>,
    port: ServerChannelPort,
}

impl Fixture {
    async fn new(channel_type: &str) -> Self {
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
                    channel_type: channel_type.into(),
                    channel_config: json!({}),
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
        let port = ServerChannelPort::new(
            db.clone(),
            Arc::new(SessionService::new(db.clone())),
            Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                EventDelivery::in_memory(),
            )),
        );
        Self {
            db,
            scope: ServerChannelScope { context, channel },
            runner,
            port,
        }
    }

    fn name(&self) -> String {
        self.scope.channel.internal_id.to_string()
    }

    fn config(&self) -> ChannelConfig {
        ChannelConfig {
            scope: ChannelScope::new(self.scope.clone()),
            ..ChannelConfig::new(self.name())
        }
    }

    fn store(&self) -> PgChannelStore {
        PgChannelStore::new(self.db.clone())
    }

    async fn session(&self) -> String {
        self.port
            .create_session(NewChannelSession {
                channel: self.config(),
                binding_key: Some("thread-1".into()),
                title: "Hello".into(),
                metadata: json!({}),
            })
            .await
            .expect("create session")
    }
}

#[tokio::test]
async fn binding_is_first_writer_wins_and_scoped_to_the_channel() {
    let fixture = Fixture::new("ag_ui").await;
    let store = fixture.store();
    let first = fixture.session().await;
    let second = fixture.session().await;
    let name = fixture.name();

    assert_eq!(store.session_for(&name, "thread-1").await.unwrap(), None);
    assert_eq!(store.bind(&name, "thread-1", &first).await.unwrap(), first);
    assert_eq!(store.bind(&name, "thread-1", &second).await.unwrap(), first);
    assert_eq!(
        store.session_for(&name, "thread-1").await.unwrap(),
        Some(first.clone())
    );

    // Another channel's key space is its own; a name that is no row has none.
    let other = Uuid::now_v7().to_string();
    assert_eq!(store.session_for(&other, "thread-1").await.unwrap(), None);
    assert_eq!(store.session_for("slack", "thread-1").await.unwrap(), None);
    assert!(store.bind("slack", "thread-1", &first).await.is_err());

    // A deleted session takes its binding with it.
    fixture
        .db
        .delete_session(DEFAULT_ORG_ID, first.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(store.session_for(&name, "thread-1").await.unwrap(), None);
}

#[tokio::test]
async fn a_delivery_key_is_new_once() {
    let fixture = Fixture::new("slack").await;
    let store = fixture.store();
    let name = fixture.name();
    assert!(store.first_sighting(&name, "C1:1.0").await.unwrap());
    assert!(!store.first_sighting(&name, "C1:1.0").await.unwrap());
    assert!(store.first_sighting(&name, "C1:2.0").await.unwrap());
}

#[tokio::test]
async fn pending_deliveries_are_leased_to_one_holder() {
    let fixture = Fixture::new("slack").await;
    let store = fixture.store();
    let session = fixture.session().await;
    let pending = PendingDelivery {
        channel: fixture.name(),
        session_id: session.clone(),
        input_message_id: "msg_1".into(),
        target: DeliveryTarget::new("C1", "1.0"),
        after_sequence: 7,
    };

    // Held by a live process: nobody else may take it.
    store.save_pending(&pending, "a", LEASE).await.unwrap();
    assert!(store.claim_pending("b", LEASE).await.unwrap().is_empty());

    // Released (its channel was unavailable): the next recovery takes it,
    // and then holds it.
    store.release_pending(&session, "msg_1", "b").await.unwrap();
    assert!(store.claim_pending("b", LEASE).await.unwrap().is_empty());
    store.release_pending(&session, "msg_1", "a").await.unwrap();
    let claimed = store.claim_pending("b", LEASE).await.unwrap();
    assert_eq!(claimed, vec![pending.clone()]);
    assert!(store.claim_pending("c", LEASE).await.unwrap().is_empty());

    // A lease that ran out is taken over.
    store
        .renew_pending(&session, "msg_1", "b", Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(store.claim_pending("c", LEASE).await.unwrap().len(), 1);

    store.clear_pending(&session, "msg_1").await.unwrap();
    store
        .renew_pending(&session, "msg_1", "c", Duration::ZERO)
        .await
        .unwrap();
    assert!(store.claim_pending("d", LEASE).await.unwrap().is_empty());
}

#[tokio::test]
async fn sessions_carry_the_channel_tenant_owner_and_origin() {
    let fixture = Fixture::new("ag_ui").await;
    let session_id: SessionId = fixture.session().await.parse().unwrap();
    let row = fixture
        .db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .unwrap()
        .expect("session");
    assert_eq!(row.channel_id, Some(fixture.scope.channel.internal_id));
    assert_eq!(
        row.agent_id.map(|agent| agent.uuid()),
        Some(fixture.scope.context.agent_internal_id)
    );
    assert_eq!(
        row.owner_principal_id,
        fixture.scope.context.owner_principal_id
    );
    assert_eq!(row.source, SessionSource::AgUi.to_string());
    assert_eq!(row.title.as_deref(), Some("Hello"));

    // Rename and discard reach the same session.
    fixture
        .port
        .control(
            &fixture.config(),
            &session_id.to_string(),
            &ControlAction::Rename {
                title: "Renamed".into(),
            },
        )
        .await
        .unwrap();
    let row = fixture
        .db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.title.as_deref(), Some("Renamed"));
    fixture
        .port
        .discard_session(&fixture.config(), &session_id.to_string())
        .await
        .unwrap();
    assert!(
        fixture
            .db
            .get_session(DEFAULT_ORG_ID, session_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_message_starts_a_turn_and_replays_from_the_cursor() {
    let fixture = Fixture::new("ag_ui").await;
    let session = fixture.session().await;
    let outcome = fixture
        .port
        .send(&fixture.config(), &session, InputMessage::user("hi there"))
        .await
        .unwrap();
    let SendOutcome::Started { input_message_id } = outcome else {
        panic!("expected a started turn, got {outcome:?}")
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.runner.started.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the turn started");

    // Replay from the start returns the stored message, carrying its turn.
    let mut events = fixture
        .port
        .events(&fixture.config(), &session, Some(0))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.next().await.expect("event");
            if event.event_type == "input.message" {
                return event;
            }
        }
    })
    .await
    .expect("replayed input message");
    assert!(event.sequence.is_some());
    assert_eq!(event.data["message"]["id"], json!(input_message_id));
}

#[tokio::test]
async fn the_host_binds_one_session_per_conversation_on_postgres() {
    let fixture = Fixture::new("ag_ui").await;
    let host = ChannelHost::builder(Arc::new(fixture.port.clone()))
        .store(Arc::new(fixture.store()))
        .build();
    let channel = ResolvedChannel::streaming(fixture.config(), "ag_ui");

    let first = host
        .conversation(&channel, "thread-9", "First")
        .await
        .unwrap();
    assert!(first.created);
    let again = host
        .conversation(&channel, "thread-9", "First")
        .await
        .unwrap();
    assert_eq!(again.session_id, first.session_id);
    assert!(!again.created);

    let turn = host
        .stream_turn(&channel, &first.session_id, InputMessage::user("hello"))
        .await
        .unwrap();
    assert!(matches!(turn.outcome, SendOutcome::Started { .. }));
}

#[tokio::test]
async fn names_that_are_no_live_row_resolve_to_nothing() {
    let fixture = Fixture::new("ag_ui").await;
    let resolver = ServerChannelResolver::new(fixture.db.clone(), None);
    assert!(resolver.resolve("slack").await.unwrap().is_none());
    assert!(
        resolver
            .resolve(&Uuid::now_v7().to_string())
            .await
            .unwrap()
            .is_none()
    );
    // Not on the host yet: each channel type joins in its own change.
    assert!(resolver.resolve(&fixture.name()).await.unwrap().is_none());
}
