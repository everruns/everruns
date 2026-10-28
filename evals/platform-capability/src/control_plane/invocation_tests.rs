use super::*;

#[tokio::test]
async fn both_surfaces_require_a_reason_before_creating_an_entity() {
    for harness in [Harness::PlatformChat, Harness::NativeEager] {
        let plane = FakeControlPlane::new(harness);
        let before = plane.snapshot()["families"].clone();
        let out = if harness == Harness::NativeEager {
            plane
                .call(
                    "create_agent",
                    &json!({"params":{"name":"new","system_prompt":"Hi"}}),
                )
                .await
        } else {
            plane
                .call(
                    "bash",
                    &json!({"commands":"everruns agents create --name new --system-prompt Hi"}),
                )
                .await
        };
        assert!(out.contains("reason_required"), "{out}");
        assert_eq!(plane.snapshot()["families"], before);
        assert_eq!(plane.snapshot()["operations"][0]["status"], "rejected");
    }
}

#[tokio::test]
async fn stale_revision_rejects_before_mutation_and_metadata_stays_out_of_rows() {
    for harness in [Harness::PlatformChat, Harness::NativeEager] {
        let plane = FakeControlPlane::new(harness);
        let before = plane.snapshot()["families"].clone();
        let out = if harness == Harness::NativeEager {
            plane.call("update_agent", &json!({"params":{"id":"agent_01triage","description":"updated"},"reason":"Requested","context_revision":2})).await
        } else {
            plane.call("bash", &json!({"commands":"everruns agents update agent_01triage --description updated --reason Requested --context-revision 2"})).await
        };
        assert!(out.contains("manager_context_changed"), "{out}");
        assert_eq!(plane.snapshot()["families"], before);
        let out = if harness == Harness::NativeEager {
            plane.call("update_agent", &json!({"params":{"id":"agent_01triage","description":"updated"},"reason":"  Requested  ","context_revision":3})).await
        } else {
            plane.call("bash", &json!({"commands":"everruns agents update agent_01triage --description updated --reason '  Requested  ' --context-revision 3"})).await
        };
        let row: Value = serde_json::from_str(&out).expect(&out);
        assert_eq!(row["description"], "updated");
        assert!(row.get("reason").is_none() && row.get("context_revision").is_none());
        let audit = plane.snapshot()["operations"][1].clone();
        assert_eq!(audit["status"], "completed");
        assert_eq!(audit["metadata"]["reason"], "Requested");
        assert!(audit["arguments"].get("reason").is_none());
    }
}

#[tokio::test]
async fn missing_context_revision_warns_but_does_not_reject() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    let out = plane
        .call(
            "update_agent",
            &json!({"params":{"id":"agent_01triage","name":"Renamed"},"reason":"Requested"}),
        )
        .await;
    let row: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(row["name"], "Renamed");
    assert!(row["warnings"][0].as_str().unwrap().contains("revision 3"));
    assert!(
        plane.snapshot()["families"]["agents"][0]
            .get("warnings")
            .is_none()
    );
}

#[tokio::test]
async fn invalid_reasons_never_mutate_entities() {
    for reason in [
        " ".to_string(),
        "x".repeat(1001),
        "bad\u{0007}".into(),
        "use sk-abcdefghijklmnopqrstuvwxyz012345".into(),
    ] {
        for harness in [Harness::PlatformChat, Harness::NativeEager] {
            let plane = FakeControlPlane::new(harness);
            let before = plane.snapshot()["families"].clone();
            let out = if harness == Harness::NativeEager {
                plane
                    .call(
                        "create_agent",
                        &json!({"params":{"name":"new","system_prompt":"Hi"},"reason":reason}),
                    )
                    .await
            } else {
                plane.call("bash", &json!({"commands":format!("everruns agents create --name new --system-prompt Hi --reason '{reason}'")})).await
            };
            assert!(out.contains("invalid_change_reason"), "{out}");
            assert_eq!(plane.snapshot()["families"], before);
        }
    }
}

#[test]
fn envelope_preserves_nested_payload_validation_and_local_references() {
    let entry = json!({"name":"example","input_schema":{"type":"object","properties":{"nested":{"$ref":"#/$defs/Nested"}},"required":["nested"],"$defs":{"Nested":{"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]}}}});
    let tool = native_tool_definition(&entry);
    let validator = jsonschema::validator_for(&tool["function"]["parameters"]).unwrap();
    assert!(validator.is_valid(&json!({"params":{"nested":{"count":2}},"reason":"Requested"})));
    assert!(!validator.is_valid(&json!({"params":{"nested":{"count":"bad"}}})));
    assert!(!validator.is_valid(&json!({"nested":{"count":2}})));
}

#[test]
fn composed_payloads_advertise_their_object_type_without_string_coercion() {
    let payload = json!({"allOf":[{"$ref":"#/$defs/Patch"},{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}],"$defs":{"Patch":{"type":"object","properties":{"description":{"type":"string"}}}}});
    let schema = native_schema(&payload);
    assert_eq!(schema["properties"]["params"]["type"], "object");
    let canonical = jsonschema::validator_for(&payload).unwrap();
    let native = jsonschema::validator_for(&schema).unwrap();
    for value in [
        json!({"id":"agent_1","description":"new"}),
        json!({"id":42}),
        json!({"description":"new"}),
        json!("{\"id\":\"agent_1\"}"),
        Value::Null,
    ] {
        assert_eq!(
            canonical.is_valid(&value),
            native.is_valid(&json!({"params":value}))
        );
    }
    let nullable = json!({"anyOf":[{"type":"object"},{"type":"null"}]});
    let schema = native_schema(&nullable);
    assert!(schema["properties"]["params"].get("type").is_none());
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&json!({"params":null}))
    );
}

#[tokio::test]
async fn advisory_and_read_calls_do_not_require_change_reasons() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    let out = plane
        .call(
            "analyze_agent",
            &json!({"params":{"system_prompt":"Triage issues"}}),
        )
        .await;
    assert!(!out.contains("reason_required"), "{out}");
    assert_eq!(plane.snapshot()["operations"][0]["status"], "completed");
    let out = plane
        .call("list_agents", &json!({"params":{},"reason":" "}))
        .await;
    assert!(out.contains("Triage"), "{out}");
}

#[tokio::test]
async fn a_failed_mock_lookup_is_not_a_completed_operation() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    let out = plane.call("update_agent", &json!({"params":{"id":"agent_absent","name":"new"},"reason":"Requested","context_revision":0})).await;
    assert!(out.contains("fixture_command_failed"), "{out}");
    assert_eq!(plane.snapshot()["operations"][0]["status"], "rejected");
}

#[test]
fn scale_fixture_is_deterministic_bounded_and_opt_in() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    assert_eq!(
        plane.snapshot()["families"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    plane.seed_agents(200).unwrap();
    let snapshot = plane.snapshot();
    let rows = snapshot["families"]["agents"].as_array().unwrap();
    assert_eq!(rows.len(), 200);
    assert_eq!(
        rows.iter()
            .filter(|row| row["default_model_id"] == "model_01terra")
            .count(),
        100
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row["default_model_id"] == "model_01muse")
            .count(),
        100
    );
    assert!(
        rows.iter()
            .all(|row| row["description"].as_str().unwrap().len() == 1024)
    );
    plane.seed_agents(200).unwrap();
    assert_eq!(plane.snapshot(), snapshot);
    assert!(plane.seed_agents(1001).is_err());
    assert_eq!(plane.snapshot(), snapshot);
}

#[tokio::test]
async fn cli_parser_rejections_are_attempts_but_help_is_not() {
    let plane = FakeControlPlane::new(Harness::PlatformChat);
    plane
        .call("bash", &json!({"commands":"everruns agents update --help"}))
        .await;
    assert_eq!(plane.snapshot()["operations"], json!([]));
    plane
        .call(
            "bash",
            &json!({"commands":"everruns agents update agent_01triage --unknown-flag x"}),
        )
        .await;
    let operation = plane.snapshot()["operations"][0].clone();
    assert_eq!(operation["name"], "update_agent");
    assert_eq!(operation["status"], "rejected");
    assert_eq!(operation["attempted"], true);
    assert_eq!(operation["read_only"], false);
}

#[tokio::test]
async fn legacy_flat_flags_separate_kebab_case_invocation_metadata() {
    let plane = FakeControlPlane::new(Harness::LegacyPlatformChat);
    let out = plane.call("execute", &json!({"commands":"update_agent --id agent_01triage --description updated --reason Requested --context-revision 3"})).await;
    let row: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(row["description"], "updated");
    assert!(row.get("context-revision").is_none() && row.get("context_revision").is_none());
    assert_eq!(
        plane.snapshot()["operations"][0]["metadata"]["context_revision"],
        3
    );
}

#[tokio::test]
async fn reading_an_unseeded_family_never_changes_fixture_state() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    let before = plane.snapshot()["families"].clone();
    assert!(before.get("providers").is_none());
    let listed = plane.call("list_providers", &json!({"params":{}})).await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed["data"], json!([]));
    assert_eq!(listed["total"], 0);
    assert_eq!(plane.snapshot()["families"], before);
    let missing = plane
        .call("get_provider", &json!({"params":{"id":"provider_absent"}}))
        .await;
    assert!(missing.contains("fixture_command_failed"), "{missing}");
    assert_eq!(plane.snapshot()["families"], before);
}

#[tokio::test]
async fn list_offset_reaches_disjoint_pages_and_the_last_fixture_row() {
    let plane = FakeControlPlane::new(Harness::NativeEager);
    plane.seed_agents(200).unwrap();
    let first: Value = serde_json::from_str(
        &plane
            .call("list_agents", &json!({"params":{"limit":100,"offset":0}}))
            .await,
    )
    .unwrap();
    let second: Value = serde_json::from_str(
        &plane
            .call("list_agents", &json!({"params":{"limit":100,"offset":100}}))
            .await,
    )
    .unwrap();
    for (page, offset) in [(&first, 0), (&second, 100)] {
        assert_eq!(page["total"], 200);
        assert_eq!(page["offset"], offset);
        assert_eq!(page["limit"], 100);
        assert_eq!(page["data"].as_array().unwrap().len(), 100);
    }
    let first_ids: std::collections::BTreeSet<_> = first["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert!(
        second["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| !first_ids.contains(row["id"].as_str().unwrap()))
    );
    let last: Value = serde_json::from_str(
        &plane
            .call("list_agents", &json!({"params":{"limit":1,"offset":199}}))
            .await,
    )
    .unwrap();
    assert_eq!(last["data"].as_array().unwrap().len(), 1);
    assert_eq!(last["data"][0]["id"], "agent_scale0200");
    assert_eq!(last["offset"], 199);
    assert_eq!(last["total"], 200);
}

#[test]
fn bounded_unknown_identity_is_unclassified_while_known_denials_keep_their_policy() {
    let plane = FakeControlPlane::new(Harness::NativeBounded);
    plane.reject_native(
        "read_unknown",
        &json!({"params":{}}),
        "operation_not_active",
        "Unknown operation",
    );
    let snapshot = plane.snapshot();
    let operations = snapshot["operations"].as_array().unwrap();
    assert_eq!(operations[0]["read_only"], Value::Null);
    assert!(
        !operations
            .iter()
            .any(|op| op["read_only"] == false && op["attempted"] != false),
        "Unknown identity cannot establish a forbidden mutation"
    );
    plane.reject_native(
        "update_agent",
        &json!({"params":{"id":"agent_01triage"}}),
        "operation_not_active",
        "Known but unrevealed operation",
    );
    assert_eq!(plane.snapshot()["operations"][1]["read_only"], false);
    plane.reject_native(
        "list_agents",
        &json!({"params":{}}),
        "operation_not_active",
        "Known but unrevealed operation",
    );
    assert_eq!(plane.snapshot()["operations"][2]["read_only"], true);
}
