use super::*;

#[test]
fn personal_browser_action_allowlist_is_strict() {
    assert!(
        actions::validate(
            "personal_browser_navigate",
            &json!({"url":"https://example.test"})
        )
        .is_ok()
    );
    for args in [
        json!({"url":"file:///etc/passwd"}),
        json!({"url":"javascript:alert(1)"}),
        json!({"url":"https://user:secret@example.test"}),
        json!({"url":"https://example.test","browser":"foreign"}),
    ] {
        assert!(actions::validate("personal_browser_navigate", &args).is_err());
    }
    assert!(actions::validate("personal_browser_click", &json!({"tab":1,"ref":"f1e3"})).is_ok());
    for args in [
        json!({"tab":1,"selector":"#submit"}),
        json!({"tab":1,"ref":"e1","force":true}),
        json!({"tab":1,"ref":"text=Delete"}),
    ] {
        assert!(actions::validate("personal_browser_click", &args).is_err());
    }
    assert!(actions::upstream("personal_browser_eval").is_none());
    assert!(actions::upstream("personal_browser_upload").is_none());
    assert!(
        actions::validate(
            actions::SCREENSHOT,
            &json!({"tab":1,"path":"/tmp/exfil.png"})
        )
        .is_err()
    );
    assert!(
        actions::output(
            actions::SCREENSHOT,
            json!({"content":[{"type":"image","mimeType":"image/svg+xml","data":"PHN2Zz4="}]})
        )
        .is_err()
    );
}
#[tokio::test]
async fn personal_browser_enable_disable_preserves_other_client_tools() {
    let profile = MemoryHosting::default();
    let engine = engine(
        profile.clone(),
        Agent::builder()
            .instructions("Test personal browser host tools.")
            .model(Model::simulated("ok"))
            .client_tool(
                "other",
                "Keep exactly",
                json!({"type":"object","properties":{"x":{"type":"string"}}}),
            )
            .build()
            .unwrap(),
    );
    let id = SessionId::new();
    engine.create_registered(id, &build()).await.unwrap();
    configure(&engine, id, true).await.unwrap();
    let before = profile
        .query_snapshot(id)
        .await
        .unwrap()
        .configuration
        .client_tools
        .unwrap();
    let other = before
        .iter()
        .find(|tool| tool.name() == "other")
        .unwrap()
        .clone();
    configure(&engine, id, false).await.unwrap();
    let after = profile
        .query_snapshot(id)
        .await
        .unwrap()
        .configuration
        .client_tools
        .unwrap();
    assert_eq!(serde_json::to_value(after).unwrap(), json!([other]));
}

fn action_samples() -> Vec<(&'static str, Value)> {
    vec![
        (
            "personal_browser_navigate",
            json!({"url":"https://example.test"}),
        ),
        ("personal_browser_snapshot", json!({"tab":42})),
        ("personal_browser_click", json!({"tab":42,"ref":"e12"})),
        (
            "personal_browser_fill",
            json!({"tab":42,"ref":"f1e3","value":"test"}),
        ),
        (
            "personal_browser_press",
            json!({"tab":42,"ref":"e1","key":"Enter"}),
        ),
        ("personal_browser_scroll", json!({"tab":42,"deltaY":500})),
        (actions::SCREENSHOT, json!({"tab":42})),
    ]
}
#[tokio::test]
async fn personal_browser_actions_and_inline_image_are_durable() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = MemoryHosting::default();
    let mut calls = action_samples()
        .into_iter()
        .enumerate()
        .map(|(i, (name, arguments))| ToolCall {
            id: format!("action-{i}"),
            name: name.into(),
            arguments,
        })
        .collect::<Vec<_>>();
    calls.push(ToolCall {
        id: "oversize".into(),
        name: actions::SCREENSHOT.into(),
        arguments: json!({"tab":999}),
    });
    let agent = Agent::builder()
        .instructions("Exercise browser fixtures")
        .model(Model::simulated_with_config(
            LlmSimConfig::fixed("done").with_tool_call_sequence(vec![calls.clone(), vec![]]),
        ))
        .build()
        .unwrap();
    let engine = engine(profile.clone(), agent);
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let admission = session.send("actions").await.unwrap();
    wait_pending(&engine, id).await;
    let turn = engine.session_status(id).await.unwrap().open_turn.unwrap();
    assert_eq!(
        worker.run_pending(&engine, id).await.unwrap().completed,
        calls.len()
    );
    tokio::time::timeout(Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    for call in calls {
        let record: ClientDispatchRecord = serde_json::from_value(
            profile
                .command(
                    id,
                    SessionCommand::InspectClientDispatch {
                        turn,
                        call_id: call.id.clone(),
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        let result = record.result.unwrap();
        if call.id == "oversize" {
            assert!(!result.success);
            assert!(result.error.unwrap().contains("size limit"));
        } else {
            assert!(result.success);
            if call.name == actions::SCREENSHOT {
                assert!(
                    result
                        .result
                        .unwrap()
                        .iter()
                        .any(|part| matches!(part, crate::ContentPart::Image(_)))
                );
            }
        }
    }
    worker.close_session(&engine, id).await.unwrap();
}
#[tokio::test]
async fn personal_browser_exact_samples_match_real_pinned_discovery() {
    let Some(path) = std::env::var_os("EVERRUNS_BROWSE_EXECUTABLE") else {
        assert!(
            std::env::var_os("EVERRUNS_BROWSE_SMOKE_REQUIRED").is_none(),
            "Pinned Browse executable required"
        );
        eprintln!("SKIP pinned upstream schema compatibility: EVERRUNS_BROWSE_EXECUTABLE unset");
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut cfg = BrowseProcessConfig::new(path);
    for key in [
        "HOME",
        "OPENCODE_BROWSER_HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        cfg.environment
            .insert(key.into(), profile.path().to_string_lossy().into_owned());
    }
    cfg.environment
        .insert("OPENCODE_BROWSER_AUTOSTART".into(), "0".into());
    cfg.environment
        .insert("OPENCODE_BROWSER_PORT".into(), port.to_string());
    let owner = Uuid::new_v4();
    let supervisor = BrowseSupervisor::start(owner, cfg).await.unwrap();
    let discovery = supervisor.list_tools(owner).await.unwrap();
    let tools = discovery["tools"].as_array().unwrap();
    let mut samples = action_samples();
    samples.extend([
        (STATUS_TOOL, json!({})),
        (TABS_TOOL, json!({"action":"list"})),
        (
            HANDOFF_TOOL,
            json!({"tab":42,"message":"Confirm test","timeoutSeconds":30}),
        ),
    ]);
    for (name, args) in samples {
        let upstream = match name {
            STATUS_TOOL => "status",
            TABS_TOOL => "tabs",
            HANDOFF_TOOL => "handoff",
            _ => actions::upstream(name).unwrap(),
        };
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == upstream)
            .unwrap_or_else(|| panic!("missing upstream {upstream}"));
        let validator = jsonschema::validator_for(&tool["inputSchema"]).unwrap();
        assert!(
            validator.is_valid(&args),
            "{name} args incompatible with pinned upstream schema: {args}"
        );
        let declaration = declarations()
            .into_iter()
            .find(|tool| tool.name == name)
            .unwrap();
        for field in declaration.parameters["properties"]
            .as_object()
            .unwrap()
            .keys()
        {
            assert!(
                tool["inputSchema"]["properties"].get(field).is_some(),
                "unknown upstream field {name}.{field}"
            );
        }
    }
    supervisor.close().await;
}

#[tokio::test]
async fn personal_browser_only_real_continued_outcome_is_success() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = MemoryHosting::default();
    let calls = ["continued", "cancelled", "timeout", "closed"]
        .into_iter()
        .map(|outcome| ToolCall {
            id: outcome.into(),
            name: HANDOFF_TOOL.into(),
            arguments: json!({"tab":42,"message":outcome}),
        })
        .collect::<Vec<_>>();
    let agent = Agent::builder()
        .instructions("Test handoff terminal states")
        .model(Model::simulated_with_config(
            LlmSimConfig::fixed("done").with_tool_call_sequence(vec![calls.clone(), vec![]]),
        ))
        .build()
        .unwrap();
    let engine = engine(profile.clone(), agent);
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let admission = session.send("handoff cases").await.unwrap();
    wait_pending(&engine, id).await;
    let turn = engine.session_status(id).await.unwrap().open_turn.unwrap();
    assert_eq!(worker.run_pending(&engine, id).await.unwrap().completed, 4);
    tokio::time::timeout(Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    for call in calls {
        let record: ClientDispatchRecord = serde_json::from_value(
            profile
                .command(
                    id,
                    SessionCommand::InspectClientDispatch {
                        turn,
                        call_id: call.id.clone(),
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        let result = record.result.unwrap();
        assert_eq!(result.success, call.id == "continued");
        if !result.success {
            assert_eq!(result.details.unwrap()["outcome"], call.id);
            assert!(
                result
                    .error
                    .unwrap()
                    .contains("human continuation was not received")
            );
        }
    }
    worker.close_session(&engine, id).await.unwrap();
}
