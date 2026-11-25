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

#[test]
fn channels_keep_deprecated_route_and_schema_aliases() {
    let spec = spec_value();
    for (canonical, legacy, method) in [
        (
            "/v1/platform/agents/{agent_id}/channels",
            "/v1/platform/agents/{agent_id}/endpoints",
            "post",
        ),
        (
            "/v1/platform/agents/{agent_id}/channels/{channel_id}",
            "/v1/platform/agents/{agent_id}/endpoints/{endpoint_id}",
            "get",
        ),
        (
            "/v1/channels/{channel_id}/fcp",
            "/v1/e/{channel_id}/fcp",
            "post",
        ),
        (
            "/v1/channels/{channel_id}/runtime-auth",
            "/v1/e/{endpoint_id}/runtime-auth",
            "post",
        ),
    ] {
        let current = &spec["paths"][canonical][method];
        let old = &spec["paths"][legacy][method];
        assert!(!current.is_null(), "{canonical}");
        assert_eq!(old["deprecated"], true, "{legacy}");
        assert_eq!(current["responses"], old["responses"]);
        if legacy.contains("{endpoint_id}") {
            assert!(
                old["parameters"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["name"] == "endpoint_id")
            );
        }
    }
    for (canonical, legacy) in [
        ("ChannelStatus", "EndpointStatus"),
        ("CreateAgentChannelRequest", "CreateAgentEndpointRequest"),
    ] {
        let schemas = &spec["components"]["schemas"];
        assert!(!schemas[canonical].is_null(), "{canonical}");
        assert_eq!(schemas[canonical], schemas[legacy]);
    }
}

#[test]
fn channel_auth_example_survives_server_record_move() {
    let spec = spec_value();
    let schemas = &spec["components"]["schemas"];
    let expected = serde_json::json!({
        "mode": "api_key",
        "requirements": {"audiences": ["everruns-api"], "scopes": ["app:invoke"]}
    });
    assert_eq!(schemas["ChannelAuthConfig"]["example"], expected);
}

#[test]
fn retired_apps_are_absent_from_openapi() {
    let spec = spec_value();
    for path in spec["paths"].as_object().unwrap().keys() {
        assert!(
            !path.starts_with("/v1/apps") && !path.starts_with("/v1/platform/apps"),
            "{path}"
        );
    }
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    for name in ["App", "AppStatus", "AppChannel", "AppEndpointAuthConfig"] {
        assert!(!schemas.contains_key(name), "{name}");
    }
}
