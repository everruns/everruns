use everruns_server::openapi::ApiDoc;
use serde_json::Value;
use std::collections::BTreeMap;
use utoipa::{OpenApi, PartialSchema};

fn spec_value() -> Value {
    serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document is valid JSON")
}

fn contains_key(value: &Value, needle: &str) -> bool {
    match value {
        Value::Array(values) => values.iter().any(|value| contains_key(value, needle)),
        Value::Object(values) => {
            values.contains_key(needle) || values.values().any(|value| contains_key(value, needle))
        }
        _ => false,
    }
}

#[test]
fn retired_cost_tier_extension_is_not_emitted() {
    assert!(!contains_key(&spec_value(), "x-cost-tier"));
}
#[test]
fn app_openapi_surface_is_read_only_and_deprecated() {
    let spec = spec_value();
    let paths = spec
        .pointer("/paths")
        .and_then(Value::as_object)
        .expect("OpenAPI paths exist");

    assert_eq!(
        paths
            .get("/v1/apps")
            .and_then(|item| item.pointer("/get/deprecated"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        paths
            .get("/v1/apps/{app_id}")
            .and_then(|item| item.pointer("/get/deprecated"))
            .and_then(Value::as_bool),
        Some(true)
    );

    for path in ["/v1/apps", "/v1/apps/{app_id}"] {
        let operations = paths[path].as_object().expect("path item is an object");
        assert_eq!(operations.len(), 1, "{path} has only one operation");
        assert!(operations.contains_key("get"), "{path} is read-only");
    }
    for retired in [
        "/v1/apps/{app_id}/publish",
        "/v1/apps/{app_id}/unpublish",
        "/v1/apps/{app_id}/run",
        "/v1/apps/{app_id}/runs",
        "/v1/apps/{app_id}/channels",
        "/v1/apps/{app_id}/channels/{channel_id}",
    ] {
        assert!(
            !paths.contains_key(retired),
            "stale management path {retired}"
        );
    }

    let schemas = spec
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .expect("OpenAPI schemas exist");
    for retired in [
        "CreateAppRequest",
        "UpdateAppRequest",
        "AddChannelRequest",
        "UpdateChannelRequest",
        "RunAppRequest",
    ] {
        assert!(!schemas.contains_key(retired), "stale schema {retired}");
    }
}

#[test]
fn scoped_mcp_schema_uses_public_wire_names() {
    let schema =
        serde_json::to_value(<everruns_core::ScopedMcpServer as PartialSchema>::schema()).unwrap();
    let properties = schema
        .pointer("/properties")
        .and_then(Value::as_object)
        .expect("ScopedMcpServer properties exist");

    assert!(properties.contains_key("use"));
    assert!(properties.contains_key("actsAs"));
    assert!(!properties.contains_key("preset"));
    assert!(!properties.contains_key("acts_as"));
}

#[test]
fn sdk_response_wrapper_metadata_is_valid() {
    let spec = spec_value();
    let schemas = spec
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .expect("components.schemas exists");
    let expected = BTreeMap::from([
        ("WithUrls_Agent", ("resource", "Agent")),
        ("WithUrls_AgentWithCounts", ("resource", "Agent")),
        ("WithUrls_Session", ("resource", "Session")),
        ("WithUrls_CapabilityInfo", ("resource", "CapabilityInfo")),
        (
            "PaginatedResponse_WithUrls_AgentWithCounts",
            ("list", "Agent"),
        ),
        ("PaginatedResponse_WithUrls_Session", ("list", "Session")),
        (
            "PaginatedResponse_WithUrls_CapabilityInfo",
            ("list", "CapabilityInfo"),
        ),
    ]);

    for (wrapper, (kind, model)) in expected {
        let metadata = schemas
            .get(wrapper)
            .and_then(|schema| schema.get("x-sdk-response-wrapper"))
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{wrapper} has SDK wrapper metadata"));
        assert_eq!(metadata.len(), 2, "{wrapper} metadata has no extra fields");
        assert_eq!(metadata.get("kind").and_then(Value::as_str), Some(kind));

        let model_ref = format!("#/components/schemas/{model}");
        assert_eq!(
            metadata.get("model").and_then(Value::as_str),
            Some(model_ref.as_str())
        );
        assert!(
            schemas.contains_key(model),
            "{wrapper} references existing canonical model {model}"
        );
    }

    let metadata_count = schemas
        .values()
        .filter(|schema| schema.get("x-sdk-response-wrapper").is_some())
        .count();
    assert_eq!(metadata_count, 7);
}

#[test]
fn sdk_resource_schemas_use_public_wire_names() {
    let spec = spec_value();
    let schemas = spec
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .expect("components.schemas exists");

    for name in [
        "ResourceStats",
        "Workspace",
        "Memory",
        "DeleteFileResponse",
        "MemoryGrepResult",
        "Budget",
        "BudgetPeriod",
        "BudgetCheckResult",
        "CreateBudgetRequest",
        "UpdateBudgetRequest",
        "TopUpRequest",
        "LedgerEntry",
        "ResumeSessionResponse",
        "Connection",
        "ApiKeyConnectionRequest",
    ] {
        assert!(schemas.contains_key(name), "missing schema {name}");
    }
    for old_name in [
        "ResourceStatsResponse",
        "WorkspaceResponse",
        "MemoryResponse",
        "DeleteResponse",
        "GrepResultEntry",
        "ResumeSessionBudgetsResult",
        "ConnectionResponse",
    ] {
        assert!(!schemas.contains_key(old_name), "stale schema {old_name}");
    }
}
