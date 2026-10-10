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

// ============================================================================
// Suggestions
// ============================================================================

/// Answers by tool name: a probability of "yes, it changes things", or a
/// failure for a tool it has no answer for.
struct StubDecisions {
    answers: HashMap<&'static str, f64>,
    asked: std::sync::Mutex<Vec<everruns_core::DecisionRequest>>,
}

#[async_trait::async_trait]
impl everruns_core::DecisionsService for StubDecisions {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: everruns_core::DecisionRequest,
    ) -> everruns_contracts::error::Result<everruns_core::DecisionOutcome> {
        let tool = request.state["tool"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.asked.lock().unwrap().push(request);
        let Some(probability) = self.answers.get(tool.as_str()).copied() else {
            return Err(everruns_contracts::error::AgentLoopError::llm(
                "rating unavailable",
            ));
        };
        Ok(everruns_core::DecisionOutcome {
            answers: [(
                "changes".to_string(),
                everruns_core::DecisionAnswer::Noul { probability },
            )]
            .into(),
            ..Default::default()
        })
    }
}

fn ctx_with(
    db: Arc<StorageBackend>,
    decisions: Option<Arc<dyn everruns_core::DecisionsService>>,
) -> Ctx {
    let ctx = Ctx::minimal_for_test(Caller::internal(ORG), db, None);
    match decisions {
        Some(service) => ctx.with_decisions(service),
        None => ctx,
    }
}

fn suggestions_of(
    tools: &[McpServerTool],
) -> HashMap<&str, (Option<McpToolLabel>, Option<McpToolLabel>)> {
    tools
        .iter()
        .map(|tool| (tool.name.as_str(), (tool.label, tool.suggested_label)))
        .collect()
}

#[tokio::test]
async fn suggestions_fill_only_unlabeled_tools_and_are_never_applied() {
    let db = Arc::new(StorageBackend::test_database());
    let docs = server_with_tools(&db, "docs", &["search", "publish", "labeled", "flaky"]).await;
    db.set_mcp_tool_label(ORG, docs, "labeled", Some("read_only"), None)
        .await
        .unwrap();
    // A suggestion from an earlier run survives a rating that fails now.
    db.set_mcp_tool_suggestion(ORG, docs, "flaky", "changes")
        .await
        .unwrap();
    let stub = Arc::new(StubDecisions {
        answers: HashMap::from([("search", 0.1), ("publish", 0.9), ("labeled", 0.9)]),
        asked: Default::default(),
    });
    let ctx = ctx_with(db.clone(), Some(stub.clone()));
    let id = McpServerId::from_uuid(docs).to_string();

    let tools = SuggestMcpToolLabels { id: id.clone() }
        .execute(&ctx)
        .await
        .unwrap();
    let got = suggestions_of(&tools);
    assert_eq!(got["search"], (None, Some(McpToolLabel::ReadOnly)));
    assert_eq!(got["publish"], (None, Some(McpToolLabel::Changes)));
    assert_eq!(got["labeled"], (Some(McpToolLabel::ReadOnly), None));
    assert_eq!(got["flaky"], (None, Some(McpToolLabel::Changes)));

    let asked = std::mem::take(&mut *stub.asked.lock().unwrap());
    let mut asked_tools: Vec<&str> = asked
        .iter()
        .map(|request| request.state["tool"].as_str().unwrap())
        .collect();
    asked_tools.sort();
    assert_eq!(
        asked_tools,
        ["flaky", "publish", "search"],
        "a labeled tool is never rated"
    );
    assert_eq!(
        asked[0].metadata.get("purpose").map(String::as_str),
        Some("mcp_servers.tool_label_suggestion")
    );

    // Never applied: the session sees no label from a suggestion.
    let capability = mcp_capability_id(docs);
    let service = McpServerService::new(db.clone(), None);
    let definitions = org_mcp_tool_definitions(&service, &db, ORG, [capability.as_str()]).await;
    assert_eq!(
        hints_of(&definitions, "mcp_docs__publish"),
        (None, None, Some(true))
    );

    // Confirming a suggestion by setting the label drops it; clearing the
    // label keeps whatever suggestion is left.
    let set = db
        .set_mcp_tool_label(ORG, docs, "publish", Some("changes"), None)
        .await
        .unwrap();
    assert_eq!(set.suggested_label, None);
    let cleared = db
        .set_mcp_tool_label(ORG, docs, "search", None, None)
        .await
        .unwrap();
    assert_eq!(cleared.suggested_label.as_deref(), Some("read_only"));
    // A suggestion never overwrites a label a person set.
    assert!(
        db.set_mcp_tool_suggestion(ORG, docs, "publish", "read_only")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn suggesting_without_a_decision_service_is_unavailable() {
    let db = Arc::new(StorageBackend::test_database());
    let docs = server_with_tools(&db, "docs", &["search"]).await;
    let id = McpServerId::from_uuid(docs).to_string();
    for decisions in [
        None,
        Some(Arc::new(everruns_core::DisabledDecisionsService)
            as Arc<dyn everruns_core::DecisionsService>),
    ] {
        let error = SuggestMcpToolLabels { id: id.clone() }
            .execute(&ctx_with(db.clone(), decisions))
            .await
            .unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
    let rows = db.list_mcp_tool_labels(ORG, &[docs]).await.unwrap();
    assert!(rows.is_empty(), "nothing is written without a rating");
}
