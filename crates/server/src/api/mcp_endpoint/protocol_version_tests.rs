use super::*;

#[test]
fn negotiate_echoes_each_supported_version() {
    assert_eq!(
        negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_LATEST)),
        MCP_PROTOCOL_VERSION_LATEST
    );
    assert_eq!(
        negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_2025_11)),
        MCP_PROTOCOL_VERSION_2025_11
    );
    assert_eq!(
        negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_2025_06)),
        MCP_PROTOCOL_VERSION_2025_06
    );
    assert_eq!(
        negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_FALLBACK)),
        MCP_PROTOCOL_VERSION_FALLBACK
    );
}

#[test]
fn negotiate_missing_version_falls_back() {
    assert_eq!(
        negotiate_protocol_version(None),
        MCP_PROTOCOL_VERSION_FALLBACK
    );
}

#[test]
fn negotiate_future_version_offers_latest() {
    // A client newer than us gets our latest and decides what to do.
    assert_eq!(
        negotiate_protocol_version(Some("2099-01-01")),
        MCP_PROTOCOL_VERSION_LATEST
    );
}

#[test]
fn negotiate_in_between_version_picks_highest_supported_below() {
    // Between 2025-11-25 and 2026-07-28 → 2025-11-25.
    assert_eq!(
        negotiate_protocol_version(Some("2025-12-01")),
        MCP_PROTOCOL_VERSION_2025_11
    );
    // Between 2025-06-18 and 2025-11-25 → 2025-06-18.
    assert_eq!(
        negotiate_protocol_version(Some("2025-09-01")),
        MCP_PROTOCOL_VERSION_2025_06
    );
    // Below the floor → fallback.
    assert_eq!(
        negotiate_protocol_version(Some("2024-01-01")),
        MCP_PROTOCOL_VERSION_FALLBACK
    );
}

#[test]
fn header_accepts_supported_versions() {
    for version in SUPPORTED_PROTOCOL_VERSIONS {
        let mut headers = HeaderMap::new();
        headers.insert("MCP-Protocol-Version", version.parse().unwrap());
        assert_eq!(protocol_version_from_headers(&headers), Ok(*version));
    }
}

#[test]
fn header_missing_falls_back() {
    let headers = HeaderMap::new();
    assert_eq!(
        protocol_version_from_headers(&headers),
        Ok(MCP_PROTOCOL_VERSION_FALLBACK)
    );
}

#[test]
fn header_unsupported_version_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.insert("MCP-Protocol-Version", "2099-01-01".parse().unwrap());
    let err = protocol_version_from_headers(&headers).expect_err("must reject");
    assert!(err.contains("Unsupported"));
    assert!(err.contains(MCP_PROTOCOL_VERSION_LATEST));
}

#[test]
fn rich_tool_shape_gated_by_version() {
    assert!(supports_rich_tool_shape(MCP_PROTOCOL_VERSION_LATEST));
    assert!(supports_rich_tool_shape(MCP_PROTOCOL_VERSION_2025_06));
    assert!(!supports_rich_tool_shape(MCP_PROTOCOL_VERSION_FALLBACK));
}

#[test]
fn card_tools_present_for_2026_absent_for_fallback() {
    let names = |version| {
        tool_definitions(version)
            .into_iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
    };
    assert!(names(MCP_PROTOCOL_VERSION_LATEST).contains(&"agent_get_card".to_string()));
    assert!(names(MCP_PROTOCOL_VERSION_2025_06).contains(&"agent_get_card".to_string()));
    assert!(!names(MCP_PROTOCOL_VERSION_FALLBACK).contains(&"agent_get_card".to_string()));
}

#[test]
fn client_info_parsed_from_meta() {
    let params = json!({
        "_meta": {
            "io.modelcontextprotocol/clientInfo": { "name": "my-app", "version": "1.2.3" }
        }
    });
    assert_eq!(
        client_info_from_params(&params),
        Some(("my-app".to_string(), "1.2.3".to_string()))
    );
}

#[test]
fn client_info_defaults_missing_fields() {
    let params = json!({
        "_meta": { "io.modelcontextprotocol/clientInfo": {} }
    });
    assert_eq!(
        client_info_from_params(&params),
        Some(("unknown".to_string(), "unknown".to_string()))
    );
}

#[test]
fn client_info_absent_returns_none() {
    assert_eq!(client_info_from_params(&json!({})), None);
}

fn req(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params,
    }
}

#[test]
fn routing_headers_absent_is_ok() {
    let headers = HeaderMap::new();
    assert!(validate_routing_headers(&headers, &req("tools/list", json!({}))).is_ok());
}

#[test]
fn routing_headers_matching_is_ok() {
    let mut headers = HeaderMap::new();
    headers.insert("Mcp-Method", "tools/call".parse().unwrap());
    headers.insert("Mcp-Name", "agent_run".parse().unwrap());
    let request = req("tools/call", json!({ "name": "agent_run" }));
    assert!(validate_routing_headers(&headers, &request).is_ok());
}

#[test]
fn duplicate_routing_header_method_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.append("Mcp-Method", "tools/call".parse().unwrap());
    headers.append("Mcp-Method", "tools/list".parse().unwrap());
    let request = req("tools/call", json!({ "name": "agent_run" }));
    let err = validate_routing_headers(&headers, &request)
        .expect_err("duplicate method header must reject");
    assert!(err.contains("Duplicate Mcp-Method"));
}

#[test]
fn duplicate_routing_header_name_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.append("Mcp-Name", "agent_run".parse().unwrap());
    headers.append("Mcp-Name", "session_get_status".parse().unwrap());
    let request = req("tools/call", json!({ "name": "agent_run" }));
    let err = validate_routing_headers(&headers, &request)
        .expect_err("duplicate name header must reject");
    assert!(err.contains("Duplicate Mcp-Name"));
}

#[test]
fn routing_header_method_mismatch_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.insert("Mcp-Method", "tools/list".parse().unwrap());
    let err = validate_routing_headers(&headers, &req("tools/call", json!({})))
        .expect_err("mismatch must reject");
    assert!(err.contains("Mcp-Method"));
}

#[test]
fn routing_header_name_mismatch_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.insert("Mcp-Name", "discover".parse().unwrap());
    let request = req("tools/call", json!({ "name": "agent_run" }));
    let err = validate_routing_headers(&headers, &request).expect_err("name mismatch must reject");
    assert!(err.contains("Mcp-Name"));
}

#[test]
fn routing_header_name_ignored_for_non_tools_call() {
    // `Mcp-Name` only cross-checks against `tools/call` bodies.
    let mut headers = HeaderMap::new();
    headers.insert("Mcp-Name", "anything".parse().unwrap());
    assert!(validate_routing_headers(&headers, &req("tools/list", json!({}))).is_ok());
}
