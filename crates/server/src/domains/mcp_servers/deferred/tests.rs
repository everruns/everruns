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

#[tokio::test]
async fn readiness_keeps_fast_peer_when_first_server_is_pending() {
    struct MixedEgress(CatalogPreviewEgress);
    #[async_trait::async_trait]
    impl EgressService for MixedEgress {
        async fn send(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
            if request.url.contains("slow") {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            self.0.send(request).await
        }
        async fn send_stream(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(everruns_core::EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter(vec![Ok(response.body)])),
            })
        }
    }
    let egress = Arc::new(MixedEgress(CatalogPreviewEgress::default()));
    let session = SessionId::new();
    let servers = ScopedMcpServers::from([
        (
            "a_slow".into(),
            ScopedMcpServer {
                url: "http://8.8.8.8/slow".into(),
                ..server(false)
            },
        ),
        ("z_fast".into(), server(false)),
    ]);
    let readiness = super::super::scoped_mcp::Readiness::new(
        everruns_core::DEFAULT_ORG_ID,
        session,
        egress.clone(),
        std::time::Duration::from_millis(100),
    );
    let storage = InMemorySessionStorageStore::new();
    let tools = build_turn_mcp_tool_definitions_inner(
        &StorageBackend::test_database(),
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        session,
        None,
        egress.as_ref(),
        Some(&storage),
        Some(&readiness),
    )
    .await
    .unwrap();
    assert!(tools.iter().any(|t| t.name().starts_with("mcp_z_fast_")));
    assert!(tools.iter().all(|t| !t.name().starts_with("mcp_a_slow_")));
    assert_eq!(*readiness.pending.lock().unwrap(), vec!["a_slow"]);
    let snapshot = storage
        .get_value(session, super::super::deferred::DISCOVERY_STATUS_KV_KEY)
        .await
        .unwrap()
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot["a_slow"]["status"], "pending");
    assert_eq!(snapshot["z_fast"]["status"], "ready");
}

#[tokio::test]
async fn readiness_bounds_last_known_catalog_restoration() {
    let session = SessionId::new();
    let storage = InMemorySessionStorageStore::new();
    let servers = ScopedMcpServers::from([("docs".into(), server(false))]);
    let tool = everruns_core::deferred_mcp_server_definition("docs", None);
    let mut definitions = vec![tool.clone(); everruns_core::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS];
    storage
        .set_value(
            session,
            &format!("{}docs", everruns_core::MCP_LAST_TOOLS_KV_PREFIX),
            &serde_json::to_string(&vec![tool]).unwrap(),
        )
        .await
        .unwrap();
    let error = keep_last_listed_tools(
        &storage,
        session,
        &servers,
        &["docs".into()],
        &mut definitions,
        true,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("aggregate catalog bounds"));
    assert_eq!(
        definitions.len(),
        everruns_core::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS
    );
}

#[test]
fn discovery_snapshot_namespace_is_reserved_and_cannot_alias_server_tools() {
    assert!(
        everruns_core::host::session_services::is_internal_session_kv_key(DISCOVERY_STATUS_KV_KEY)
    );
    let suffix = DISCOVERY_STATUS_KV_KEY
        .strip_prefix(everruns_core::MCP_LAST_TOOLS_KV_PREFIX)
        .unwrap();
    assert_ne!(
        suffix,
        everruns_core::mcp_server::sanitize_mcp_server_name(suffix)
    );
}

#[tokio::test]
async fn blocking_turn_clears_previous_readiness_snapshot() {
    let session = SessionId::new();
    let storage = InMemorySessionStorageStore::new();
    let servers = ScopedMcpServers::from([("docs".into(), server(false))]);
    let egress = CatalogPreviewEgress::default();
    for status in ["pending", "failed"] {
        storage
            .set_value(
                session,
                DISCOVERY_STATUS_KV_KEY,
                &serde_json::json!({"docs": {"status": status}}).to_string(),
            )
            .await
            .unwrap();
        assert!(!turn(&servers, session, &storage, &egress).await.is_empty());
        assert!(
            storage
                .get_value(session, DISCOVERY_STATUS_KV_KEY)
                .await
                .unwrap()
                .is_none()
        );
    }
}
