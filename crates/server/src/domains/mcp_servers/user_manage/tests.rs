use super::*;
use crate::domains::mcp_servers::user_layer::tests::{Fixture, MESSAGE, custom};
use crate::domains::mcp_servers::user_layer::{UserMcpTurn, user_mcp_layer};
use crate::kernel_imports::ScopedMcpServer;
use crate::storage::models::CreateMcpServerRow;
use everruns_core::DEFAULT_ORG_ID;
use serde_json::json;

const MANAGE: Option<serde_json::Value> = None;

fn manage() -> Option<serde_json::Value> {
    Some(json!({"manage": true}))
}

impl Fixture {
    async fn call_for(
        &self,
        input_message: Option<Uuid>,
        call: UserMcpStoreCall,
    ) -> UserMcpStoreResult<UserMcpStoreReply> {
        invoke_user_mcp_store(
            &UserMcpManageTurn {
                db: &self.db,
                encryption: Some(&self.encryption),
                org_id: DEFAULT_ORG_ID,
                harness: &self.harness,
                agent: Some(&self.agent),
                session: &self.session,
                registry: &self.registry,
                input_message,
                storage: Some(&self.storage),
            },
            call,
        )
        .await
    }

    async fn call(&self, call: UserMcpStoreCall) -> UserMcpStoreResult<UserMcpStoreReply> {
        self.call_for(Some(MESSAGE), call).await
    }

    async fn list(&self) -> Vec<UserMcpServerSummary> {
        match self.call(UserMcpStoreCall::List).await.unwrap() {
            UserMcpStoreReply::Servers { servers } => servers,
            other => panic!("unexpected reply {other:?}"),
        }
    }

    async fn turn_layer(&self) -> crate::kernel_imports::ScopedMcpServers {
        user_mcp_layer(&UserMcpTurn {
            db: &self.db,
            encryption: Some(&self.encryption),
            org_id: DEFAULT_ORG_ID,
            harness: &self.harness,
            agent: Some(&self.agent),
            session: &self.session,
            registry: &self.registry,
            input_message: Some(MESSAGE),
        })
        .await
    }

    async fn catalog_preset(&self, name: &str, auth_mode: &str) {
        self.db
            .create_mcp_server(
                DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: name.to_string(),
                    description: None,
                    url: format!("https://mcp.{name}.app/mcp"),
                    transport_type: "http".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: Some(json!({ "auth_mode": auth_mode })),
                },
            )
            .await
            .unwrap();
    }
}

fn catalog_entry(catalog: &str) -> UserMcpServerEntry {
    UserMcpServerEntry::enabled(ScopedMcpServer {
        preset: Some(format!("catalog:{catalog}").parse().unwrap()),
        ..Default::default()
    })
}

fn url_entry(url: &str) -> UserMcpServerEntry {
    UserMcpServerEntry::enabled(ScopedMcpServer {
        url: url.to_string(),
        ..Default::default()
    })
}

fn upsert(name: &str, entry: UserMcpServerEntry) -> UserMcpStoreCall {
    UserMcpStoreCall::Upsert {
        name: name.to_string(),
        entry: Box::new(entry),
    }
}

#[tokio::test]
async fn add_from_catalog_is_usable_on_the_next_turn_and_connects_by_card() {
    let fixture = Fixture::new(manage()).await;
    fixture.catalog_preset("linear", "oauth").await;
    assert!(fixture.turn_layer().await.is_empty());

    let reply = fixture
        .call(upsert("linear", catalog_entry("linear")))
        .await
        .unwrap();
    let UserMcpStoreReply::Server { server } = reply else {
        panic!("unexpected reply {reply:?}");
    };
    assert_eq!(server.catalog.as_deref(), Some("linear"));
    assert_eq!(server.login, UserMcpLoginStatus::NotConnected);
    // The next turn's layer carries it, acting as the person.
    assert!(fixture.turn_layer().await.contains_key("linear"));

    let reply = fixture
        .call(UserMcpStoreCall::StartLogin {
            name: "linear".into(),
        })
        .await
        .unwrap();
    let UserMcpStoreReply::Login {
        login: McpLogin::Pending { provider, .. },
    } = reply
    else {
        panic!("expected a pending sign-in, got {reply:?}");
    };
    assert!(provider.starts_with("mcp_oauth_"));
}

#[tokio::test]
async fn custom_urls_need_allow_custom_urls_and_pass_url_validation() {
    let fixture = Fixture::new(manage()).await;
    let refused = fixture
        .call(upsert("notes", url_entry("https://notes.example.com/mcp")))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, UserMcpStoreError::Invalid(_)),
        "{refused:?}"
    );
    assert!(fixture.list().await.is_empty());

    let fixture = Fixture::new(Some(json!({"manage": true, "allow_custom_urls": true}))).await;
    fixture
        .call(upsert("notes", url_entry("https://notes.example.com/mcp")))
        .await
        .unwrap();
    // The same SSRF checks as the API.
    let refused = fixture
        .call(upsert("local", url_entry("http://169.254.169.254/latest")))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, UserMcpStoreError::Invalid(_)),
        "{refused:?}"
    );
    // No credentials through chat.
    let mut keyed = url_entry("https://keyed.example.com/mcp");
    keyed
        .server
        .headers
        .insert("Authorization".into(), "Bearer x".into());
    assert!(fixture.call(upsert("keyed", keyed)).await.is_err());
    let names: Vec<_> = fixture.list().await.into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["notes"]);
}

#[tokio::test]
async fn upsert_never_repoints_an_existing_server() {
    let fixture = Fixture::new(Some(json!({"manage": true, "allow_custom_urls": true}))).await;
    fixture
        .call(upsert("notes", url_entry("https://notes.example.com/mcp")))
        .await
        .unwrap();
    let refused = fixture
        .call(upsert(
            "notes",
            url_entry("https://elsewhere.example.com/mcp"),
        ))
        .await
        .unwrap_err();
    assert!(matches!(refused, UserMcpStoreError::Invalid(_)));
    // Re-adding the same server only applies its enabled flag.
    let mut disabled = url_entry("https://notes.example.com/mcp");
    disabled.enabled = false;
    fixture.call(upsert("notes", disabled)).await.unwrap();
    assert!(!fixture.list().await[0].enabled);
}

#[tokio::test]
async fn enable_disable_and_remove_by_name() {
    let fixture = Fixture::new(manage()).await;
    fixture
        .servers()
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    fixture
        .call(UserMcpStoreCall::SetEnabled {
            name: "notes".into(),
            enabled: false,
        })
        .await
        .unwrap();
    assert!(fixture.turn_layer().await.is_empty());
    fixture
        .call(UserMcpStoreCall::SetEnabled {
            name: "notes".into(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(fixture.turn_layer().await.contains_key("notes"));

    let reply = fixture
        .call(UserMcpStoreCall::StartLogin {
            name: "notes".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        reply,
        UserMcpStoreReply::Login {
            login: McpLogin::NotNeeded
        }
    ));

    let removed = |reply| matches!(reply, UserMcpStoreReply::Removed { removed: true });
    assert!(removed(
        fixture
            .call(UserMcpStoreCall::Remove {
                name: "notes".into()
            })
            .await
            .unwrap()
    ));
    assert!(!removed(
        fixture
            .call(UserMcpStoreCall::Remove {
                name: "notes".into()
            })
            .await
            .unwrap()
    ));
    let missing = fixture
        .call(UserMcpStoreCall::SetEnabled {
            name: "notes".into(),
            enabled: true,
        })
        .await
        .unwrap_err();
    assert_eq!(missing, UserMcpStoreError::NotFound("notes".into()));
}

#[tokio::test]
async fn list_reports_servers_skipped_for_a_name_clash() {
    let mut fixture = Fixture::new(manage()).await;
    for name in ["notes", "wiki"] {
        fixture
            .servers()
            .add(custom(name, McpServerAuthMode::None, None))
            .await
            .unwrap();
    }
    fixture.agent.mcp_servers.insert(
        "notes".to_string(),
        ScopedMcpServer {
            url: "https://agent-notes.example.com/mcp".to_string(),
            ..Default::default()
        },
    );
    let servers = fixture.list().await;
    let notes = servers.iter().find(|s| s.name == "notes").unwrap();
    assert!(
        notes
            .skipped
            .as_deref()
            .unwrap()
            .contains("agent server 'notes'")
    );
    let wiki = servers.iter().find(|s| s.name == "wiki").unwrap();
    assert!(wiki.skipped.is_none());
}

#[tokio::test]
async fn refuses_without_manage_a_person_or_in_shared_sessions() {
    // `use` only: the store refuses even if a worker forwards a call.
    let fixture = Fixture::new(Some(json!({}))).await;
    let refused = fixture.call(UserMcpStoreCall::List).await.unwrap_err();
    assert!(matches!(refused, UserMcpStoreError::Unavailable(_)));
    let fixture = Fixture::new(MANAGE).await;
    assert!(fixture.call(UserMcpStoreCall::List).await.is_err());

    let fixture = Fixture::new(manage()).await;
    assert!(fixture.call(UserMcpStoreCall::List).await.is_ok());
    // Unattended: nobody is chatting.
    let refused = fixture
        .call_for(None, UserMcpStoreCall::List)
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, UserMcpStoreError::Unavailable(m) if m.contains("No person")),
        "{refused:?}"
    );
    // A message nobody recorded as theirs.
    assert!(
        fixture
            .call_for(Some(Uuid::from_u128(92)), UserMcpStoreCall::List)
            .await
            .is_err()
    );
    // Two people in the session.
    fixture.join(201).await;
    fixture.join(202).await;
    let refused = fixture.call(UserMcpStoreCall::List).await.unwrap_err();
    assert!(matches!(refused, UserMcpStoreError::Unavailable(_)));
}

#[tokio::test]
async fn acts_only_on_the_initiating_persons_own_list() {
    let fixture = Fixture::new(manage()).await;
    // Someone else in the org owns a server with the same name.
    let other = fixture.db.create_test_user(Uuid::now_v7()).await;
    fixture
        .db
        .create_virtual_user(crate::storage::models::CreateVirtualUserRow {
            org_id: DEFAULT_ORG_ID,
            id: everruns_contracts::typed_id::VirtualUserId::from_uuid(other),
            usage: "end_user".to_string(),
            name: "Bob".to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
    let others = UserMcpServers {
        db: &fixture.db,
        encryption: Some(&fixture.encryption),
        org_id: DEFAULT_ORG_ID,
        owner: other,
    };
    others
        .add(custom("notes", McpServerAuthMode::None, None))
        .await
        .unwrap();
    assert!(fixture.list().await.is_empty());
    let reply = fixture
        .call(UserMcpStoreCall::Remove {
            name: "notes".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        reply,
        UserMcpStoreReply::Removed { removed: false }
    ));
    assert_eq!(others.list().await.unwrap().len(), 1);
    let _ = fixture.person;
}

fn agent_server(catalog: &str, acts_as: &str) -> ScopedMcpServer {
    serde_json::from_value(json!({ "use": format!("catalog:{catalog}"), "actsAs": acts_as }))
        .unwrap()
}

async fn connect_card(fixture: &Fixture, name: &str) -> McpLogin {
    match fixture
        .call(UserMcpStoreCall::StartLogin { name: name.into() })
        .await
        .unwrap()
    {
        UserMcpStoreReply::Login { login } => login,
        other => panic!("unexpected reply {other:?}"),
    }
}

#[tokio::test]
async fn agent_servers_acting_as_the_person_connect_without_manage() {
    // No `user_mcp` authored at all: the agent server acting as the person
    // is what makes `connect_mcp_server` available, and nothing else.
    let mut fixture = Fixture::new(None).await;
    fixture.catalog_preset("linear", "oauth").await;
    fixture
        .agent
        .mcp_servers
        .insert("linear".into(), agent_server("linear", "user_or_service"));

    let McpLogin::Pending {
        provider,
        setup_url,
        for_agent,
        ..
    } = connect_card(&fixture, "linear").await
    else {
        panic!("expected a pending sign-in");
    };
    assert!(provider.starts_with("mcp_oauth_"));
    assert_eq!(setup_url, SETUP_URL);
    assert!(!for_agent, "the person signs in for themselves");
    // Connect only: the manage calls stay refused.
    assert!(fixture.call(UserMcpStoreCall::List).await.is_err());

    fixture
        .db
        .upsert_virtual_user_connection(crate::storage::models::CreateVirtualUserConnectionRow {
            virtual_user_id: everruns_contracts::typed_id::VirtualUserId::from_uuid(fixture.person),
            provider,
            connection_type: "oauth".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(fixture.encryption.encrypt_string("t").unwrap()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(
        connect_card(&fixture, "linear").await,
        McpLogin::AlreadyConnected
    );
}

#[tokio::test]
async fn agent_servers_acting_as_the_agent_route_to_the_agents_sheet() {
    let mut fixture = Fixture::new(manage()).await;
    fixture
        .db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "github".into(),
                description: None,
                url: "https://api.githubcopilot.com/mcp/".into(),
                transport_type: "http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(json!({
                    "auth_mode": "oauth",
                    "service_connection_provider": "github"
                })),
            },
        )
        .await
        .unwrap();
    fixture
        .agent
        .mcp_servers
        .insert("github".into(), agent_server("github", "service"));

    let McpLogin::Pending {
        setup_url,
        for_agent,
        ..
    } = connect_card(&fixture, "github").await
    else {
        panic!("expected a pending sign-in");
    };
    assert!(for_agent, "the card asks for the agent's login");
    assert_eq!(
        setup_url,
        format!("/agents/{}?tab=mcp", fixture.agent.public_id)
    );

    // A connection-backed preset is satisfied by the agent's GitHub
    // connection: no second login is asked for.
    let identity = everruns_contracts::typed_id::VirtualUserId::from_seed(9);
    fixture
        .db
        .create_virtual_user(crate::storage::models::CreateVirtualUserRow {
            org_id: DEFAULT_ORG_ID,
            id: identity,
            usage: "service".into(),
            name: "Agent".into(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
    fixture
        .db
        .upsert_virtual_user_connection(crate::storage::models::CreateVirtualUserConnectionRow {
            virtual_user_id: identity,
            provider: "github".into(),
            connection_type: "github_app".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: Some(42),
            provider_metadata: None,
        })
        .await
        .unwrap();
    fixture.agent.service_virtual_user_id = Some(identity);
    assert_eq!(
        connect_card(&fixture, "github").await,
        McpLogin::AlreadyConnected
    );
}

#[tokio::test]
async fn connect_in_chat_never_travels_with_the_agent_servers_sign_in() {
    let mut fixture = Fixture::new(None).await;
    fixture.catalog_preset("linear", "oauth").await;
    fixture.catalog_preset("notion", "oauth").await;
    let quiet = |acts_as: &str| -> ScopedMcpServer {
        serde_json::from_value(json!({
            "use": "catalog:linear",
            "actsAs": acts_as,
            "connectInChat": "never",
        }))
        .unwrap()
    };
    fixture
        .agent
        .mcp_servers
        .insert("linear".into(), quiet("user"));
    fixture
        .agent
        .mcp_servers
        .insert("notion".into(), agent_server("notion", "user"));

    let McpLogin::Pending {
        setup_url,
        for_agent,
        connect_in_chat,
        ..
    } = connect_card(&fixture, "linear").await
    else {
        panic!("expected a pending sign-in");
    };
    assert_eq!(connect_in_chat, everruns_core::McpConnectInChat::Never);
    assert_eq!(setup_url, SETUP_URL);
    assert!(!for_agent);

    // The default stays `ask`, so existing agents keep their card.
    let McpLogin::Pending {
        connect_in_chat, ..
    } = connect_card(&fixture, "notion").await
    else {
        panic!("expected a pending sign-in");
    };
    assert_eq!(connect_in_chat, everruns_core::McpConnectInChat::Ask);

    // An agent's own login keeps the agent sheet link under `never` too.
    fixture
        .agent
        .mcp_servers
        .insert("linear".into(), quiet("service"));
    let McpLogin::Pending {
        setup_url,
        for_agent,
        connect_in_chat,
        ..
    } = connect_card(&fixture, "linear").await
    else {
        panic!("expected a pending sign-in");
    };
    assert!(for_agent);
    assert_eq!(connect_in_chat, everruns_core::McpConnectInChat::Never);
    assert_eq!(
        setup_url,
        format!("/agents/{}?tab=mcp", fixture.agent.public_id)
    );
}

fn add_to_chat(name: &str, server: ScopedMcpServer) -> UserMcpStoreCall {
    UserMcpStoreCall::AddToChat {
        name: name.to_string(),
        server: Box::new(server),
    }
}

/// The session as the next turn sees it, with run-time records folded in.
async fn next_turn_session(fixture: &Fixture) -> Session {
    let mut session = fixture.session.clone();
    super::super::session_servers::fold_session_records(&fixture.storage, &mut session).await;
    session
}

#[tokio::test]
async fn chat_only_server_joins_the_next_turn_and_remove_drops_it() {
    let fixture = Fixture::new(manage()).await;
    // A session server acting as the person needs the preset's OAuth client.
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
                settings: Some(json!({ "auth_mode": "oauth", "oauth": {} })),
            },
        )
        .await
        .unwrap();

    let reply = fixture
        .call(add_to_chat("linear", catalog_entry("linear").server))
        .await
        .unwrap();
    let UserMcpStoreReply::Server { server } = reply else {
        panic!("unexpected reply {reply:?}");
    };
    assert!(server.chat_only);
    assert_eq!(server.catalog.as_deref(), Some("linear"));
    assert_eq!(server.login, UserMcpLoginStatus::NotConnected);

    // Written as the general session record, not into the person's list.
    let record =
        everruns_core::get_session_mcp_server(&fixture.storage, fixture.session.id, "linear")
            .await
            .expect("session MCP server record");
    assert_eq!(
        record.source,
        everruns_core::SessionMcpServerSource::UserMcp
    );
    assert!(fixture.servers().list().await.unwrap().is_empty());
    let listed = fixture.list().await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].chat_only);

    // The next turn carries it, signing in as the person and loading on demand.
    let session = next_turn_session(&fixture).await;
    let linear = &session.mcp_servers["linear"];
    assert_eq!(linear.acts_as, crate::kernel_imports::McpServerActsAs::User);
    assert!(linear.deferred);
    let McpLogin::Pending { for_agent, .. } = connect_card(&fixture, "linear").await else {
        panic!("expected a pending sign-in");
    };
    assert!(!for_agent);

    let reply = fixture
        .call(UserMcpStoreCall::Remove {
            name: "linear".into(),
        })
        .await
        .unwrap();
    assert_eq!(reply, UserMcpStoreReply::Removed { removed: true });
    assert!(next_turn_session(&fixture).await.mcp_servers.is_empty());
    assert!(fixture.list().await.is_empty());
}

#[tokio::test]
async fn chat_only_servers_follow_the_same_rules_as_the_list() {
    let mut fixture = Fixture::new(manage()).await;
    fixture.catalog_preset("linear", "none").await;

    // Custom URLs need allow_custom_urls; unknown catalog names are refused.
    for call in [
        add_to_chat("notes", url_entry("https://notes.example.com/mcp").server),
        add_to_chat("ghost", catalog_entry("ghost").server),
    ] {
        let refused = fixture.call(call).await.unwrap_err();
        assert!(
            matches!(refused, UserMcpStoreError::Invalid(_)),
            "{refused:?}"
        );
    }

    // A name the agent already uses would win over the agent's own server.
    fixture.agent.mcp_servers.insert(
        "linear".into(),
        ScopedMcpServer {
            url: "https://agent-linear.example.com/mcp".into(),
            ..Default::default()
        },
    );
    let refused = fixture
        .call(add_to_chat("linear", catalog_entry("linear").server))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, UserMcpStoreError::Invalid(m) if m.contains("already has")),
        "{refused:?}"
    );

    // An ARD attachment owns its name in this session.
    everruns_core::put_session_mcp_server(
        &fixture.storage,
        fixture.session.id,
        &everruns_core::SessionMcpServer {
            name: "docs".into(),
            server: url_entry("https://docs.example.com/mcp").server,
            source: everruns_core::SessionMcpServerSource::Ard {
                urn: "urn:ai:example.com:mcp:docs".into(),
            },
        },
    )
    .await
    .unwrap();
    let refused = fixture
        .call(add_to_chat("docs", catalog_entry("linear").server))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, UserMcpStoreError::Invalid(_)),
        "{refused:?}"
    );
    // ...and the manage tools neither list nor remove it.
    assert!(fixture.list().await.is_empty());
    let reply = fixture
        .call(UserMcpStoreCall::Remove {
            name: "docs".into(),
        })
        .await
        .unwrap();
    assert_eq!(reply, UserMcpStoreReply::Removed { removed: false });

    let fixture = Fixture::new(Some(json!({"manage": true, "allow_custom_urls": true}))).await;
    let mut oauth = url_entry("https://oauth.example.com/mcp").server;
    oauth.auth_mode = McpServerAuthMode::OAuth;
    for (name, server) in [
        // OAuth needs the list row that keeps its client.
        ("oauth", oauth),
        // The same SSRF checks as any session server.
        ("local", url_entry("http://169.254.169.254/latest").server),
    ] {
        let refused = fixture.call(add_to_chat(name, server)).await.unwrap_err();
        assert!(
            matches!(refused, UserMcpStoreError::Invalid(_)),
            "{refused:?}"
        );
    }
    let reply = fixture
        .call(add_to_chat(
            "notes",
            url_entry("https://notes.example.com/mcp").server,
        ))
        .await
        .unwrap();
    let UserMcpStoreReply::Server { server } = reply else {
        panic!("unexpected reply {reply:?}");
    };
    assert_eq!(server.login, UserMcpLoginStatus::NotNeeded);
    let session = next_turn_session(&fixture).await;
    assert_eq!(
        session.mcp_servers["notes"].url,
        "https://notes.example.com/mcp"
    );
}

#[tokio::test]
async fn chat_only_servers_need_session_storage_and_a_person() {
    let fixture = Fixture::new(manage()).await;
    fixture.catalog_preset("linear", "none").await;
    let without_storage = invoke_user_mcp_store(
        &UserMcpManageTurn {
            db: &fixture.db,
            encryption: Some(&fixture.encryption),
            org_id: DEFAULT_ORG_ID,
            harness: &fixture.harness,
            agent: Some(&fixture.agent),
            session: &fixture.session,
            registry: &fixture.registry,
            input_message: Some(MESSAGE),
            storage: None,
        },
        add_to_chat("linear", catalog_entry("linear").server),
    )
    .await
    .unwrap_err();
    assert!(matches!(without_storage, UserMcpStoreError::Unavailable(_)));

    let unattended = fixture
        .call_for(None, add_to_chat("linear", catalog_entry("linear").server))
        .await
        .unwrap_err();
    assert!(matches!(unattended, UserMcpStoreError::Unavailable(_)));
    assert!(
        everruns_core::load_session_mcp_servers(&fixture.storage, fixture.session.id)
            .await
            .is_empty()
    );
}
