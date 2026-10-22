use super::*;
use crate::domains::mcp_servers::scoped_mcp::tests::CatalogPreviewEgress;
use everruns_core::ScopedMcpServer;
use everruns_core::host::InMemorySessionStorageStore;
use std::sync::atomic::Ordering;

fn server(deferred: bool) -> ScopedMcpServer {
    ScopedMcpServer {
        url: "http://8.8.8.8/mcp".to_string(),
        deferred,
        ..Default::default()
    }
}

async fn turn(
    servers: &ScopedMcpServers,
    session: SessionId,
    storage: &InMemorySessionStorageStore,
    egress: &CatalogPreviewEgress,
) -> Vec<String> {
    build_turn_mcp_tool_definitions(
        &StorageBackend::test_database(),
        everruns_core::DEFAULT_ORG_ID,
        servers,
        session,
        None,
        egress,
        Some(storage),
    )
    .await
    .unwrap()
    .iter()
    .map(|tool| tool.name().to_string())
    .collect()
}

#[tokio::test]
async fn deferred_server_is_not_listed_until_revealed() {
    let servers = ScopedMcpServers::from([("docs".to_string(), server(true))]);
    let session = SessionId::new();
    let storage = InMemorySessionStorageStore::new();
    let egress = CatalogPreviewEgress::default();

    // Turn start: one placeholder line, no tools/list round trip.
    assert_eq!(
        turn(&servers, session, &storage, &egress).await,
        ["mcp_docs"]
    );
    assert_eq!(egress.calls.load(Ordering::SeqCst), 0);

    // The model reveals it (tool search or the placeholder itself).
    everruns_core::reveal_deferred_mcp_server(&storage, session, "docs")
        .await
        .unwrap();
    assert_eq!(
        turn(&servers, session, &storage, &egress).await,
        ["mcp_docs__echo"]
    );
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);

    // Another session starts deferred again.
    assert_eq!(
        turn(&servers, SessionId::new(), &storage, &egress).await,
        ["mcp_docs"]
    );
}

#[tokio::test]
async fn placeholder_describes_the_server_by_host() {
    let servers = ScopedMcpServers::from([("docs".to_string(), server(true))]);
    let definitions = build_turn_mcp_tool_definitions(
        &StorageBackend::test_database(),
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        SessionId::new(),
        None,
        &CatalogPreviewEgress::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(
        definitions[0]
            .description()
            .contains("`docs`: tools at 8.8.8.8")
    );
}

#[tokio::test]
async fn placeholder_preserves_attached_description_for_search() {
    let mut server = server(true);
    server.description = Some("Audit ledger records for visible endpoint 01".to_string());
    let servers = ScopedMcpServers::from([("audit01".to_string(), server)]);
    let definitions = build_turn_mcp_tool_definitions(
        &StorageBackend::test_database(),
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        SessionId::new(),
        None,
        &CatalogPreviewEgress::default(),
        None,
    )
    .await
    .unwrap();

    let hits = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&definitions)
        .search_operations("audit ledger", 8, |_| true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "mcp_audit01");
    assert!(hits[0].server_match);
}

#[tokio::test]
async fn deferred_catalog_placeholder_search_uses_preset_description() {
    let db = StorageBackend::test_database();
    let preset_id =
        crate::domains::mcp_servers::scoped_mcp::tests::seed_catalog_server(&db, "audit", false)
            .await;
    db.update_mcp_server(
        everruns_core::DEFAULT_ORG_ID,
        preset_id.uuid(),
        crate::storage::UpdateMcpServer {
            description: Some("Audit ledger records for visible endpoints".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let servers = ScopedMcpServers::from([(
        "audit01".to_string(),
        ScopedMcpServer {
            preset: Some("catalog:audit".parse().unwrap()),
            deferred: true,
            ..Default::default()
        },
    )]);
    let definitions = build_turn_mcp_tool_definitions(
        &db,
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        SessionId::new(),
        None,
        &CatalogPreviewEgress::default(),
        None,
    )
    .await
    .unwrap();

    let hits = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&definitions)
        .search("audit ledger", 8, |_| true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "mcp_audit01");
    assert!(hits[0].server_match);
}

#[tokio::test]
async fn eager_servers_are_listed_as_before() {
    let servers = ScopedMcpServers::from([("docs".to_string(), server(false))]);
    let egress = CatalogPreviewEgress::default();
    assert_eq!(
        turn(
            &servers,
            SessionId::new(),
            &InMemorySessionStorageStore::new(),
            &egress
        )
        .await,
        ["mcp_docs__echo"]
    );
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_listing_keeps_the_tools_listed_last() {
    let servers = ScopedMcpServers::from([("docs".to_string(), server(false))]);
    let session = SessionId::new();
    let storage = InMemorySessionStorageStore::new();
    let egress = CatalogPreviewEgress::default();

    assert_eq!(
        turn(&servers, session, &storage, &egress).await,
        ["mcp_docs__echo"]
    );

    // The server goes down: the turn keeps the tools it listed last instead
    // of dropping them, so the model's tool list does not change.
    egress.down.store(true, Ordering::SeqCst);
    assert_eq!(
        turn(&servers, session, &storage, &egress).await,
        ["mcp_docs__echo"]
    );

    // A session with no earlier listing has nothing to keep.
    assert!(
        turn(&servers, SessionId::new(), &storage, &egress)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn the_last_listed_record_is_reserved_from_kv_store() {
    let key = format!("{}docs", everruns_core::MCP_LAST_TOOLS_KV_PREFIX);
    assert!(everruns_core::host::session_services::capabilities::is_internal_session_kv_key(&key));
}
