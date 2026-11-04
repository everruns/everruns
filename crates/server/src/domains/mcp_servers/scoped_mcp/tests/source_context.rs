//! Source context survives catalog materialization and deferred disclosure.

use super::*;

#[tokio::test]
async fn listed_catalog_tool_search_uses_preset_description() {
    let db = StorageBackend::test_database();
    let preset_id = seed_catalog_server(&db, "geothermal", false).await;
    db.update_mcp_server(
        everruns_core::DEFAULT_ORG_ID,
        preset_id.uuid(),
        UpdateMcpServer {
            description: Some("Geothermal drilling telemetry records".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut attachment = catalog_server("geothermal", McpServerActsAs::None);
    attachment.capability_context = Some(everruns_core::mcp_server::McpCapabilityContext {
        id: "retail".into(),
        name: "Retail operations".into(),
        description: "Cobalt warehouse".into(),
    });
    let servers = ScopedMcpServers::from([("geothermal".to_string(), attachment)]);
    let materialized = materialize_scoped_mcp_servers(&db, everruns_core::DEFAULT_ORG_ID, &servers)
        .await
        .unwrap();
    assert_eq!(
        materialized["geothermal"].capability_context,
        servers["geothermal"].capability_context
    );
    assert_eq!(
        materialized["geothermal"].description.as_deref(),
        Some("Geothermal drilling telemetry records")
    );

    let definitions = build_materialized_scoped_mcp_tool_definitions(
        &db,
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        None,
        None,
        &CatalogPreviewEgress::default(),
    )
    .await
    .unwrap();
    let hits = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&definitions)
        .search("drilling telemetry", 8, |_| true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "mcp_geothermal__echo");
    assert!(hits[0].server_match);
    assert!(!hits[0].tool_match);
    let owner = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&definitions)
        .search("cobalt", 8, |_| true);
    assert_eq!(owner.len(), 1);
    assert!(owner[0].server_match);
    assert!(!owner[0].tool_match);
    assert_eq!(
        definitions[0].hints().capability_description.as_deref(),
        Some("Cobalt warehouse")
    );
    let mut placeholders = vec![everruns_core::deferred_mcp_server_definition(
        "geothermal",
        None,
    )];
    attach_server_search_metadata(&mut placeholders, &materialized);
    let deferred = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&placeholders)
        .search("cobalt", 8, |_| true);
    assert_eq!(deferred.len(), 1);
    assert!(deferred[0].server_match);
}
