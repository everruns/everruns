use super::tests::{catalog_server, test_agent, test_harness, test_session};
use super::*;
use crate::storage::CreateMcpServerRow;

#[tokio::test]
async fn catalog_attachment_inherits_or_overrides_the_preset_tool_timeout() {
    let db = Arc::new(StorageBackend::test_database());
    let settings = crate::domains::mcp_servers::service::McpServerSettings {
        tool_timeout_ms: Some(300_000),
        ..Default::default()
    };
    db.create_mcp_server(
        everruns_core::DEFAULT_ORG_ID,
        CreateMcpServerRow {
            name: "ci".to_string(),
            description: None,
            url: "http://8.8.8.8/mcp".to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::to_value(settings).unwrap()),
        },
    )
    .await
    .unwrap();
    let service = McpServerService::new(db.clone(), None);
    let harness = test_harness();
    let agent = test_agent();

    for (attached, expected) in [(None, Some(300_000)), (Some(900_000), Some(900_000))] {
        let mut server = catalog_server("ci", McpServerActsAs::None);
        server.tool_timeout_ms = attached;
        let servers = ScopedMcpServers::from([("ci".to_string(), server)]);
        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers = servers.clone();

        let resolved = resolve_scoped_mcp_server(
            &service,
            everruns_core::DEFAULT_ORG_ID,
            &harness,
            Some(&agent),
            &session,
            "ci",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(resolved.tool_timeout_ms, expected);

        let materialized =
            materialize_scoped_mcp_servers(&db, everruns_core::DEFAULT_ORG_ID, &servers)
                .await
                .unwrap();
        assert_eq!(materialized["ci"].tool_timeout_ms, expected);
    }
}
