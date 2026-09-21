//! Tests: the registry, the executor, and result shaping.

use super::*;

struct CountingTool {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    label: &'static str,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counting"
    }
    fn display_name(&self) -> Option<&str> {
        Some(self.label)
    }
    fn description(&self) -> &str {
        "Count validated dispatches"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false})
    }
    fn policy(&self) -> ToolPolicy {
        ToolPolicy::RequiresApproval
    }
    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolExecutionResult::success(serde_json::json!({"label":self.label,"arguments":arguments}))
    }
}

#[tokio::test]
async fn registry_registration_paths_replace_and_dispatch_complete_definitions() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tool = |label| CountingTool {
        calls: calls.clone(),
        label,
    };
    let mut registry = ToolRegistry::builder()
        .tool(tool("first"))
        .tool_boxed(Box::new(tool("boxed")))
        .tool_arc(Arc::new(tool("last")))
        .build();
    assert_eq!(registry.tool_names(), ["counting"]);
    let definitions = registry.tool_definitions();
    assert_eq!(definitions.len(), 1);
    let ToolDefinition::Builtin(definition) = &definitions[0] else {
        panic!("builtin expected")
    };
    assert_eq!(definition.name, "counting");
    assert_eq!(definition.display_name.as_deref(), Some("last"));
    assert_eq!(definition.description, "Count validated dispatches");
    assert_eq!(
        definition.parameters,
        serde_json::json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false})
    );
    assert_eq!(definition.policy, ToolPolicy::RequiresApproval);
    assert_eq!(definition.deferrable, DeferrablePolicy::Never);
    assert_eq!(
        definition.hints,
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    );
    assert!(definition.category.is_none());
    assert!(definition.full_parameters.is_none());
    let call = ToolCall {
        id: "dispatch-id".into(),
        name: "counting".into(),
        arguments: serde_json::json!({"message":"payload"}),
    };
    let result = registry.execute(&call, &definitions[0]).await.unwrap();
    assert_eq!(result.tool_call_id, "dispatch-id");
    assert_eq!(
        result.result,
        Some(serde_json::json!({"label":"last","arguments":{"message":"payload"}}))
    );
    assert!(result.error.is_none());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        registry.unregister("counting").unwrap().display_name(),
        Some("last")
    );
    assert!(registry.is_empty());
    assert!(registry.unregister("counting").is_none());
    registry.register(tool("again"));
    registry.clear();
    assert!(registry.tool_definitions().is_empty());
}

#[tokio::test]
async fn registry_errors_preserve_public_failures_and_hide_internal_details() {
    for (tool, expected) in [
        (FailingTool::with_tool_error("Invalid city"), "Invalid city"),
        (
            FailingTool::with_internal_error("PRIVATE-DATABASE-TOKEN"),
            "An internal error occurred while executing the tool",
        ),
    ] {
        let registry = ToolRegistry::builder().tool(tool).build();
        let call = ToolCall {
            id: "failure-id".into(),
            name: "failing_tool".into(),
            arguments: serde_json::json!({}),
        };
        let result = registry
            .execute(&call, &registry.tool_definitions()[0])
            .await
            .unwrap();
        assert_eq!(result.tool_call_id, "failure-id");
        assert_eq!(result.error.as_deref(), Some(expected));
        assert_eq!(result.result, Some(serde_json::json!({"error":expected})));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE-DATABASE-TOKEN")
        );
    }
}

struct RequiresOrgId;

#[async_trait]
impl Tool for RequiresOrgId {
    fn name(&self) -> &str {
        "requires_org_id"
    }

    fn description(&self) -> &str {
        "Exercises required ToolContext service validation"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type": "object", "additionalProperties": false})
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::OrgId]
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(Value::Null)
    }
}

#[test]
fn required_context_service_validation_is_structured() {
    let mut registry = ToolRegistry::new();
    registry.register(RequiresOrgId);

    let error = registry
        .validate_context_services(&ToolContextServices::default())
        .expect_err("missing required service must fail before tool exposure");

    assert!(matches!(
        error,
        crate::AgentLoopError::Configuration(message)
            if message.contains("requires_org_id") && message.contains("OrgId")
    ));
}

#[test]
fn required_context_service_validation_accepts_supplied_service() {
    let mut registry = ToolRegistry::new();
    registry.register(RequiresOrgId);
    let services = ToolContextServices {
        org_id: Some(crate::typed_id::OrgId::from_seed(1)),
        ..ToolContextServices::default()
    };

    registry
        .validate_context_services(&services)
        .expect("advertised required service should validate");
}

#[test]
fn test_tool_result_conversion() {
    // Success
    let result = ToolExecutionResult::success(serde_json::json!({"value": 42}));
    let tool_result = result.into_tool_result("call_1", "test_tool");
    assert_eq!(tool_result.tool_call_id, "call_1");
    assert!(tool_result.error.is_none());
    assert!(tool_result.images.is_none());
    assert!(tool_result.connection_required.is_none());
    assert!(tool_result.raw_output.is_none());
    assert_eq!(tool_result.result, Some(serde_json::json!({"value": 42})));

    // Tool error (packaged as {"error": "..."} in result field, also sets error)
    let result = ToolExecutionResult::tool_error("Invalid input");
    let tool_result = result.into_tool_result("call_2", "test_tool");
    assert_eq!(tool_result.error.as_deref(), Some("Invalid input"));
    assert_eq!(
        tool_result.result.unwrap(),
        serde_json::json!({"error": "Invalid input"})
    );

    // Internal error (packaged as {"error": "..."} with generic message)
    let result = ToolExecutionResult::internal_error_msg("Secret database error");
    let tool_result = result.into_tool_result("call_3", "test_tool");
    assert_eq!(
        tool_result.error.as_deref(),
        Some("An internal error occurred while executing the tool")
    );
    assert_eq!(
        tool_result.result.unwrap(),
        serde_json::json!({"error": "An internal error occurred while executing the tool"})
    );
}

#[test]
fn test_success_with_raw_output_object_preserves_shape() {
    let res = ToolExecutionResult::success_with_raw_output(
        serde_json::json!({"stdout": "hello"}),
        "raw stdout bytes".to_string(),
    );
    let tr = res.into_tool_result("call_1", "demo");
    assert_eq!(tr.result.as_ref().unwrap()["stdout"], "hello");
    assert!(
        tr.result
            .as_ref()
            .unwrap()
            .as_object()
            .unwrap()
            .get("_raw_output")
            .is_none(),
        "sidecar key must not leak to the LLM-visible result"
    );
    assert_eq!(tr.raw_output.as_deref(), Some("raw stdout bytes"));
}

#[test]
fn raw_output_round_trips_all_nonobject_shapes_without_serializing_sidecar() {
    for value in [
        serde_json::json!("compact summary"),
        Value::Null,
        serde_json::json!(false),
        serde_json::json!(42),
        serde_json::json!(["a", 2]),
    ] {
        let result =
            ToolExecutionResult::success_with_raw_output(value.clone(), "PRIVATE-RAW".into())
                .into_tool_result("raw-id", "demo");
        assert_eq!(result.result, Some(value));
        assert_eq!(result.raw_output.as_deref(), Some("PRIVATE-RAW"));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE-RAW")
        );
    }
}

#[test]
fn test_success_result_with_raw_output_scalar_key_is_not_unwrapped() {
    let res = ToolExecutionResult::success(
        serde_json::json!({"_raw_output_scalar": "user_value", "kept": true}),
    );
    let tr = res.into_tool_result("call_1", "demo");
    assert_eq!(
        tr.result,
        Some(serde_json::json!({"_raw_output_scalar": "user_value", "kept": true}))
    );
    assert_eq!(tr.raw_output, None);
}

#[test]
fn test_success_result_with_only_raw_output_scalar_key_is_not_unwrapped() {
    // Single-key object with _raw_output_scalar must not be mistaken for a
    // success_with_raw_output carrier when raw_output is absent.
    let res = ToolExecutionResult::success(serde_json::json!({"_raw_output_scalar": "v"}));
    let tr = res.into_tool_result("call_1", "demo");
    assert_eq!(
        tr.result,
        Some(serde_json::json!({"_raw_output_scalar": "v"}))
    );
    assert_eq!(tr.raw_output, None);
}

#[tokio::test]
async fn invalid_arguments_never_dispatch_through_either_executor_path() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let registry = ToolRegistry::builder()
        .tool(CountingTool {
            calls: calls.clone(),
            label: "validated",
        })
        .build();
    let definition = registry.tool_definitions().remove(0);
    let context = ToolContext::new(crate::typed_id::SessionId::new());
    for (arguments, instance, keyword) in [
        (serde_json::json!({}), "", "required"),
        (serde_json::json!({"message":42}), "/message", "type"),
        (
            serde_json::json!({"message":"ok","unexpected":true}),
            "",
            "additionalProperties",
        ),
    ] {
        let call = ToolCall {
            id: "invalid-id".into(),
            name: "counting".into(),
            arguments,
        };
        for with_context in [false, true] {
            let result = if with_context {
                registry
                    .execute_with_context(&call, &definition, &context)
                    .await
                    .unwrap()
            } else {
                registry.execute(&call, &definition).await.unwrap()
            };
            assert_eq!(result.tool_call_id, "invalid-id");
            let message = result.error.unwrap();
            assert_eq!(result.result, Some(serde_json::json!({"error":message})));
            let error: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(error["code"], "invalid_tool_arguments");
            assert_eq!(error["tool"], "counting");
            let issues = error["issues"].as_array().unwrap();
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0]["instance_path"], instance);
            assert!(issues[0]["schema_path"].as_str().unwrap().contains(keyword));
            assert!(!issues[0]["message"].as_str().unwrap().is_empty());
        }
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let valid = ToolCall {
        id: "valid-id".into(),
        name: "counting".into(),
        arguments: serde_json::json!({"message":"accepted"}),
    };
    let result = registry
        .execute_with_context(&valid, &definition, &context)
        .await
        .unwrap();
    assert_eq!(
        result.result,
        Some(serde_json::json!({"label":"validated","arguments":{"message":"accepted"}}))
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn result_variants_keep_images_connections_and_classification_distinct() {
    use serde_json::json;
    for (result, classification, expected) in [
        (
            ToolExecutionResult::success_with_images(
                json!({"page":2}),
                vec![ToolResultImage {
                    base64: "aW1hZ2U=".into(),
                    media_type: "image/jpeg".into(),
                }],
            ),
            (true, false, false),
            json!({"tool_call_id":"variant-id","result":{"page":2},"error":null,"images":[{"base64":"aW1hZ2U=","media_type":"image/jpeg"}]}),
        ),
        (
            ToolExecutionResult::success_with_images(Value::Null, vec![]),
            (true, false, false),
            json!({"tool_call_id":"variant-id","result":null,"error":null}),
        ),
        (
            ToolExecutionResult::connection_required("daytona"),
            (false, false, true),
            json!({"tool_call_id":"variant-id","result":{"connection_required":"daytona"},"error":null,"connection_required":"daytona"}),
        ),
        (
            ToolExecutionResult::connection_required_with_setup(
                "mcp_oauth_linear",
                crate::tool_types::ConnectionRequiredSubject::Agent,
                "/agents/agent_123?tab=mcp",
            ),
            (false, false, true),
            json!({"tool_call_id":"variant-id","result":{"connection_required":{"provider":"mcp_oauth_linear","subject":"agent","setup_url":"/agents/agent_123?tab=mcp"}},"error":null,"connection_required":{"provider":"mcp_oauth_linear","subject":"agent","setup_url":"/agents/agent_123?tab=mcp"}}),
        ),
        (
            ToolExecutionResult::tool_error("visible"),
            (false, true, false),
            json!({"tool_call_id":"variant-id","result":{"error":"visible"},"error":"visible"}),
        ),
        (
            ToolExecutionResult::internal_error(std::io::Error::other("PRIVATE-SOURCE")),
            (false, true, false),
            json!({"tool_call_id":"variant-id","result":{"error":"An internal error occurred while executing the tool"},"error":"An internal error occurred while executing the tool"}),
        ),
    ] {
        assert_eq!(
            (
                result.is_success(),
                result.is_error(),
                result.is_connection_required()
            ),
            classification
        );
        let result = result.into_tool_result("variant-id", "tool");
        assert!(result.raw_output.is_none());
        assert_eq!(serde_json::to_value(result).unwrap(), expected);
    }
}

#[tokio::test]
async fn test_tool_registry_as_executor() {
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool);

    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "echo".to_string(),
        arguments: serde_json::json!({"message": "test"}),
    };

    let tool_def = registry.get("echo").unwrap().to_definition();
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["echoed"], "test");
}

#[test]
fn test_with_defaults_has_expected_tools() {
    let registry = ToolRegistry::with_defaults();
    // Exact inventory excludes test doubles and capability-owned tools:
    // exposing those here would bypass host composition or capability policy.
    assert_eq!(registry.tool_names(), ["report_progress"]);
    assert!(registry.tool_definitions()[0].display_name().is_some());
}

#[tokio::test]
async fn test_with_defaults_tools_are_executable() {
    let registry = ToolRegistry::with_defaults();

    // The neutral progress contract remains executable as a core default.
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "report_progress".to_string(),
        arguments: serde_json::json!({
            "status": "completed",
            "summary": "Boundary audit complete"
        }),
    };

    let tool_def = registry.get("report_progress").unwrap().to_definition();
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["summary"], "Boundary audit complete");
}

/// Regression: with_defaults() must NOT include capability-provided tools like
/// 'bash'. These tools come from capabilities and must be registered separately.
/// If bash were in defaults, the harness capability fallback would be masked.

#[test]
fn raw_output_preserves_object_keys_that_resemble_carriers() {
    for value in [
        serde_json::json!({"_raw_output_scalar": "user-value"}),
        serde_json::json!({"_raw_output": "user-value", "kept": true}),
    ] {
        let result =
            ToolExecutionResult::success_with_raw_output(value.clone(), "actual raw output".into())
                .into_tool_result("call", "tool");
        assert_eq!(result.result, Some(value));
        assert_eq!(result.raw_output.as_deref(), Some("actual raw output"));
    }
}
#[tokio::test]
async fn monitor_probe_registry_rejects_unregistered_tools() {
    let registry = ToolRegistry::with_monitor_probe_defaults();
    let call = ToolCall {
        id: "missing-id".into(),
        name: "echo".into(),
        arguments: serde_json::json!({"message":"x"}),
    };
    let definition = EchoTool.to_definition();
    let context = ToolContext::new(crate::typed_id::SessionId::new());
    for with_context in [false, true] {
        let error = if with_context {
            registry
                .execute_with_context(&call, &definition, &context)
                .await
                .unwrap_err()
        } else {
            registry.execute(&call, &definition).await.unwrap_err()
        };
        assert!(
            matches!(error, AgentLoopError::ToolExecution(message) if message.contains("echo"))
        );
    }
}
#[tokio::test]
async fn invalid_registered_schema_fails_configuration_before_dispatch() {
    struct InvalidSchema;
    #[async_trait]
    impl Tool for InvalidSchema {
        fn name(&self) -> &str {
            "invalid_schema"
        }
        fn description(&self) -> &str {
            "Invalid schema fixture"
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":42})
        }
        async fn execute(&self, _: Value) -> ToolExecutionResult {
            panic!("invalid schema must never dispatch")
        }
    }
    let registry = ToolRegistry::builder().tool(InvalidSchema).build();
    let call = ToolCall {
        id: "schema-id".into(),
        name: "invalid_schema".into(),
        arguments: serde_json::json!({}),
    };
    let context = ToolContext::new(crate::typed_id::SessionId::new());
    // The caller-supplied definition cannot replace the registered schema.
    let supplied = EchoTool.to_definition();
    for with_context in [false, true] {
        let error = if with_context {
            registry
                .execute_with_context(&call, &supplied, &context)
                .await
                .unwrap_err()
        } else {
            registry.execute(&call, &supplied).await.unwrap_err()
        };
        assert!(
            matches!(error,AgentLoopError::Configuration(message) if message.contains("invalid_schema") && message.contains("invalid parameters schema"))
        );
    }
}
