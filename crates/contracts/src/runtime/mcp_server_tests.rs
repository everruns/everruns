use super::*;
use serde_json::json;

#[test]
fn bound_parameters_are_removed_from_both_schemas_of_only_the_matching_tool() {
    use crate::runtime::tool_types::{BuiltinTool, ToolDefinition};
    let schema = json!({"type":"object","properties":{"message":{"type":"string"},"channel_key":{"type":"string"}},"required":["message","channel_key"],"additionalProperties":false});
    let builtin = |name: &str| {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: "Send message".into(),
            parameters: schema.clone(),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: Some(schema.clone()),
        })
    };
    let mut definitions = vec![
        builtin("mcp_notify__send"),
        builtin("mcp_other__send"),
        builtin("mcp_notify__read"),
    ];
    let unrelated = serde_json::to_value(&definitions[1..]).unwrap();
    for configured in [true, false] {
        definitions[0] = builtin("mcp_notify__send");
        apply_mcp_secret_binding_schemas(
            &mut definitions,
            &[
                McpSecretBindingMetadata {
                    server_name: "missing".into(),
                    tool_name: "send".into(),
                    parameter_name: "message".into(),
                    configured: true,
                    setup_url: "/missing".into(),
                },
                McpSecretBindingMetadata {
                    server_name: "Notify".into(),
                    tool_name: "send".into(),
                    parameter_name: "channel_key".into(),
                    configured,
                    setup_url: "/agent/credentials".into(),
                },
            ],
        );
        let ToolDefinition::Builtin(bound) = &definitions[0] else {
            panic!("expected builtin")
        };
        let expected = json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false});
        assert_eq!(bound.parameters, expected);
        assert_eq!(bound.full_parameters.as_ref(), Some(&expected));
        let status = if configured {
            "configured"
        } else {
            "setup required"
        };
        assert_eq!(
            bound.description,
            format!(
                "Send message\n\nCredential 'channel_key' is securely bound ({status}); do not request or supply it. Setup: /agent/credentials"
            )
        );
        assert_eq!(serde_json::to_value(&definitions[1..]).unwrap(), unrelated);
    }
}

#[test]
fn bound_parameter_removal_preserves_missing_or_nonobject_schema_parts() {
    for mut schema in [
        json!(null),
        json!([]),
        json!({}),
        json!({"properties":[],"required":"message"}),
        json!({"properties":{"message":{}},"required":["message",7]}),
    ] {
        let original = schema.clone();
        remove_bound_parameter(&mut schema, "channel_key");
        assert_eq!(schema, original);
    }
}

#[test]
fn protocol_modes_accept_aliases_but_emit_canonical_versions_and_policies() {
    for (mode, canonical, alias, version, stateful) in [
        (McpProtocolMode::Auto, "auto", "auto", None, None),
        (
            McpProtocolMode::V2025March,
            "2025-03-26",
            "legacy",
            Some("2025-03-26"),
            Some(true),
        ),
        (
            McpProtocolMode::V2025June,
            "2025-06-18",
            "stable",
            Some("2025-06-18"),
            Some(true),
        ),
        (
            McpProtocolMode::V2026July,
            "2026-07-28",
            "rc",
            Some("2026-07-28"),
            Some(false),
        ),
    ] {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(canonical));
        assert_eq!(mode.to_string(), canonical);
        assert_eq!(mode.pinned_version(), version);
        assert_eq!(mode.pinned_stateful(), stateful);
        assert_eq!(mode.is_auto(), canonical == "auto");
        for input in [canonical, alias] {
            assert_eq!(McpProtocolMode::from(input), mode);
            assert_eq!(
                serde_json::from_value::<McpProtocolMode>(json!(input)).unwrap(),
                mode
            );
        }
    }
    assert_eq!(McpProtocolMode::from("nonsense"), McpProtocolMode::Auto);
    assert!(serde_json::from_value::<McpProtocolMode>(json!("nonsense")).is_err());
}

#[test]
fn scoped_config_defaults_omit_optional_values_and_canonicalize_aliases() {
    for (input, expected_mode, expected_wire) in [
        (
            json!({"url":"https://example.com/mcp"}),
            McpProtocolMode::Auto,
            json!({"type":"http","url":"https://example.com/mcp"}),
        ),
        (
            json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"rc"}),
            McpProtocolMode::V2026July,
            json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"2026-07-28"}),
        ),
        (
            json!({"transport_type":"http","url":"https://example.com/mcp","protocol_mode":"legacy"}),
            McpProtocolMode::V2025March,
            json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"2025-03-26"}),
        ),
    ] {
        let config: ScopedMcpServer = serde_json::from_value(input).unwrap();
        assert_eq!(config.protocol_mode, expected_mode);
        assert!(config.tool_discovery);
        assert_eq!(serde_json::to_value(config).unwrap(), expected_wire);
    }
    let defaults = ScopedMcpServer::default();
    assert_eq!(defaults.protocol_mode, McpProtocolMode::Auto);
    assert_eq!(
        serde_json::to_value(defaults).unwrap(),
        json!({"type":"http"})
    );
}

#[test]
fn scoped_config_preserves_nondefault_transport_auth_and_discovery_fields() {
    let wire = json!({"type":"stdio","command":"mcp-server","args":["--project","demo"],"env":{"MODE":"test"},"headers":{"X-Trace":"trace"},"auth_mode":"oauth","oauth_provider_id":"provider","protocol_mode":"2025-06-18","tool_discovery":false});
    let config: ScopedMcpServer = serde_json::from_value(wire.clone()).unwrap();
    assert!(config.transport_type.is_local());
    assert!(!config.tool_discovery);
    assert_eq!(config.auth_mode, McpServerAuthMode::OAuth);
    assert_eq!(serde_json::to_value(config).unwrap(), wire);
}
#[test]
fn scoped_config_acts_as_is_strict_and_omits_none() {
    let legacy = r#"{"type":"http","url":"https://example.com/mcp"}"#;
    let config: ScopedMcpServer = serde_json::from_str(legacy).unwrap();
    assert_eq!(config.acts_as, McpServerActsAs::None);
    assert_eq!(serde_json::to_string(&config).unwrap(), legacy);

    for (wire_name, expected) in [
        ("service", McpServerActsAs::Service),
        ("user", McpServerActsAs::User),
        ("user_or_service", McpServerActsAs::UserOrService),
    ] {
        assert_eq!(expected.to_string(), wire_name);
        assert_eq!(McpServerActsAs::from(wire_name), expected);
        let config: ScopedMcpServer = serde_json::from_value(json!({
            "url": "https://example.com/mcp",
            "actsAs": wire_name
        }))
        .unwrap();
        assert_eq!(config.acts_as, expected);
        assert_eq!(
            serde_json::to_value(config).unwrap()["actsAs"],
            json!(wire_name)
        );
    }

    let alias: ScopedMcpServer = serde_json::from_value(json!({
        "url": "https://example.com/mcp",
        "acts_as": "service"
    }))
    .unwrap();
    assert_eq!(alias.acts_as, McpServerActsAs::Service);
    assert_eq!(
        serde_json::to_value(alias).unwrap()["actsAs"],
        json!("service")
    );

    for invalid in ["Service", "users", ""] {
        assert!(
            serde_json::from_value::<ScopedMcpServer>(json!({
                "use": "catalog:linear",
                "actsAs": invalid
            }))
            .is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn scoped_config_parses_only_nonempty_catalog_references() {
    let config: ScopedMcpServer = serde_json::from_value(json!({
        "use": "catalog:linear",
        "actsAs": "service"
    }))
    .unwrap();
    assert_eq!(
        config.preset.as_ref().map(McpServerPresetRef::catalog_name),
        Some("linear")
    );
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        json!({"use":"catalog:linear","actsAs":"service"})
    );

    for invalid in ["garbage", "", "catalog:"] {
        assert!(
            serde_json::from_value::<ScopedMcpServer>(json!({"use": invalid})).is_err(),
            "{invalid}"
        );
    }
    assert!(
        serde_json::from_value::<ScopedMcpServer>(json!({"use":"catalog:linear","type":"http"}))
            .is_err()
    );
}

#[test]
fn scoped_merge_replaces_entire_matching_connection_without_mutating_inputs() {
    let base: ScopedMcpServers = serde_json::from_value(json!({
            "base_only":{"url":"https://base.test/mcp"},
            "shared":{"url":"https://old.test/mcp","headers":{"old":"value"},"protocol_mode":"2025-03-26","tool_discovery":false}
        })).unwrap();
    let overlay: ScopedMcpServers = serde_json::from_value(json!({
            "overlay_only":{"url":"https://overlay.test/mcp"},
            "shared":{"url":"https://new.test/mcp","headers":{"new":"value"},"protocol_mode":"2026-07-28","auth_mode":"oauth","oauth_provider_id":"provider"}
        })).unwrap();
    let before_base = base.clone();
    let before_overlay = overlay.clone();
    let merged = merge_scoped_mcp_servers(&base, &overlay);
    let expected = BTreeMap::from([
        ("base_only".into(), base["base_only"].clone()),
        ("shared".into(), overlay["shared"].clone()),
        ("overlay_only".into(), overlay["overlay_only"].clone()),
    ]);
    assert_eq!(merged, expected);
    assert_eq!(base, before_base);
    assert_eq!(overlay, before_overlay);
    assert_eq!(merge_scoped_mcp_servers(&base, &BTreeMap::new()), base);
    assert_eq!(
        merge_scoped_mcp_servers(&BTreeMap::new(), &overlay),
        overlay
    );
}

#[test]
fn normalize_mcp_error_code_maps_legacy_to_rc() {
    for (input, expected) in [
        (-32002, -32602),
        (-32602, -32602),
        (-32601, -32601),
        (0, 0),
        (i64::MIN, i64::MIN),
        (i64::MAX, i64::MAX),
    ] {
        assert_eq!(normalize_mcp_error_code(input), expected);
    }
}

#[test]
fn tool_names_sanitize_server_names_and_preserve_tool_components() {
    for (server, tool, full, prefix) in [
        ("github", "search", "mcp_github__search", "github"),
        (
            "microsoft_learn",
            "docs_search",
            "mcp_microsoft_learn__docs_search",
            "microsoft_learn",
        ),
        (
            "microsoft-learn",
            "search",
            "mcp_microsoft_learn__search",
            "microsoft_learn",
        ),
        ("GitHub", "search", "mcp_github__search", "github"),
        (
            "my.server.name",
            "tool",
            "mcp_my_server_name__tool",
            "my_server_name",
        ),
        (
            "my_long_server_name",
            "my_complex_tool",
            "mcp_my_long_server_name__my_complex_tool",
            "my_long_server_name",
        ),
        ("github", "read__file", "mcp_github__read__file", "github"),
    ] {
        assert_eq!(mcp_tool_name(server, tool), full);
        assert_eq!(
            parse_mcp_tool_name(full),
            Some((prefix.into(), tool.into()))
        );
        assert!(is_mcp_tool(full));
    }
}

#[test]
fn tool_name_parser_rejects_missing_prefix_separator_or_components() {
    for name in [
        "get_weather",
        "mcpsearch",
        "mcp_github_search",
        "mcp___search",
        "mcp_github__",
        "mcp_",
        "",
    ] {
        assert_eq!(parse_mcp_tool_name(name), None, "{name}");
    }
    assert!(!is_mcp_tool("get_weather"));
    assert!(!is_mcp_tool("mcpsearch"));
    // Routing classification intentionally accepts an invalid MCP-shaped name.
    assert!(is_mcp_tool("mcp_"));
}

#[test]
fn error_codes_have_independent_wire_category_and_retry_contracts() {
    for (code, wire, category, retryable) in [
        (
            McpErrorCode::ToolNotFound,
            "tool_not_found",
            McpErrorCategory::Permanent,
            false,
        ),
        (
            McpErrorCode::ToolTimeout,
            "tool_timeout",
            McpErrorCategory::Transient,
            true,
        ),
        (
            McpErrorCode::ToolPanicked,
            "tool_panicked",
            McpErrorCategory::Permanent,
            false,
        ),
        (
            McpErrorCode::InvalidArguments,
            "invalid_arguments",
            McpErrorCategory::Validation,
            false,
        ),
        (
            McpErrorCode::PermissionDenied,
            "permission_denied",
            McpErrorCategory::Auth,
            false,
        ),
        (
            McpErrorCode::QuotaExceeded,
            "quota_exceeded",
            McpErrorCategory::Transient,
            true,
        ),
        (
            McpErrorCode::NetworkBlocked,
            "network_blocked",
            McpErrorCategory::Permanent,
            false,
        ),
        (
            McpErrorCode::McpServerUnreachable,
            "mcp_server_unreachable",
            McpErrorCategory::Transient,
            true,
        ),
        (
            McpErrorCode::Internal,
            "internal",
            McpErrorCategory::Permanent,
            false,
        ),
        (
            McpErrorCode::Unknown,
            "unknown",
            McpErrorCategory::Permanent,
            false,
        ),
    ] {
        assert_eq!(code.as_str(), wire);
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<McpErrorCode>(json!(wire)).unwrap(),
            code
        );
        let error = McpExecuteError::new(code, "original message");
        assert_eq!(error.category, category);
        assert_eq!(error.retryable, retryable);
        assert_eq!(error.message, "original message");
    }
}

#[test]
fn unknown_error_code_and_category_deserialize_to_forward_compatible_sentinels() {
    assert_eq!(
        serde_json::from_value::<McpErrorCode>(json!("future_code")).unwrap(),
        McpErrorCode::Unknown
    );
    assert_eq!(
        serde_json::from_value::<McpErrorCategory>(json!("future_category")).unwrap(),
        McpErrorCategory::Unknown
    );
}

#[test]
fn error_classifier_preserves_messages_and_routes_every_marker_and_prefix() {
    for (message, code, category, retryable) in [
        (
            "Tool timed out after 30000ms",
            McpErrorCode::ToolTimeout,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "Command timed out after 5000ms",
            McpErrorCode::ToolTimeout,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "TIMEOUT",
            McpErrorCode::ToolTimeout,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "Unknown tool: github.foo",
            McpErrorCode::ToolNotFound,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "Missing required parameter: query",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "invalid argument: query",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "permission denied for org",
            McpErrorCode::PermissionDenied,
            McpErrorCategory::Auth,
            false,
        ),
        (
            "Forbidden: org scope not allowed",
            McpErrorCode::PermissionDenied,
            McpErrorCategory::Auth,
            false,
        ),
        (
            "not authorized to call this tool",
            McpErrorCode::PermissionDenied,
            McpErrorCategory::Auth,
            false,
        ),
        (
            "Unauthorized request",
            McpErrorCode::PermissionDenied,
            McpErrorCategory::Auth,
            false,
        ),
        (
            "Quota exceeded for org",
            McpErrorCode::QuotaExceeded,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "Rate limit hit",
            McpErrorCode::QuotaExceeded,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "network blocked",
            McpErrorCode::NetworkBlocked,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "EGRESS denied",
            McpErrorCode::NetworkBlocked,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "MCP server unreachable",
            McpErrorCode::McpServerUnreachable,
            McpErrorCategory::Transient,
            true,
        ),
        (
            "tool panicked",
            McpErrorCode::ToolPanicked,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "bad_request: name must be <=200 chars",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "unprocessable: cycle detected in capability graph",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "conflict: session is already paused",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "not_found: agent agent_xyz not in this org",
            McpErrorCode::ToolNotFound,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "forbidden: principal lacks SESSION_WRITE",
            McpErrorCode::PermissionDenied,
            McpErrorCategory::Auth,
            false,
        ),
        (
            "internal: storage backend returned 503",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "INTERNAL: upstream timed out",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "bad_request: invalid timeout",
            McpErrorCode::InvalidArguments,
            McpErrorCategory::Validation,
            false,
        ),
        (
            "strange unanticipated message",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "unreachable",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "mcp server available",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
        (
            "",
            McpErrorCode::Internal,
            McpErrorCategory::Permanent,
            false,
        ),
    ] {
        let error = classify_mcp_execute_error(message);
        assert_eq!(error.code, code, "{message}");
        assert_eq!(error.category, category, "{message}");
        assert_eq!(error.retryable, retryable, "{message}");
        assert_eq!(error.message, message);
    }
}

#[test]
fn error_envelopes_omit_empty_optionals_and_preserve_explicit_overrides() {
    let minimal = McpExecuteError::new(McpErrorCode::ToolNotFound, "no such tool");
    assert_eq!(
        serde_json::to_value(minimal).unwrap(),
        json!({"code":"tool_not_found","message":"no such tool","category":"permanent","retryable":false})
    );
    let full = McpExecuteError::new(McpErrorCode::ToolTimeout, "tool timed out after 30000ms")
        .with_category(McpErrorCategory::Auth)
        .with_retryable(false)
        .with_retry_after_seconds(10)
        .with_hint("Reduce input size before retrying.")
        .with_cause("root cause")
        .with_cause("downstream: upstream gateway timeout");
    assert_eq!(
        serde_json::to_value(full).unwrap(),
        json!({"code":"tool_timeout","message":"tool timed out after 30000ms","category":"auth","retryable":false,"retry_after_seconds":10,"hint":"Reduce input size before retrying.","cause_chain":["root cause","downstream: upstream gateway timeout"]})
    );
}
#[test]
fn server_name_validation_preserves_unambiguous_generated_names() {
    for name in [
        "",
        "_",
        "docs_",
        "docs-",
        "docs ",
        "docs__private",
        "docs..private",
        "--docs",
    ] {
        assert!(!is_valid_mcp_server_name(name), "{name:?}");
    }
    for (name, prefix) in [
        ("docs", "docs"),
        ("docs-api", "docs_api"),
        ("_docs", "_docs"),
        ("Docs API", "docs_api"),
    ] {
        assert!(is_valid_mcp_server_name(name), "{name:?}");
        for tool in ["search", "_search", "read__file"] {
            assert_eq!(
                parse_mcp_tool_name(&mcp_tool_name(name, tool)),
                Some((prefix.into(), tool.into()))
            );
        }
    }
}
#[test]
fn ambiguous_bindings_do_not_rewrite_a_different_tool() {
    let schema = serde_json::json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"]});
    let mut definitions = vec![crate::runtime::ToolDefinition::Builtin(
        crate::runtime::BuiltinTool {
            name: "mcp_docs___search".into(),
            display_name: None,
            description: "Search".into(),
            parameters: schema,
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        },
    )];
    let before = serde_json::to_value(&definitions).unwrap();
    let mut binding = McpSecretBindingMetadata {
        server_name: "docs_".into(),
        tool_name: "search".into(),
        parameter_name: "key".into(),
        configured: true,
        setup_url: "/setup".into(),
    };
    apply_mcp_secret_binding_schemas(&mut definitions, &[binding.clone()]);
    assert_eq!(serde_json::to_value(&definitions).unwrap(), before);
    binding.server_name = "docs".into();
    binding.tool_name = "_search".into();
    apply_mcp_secret_binding_schemas(&mut definitions, &[binding]);
    let crate::runtime::ToolDefinition::Builtin(definition) = &definitions[0] else {
        panic!("expected builtin")
    };
    assert_eq!(
        definition.parameters,
        serde_json::json!({"type":"object","properties":{},"required":[]})
    );
}
#[test]
fn auth_modes_use_canonical_wire_values_and_accept_legacy_oauth_spelling() {
    for (mode, wire) in [
        (McpServerAuthMode::None, "none"),
        (McpServerAuthMode::ApiKey, "api_key"),
        (McpServerAuthMode::OAuth, "oauth"),
    ] {
        assert_eq!(serde_json::to_value(&mode).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<McpServerAuthMode>(json!(wire)).unwrap(),
            mode
        );
        assert_eq!(McpServerAuthMode::from(wire), mode);
        assert_eq!(mode.to_string(), wire);
    }
    let legacy: McpServerAuthMode = serde_json::from_value(json!("o_auth")).unwrap();
    assert_eq!(legacy, McpServerAuthMode::OAuth);
    assert_eq!(serde_json::to_value(legacy).unwrap(), json!("oauth"));
}

#[test]
fn user_or_service_tries_the_user_before_the_agent() {
    assert_eq!(
        McpServerActsAs::UserOrService.resolution_order(),
        &[McpServerActsAs::User, McpServerActsAs::Service]
    );
    assert_eq!(
        McpServerActsAs::User.resolution_order(),
        &[McpServerActsAs::User]
    );
    assert_eq!(
        McpServerActsAs::Service.resolution_order(),
        &[McpServerActsAs::Service]
    );
    assert!(McpServerActsAs::None.resolution_order().is_empty());
    assert!(McpServerActsAs::UserOrService.uses_user_grant());
    assert!(McpServerActsAs::UserOrService.uses_service_grant());
    assert!(!McpServerActsAs::User.uses_service_grant());
    assert!(!McpServerActsAs::Service.uses_user_grant());
}

#[test]
fn connect_in_chat_defaults_to_ask_and_round_trips_never() {
    let server: ScopedMcpServer =
        serde_json::from_value(json!({"use": "catalog:github", "actsAs": "user"})).unwrap();
    assert_eq!(server.connect_in_chat, McpConnectInChat::Ask);
    // The default stays off the wire, so existing configs serialize unchanged.
    let wire = serde_json::to_value(&server).unwrap();
    assert!(wire.get("connectInChat").is_none());

    let server: ScopedMcpServer = serde_json::from_value(json!({
        "use": "catalog:github",
        "actsAs": "user",
        "connectInChat": "never",
    }))
    .unwrap();
    assert_eq!(server.connect_in_chat, McpConnectInChat::Never);
    assert!(!server.connect_in_chat.allows_card());
    let wire = serde_json::to_value(&server).unwrap();
    assert_eq!(wire["connectInChat"], "never");

    let snake: ScopedMcpServer =
        serde_json::from_value(json!({"url": "https://x.example/mcp", "connect_in_chat": "never"}))
            .unwrap();
    assert_eq!(snake.connect_in_chat, McpConnectInChat::Never);
    assert!(
        serde_json::from_value::<ScopedMcpServer>(
            json!({"url": "https://x.example/mcp", "connectInChat": "sometimes"})
        )
        .is_err()
    );
    assert_eq!(McpConnectInChat::from(""), McpConnectInChat::Ask);
    assert_eq!(McpConnectInChat::from("never"), McpConnectInChat::Never);
    assert_eq!(McpConnectInChat::Never.to_string(), "never");
}

#[test]
fn deferred_defaults_to_off_and_round_trips() {
    let server: ScopedMcpServer =
        serde_json::from_value(json!({"use": "catalog:linear", "actsAs": "user"})).unwrap();
    assert!(!server.deferred, "existing attachments keep listing tools");
    // Off stays off the wire, so existing configs serialize unchanged.
    assert!(
        serde_json::to_value(&server)
            .unwrap()
            .get("deferred")
            .is_none()
    );

    let server: ScopedMcpServer =
        serde_json::from_value(json!({"use": "catalog:linear", "deferred": true})).unwrap();
    assert!(server.deferred);
    assert_eq!(serde_json::to_value(&server).unwrap()["deferred"], true);
}
