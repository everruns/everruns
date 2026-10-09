use super::*;
use crate::storage::CreateUserConnectionRow;
use crate::storage::CreateVirtualUserRow;
use everruns_contracts::typed_id::VirtualUserId;
use everruns_core::DEFAULT_ORG_ID;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

async fn person(db: &StorageBackend, name: &str) -> Uuid {
    let id = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        org_id: DEFAULT_ORG_ID,
        id,
        usage: "end_user".to_string(),
        name: name.to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    id.uuid()
}

async fn catalog_preset(db: &StorageBackend, name: &str) -> Uuid {
    db.create_mcp_server(
        DEFAULT_ORG_ID,
        CreateMcpServerRow {
            name: name.to_string(),
            description: Some("Issues".to_string()),
            url: "https://mcp.linear.app/mcp".to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::json!({"auth_mode": "oauth"})),
        },
    )
    .await
    .unwrap()
    .id
    .uuid()
}

fn servers<'a>(
    db: &'a StorageBackend,
    encryption: &'a EncryptionService,
    owner: Uuid,
) -> UserMcpServers<'a> {
    UserMcpServers {
        db,
        encryption: Some(encryption),
        org_id: DEFAULT_ORG_ID,
        owner,
    }
}

fn custom(name: &str) -> AddUserMcpServerRequest {
    AddUserMcpServerRequest {
        name: Some(name.to_string()),
        url: Some("https://notes.example.com/mcp".to_string()),
        auth_mode: Some(McpServerAuthMode::OAuth),
        ..Default::default()
    }
}

#[tokio::test]
async fn user_servers_stay_out_of_the_org_catalog() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let added = servers(&db, &encryption, alice)
        .add(custom("notes"))
        .await
        .unwrap();
    let id: Uuid = added
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();

    assert!(
        db.get_mcp_server(DEFAULT_ORG_ID, id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_mcp_server_by_name(DEFAULT_ORG_ID, "notes")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.list_mcp_servers(DEFAULT_ORG_ID, None, true)
            .await
            .unwrap()
            .iter()
            .all(|row| row.id.uuid() != id)
    );
    assert!(!db.delete_mcp_server(DEFAULT_ORG_ID, id).await.unwrap());
    // A catalog preset may reuse the name a person picked for their own server.
    catalog_preset(&db, "notes").await;
}

#[tokio::test]
async fn names_are_unique_per_person_not_per_org() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let bob = person(&db, "Bob").await;

    servers(&db, &encryption, alice)
        .add(custom("notes"))
        .await
        .unwrap();
    let duplicate = servers(&db, &encryption, alice).add(custom("notes")).await;
    assert!(matches!(duplicate, Err(UserMcpServerError::Conflict(_))));
    servers(&db, &encryption, bob)
        .add(custom("notes"))
        .await
        .unwrap();
}

#[tokio::test]
async fn another_person_cannot_read_change_or_remove_a_server() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let bob = person(&db, "Bob").await;
    let added = servers(&db, &encryption, alice)
        .add(custom("notes"))
        .await
        .unwrap();
    let id = added
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();
    let bobs = servers(&db, &encryption, bob);

    assert!(bobs.list().await.unwrap().is_empty());
    assert!(matches!(
        bobs.get(id).await,
        Err(UserMcpServerError::NotFound)
    ));
    assert!(matches!(
        bobs.update(
            id,
            UpdateUserMcpServerRequest {
                enabled: Some(false),
                ..Default::default()
            }
        )
        .await,
        Err(UserMcpServerError::NotFound)
    ));
    assert!(matches!(
        bobs.remove(id).await,
        Err(UserMcpServerError::NotFound)
    ));
    assert!(
        servers(&db, &encryption, alice)
            .get(id)
            .await
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn catalog_server_reuses_the_preset_sign_in() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let preset = catalog_preset(&db, "linear").await;
    let provider = mcp_oauth_provider_id_for_uuid(preset);
    let mine = servers(&db, &encryption, alice);

    let added = mine
        .add(AddUserMcpServerRequest {
            catalog: Some("linear".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(added.source, UserMcpServerSource::Catalog);
    assert_eq!(added.catalog_name.as_deref(), Some("linear"));
    assert_eq!(added.auth_mode, McpServerAuthMode::OAuth);
    assert_eq!(
        added.connection.provider.as_deref(),
        Some(provider.as_str())
    );
    assert_eq!(
        added.connection.status,
        UserMcpConnectionStatus::NotConnected
    );

    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: alice,
        provider: provider.clone(),
        connection_type: "oauth".to_string(),
        provider_user_id: None,
        provider_username: None,
        access_token_encrypted: Some(encryption.encrypt_string("token").unwrap()),
        refresh_token_encrypted: None,
        scopes: None,
        expires_at: None,
        installation_id: None,
        provider_metadata: None,
    })
    .await
    .unwrap();
    let listed = mine.list().await.unwrap();
    assert_eq!(
        listed[0].connection.status,
        UserMcpConnectionStatus::Connected
    );

    let refused = mine
        .add(AddUserMcpServerRequest {
            catalog: Some("linear".to_string()),
            name: Some("linear_two".to_string()),
            url: Some("https://attacker.example/mcp".to_string()),
            ..Default::default()
        })
        .await;
    assert!(matches!(refused, Err(UserMcpServerError::Invalid(_))));
}

#[tokio::test]
async fn removing_a_custom_server_drops_its_sign_in() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let mine = servers(&db, &encryption, alice);
    let added = mine.add(custom("notes")).await.unwrap();
    let id = added
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();
    let provider = added.connection.provider.clone().unwrap();
    assert_eq!(provider, mcp_oauth_provider_id_for_uuid(id));
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: alice,
        provider: provider.clone(),
        connection_type: "oauth".to_string(),
        provider_user_id: None,
        provider_username: None,
        access_token_encrypted: Some(encryption.encrypt_string("token").unwrap()),
        refresh_token_encrypted: None,
        scopes: None,
        expires_at: None,
        installation_id: None,
        provider_metadata: None,
    })
    .await
    .unwrap();

    mine.remove(id).await.unwrap();

    assert!(mine.list().await.unwrap().is_empty());
    assert!(
        db.get_user_connection(alice, &provider)
            .await
            .unwrap()
            .is_none()
    );
    // The name is free again.
    mine.add(custom("notes")).await.unwrap();
}

#[tokio::test]
async fn custom_servers_are_validated() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    let mine = servers(&db, &encryption, alice);

    let private = mine
        .add(AddUserMcpServerRequest {
            url: Some("http://10.0.0.1/mcp".to_string()),
            ..custom("internal")
        })
        .await;
    assert!(matches!(private, Err(UserMcpServerError::Invalid(_))));

    let keyless = mine
        .add(AddUserMcpServerRequest {
            auth_mode: Some(McpServerAuthMode::ApiKey),
            ..custom("keyed")
        })
        .await;
    assert!(matches!(keyless, Err(UserMcpServerError::Invalid(_))));

    let keyed = mine
        .add(AddUserMcpServerRequest {
            auth_mode: None,
            api_key: Some("secret".to_string()),
            enabled: Some(false),
            ..custom("keyed")
        })
        .await
        .unwrap();
    assert_eq!(keyed.auth_mode, McpServerAuthMode::ApiKey);
    assert_eq!(keyed.connection.status, UserMcpConnectionStatus::Connected);
    assert!(!keyed.enabled);

    let oauth = mine.add(custom("notes")).await.unwrap();
    let oauth_id = oauth
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();
    let refused = mine
        .update(
            oauth_id,
            UpdateUserMcpServerRequest {
                api_key: Some("secret".to_string()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(refused, Err(UserMcpServerError::Invalid(_))));
}

#[tokio::test]
async fn load_on_demand_round_trips_through_add_and_update() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = person(&db, "Alice").await;
    catalog_preset(&db, "linear").await;
    let mine = servers(&db, &encryption, alice);

    let added = mine.add(custom("notes")).await.unwrap();
    assert!(
        added.deferred,
        "a person's server loads on demand by default"
    );
    let id = added
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();

    let changed = mine
        .update(
            id,
            UpdateUserMcpServerRequest {
                deferred: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!changed.deferred);
    assert!(!mine.get(id).await.unwrap().deferred);
    assert!(!mine.list().await.unwrap()[0].deferred);

    // An update that leaves the field out keeps it.
    let renamed = mine
        .update(
            id,
            UpdateUserMcpServerRequest {
                name: Some("notes_two".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!renamed.deferred);

    let catalog = mine
        .add(AddUserMcpServerRequest {
            catalog: Some("linear".to_string()),
            deferred: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!catalog.deferred, "catalog servers take the setting too");
}
