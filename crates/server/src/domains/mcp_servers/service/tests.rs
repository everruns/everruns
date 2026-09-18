use super::*;
use crate::storage::{EncryptionService, StorageBackend, models::CreateMcpServerRow};
use everruns_core::{McpServerTransportType, OrgRole};

fn test_caller(org_id: i64) -> Caller {
    Caller {
        org_id,
        org_public_id: format!("org_{org_id:032}"),
        user_id: None,
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
    }
}

fn test_encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    )
}

// SSE/JSON extraction is now covered by `everruns-mcp` (the shared client);
// see crates/mcp/src/result.rs tests.

#[tokio::test]
async fn decrypt_api_key_returns_none_when_no_key_set() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));

    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "test-server".into(),
                description: None,
                url: "https://example.com".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let result = svc
        .decrypt_api_key(&test_caller(1), row.id.uuid())
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn update_rejects_oauth_server_retargeting() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let settings = McpServerSettings {
        auth_mode: McpServerAuthMode::OAuth,
        ..Default::default()
    };
    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "oauth-server".into(),
                description: None,
                url: "https://original.example/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(McpServerService::settings_to_value(&settings)),
            },
        )
        .await
        .unwrap();
    let req: UpdateMcpServerRequest = serde_json::from_value(serde_json::json!({
        "url": "https://attacker.example/mcp"
    }))
    .unwrap();

    let error = svc
        .update(&test_caller(1), row.id.uuid(), req)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("create a new server"));
}

#[tokio::test]
async fn decrypt_api_key_returns_decrypted_key() {
    let encryption = test_encryption();
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(encryption.clone()));

    let encrypted = encryption.encrypt_string("sk-secret-123").unwrap();

    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "authed-server".into(),
                description: None,
                url: "https://example.com".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: Some(encrypted),
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let result = svc
        .decrypt_api_key(&test_caller(1), row.id.uuid())
        .await
        .unwrap();
    assert_eq!(result.as_deref(), Some("sk-secret-123"));
}

#[tokio::test]
async fn decrypt_api_key_errors_without_encryption_service() {
    let encryption = test_encryption();
    let db = Arc::new(StorageBackend::in_memory());

    // Create server WITH encrypted key
    let encrypted = encryption.encrypt_string("sk-secret").unwrap();
    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "no-enc-server".into(),
                description: None,
                url: "https://example.com".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: Some(encrypted),
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Service WITHOUT encryption configured
    let svc = McpServerService::new(db, None);
    let result = svc.decrypt_api_key(&test_caller(1), row.id.uuid()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn decrypt_api_key_errors_for_missing_server() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db, Some(test_encryption()));

    let result = svc.decrypt_api_key(&test_caller(1), Uuid::new_v4()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn catalog_transport_resolution_omits_configured_credentials() {
    let db = Arc::new(StorageBackend::in_memory());
    db.create_mcp_server(
        1,
        CreateMcpServerRow {
            name: "catalog-auth".into(),
            description: None,
            url: "https://example.com/mcp".into(),
            transport_type: "streamable_http".into(),
            api_key_encrypted: Some(vec![1, 2, 3]),
            headers: Some(serde_json::json!({"X-Literal": "value"})),
            settings: None,
        },
    )
    .await
    .unwrap();
    let svc = McpServerService::new(db, None);

    let resolved = svc
        .resolve_transport_by_name(&test_caller(1), "catalog-auth")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(resolved.auth_mode, McpServerAuthMode::None);
    assert!(resolved.oauth_provider_id.is_none());
    assert!(resolved.api_key.is_none());
    assert_eq!(
        resolved.headers.get("X-Literal"),
        Some(&"value".to_string())
    );
}

// --- resolve_by_prefix tests ---

#[tokio::test]
async fn resolve_by_prefix_finds_matching_server() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));

    db.create_mcp_server(
        1,
        CreateMcpServerRow {
            name: "My Cool Server".into(),
            description: None,
            url: "https://example.com/mcp".into(),
            transport_type: "streamable_http".into(),
            api_key_encrypted: None,
            headers: None,
            settings: None,
        },
    )
    .await
    .unwrap();

    for name in ["docs_", "docs__private", "docs-"] {
        db.create_mcp_server(
            1,
            CreateMcpServerRow {
                name: name.into(),
                description: None,
                url: "https://invalid.example/mcp".into(),
                transport_type: "http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();
        let prefix = everruns_core::sanitize_mcp_server_name(name);
        assert!(
            svc.resolve_by_prefix(&test_caller(1), &prefix)
                .await
                .unwrap()
                .is_none(),
            "stored invalid name {name} resolved"
        );
    }

    let resolved = svc
        .resolve_by_prefix(&test_caller(1), "my_cool_server")
        .await
        .unwrap();
    assert!(resolved.is_some());
    let r = resolved.unwrap();
    assert_eq!(r.name, "My Cool Server");
    assert_eq!(r.url, "https://example.com/mcp");
    assert!(r.api_key.is_none());
}

#[tokio::test]
async fn resolve_by_prefix_returns_none_for_no_match() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db, Some(test_encryption()));

    let resolved = svc
        .resolve_by_prefix(&test_caller(1), "nonexistent")
        .await
        .unwrap();
    assert!(resolved.is_none());
}

#[tokio::test]
async fn resolve_by_prefix_decrypts_api_key() {
    let encryption = test_encryption();
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(encryption.clone()));

    let encrypted = encryption.encrypt_string("sk-mcp-secret").unwrap();
    db.create_mcp_server(
        1,
        CreateMcpServerRow {
            name: "Auth Server".into(),
            description: None,
            url: "https://example.com/mcp".into(),
            transport_type: "streamable_http".into(),
            api_key_encrypted: Some(encrypted),
            headers: None,
            settings: None,
        },
    )
    .await
    .unwrap();

    let resolved = svc
        .resolve_by_prefix(&test_caller(1), "auth_server")
        .await
        .unwrap();
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().api_key.as_deref(), Some("sk-mcp-secret"));
}

#[tokio::test]
async fn resolve_by_prefix_ignores_disabled_server() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));

    let created = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "Disabled Server".into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    db.update_mcp_server(
        1,
        created.id.uuid(),
        UpdateMcpServer {
            status: Some("disabled".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let resolved = svc
        .resolve_by_prefix(&test_caller(1), "disabled_server")
        .await
        .unwrap();
    assert!(resolved.is_none());
}

// --- SSRF: fetch_mcp_tools blocks unsafe URLs ---

fn test_egress() -> DirectEgressService {
    DirectEgressService::default()
}

#[tokio::test]
async fn fetch_tools_blocks_localhost() {
    let egress = test_egress();
    let result =
        super::fetch_mcp_tools(&egress, "http://localhost:9999/mcp", None, &HashMap::new()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("blocked"));
}

#[tokio::test]
async fn fetch_tools_blocks_private_ip() {
    let egress = test_egress();
    let result =
        super::fetch_mcp_tools(&egress, "http://10.0.0.1/mcp", None, &HashMap::new()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("blocked"));
}

#[tokio::test]
async fn fetch_tools_blocks_metadata_endpoint() {
    let egress = test_egress();
    let result = super::fetch_mcp_tools(
        &egress,
        "http://169.254.169.254/latest/meta-data/",
        None,
        &HashMap::new(),
    )
    .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("blocked"));
}

#[tokio::test]
async fn resolve_by_prefix_errors_when_encryption_missing_for_key() {
    let encryption = test_encryption();
    let db = Arc::new(StorageBackend::in_memory());

    let encrypted = encryption.encrypt_string("sk-secret").unwrap();
    db.create_mcp_server(
        1,
        CreateMcpServerRow {
            name: "No Enc".into(),
            description: None,
            url: "https://example.com".into(),
            transport_type: "streamable_http".into(),
            api_key_encrypted: Some(encrypted),
            headers: None,
            settings: None,
        },
    )
    .await
    .unwrap();

    // Service without encryption
    let svc = McpServerService::new(db, None);
    let result = svc.resolve_by_prefix(&test_caller(1), "no_enc").await;
    assert!(result.is_err());
}

#[test]
fn settings_from_row_defaults_auth_mode_for_legacy_api_key_servers() {
    let row = McpServerRow {
        id: everruns_provider::typed_id::McpServerId::new(),
        org_id: 1,
        name: "legacy".into(),
        description: None,
        url: "https://example.com/mcp".into(),
        transport_type: "http".into(),
        status: "active".into(),
        api_key_encrypted: Some(vec![1, 2, 3]),
        api_key_set: true,
        headers: serde_json::json!({}),
        settings: serde_json::json!({}),
        cached_tools: serde_json::json!([]),
        tools_cached_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    };

    let settings = McpServerService::settings_from_row(&row);
    assert_eq!(settings.auth_mode, McpServerAuthMode::ApiKey);
}

#[tokio::test]
async fn create_rejects_api_key_when_auth_mode_is_not_api_key() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db, Some(test_encryption()));

    let result = svc
        .create(
            &test_caller(1),
            CreateMcpServerRequest {
                name: "bad-server".into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: McpServerTransportType::Http,
                auth_mode: Some(McpServerAuthMode::None),
                protocol_mode: None,
                api_key: Some("secret".into()),
                headers: None,
            },
        )
        .await;

    assert!(result.is_err());
}

// --- Tool-cache freshness + single-flight refresh ---

fn sample_row() -> McpServerRow {
    McpServerRow {
        id: everruns_provider::typed_id::McpServerId::new(),
        org_id: 1,
        name: "srv".into(),
        description: None,
        url: "https://example.com/mcp".into(),
        transport_type: "streamable_http".into(),
        status: "active".into(),
        api_key_encrypted: None,
        api_key_set: false,
        headers: serde_json::json!({}),
        settings: serde_json::json!({}),
        cached_tools: serde_json::json!([]),
        tools_cached_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

#[test]
fn cache_fresh_respects_ttl() {
    let mut row = sample_row();

    // Never fetched -> not fresh.
    row.tools_cached_at = None;
    assert!(!McpServerService::cache_fresh(&row));

    // Just fetched -> fresh.
    row.tools_cached_at = Some(Utc::now());
    assert!(McpServerService::cache_fresh(&row));

    // Older than the TTL -> stale.
    row.tools_cached_at = Some(Utc::now() - chrono::Duration::hours(2));
    assert!(!McpServerService::cache_fresh(&row));
}

#[test]
fn cache_within_max_stale_bounds_stale_serving() {
    let mut row = sample_row();

    row.tools_cached_at = None;
    assert!(!McpServerService::cache_within_max_stale(&row));

    row.tools_cached_at = Some(Utc::now() - chrono::Duration::hours(2));
    assert!(McpServerService::cache_within_max_stale(&row));

    row.tools_cached_at = Some(Utc::now() - chrono::Duration::hours(25));
    assert!(!McpServerService::cache_within_max_stale(&row));
}

#[tokio::test]
async fn keyed_locks_coalesce_and_prune() {
    let locks = KeyedLocks::default();
    let key = (1, Uuid::new_v4());

    let lock = locks.lock_for(key);
    let guard = lock.clone().try_lock_owned().expect("first acquire");

    // A second request for the same key shares the lock and cannot acquire
    // it while the first guard is held (single-flight).
    assert!(locks.lock_for(key).try_lock_owned().is_err());

    drop(guard);
    // Once released the lock is acquirable again.
    assert!(lock.clone().try_lock_owned().is_ok());

    // Idle keys are pruned when a new key is requested.
    drop(lock);
    let _other = locks.lock_for((2, Uuid::new_v4()));
    assert_eq!(locks.map.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn get_tools_serves_stale_cache_while_revalidating() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let caller = test_caller(1);

    // Non-OAuth server with an SSRF-blocked URL: a live refresh would fail,
    // so returning the cached tool proves we served the stale cache.
    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "stale-server".into(),
                description: None,
                url: "http://10.0.0.1/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let id = row.id.uuid();

    // Seed a successful fetch, then backdate it past the TTL.
    db.update_mcp_server_tools(
        1,
        id,
        UpdateMcpServerTools {
            cached_tools: serde_json::json!([
                {"name": "alpha", "description": "A", "inputSchema": {"type": "object"}}
            ]),
        },
    )
    .await
    .unwrap();
    let StorageBackend::InMemory(mem) = db.as_ref() else {
        panic!("expected in-memory backend");
    };
    mem.set_tools_cached_at_for_test(id, Utc::now() - chrono::Duration::hours(2));

    let tools = svc.get_tools(&caller, id, false).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "alpha");
}

#[tokio::test]
async fn get_tools_rejects_expired_stale_cache_when_refresh_fails() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let caller = test_caller(1);

    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "expired-stale-server".into(),
                description: None,
                url: "http://10.0.0.1/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let id = row.id.uuid();

    db.update_mcp_server_tools(
        1,
        id,
        UpdateMcpServerTools {
            cached_tools: serde_json::json!([
                {"name": "poisoned", "description": "P", "inputSchema": {"type": "object"}}
            ]),
        },
    )
    .await
    .unwrap();
    let StorageBackend::InMemory(mem) = db.as_ref() else {
        panic!("expected in-memory backend");
    };
    mem.set_tools_cached_at_for_test(id, Utc::now() - chrono::Duration::hours(25));

    let result = svc.get_tools(&caller, id, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn batch_omits_expired_stale_cache_when_refresh_fails() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let caller = test_caller(1);

    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "expired-batch-server".into(),
                description: None,
                url: "http://10.0.0.1/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let id = row.id.uuid();

    db.update_mcp_server_tools(
        1,
        id,
        UpdateMcpServerTools {
            cached_tools: serde_json::json!([
                {"name": "poisoned", "description": "P", "inputSchema": {"type": "object"}}
            ]),
        },
    )
    .await
    .unwrap();
    let StorageBackend::InMemory(mem) = db.as_ref() else {
        panic!("expected in-memory backend");
    };
    mem.set_tools_cached_at_for_test(id, Utc::now() - chrono::Duration::hours(25));

    let servers = svc.get_batch_with_tools(&caller, &[id]).await.unwrap();
    let (_, tools) = servers.get(&id).expect("server is returned");
    assert!(tools.is_empty());
}

#[tokio::test]
async fn get_tools_cold_cache_surfaces_refresh_error() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));

    // Never fetched and unreachable: with no stale cache to serve, the cold
    // path must block on the refresh and surface its error.
    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "cold-server".into(),
                description: None,
                url: "http://10.0.0.1/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let result = svc.get_tools(&test_caller(1), row.id.uuid(), false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn oauth_tools_are_never_served_from_the_shared_org_row() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let settings = McpServerSettings {
        auth_mode: McpServerAuthMode::OAuth,
        ..Default::default()
    };
    let row = db
        .create_mcp_server(
            1,
            CreateMcpServerRow {
                name: "oauth-shared-cache".into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: "streamable_http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(McpServerService::settings_to_value(&settings)),
            },
        )
        .await
        .unwrap();
    let id = row.id.uuid();
    db.update_mcp_server_tools(
        1,
        id,
        UpdateMcpServerTools {
            cached_tools: serde_json::json!([{
                "name": "user_a_private_tool",
                "description": "Only visible to user A",
                "inputSchema": {"type": "object"}
            }]),
        },
    )
    .await
    .unwrap();

    assert!(svc.get_tools(&test_caller(1), id, false).await.is_err());
    let listed = svc.list_active_with_tools(&test_caller(1)).await.unwrap();
    assert!(listed[0].cached_tools.is_empty());
    assert!(
        svc.get_cached_tools(&test_caller(1), id)
            .await
            .unwrap()
            .is_empty()
    );
    let batch = svc
        .get_batch_with_tools(&test_caller(1), &[id])
        .await
        .unwrap();
    assert!(
        batch
            .get(&id)
            .expect("OAuth server remains in the batch")
            .1
            .is_empty()
    );
    let persisted = db.get_mcp_server(1, id).await.unwrap().unwrap();
    assert_eq!(persisted.cached_tools, serde_json::json!([]));
    assert!(persisted.tools_cached_at.is_none());
}
#[tokio::test]
async fn create_and_rename_reject_ambiguous_names_without_changing_stored_identity() {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = McpServerService::new(db.clone(), Some(test_encryption()));
    let caller = test_caller(1);
    for name in ["docs_", "docs-", "docs__private", ""] {
        let request: CreateMcpServerRequest = serde_json::from_value(
            serde_json::json!({"name":name,"url":"https://example.com/mcp"}),
        )
        .unwrap();
        assert!(svc.create(&caller, request).await.is_err(), "{name}");
    }
    assert!(svc.list(&caller, None, false).await.unwrap().is_empty());
    let request: CreateMcpServerRequest = serde_json::from_value(
        serde_json::json!({"name":"docs_api","url":"https://example.com/mcp"}),
    )
    .unwrap();
    let server = svc.create(&caller, request).await.unwrap();
    let update: UpdateMcpServerRequest =
        serde_json::from_value(serde_json::json!({"name":"docs_"})).unwrap();
    assert!(svc.update(&caller, server.id.uuid(), update).await.is_err());
    assert_eq!(
        svc.get(&caller, server.id.uuid())
            .await
            .unwrap()
            .unwrap()
            .name,
        "docs_api"
    );
}
