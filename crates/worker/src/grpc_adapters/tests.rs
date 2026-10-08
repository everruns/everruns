use super::*;
use uuid::Uuid;

#[test]
fn worker_parses_the_neutral_capability_reference_shape() {
    // EVE-873: worker resolution consumes the same `{"ref", "config"}`
    // representation the Framework serializes and the control plane
    // persists — no worker-side semantic model.
    let framework_ref = everruns_contracts::CapabilityRef::new("web_fetch")
        .config(serde_json::json!({"enable_file_download": true}));
    let wire = serde_json::to_string(&framework_ref).unwrap();

    let parsed = serde_json::from_str::<everruns_contracts::CapabilityRef>(&wire).unwrap();
    assert_eq!(parsed, framework_ref);
    assert_eq!(parsed.capability_id(), "web_fetch");

    // Legacy rows without a config payload load as `{}`.
    let bare =
        serde_json::from_str::<everruns_contracts::CapabilityRef>(r#"{"ref":"current_time"}"#)
            .unwrap();
    assert_eq!(bare.config_value(), &serde_json::json!({}));
}

#[test]
fn resolved_model_proto_conversion_is_credential_free() {
    let resolved = proto_model_to_model_spec(proto::ResolvedModel {
        model: "custom-model".into(),
        provider_id: "provider-123".into(),
        provider_type: "custom-protocol".into(),
    })
    .unwrap();

    assert_eq!(resolved.model, "custom-model");
    assert_eq!(resolved.provider.as_str(), "provider-123");
}

#[test]
fn grpc_worker_adapter_parses_acts_as_and_defaults_old_servers() {
    let id = Uuid::new_v4();
    let proto_server = proto::McpServerInfo {
        id: Some(uuid_to_proto(id)),
        name: "linear".to_string(),
        url: "https://mcp.linear.app/mcp".to_string(),
        acts_as: "service".to_string(),
        connect_in_chat: "never".to_string(),
        ..Default::default()
    };

    let info = proto_mcp_server_to_info(proto_server).unwrap();
    assert_eq!(info.acts_as, crate::core::McpServerActsAs::Service);
    assert_eq!(info.connect_in_chat, crate::core::McpConnectInChat::Never);

    let old_server = proto::McpServerInfo {
        id: Some(uuid_to_proto(id)),
        ..Default::default()
    };
    let info = proto_mcp_server_to_info(old_server).unwrap();
    assert_eq!(info.acts_as, crate::core::McpServerActsAs::None);
    // An older control plane sends no `connect_in_chat`: the card stays.
    assert_eq!(info.connect_in_chat, crate::core::McpConnectInChat::Ask);
}

#[test]
fn test_proto_harness_projects_execution_configuration() {
    let harness_id = Uuid::new_v4();
    let parent_id = Uuid::new_v4();
    let proto = proto::Harness {
        id: Some(uuid_to_proto(harness_id)),
        name: "platform-chat".into(),
        description: "Built-in chat harness".into(),
        system_prompt: "prompt".into(),
        default_model_id: None,
        status: "active".into(),
        created_at: None,
        updated_at: None,
        capability_ids: vec!["platform".into()],
        tags: vec!["chat".into(), "built-in".into()],
        parent_harness_id: Some(uuid_to_proto(parent_id)),
        is_built_in: true,
        display_name: Some("Platform Chat".into()),
        capabilities: vec![
            serde_json::json!({
                "ref": "plugin:plugin_019fda530ed27b4291c67d9f786961d9",
                "config": {"name": "resend", "description": "Send email"}
            })
            .to_string(),
        ],
    };

    let harness = proto_harness_to_definition(proto).expect("proto harness should convert");

    assert_eq!(harness.name, "platform-chat");
    assert_eq!(harness.system_prompt.as_deref(), Some("prompt"));
    let projected = serde_json::to_value(&harness).unwrap();
    for field in [
        "id",
        "parent_harness_id",
        "tags",
        "is_built_in",
        "status",
        "created_at",
    ] {
        assert!(
            projected.get(field).is_none(),
            "record field {field} leaked"
        );
    }
    assert_eq!(harness.capabilities[0].config_value()["name"], "resend");
}

#[test]
fn test_proto_value_to_json_null() {
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::NullValue(0)),
    };
    assert_eq!(proto_value_to_json(val), serde_json::Value::Null);
}

#[test]
fn test_proto_value_to_json_none_kind() {
    let val = prost_types::Value { kind: None };
    assert_eq!(proto_value_to_json(val), serde_json::Value::Null);
}

#[test]
fn test_proto_value_to_json_string() {
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::StringValue("hello".into())),
    };
    assert_eq!(
        proto_value_to_json(val),
        serde_json::Value::String("hello".into())
    );
}

#[test]
fn test_proto_value_to_json_number() {
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::NumberValue(42.5)),
    };
    assert_eq!(proto_value_to_json(val), serde_json::json!(42.5));
}

#[test]
fn test_proto_value_to_json_bool() {
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::BoolValue(true)),
    };
    assert_eq!(proto_value_to_json(val), serde_json::Value::Bool(true));
}

#[test]
fn test_proto_value_to_json_list() {
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::ListValue(
            prost_types::ListValue {
                values: vec![
                    prost_types::Value {
                        kind: Some(prost_types::value::Kind::NumberValue(1.0)),
                    },
                    prost_types::Value {
                        kind: Some(prost_types::value::Kind::StringValue("two".into())),
                    },
                ],
            },
        )),
    };
    assert_eq!(proto_value_to_json(val), serde_json::json!([1.0, "two"]));
}

#[test]
fn test_proto_value_to_json_struct() {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "key".to_string(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue("value".into())),
        },
    );
    fields.insert(
        "num".to_string(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::NumberValue(42.0)),
        },
    );
    let val = prost_types::Value {
        kind: Some(prost_types::value::Kind::StructValue(prost_types::Struct {
            fields: fields.into_iter().collect(),
        })),
    };
    let json = proto_value_to_json(val);
    assert_eq!(json["key"], "value");
    assert_eq!(json["num"], 42.0);
}

#[test]
fn test_grpc_status_to_error_not_found() {
    let status = tonic::Status::not_found("Session not found");
    let err = grpc_status_to_error(status);
    assert!(matches!(err, AgentLoopError::MessageStore(_)));
    assert!(err.to_string().contains("not found"));
}

#[test]
fn test_grpc_status_to_error_invalid_argument() {
    let status = tonic::Status::invalid_argument("bad field");
    let err = grpc_status_to_error(status);
    assert!(matches!(err, AgentLoopError::Configuration(_)));
}

#[test]
fn test_grpc_status_to_error_resource_exhausted_payload() {
    let status = tonic::Status::resource_exhausted("message too large");
    let err = grpc_status_to_error(status);
    assert!(err.is_request_too_large());
}

#[test]
fn test_grpc_status_to_error_resource_exhausted_non_payload() {
    let status = tonic::Status::resource_exhausted("task queue limit exceeded");
    let err = grpc_status_to_error(status);
    assert!(!err.is_request_too_large());
    assert!(matches!(err, AgentLoopError::MessageStore(_)));
}

#[test]
fn test_grpc_status_to_error_unauthenticated() {
    let status = tonic::Status::unauthenticated("bad token");
    let err = grpc_status_to_error(status);
    assert!(matches!(err, AgentLoopError::Configuration(_)));
    assert!(err.to_string().contains("Auth error"));
}

#[test]
fn test_grpc_status_to_error_unavailable() {
    let status = tonic::Status::unavailable("service down");
    let err = grpc_status_to_error(status);
    assert!(matches!(err, AgentLoopError::MessageStore(_)));
    assert!(err.to_string().contains("unavailable"));
}

#[test]
fn test_grpc_status_to_error_internal_fallback() {
    let status = tonic::Status::internal("server error");
    let err = grpc_status_to_error(status);
    assert!(matches!(err, AgentLoopError::MessageStore(_)));
    assert!(err.to_string().contains("Internal"));
}

#[test]
fn test_grpc_missing_field() {
    let err = grpc_missing_field("No session in response");
    assert!(matches!(err, AgentLoopError::MessageStore(_)));
    assert!(err.to_string().contains("No session in response"));
}

#[test]
fn test_proto_stored_image_info_to_schema_roundtrips_image_id_uuid_transport() {
    let image_id = everruns_contracts::typed_id::ImageId::new();
    let info = proto_stored_image_info_to_schema(proto::StoredImageInfo {
        id: Some(proto::Uuid {
            value: image_id.uuid().to_string(),
        }),
        filename: "generated-image.png".into(),
        content_type: "image/png".into(),
        size_bytes: 128,
        metadata: None,
        created_at: None,
    })
    .expect("stored image info should convert");

    assert_eq!(info.id, image_id);
}

#[test]
fn test_grpc_command_error_to_error_not_found() {
    let err = grpc_command_error_to_error(proto::CommandError {
        kind: 3,
        message: "Harness not found".into(),
    });

    assert!(matches!(err, AgentLoopError::MessageStore(_)));
    assert!(err.to_string().contains("Harness not found"));
}

#[test]
fn agent_transport_projection_enforces_lifecycle_before_execution() {
    for status in ["archived", "deleted", "ArChIvEd", "DeLeTeD"] {
        let value = proto::Agent {
            id: Some(uuid_to_proto(Uuid::new_v4())),
            status: status.into(),
            ..Default::default()
        };
        let error = proto_agent_to_definition(value).expect_err("inactive agent must be rejected");
        assert!(error.to_string().contains("cannot execute turns"));
    }
    for status in ["active", "ACTIVE", "unknown-legacy-status"] {
        let value = proto::Agent {
            id: Some(uuid_to_proto(Uuid::new_v4())),
            status: status.into(),
            ..Default::default()
        };
        assert!(
            proto_agent_to_definition(value).is_ok(),
            "legacy status {status} must keep its active fallback"
        );
    }
}

#[test]
fn harness_transport_projection_enforces_lifecycle_before_execution() {
    for status in ["archived", "deleted", "ArChIvEd", "DeLeTeD"] {
        let value = proto::Harness {
            id: Some(uuid_to_proto(Uuid::new_v4())),
            status: status.into(),
            ..Default::default()
        };
        let error =
            proto_harness_to_definition(value).expect_err("inactive harness must be rejected");
        assert!(error.to_string().contains("cannot execute turns"));
    }
    for status in ["active", "ACTIVE", "unknown-legacy-status"] {
        let value = proto::Harness {
            id: Some(uuid_to_proto(Uuid::new_v4())),
            status: status.into(),
            ..Default::default()
        };
        assert!(
            proto_harness_to_definition(value).is_ok(),
            "legacy status {status} must keep its active fallback"
        );
    }
}
