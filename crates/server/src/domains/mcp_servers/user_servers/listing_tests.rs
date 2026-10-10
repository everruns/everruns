//! A personal sign-in to a catalog server puts it on the person's list (D8).

use super::*;
use crate::storage::{CreateUserConnectionRow, CreateVirtualUserRow, free_user_server_name};
use everruns_contracts::typed_id::{McpServerId, VirtualUserId};
use everruns_core::DEFAULT_ORG_ID;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

async fn virtual_user(db: &StorageBackend, usage: &str) -> Uuid {
    let id = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        org_id: DEFAULT_ORG_ID,
        id,
        usage: usage.to_string(),
        name: "Alice".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    id.uuid()
}

async fn preset(db: &StorageBackend, name: &str) -> Uuid {
    db.create_mcp_server(
        DEFAULT_ORG_ID,
        CreateMcpServerRow {
            name: name.to_string(),
            description: Some("Visti".to_string()),
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
    .uuid()
}

async fn sign_in(db: &StorageBackend, encryption: &EncryptionService, owner: Uuid, server: Uuid) {
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id: owner,
        provider: mcp_oauth_provider_id_for_uuid(server),
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
        url: Some(format!("https://{name}.example.com/mcp")),
        ..Default::default()
    }
}

fn uuid_of(id: &str) -> Uuid {
    id.parse::<McpServerId>().unwrap().uuid()
}

#[tokio::test]
async fn a_sign_in_lists_the_catalog_server_once() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let visti = preset(&db, "visti").await;
    sign_in(&db, &encryption, alice, visti).await;
    let mine = servers(&db, &encryption, alice);

    let first = mine.list_signed_in_catalog_server(visti).await.unwrap();
    let CatalogListing::Added { id, name } = first else {
        panic!("expected a new row, got {first:?}");
    };
    assert_eq!(name, "visti");
    assert_eq!(
        mine.list_signed_in_catalog_server(visti).await.unwrap(),
        CatalogListing::AlreadyListed { id }
    );

    let listed = mine.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    let server = &listed[0];
    assert_eq!(server.source, UserMcpServerSource::Catalog);
    assert_eq!(server.catalog_name.as_deref(), Some("visti"));
    assert_eq!(server.description.as_deref(), Some("Visti"));
    assert!(server.enabled && server.deferred);
    assert_eq!(server.connection.status, UserMcpConnectionStatus::Connected);
    assert_eq!(
        server.connection.provider.as_deref(),
        Some(mcp_oauth_provider_id_for_uuid(visti).as_str())
    );
}

#[tokio::test]
async fn a_server_the_person_turned_off_stays_off() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    preset(&db, "visti").await;
    let mine = servers(&db, &encryption, alice);
    let added = mine
        .add(AddUserMcpServerRequest {
            catalog: Some("visti".to_string()),
            name: Some("my-visti".to_string()),
            enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    let preset_id = db
        .get_mcp_server_by_name(DEFAULT_ORG_ID, "visti")
        .await
        .unwrap()
        .unwrap()
        .id
        .uuid();

    assert_eq!(
        mine.list_signed_in_catalog_server(preset_id).await.unwrap(),
        CatalogListing::AlreadyListed {
            id: uuid_of(&added.id)
        }
    );
    let listed = mine.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].enabled);
}

#[tokio::test]
async fn a_taken_name_gets_the_next_free_suffix() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let linear = preset(&db, "linear").await;
    let mine = servers(&db, &encryption, alice);
    // `Linear` and `linear_2` produce the tool prefixes `linear` and `linear_2`.
    mine.add(custom("Linear")).await.unwrap();
    mine.add(custom("linear_2")).await.unwrap();

    let listed = mine.list_signed_in_catalog_server(linear).await.unwrap();
    assert!(
        matches!(&listed, CatalogListing::Added { name, .. } if name == "linear-3"),
        "{listed:?}"
    );
}

#[test]
fn free_names_keep_the_tool_prefix_valid() {
    assert_eq!(free_user_server_name("visti", []).as_deref(), Some("visti"));
    assert_eq!(
        free_user_server_name("visti", ["visti", "visti-2"]).as_deref(),
        Some("visti-3")
    );
    let long = "a".repeat(64);
    let next = free_user_server_name(&long, [long.as_str()]).unwrap();
    assert_eq!(next.len(), 64);
    assert!(next.ends_with("-2"));
    // A stem cut on a separator never leaves `__` in the prefix.
    let edge = format!("{}-b", "a".repeat(61));
    let next = free_user_server_name(&edge, [edge.as_str()]).unwrap();
    assert_eq!(next, format!("{}-2", "a".repeat(61)));
}

#[tokio::test]
async fn only_people_and_active_presets_are_listed() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let service = virtual_user(&db, "service").await;
    let visti = preset(&db, "visti").await;
    let archived = preset(&db, "old").await;
    db.update_mcp_server(
        DEFAULT_ORG_ID,
        archived,
        crate::storage::UpdateMcpServer {
            status: Some("archived".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mine = servers(&db, &encryption, alice);
    let notes = mine.add(custom("notes")).await.unwrap();

    // An agent's service account signs in as the agent, not as a person.
    assert!(matches!(
        servers(&db, &encryption, service)
            .list_signed_in_catalog_server(visti)
            .await
            .unwrap(),
        CatalogListing::Skipped(_)
    ));
    assert!(matches!(
        mine.list_signed_in_catalog_server(archived).await.unwrap(),
        CatalogListing::Skipped(_)
    ));
    // A custom server's own id is not a catalog preset.
    assert!(matches!(
        mine.list_signed_in_catalog_server(uuid_of(&notes.id))
            .await
            .unwrap(),
        CatalogListing::Skipped(_)
    ));
    assert_eq!(mine.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_full_list_is_left_alone() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let visti = preset(&db, "visti").await;
    let mine = servers(&db, &encryption, alice);
    for n in 0..MAX_USER_MCP_SERVERS {
        mine.add(custom(&format!("s{n}"))).await.unwrap();
    }
    assert!(matches!(
        mine.list_signed_in_catalog_server(visti).await.unwrap(),
        CatalogListing::Skipped(_)
    ));
}

#[tokio::test]
async fn removing_a_catalog_server_signs_the_person_out_of_it() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let visti = preset(&db, "visti").await;
    let provider = mcp_oauth_provider_id_for_uuid(visti);
    sign_in(&db, &encryption, alice, visti).await;
    let mine = servers(&db, &encryption, alice);
    let CatalogListing::Added { id, .. } = mine.list_signed_in_catalog_server(visti).await.unwrap()
    else {
        panic!("expected a new row");
    };

    // Revoking the sign-in alone keeps the server listed, waiting for a sign-in.
    db.delete_user_connection(alice, &provider).await.unwrap();
    let listed = mine.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].connection.status,
        UserMcpConnectionStatus::NotConnected
    );

    // Removing the server deletes the sign-in too.
    sign_in(&db, &encryption, alice, visti).await;
    mine.remove(id).await.unwrap();
    assert!(mine.list().await.unwrap().is_empty());
    assert!(
        db.get_user_connection(alice, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_sign_in_shared_with_another_listed_row_is_kept() {
    let db = StorageBackend::test_database();
    let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
    let alice = virtual_user(&db, "end_user").await;
    let visti = preset(&db, "visti").await;
    let provider = mcp_oauth_provider_id_for_uuid(visti);
    sign_in(&db, &encryption, alice, visti).await;
    let mine = servers(&db, &encryption, alice);
    let first = mine
        .add(AddUserMcpServerRequest {
            catalog: Some("visti".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    mine.add(AddUserMcpServerRequest {
        catalog: Some("visti".to_string()),
        name: Some("visti-work".to_string()),
        ..Default::default()
    })
    .await
    .unwrap();

    mine.remove(uuid_of(&first.id)).await.unwrap();
    assert!(
        db.get_user_connection(alice, &provider)
            .await
            .unwrap()
            .is_some()
    );
}
