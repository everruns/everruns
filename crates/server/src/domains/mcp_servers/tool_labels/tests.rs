use super::*;
use crate::storage::{CreateMcpServerRow, UpdateMcpServerTools};
use everruns_core::mcp::mcp_capability_id;
use serde_json::json;
use std::sync::Arc;

const ORG: i64 = 1;

async fn server_with_tools(db: &StorageBackend, name: &str, tools: &[&str]) -> Uuid {
    let id = db
        .create_mcp_server(
            ORG,
            CreateMcpServerRow {
                name: name.into(),
                description: None,
                url: "https://example.com/mcp".into(),
                transport_type: "http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap()
        .id
        .uuid();
    cache_tools(db, id, tools).await;
    id
}

async fn cache_tools(db: &StorageBackend, id: Uuid, tools: &[&str]) {
    let tools: Vec<_> = tools
        .iter()
        .map(|name| json!({"name": name, "inputSchema": {"type": "object"}}))
        .collect();
    db.update_mcp_server_tools(
        ORG,
        id,
        UpdateMcpServerTools {
            cached_tools: json!(tools),
        },
    )
    .await
    .unwrap();
}

fn hints_of(
    definitions: &[ToolDefinition],
    name: &str,
) -> (Option<bool>, Option<bool>, Option<bool>) {
    let hints = definitions
        .iter()
        .find(|def| def.name() == name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .hints();
    (hints.readonly, hints.destructive, hints.open_world)
}

#[tokio::test]
async fn set_clear_and_load_labels_in_one_batch() {
    let db = StorageBackend::test_database();
    let docs = server_with_tools(&db, "docs", &["search", "delete"]).await;
    let crm = server_with_tools(&db, "crm", &["update"]).await;

    db.set_mcp_tool_label(ORG, docs, "search", Some("read_only"), None)
        .await
        .unwrap();
    db.set_mcp_tool_label(ORG, docs, "delete", Some("changes"), None)
        .await
        .unwrap();
    db.set_mcp_tool_label(ORG, crm, "update", Some("read_only"), None)
        .await
        .unwrap();
    // Clearing keeps the row but drops the label.
    let cleared = db
        .set_mcp_tool_label(ORG, crm, "update", None, None)
        .await
        .unwrap();
    assert_eq!(cleared.label, None);

    let labels = load_tool_labels(&db, ORG, &[docs, crm]).await.unwrap();
    assert_eq!(
        labels.get(&docs),
        Some(&McpToolLabels::from([
            ("search".to_string(), McpToolLabel::ReadOnly),
            ("delete".to_string(), McpToolLabel::Changes),
        ]))
    );
    assert!(!labels.contains_key(&crm), "a cleared label is not applied");

    let by_name = load_tool_labels_by_server_names(&db, ORG, &["docs".to_string()])
        .await
        .unwrap();
    assert_eq!(by_name["docs"].get("search"), Some(&McpToolLabel::ReadOnly));
    // Another org never sees them.
    assert!(load_tool_labels(&db, 2, &[docs]).await.unwrap().is_empty());
}

#[tokio::test]
async fn session_definitions_apply_labels_and_keep_them_across_tool_refreshes() {
    let db = Arc::new(StorageBackend::test_database());
    let service = McpServerService::new(db.clone(), None);
    let docs = server_with_tools(&db, "docs", &["search", "delete", "other"]).await;
    db.set_mcp_tool_label(ORG, docs, "search", Some("read_only"), None)
        .await
        .unwrap();
    db.set_mcp_tool_label(ORG, docs, "delete", Some("changes"), None)
        .await
        .unwrap();
    let capability = mcp_capability_id(docs);

    let definitions =
        org_mcp_tool_definitions(&service, &db, ORG, [capability.as_str(), "current_time"]).await;
    assert_eq!(definitions.len(), 3);
    assert_eq!(
        hints_of(&definitions, "mcp_docs__search"),
        (Some(true), Some(false), Some(false))
    );
    assert_eq!(
        hints_of(&definitions, "mcp_docs__delete"),
        (Some(false), Some(true), Some(true))
    );
    // No label: today's behaviour, an external tool that asks in normal mode.
    assert_eq!(
        hints_of(&definitions, "mcp_docs__other"),
        (None, None, Some(true))
    );

    // The tool list changes; the label stays with the tool name.
    cache_tools(&db, docs, &["search", "renamed"]).await;
    let definitions = org_mcp_tool_definitions(&service, &db, ORG, [capability.as_str()]).await;
    assert_eq!(definitions.len(), 2);
    assert_eq!(
        hints_of(&definitions, "mcp_docs__search"),
        (Some(true), Some(false), Some(false))
    );
}

#[tokio::test]
async fn a_deleted_server_contributes_no_tools_or_labels() {
    let db = Arc::new(StorageBackend::test_database());
    let service = McpServerService::new(db.clone(), None);
    let docs = server_with_tools(&db, "docs", &["search"]).await;
    db.set_mcp_tool_label(ORG, docs, "search", Some("read_only"), None)
        .await
        .unwrap();
    db.update_mcp_server(
        ORG,
        docs,
        crate::storage::UpdateMcpServer {
            status: Some("archived".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(db.destroy_mcp_server(ORG, docs).await.unwrap());

    let capability = mcp_capability_id(docs);
    assert!(
        org_mcp_tool_definitions(&service, &db, ORG, [capability.as_str()])
            .await
            .is_empty()
    );
    assert!(
        load_tool_labels_by_server_names(&db, ORG, &["docs".to_string()])
            .await
            .unwrap()
            .is_empty()
    );
}
