use super::*;
use serde_json::json;

fn definition(kind: &str) -> ToolDefinition {
    serde_json::from_value(json!({"type":kind,"name":"tool","description":"Description","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}})).unwrap()
}

#[test]
fn declared_capability_context_round_trips_and_does_not_follow_a_new_owner() {
    for kind in ["builtin", "client_side"] {
        let described = definition(kind)
            .with_hints(ToolHints::default().with_metadata(json!({"extension":"kept"})))
            .with_capability_context("minerals", Some("Minerals"), "Scandium prospecting");
        let restored: ToolDefinition =
            serde_json::from_value(serde_json::to_value(&described).unwrap()).unwrap();
        assert_eq!(
            restored.hints().capability_description.as_deref(),
            Some("Scandium prospecting")
        );
        assert_eq!(restored.hints().metadata, Some(json!({"extension":"kept"})));
        let reassigned = restored.with_capability_attribution("other", Some("Other"));
        assert!(reassigned.hints().capability_description.is_none());
    }
}

#[test]
fn tool_variants_have_complete_literal_wire_defaults_and_effective_policies() {
    for kind in ["builtin", "client_side"] {
        let tool = definition(kind);
        let mut expected = json!({"type":kind,"name":"tool","description":"Description","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}});
        if kind == "builtin" {
            expected["policy"] = json!("auto");
        }
        assert_eq!(serde_json::to_value(&tool).unwrap(), expected);
        assert_eq!(tool.name(), "tool");
        assert_eq!(tool.description(), "Description");
        assert_eq!(tool.display_name(), None);
        assert_eq!(
            tool.parameters(),
            &json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"]})
        );
        assert_eq!(
            tool.policy(),
            if kind == "builtin" {
                &ToolPolicy::Auto
            } else {
                &ToolPolicy::ClientSide
            }
        );
        assert_eq!(tool.deferrable(), &DeferrablePolicy::Automatic);
        assert!(tool.hints().is_empty());
        assert_eq!(tool.concurrency_class(), None);
        assert!(!tool.is_cpu_bound());
        assert_eq!(tool.side_effect_class(), SideEffectClass::AtMostOnce);
    }
    let mixed:Vec<ToolDefinition>=serde_json::from_value(json!([
        {"type":"builtin","name":"server","description":"Server","parameters":{},"policy":"requires_approval"},
        {"type":"client_side","name":"client","description":"Client","parameters":{},"policy":"auto"}
    ])).unwrap();
    assert_eq!(
        (
            mixed[0].name(),
            mixed[0].policy(),
            mixed[1].name(),
            mixed[1].policy()
        ),
        (
            "server",
            &ToolPolicy::RequiresApproval,
            "client",
            &ToolPolicy::ClientSide
        )
    );
    assert!(matches!(&mixed[0], ToolDefinition::Builtin(_)));
    assert!(matches!(&mixed[1], ToolDefinition::ClientSide(_)));
    for (policy, wire) in [
        (ToolPolicy::Auto, "auto"),
        (ToolPolicy::RequiresApproval, "requires_approval"),
        (ToolPolicy::ClientSide, "client_side"),
    ] {
        assert_eq!(serde_json::to_value(&policy).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<ToolPolicy>(json!(wire)).unwrap(),
            policy
        );
    }
}

#[test]
fn hints_builders_preserve_false_values_metadata_and_scheduling_fields() {
    for kind in ["builtin", "client_side"] {
        let reader = definition(kind).with_hints(ToolHints::default().with_readonly(true));
        assert_eq!(reader.concurrency_class(), None);
        assert!(!reader.is_cpu_bound());
        // A read-only tool is safe to run again.
        assert_eq!(reader.side_effect_class(), SideEffectClass::Pure);
    }

    for value in [false, true] {
        let hints = ToolHints::default()
            .with_readonly(value)
            .with_destructive(value)
            .with_idempotent(value)
            .with_open_world(value)
            .with_requires_secrets(value)
            .with_long_running(value)
            .with_supports_background(value)
            .with_cpu_bound(value)
            .with_persist_output(value)
            .with_concurrency_class("session_workspace")
            .with_narration_noun("file")
            .with_side_effect_class(SideEffectClass::Idempotent)
            .with_capability_attribution("files", Some("Files"))
            .with_metadata(json!({"risk_tier":"high","nested":[1,false,null]}));
        let expected = json!({"readonly":value,"destructive":value,"idempotent":value,"open_world":value,"requires_secrets":value,"long_running":value,"supports_background":value,"cpu_bound":value,"persist_output":value,"concurrency_class":"session_workspace","narration_noun":"file","side_effect_class":"Idempotent","capability_id":"files","capability_name":"Files","metadata":{"risk_tier":"high","nested":[1,false,null]}});
        assert_eq!(serde_json::to_value(&hints).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<ToolHints>(expected).unwrap(),
            hints
        );
        for kind in ["builtin", "client_side"] {
            let tool = definition(kind)
                .with_hints(hints.clone())
                .with_category("workspace")
                .with_capability_attribution("new-files", Some("New Files"));
            assert_eq!(tool.category(), Some("workspace"));
            assert_eq!(tool.concurrency_class(), Some("session_workspace"));
            assert_eq!(tool.is_cpu_bound(), value);
            assert_eq!(tool.side_effect_class(), SideEffectClass::Idempotent);
            assert_eq!(
                tool.capability_attribution(),
                Some(("new-files", Some("New Files")))
            );
            let mut expected_hints = serde_json::to_value(&hints).unwrap();
            expected_hints["capability_id"] = json!("new-files");
            expected_hints["capability_name"] = json!("New Files");
            assert_eq!(serde_json::to_value(tool.hints()).unwrap(), expected_hints);
        }
    }
    assert_eq!(
        serde_json::to_value(ToolHints::default()).unwrap(),
        json!({})
    );
    let metadata = ToolHints::default().with_metadata(json!({"any":"thing"}));
    assert!(!metadata.is_empty());
    assert_eq!(
        serde_json::to_value(metadata).unwrap(),
        json!({"metadata":{"any":"thing"}})
    );
    for class in [
        SideEffectClass::Pure,
        SideEffectClass::Idempotent,
        SideEffectClass::AtMostOnce,
    ] {
        assert_eq!(
            ToolHints::default()
                .with_side_effect_class(class.clone())
                .effective_side_effect_class(),
            class
        );
    }
}

#[test]
fn read_only_and_idempotent_hints_imply_a_side_effect_class() {
    use SideEffectClass::*;
    let hints = ToolHints::default;
    for (hints, class) in [
        (hints(), AtMostOnce),
        (hints().with_readonly(true), Pure),
        (hints().with_idempotent(true), Idempotent),
        (hints().with_readonly(false), AtMostOnce),
        // An explicit class wins over the hints.
        (
            hints()
                .with_readonly(true)
                .with_side_effect_class(Idempotent),
            Idempotent,
        ),
    ] {
        assert_eq!(hints.effective_side_effect_class(), class);
    }
}

#[test]
fn tool_display_and_deferred_schemas_survive_both_wire_variants() {
    for kind in ["builtin", "client_side"] {
        for (deferrable, wire) in [
            (DeferrablePolicy::Never, Some("never")),
            (DeferrablePolicy::Automatic, None),
            (DeferrablePolicy::Always, Some("always")),
        ] {
            let mut payload = json!({"type":kind,"name":"tool","display_name":"Display","description":"Description","parameters":{"type":"object"},"full_parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]},"category":"files"});
            if let Some(wire) = wire {
                payload["deferrable"] = json!(wire);
            }
            let tool: ToolDefinition = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(tool.display_name(), Some("Display"));
            assert_eq!(tool.deferrable(), &deferrable);
            assert_eq!(tool.parameters(), &json!({"type":"object"}));
            assert_eq!(
                tool.full_parameters(),
                &json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
            );
            if kind == "builtin" {
                payload["policy"] = json!("auto");
            }
            assert_eq!(serde_json::to_value(tool).unwrap(), payload);
        }
        let tool = definition(kind);
        assert_eq!(tool.full_parameters(), tool.parameters());
    }
}

#[test]
fn tool_call_wire_and_openai_arguments_preserve_the_complete_payload() {
    for (arguments, text) in [
        (json!({"city":"New York"}), r#"{"city":"New York"}"#),
        (json!({}), "{}"),
        (
            json!({"count":9007199254740993_u64,"text":"line\nquoted\""}),
            r#"{"count":9007199254740993,"text":"line\nquoted\""}"#,
        ),
    ] {
        let call = ToolCall {
            id: "call_123".into(),
            name: "get_weather".into(),
            arguments: arguments.clone(),
        };
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            json!({"id":"call_123","name":"get_weather","arguments":arguments})
        );
        let parsed: ToolCall = serde_json::from_value(
            json!({"id":"call_123","name":"get_weather","arguments":arguments}),
        )
        .unwrap();
        assert_eq!(parsed.arguments, arguments);
        assert_eq!(
            call.to_openai_format(),
            json!({"id":"call_123","type":"function","function":{"name":"get_weather","arguments":text}})
        );
    }
}

#[test]
fn tool_result_wire_excludes_raw_output_but_preserves_images_errors_and_data() {
    let expected = json!({"tool_call_id":"call_123","result":{"temperature":72},"images":[{"base64":"aGk=","media_type":"image/png"}],"error":"partial failure","connection_required":"provider"});
    let mut injected = expected.clone();
    injected["raw_output"] = json!("private-raw");
    let mut result: ToolResult = serde_json::from_value(injected).unwrap();
    assert_eq!(
        result.connection_required,
        Some(ConnectionRequired::provider_only("provider"))
    );
    assert!(result.raw_output.is_none());
    result.raw_output = Some("private-raw".into());
    assert_eq!(serde_json::to_value(result).unwrap(), expected);
    assert_eq!(
        serde_json::to_value(ToolResult::error("failed")).unwrap(),
        json!({"tool_call_id":"","result":null,"error":"failed"})
    );
    let success: ToolResult = serde_json::from_value(
        json!({"tool_call_id":"call_123","result":{"temperature":72},"error":null}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(success).unwrap(),
        json!({"tool_call_id":"call_123","result":{"temperature":72},"error":null})
    );
}

#[test]
fn connection_required_wire_distinguishes_subjects_and_accepts_provider_only_values() {
    for (subject, setup_url) in [
        (
            ConnectionRequiredSubject::Agent,
            "/agents/agent_123?tab=mcp",
        ),
        (ConnectionRequiredSubject::User, "/settings/connections"),
    ] {
        let required = ConnectionRequired::with_setup("mcp_oauth_123", subject, setup_url);
        let wire = serde_json::to_value(&required).unwrap();
        assert_eq!(wire["provider"], "mcp_oauth_123");
        assert_eq!(
            wire["subject"],
            match subject {
                ConnectionRequiredSubject::Agent => "agent",
                ConnectionRequiredSubject::User => "user",
            }
        );
        assert_eq!(wire["setup_url"], setup_url);
        assert_eq!(
            serde_json::from_value::<ConnectionRequired>(wire).unwrap(),
            required
        );
    }

    let legacy: ConnectionRequired = serde_json::from_value(json!("daytona")).unwrap();
    assert_eq!(legacy, ConnectionRequired::provider_only("daytona"));
    assert_eq!(serde_json::to_value(legacy).unwrap(), json!("daytona"));

    let provider_object: ConnectionRequired =
        serde_json::from_value(json!({"provider":"github"})).unwrap();
    assert_eq!(provider_object, ConnectionRequired::provider_only("github"));
}

#[test]
fn narration_schema_is_optional_idempotent_and_does_not_mutate_input_definitions() {
    let original = vec![definition("builtin"), definition("client_side")];
    let before = serde_json::to_value(&original).unwrap();
    let augmented = add_human_intent_to_tool_definitions(&original);
    assert_eq!(serde_json::to_value(&original).unwrap(), before);
    let property = json!({"type":"string","description":"Short user-facing narration of what this tool call will do, written as an action phrase like \"Listing all harnesses\". Do not include hidden reasoning, private chain of thought, secrets, or credential values.","maxLength":120});
    for tool in &augmented {
        assert_eq!(
            tool.parameters(),
            &json!({"type":"object","properties":{"input":{"type":"string"},"human_intent":property},"required":["input"]})
        );
    }
    assert_eq!(
        serde_json::to_value(add_human_intent_to_tool_definitions(&augmented)).unwrap(),
        serde_json::to_value(&augmented).unwrap()
    );
    let mut closed = json!({"properties":{"operation":{"type":"string","enum":["list"]}},"required":["operation"],"additionalProperties":false});
    add_human_intent_to_schema(&mut closed);
    assert_eq!(
        closed,
        json!({"type":"object","properties":{"operation":{"type":"string","enum":["list"]},"human_intent":property},"required":["operation"],"additionalProperties":false})
    );
    let mut empty = json!({});
    add_human_intent_to_schema(&mut empty);
    assert_eq!(
        empty,
        json!({"type":"object","properties":{"human_intent":property}})
    );
    for mut scalar in [json!(null), json!(false), json!([])] {
        let before = scalar.clone();
        add_human_intent_to_schema(&mut scalar);
        assert_eq!(scalar, before);
    }
}

#[test]
fn execution_strips_only_top_level_narration_and_trims_display_text() {
    for (value, expected) in [
        (
            json!(" Listing all harnesses "),
            Some("Listing all harnesses"),
        ),
        (json!(" \t"), None),
        (json!(7), None),
        (json!(null), None),
    ] {
        let arguments = json!({"operation":"list","human_intent":value,"nested":{"human_intent":"ordinary data"}});
        let call = ToolCall {
            id: "call".into(),
            name: "manage_harnesses".into(),
            arguments: arguments.clone(),
        };
        assert_eq!(human_intent(&call.arguments), expected);
        assert_eq!(
            call.execution_arguments(),
            json!({"operation":"list","nested":{"human_intent":"ordinary data"}})
        );
        assert_eq!(call.arguments, arguments);
    }
    for value in [json!(null), json!([1, "text"]), json!({"operation":"list"})] {
        assert_eq!(strip_human_intent_argument(&value), value);
        assert_eq!(human_intent(&value), None);
    }
}

#[test]
fn unattended_never_answers_a_servers_questions() {
    let arguments = json!({
        "questions": [{"id": "plan", "header": "billing", "question": "Plan?",
            "options": [{"label": "Pro", "description": "", "default": true},
                        {"label": "Team", "description": ""}]}],
        "mcp_elicitation": {"server": "billing", "tool": "buy"}
    });
    let result = unattended_ask_user_result(&arguments);
    assert_eq!(result["status"], "declined");
    assert_eq!(result["answers"], json!([]));
}

#[test]
fn url_elicitation_requires_discriminator_and_complete_payload() {
    let payload = json!({"code":"url_elicitation_required","error":"Waiting","url":"https://consent.example/path","url_host":"consent.example","url_is_punycode":false,"server":"server","tool":"tool","retry_tool":"mcp_server_tool","message":"Connect account","declined":false});
    let mut result = ToolResult::error("unrelated");
    result.result = Some(payload.clone());
    assert_eq!(
        serde_json::to_value(UrlElicitationRequired::from_tool_result(&result).unwrap()).unwrap(),
        payload
    );
    let mut declined = payload.clone();
    declined["declined"] = json!(true);
    result.result = Some(declined.clone());
    assert_eq!(
        serde_json::to_value(UrlElicitationRequired::from_tool_result(&result).unwrap()).unwrap(),
        declined
    );
    for malformed in [
        None,
        Some(json!(null)),
        Some(json!({"code":"url_elicitation_required"})),
        Some({
            let mut value = payload.clone();
            value["code"] = json!("connection_required");
            value
        }),
        Some({
            let mut value = payload.clone();
            value["declined"] = json!("false");
            value
        }),
    ] {
        result.result = malformed;
        assert!(UrlElicitationRequired::from_tool_result(&result).is_none());
    }
}
