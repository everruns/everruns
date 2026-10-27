use super::*;
use crate::capabilities::platform::PlatformCapability;
use crate::platform_store::{PlatformStoreExt, tests::MockPlatformStore};
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
use everruns_contracts::typed_id::SessionId;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

#[test]
fn complete_catalog_has_unique_deferable_names_and_valid_structured_schemas() {
    let tools = PlatformCapability.tools();
    assert!(tools.len() >= 333);
    let mut names = HashSet::new();
    for tool in &tools {
        assert!(names.insert(tool.name()));
        assert!(tool.name().starts_with("platform_"));
        assert_ne!(
            tool.deferrable_policy(),
            everruns_contracts::tool_types::DeferrablePolicy::Never
        );
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("reason").is_some());
        assert!(schema["properties"].get("context_revision").is_some());
        assert!(schema["properties"].get("organization_id").is_none());
        jsonschema::validator_for(&schema)
            .unwrap_or_else(|error| panic!("{}: {error}", tool.name()));
    }
    let definitions: Vec<_> = tools.iter().map(|tool| tool.to_definition()).collect();
    let index = ToolSearchIndex::new(&definitions);
    for tool in &tools {
        let hits = index.search_operations(tool.name(), 8, |_| true);
        assert_eq!(hits[0].name, tool.name());
        assert!(hits.len() <= 8);
    }
}

#[test]
fn flattened_schemas_preserve_typed_fields_and_required_ids() {
    let tools = tools();
    let update = tools
        .iter()
        .find(|tool| tool.name() == "platform_update_agent")
        .unwrap();
    let schema = update.parameters_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&json!({"id":"agent_example", "name":"Updated", "reason":"User request", "context_revision":4})));
    assert!(!validator.is_valid(&json!({"name":"Updated"})));
    assert!(!validator.is_valid(&json!({"id":"agent_example", "context_revision":"four"})));
    assert!(!validator.is_valid(&json!({"id":"agent_example", "organization_id":42})));
    assert!(schema["properties"].get("capabilities").is_some());
}

#[tokio::test]
async fn native_dispatch_preserves_structured_arguments_and_invocation_metadata() {
    let store = Arc::new(MockPlatformStore::new());
    let context = ToolContext::new(SessionId::new())
        .with_extension(Arc::new(PlatformStoreExt(store.clone())));
    let tools = tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "platform_update_agent")
        .unwrap();
    let arguments = json!({"id":"agent_example", "capabilities":[{"ref":"current_time"}], "reason":"User requested time access", "context_revision":7});
    assert!(
        tool.execute_with_context(arguments.clone(), &context)
            .await
            .is_success()
    );
    assert_eq!(
        *store.platform_calls.lock().unwrap(),
        vec![("update_agent".into(), arguments)]
    );
    for arguments in [json!({"organization_id":2}), json!([])] {
        assert!(
            !tool
                .execute_with_context(arguments, &context)
                .await
                .is_success()
        );
    }
    assert_eq!(store.platform_calls.lock().unwrap().len(), 1);
    assert!(
        !tool
            .execute_with_context(json!({}), &ToolContext::new(SessionId::new()))
            .await
            .is_success()
    );
    assert!(!tool.execute(json!({})).await.is_success());
}
