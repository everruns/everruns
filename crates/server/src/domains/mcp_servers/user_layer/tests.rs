use super::*;
use crate::domains::mcp_servers::scoped_mcp::tests::{test_agent, test_harness, test_session};
use crate::domains::mcp_servers::user_servers::{
    AddUserMcpServerRequest, UpdateUserMcpServerRequest, UserMcpServers,
};
use crate::domains::sessions::record::{SessionParticipantKind, SessionParticipantRole};
use crate::storage::CreateVirtualUserRow;
use crate::storage::{CreateMcpServerRow, CreateSessionParticipantRow};
use everruns_contracts::CapabilityRef;
use everruns_contracts::typed_id::{PrincipalId, VirtualUserId};
use everruns_core::DEFAULT_ORG_ID;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
pub(crate) const MESSAGE: Uuid = Uuid::from_u128(91);

pub(crate) struct Fixture {
    pub(crate) db: StorageBackend,
    pub(crate) encryption: EncryptionService,
    pub(crate) registry: CapabilityRegistry,
    pub(crate) harness: Harness,
    pub(crate) agent: Agent,
    pub(crate) session: Session,
    pub(crate) person: Uuid,
    /// The session's KV storage (chat-only servers).
    pub(crate) storage: everruns_core::host::InMemorySessionStorageStore,
}

impl Fixture {
    /// A session whose input message was sent by `person`, on an agent with
    /// the `user_mcp` capability configured as given (None leaves it off).
    pub(crate) async fn new(capability: Option<serde_json::Value>) -> Self {
        let db = StorageBackend::test_database();
        let person = db.create_test_user(Uuid::now_v7()).await;
        db.create_virtual_user(CreateVirtualUserRow {
            org_id: DEFAULT_ORG_ID,
            id: VirtualUserId::from_uuid(person),
            usage: "end_user".to_string(),
            name: "Alice".to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
        let session_id = db.create_test_session().await;
        db.record_runtime_invocation(
            DEFAULT_ORG_ID,
            session_id,
            MESSAGE,
            Some(VirtualUserId::from_uuid(person)),
            None,
            None,
        )
        .await
        .unwrap();

        let harness = test_harness();
        let mut agent = test_agent();
        if let Some(config) = capability {
            agent
                .capabilities
                .push(CapabilityRef::with_config(USER_MCP_CAPABILITY_ID, config));
        }
        let mut session = test_session(harness.id, agent.public_id);
        session.id = session_id;
        Self {
            db,
            encryption: EncryptionService::new(TEST_KEY, &[]).unwrap(),
            registry: crate::platform::oss_capability_registry(),
            harness,
            agent,
            session,
            person,
            storage: Default::default(),
        }
    }

    pub(crate) fn servers(&self) -> UserMcpServers<'_> {
        UserMcpServers {
            db: &self.db,
            encryption: Some(&self.encryption),
            org_id: DEFAULT_ORG_ID,
            owner: self.person,
        }
    }

    async fn layer_for(&self, input_message: Option<Uuid>) -> ScopedMcpServers {
        user_mcp_layer(&UserMcpTurn {
            db: &self.db,
            encryption: Some(&self.encryption),
            org_id: DEFAULT_ORG_ID,
            harness: &self.harness,
            agent: Some(&self.agent),
            session: &self.session,
            registry: &self.registry,
            input_message,
        })
        .await
    }

    async fn layer(&self) -> ScopedMcpServers {
        self.layer_for(Some(MESSAGE)).await
    }

    pub(crate) async fn join(&self, seed: u128) {
        let principal = self
            .db
            .create_test_principal(PrincipalId::from_seed(seed))
            .await;
        self.db
            .create_session_participant(CreateSessionParticipantRow {
                org_id: DEFAULT_ORG_ID,
                session_id: self.session.id,
                kind: SessionParticipantKind::User,
                agent_id: None,
                principal_id: principal,
                display_name: None,
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await
            .unwrap();
    }
}

pub(crate) fn custom(
    name: &str,
    auth: McpServerAuthMode,
    api_key: Option<&str>,
) -> AddUserMcpServerRequest {
    AddUserMcpServerRequest {
        name: Some(name.to_string()),
        url: Some(format!("https://{name}.example.com/mcp")),
        auth_mode: Some(auth),
        api_key: api_key.map(str::to_string),
        ..Default::default()
    }
}

fn uuid_of(id: &str) -> Uuid {
    id.parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid()
}

#[tokio::test]
async fn agents_without_the_capability_never_see_the_persons_servers() {
    let fixture = Fixture::new(None).await;
    fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    assert!(fixture.layer().await.is_empty());

    let off = Fixture::new(Some(serde_json::json!({"use": false}))).await;
    off.servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    assert!(off.layer().await.is_empty());
}

#[tokio::test]
async fn custom_servers_carry_the_persons_sign_in() {
    let fixture = Fixture::new(Some(serde_json::json!({}))).await;
    let oauth = fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::OAuth, None))
        .await
        .unwrap();
    fixture
        .servers()
        .add(custom("wiki", McpServerAuthMode::ApiKey, Some("wiki-key")))
        .await
        .unwrap();

    let layer = fixture.layer().await;
    let notes = layer.get("notes").expect("oauth server present");
    assert_eq!(notes.acts_as, McpServerActsAs::User);
    assert_eq!(notes.auth_mode, McpServerAuthMode::OAuth);
    assert_eq!(
        notes.oauth_provider_id.as_deref(),
        Some(mcp_oauth_provider_id_for_uuid(uuid_of(&oauth.id)).as_str())
    );

    let wiki = layer.get("wiki").expect("api key server present");
    assert_eq!(wiki.acts_as, McpServerActsAs::None);
    assert_eq!(
        wiki.headers.get("Authorization").map(String::as_str),
        Some("Bearer wiki-key")
    );
}

#[tokio::test]
async fn catalog_servers_resolve_through_their_preset() {
    let fixture = Fixture::new(Some(serde_json::json!({"use": true}))).await;
    fixture
        .db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "linear".to_string(),
                description: None,
                url: "https://mcp.linear.app/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({"auth_mode": "oauth"})),
            },
        )
        .await
        .unwrap();
    fixture
        .servers()
        .add(AddUserMcpServerRequest {
            catalog: Some("linear".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let layer = fixture.layer().await;
    let linear = layer.get("linear").expect("catalog server present");
    assert_eq!(
        linear.preset.as_ref().map(|p| p.catalog_name()),
        Some("linear")
    );
    assert_eq!(linear.acts_as, McpServerActsAs::User);
    assert!(linear.url.is_empty());
}

#[tokio::test]
async fn turned_off_servers_are_left_out() {
    let fixture = Fixture::new(Some(serde_json::json!({}))).await;
    let added = fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    fixture
        .servers()
        .update(
            uuid_of(&added.id),
            UpdateUserMcpServerRequest {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(fixture.layer().await.is_empty());
}

#[tokio::test]
async fn unattended_and_shared_turns_get_no_personal_servers() {
    let fixture = Fixture::new(Some(serde_json::json!({}))).await;
    fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    assert_eq!(fixture.layer().await.len(), 1);

    // No input message: a schedule or trigger, nobody is chatting.
    assert!(fixture.layer_for(None).await.is_empty());
    // A message nobody recorded as theirs.
    assert!(
        fixture
            .layer_for(Some(Uuid::from_u128(92)))
            .await
            .is_empty()
    );

    // Two people in the session: neither person's servers apply.
    fixture.join(201).await;
    fixture.join(202).await;
    assert!(fixture.layer().await.is_empty());
}

#[tokio::test]
async fn agent_servers_win_a_name_clash() {
    let mut fixture = Fixture::new(Some(serde_json::json!({}))).await;
    fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    fixture.agent.mcp_servers.insert(
        "notes".to_string(),
        ScopedMcpServer {
            url: "https://agent-notes.example.com/mcp".to_string(),
            ..Default::default()
        },
    );
    let layer = fixture.layer().await;
    let merged = merge_turn_scoped_mcp_servers(
        &fixture.harness,
        Some(&fixture.agent),
        &fixture.session,
        &fixture.registry,
        &layer,
    );
    assert_eq!(
        merged.get("notes").map(|s| s.url.as_str()),
        Some("https://agent-notes.example.com/mcp")
    );
}

#[tokio::test]
async fn persons_servers_are_deferred_while_agent_servers_stay_eager() {
    let mut fixture = Fixture::new(Some(serde_json::json!({}))).await;
    fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    fixture.agent.mcp_servers.insert(
        "docs".to_string(),
        ScopedMcpServer {
            url: "https://docs.example.com/mcp".to_string(),
            ..Default::default()
        },
    );
    let layer = fixture.layer().await;
    assert!(
        layer["notes"].deferred,
        "a person's own servers load on demand by default"
    );
    let merged = merge_turn_scoped_mcp_servers(
        &fixture.harness,
        Some(&fixture.agent),
        &fixture.session,
        &fixture.registry,
        &layer,
    );
    assert!(merged["notes"].deferred);
    assert!(
        !merged["docs"].deferred,
        "agent servers keep listing their tools unless they opt in"
    );
}

#[tokio::test]
async fn a_server_set_to_always_load_is_not_deferred() {
    let fixture = Fixture::new(Some(serde_json::json!({}))).await;
    let mine = fixture.servers();
    mine.add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    let eager = mine
        .add(AddUserMcpServerRequest {
            deferred: Some(false),
            ..custom("docs", McpServerAuthMode::None, None)
        })
        .await
        .unwrap();
    assert!(!eager.deferred);

    let layer = fixture.layer().await;
    assert!(layer["notes"].deferred, "the default stays on demand");
    assert!(
        !layer["docs"].deferred,
        "a server set to always load lists its tools directly"
    );

    // Turning on-demand loading back on defers it again.
    mine.update(
        uuid_of(&eager.id),
        UpdateUserMcpServerRequest {
            deferred: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(fixture.layer().await["docs"].deferred);
}

#[tokio::test]
async fn a_catalog_server_the_person_signed_in_to_reaches_the_agent() {
    // D8: a personal connect lists the catalog server, so `use` picks it up
    // without the person adding it by hand.
    let fixture = Fixture::new(Some(serde_json::json!({"use": true}))).await;
    let visti = fixture
        .db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "visti".to_string(),
                description: None,
                url: "https://mcp.visti.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({"auth_mode": "oauth"})),
            },
        )
        .await
        .unwrap()
        .id
        .uuid();
    assert!(fixture.layer().await.is_empty());

    fixture
        .servers()
        .list_signed_in_catalog_server(visti)
        .await
        .unwrap();

    let layer = fixture.layer().await;
    let server = layer
        .get("visti")
        .expect("signed-in catalog server present");
    assert_eq!(server.acts_as, McpServerActsAs::User);
    assert_eq!(
        server.preset.as_ref().map(|p| p.catalog_name()),
        Some("visti")
    );
}
