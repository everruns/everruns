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
