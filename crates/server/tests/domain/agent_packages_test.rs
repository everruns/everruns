use super::test_harness::TestServer;
use axum::http::{Method, StatusCode};
use everruns_core::agent_package::{AgentPackage, Format};
use serde_json::{Value, json};

fn definition(name: &str) -> Value {
    json!({"schema_version":1,"name":name,"instructions":"Read the runbook.","harness":"base","capabilities":["session_file_system"],"initial_files":[{"path":"/runbook.md","content":"The runbook"}],"channels":{"chat":{"type":"ag_ui"}}})
}

#[tokio::test]
async fn api_import_export_zip_and_diff_round_trip_without_ids() {
    let server = TestServer::in_memory().await;
    let content = definition("package-test").to_string();
    let body = json!({"content":content,"format":"json"});
    let validation = server
        .post("/v1/agents/validate", &body)
        .await
        .assert_success()
        .json_value();
    assert_eq!(validation["valid"], true, "{validation}");
    let agent = server
        .post("/v1/agents/import", &body)
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let exported = server
        .get("/v1/agents/package-test/export?format=json")
        .await
        .assert_success()
        .json_value();
    assert!(exported.get("id").is_none());
    assert!(exported.get("harness_id").is_none());
    assert_eq!(exported["instructions"], "Read the runbook.");
    assert_eq!(exported["files"][0]["content"], "The runbook");
    assert_eq!(exported["channels"]["chat"]["enabled"], false);
    let mut proposed = exported.clone();
    proposed["instructions"] = json!("Changed instructions");
    let diff = server
        .post(
            "/v1/agents/diff",
            json!({"content":proposed.to_string(),"format":"json","target":"package-test"}),
        )
        .await
        .assert_success()
        .json_value();
    assert_eq!(diff["changes"].as_array().unwrap().len(), 1, "{diff}");
    assert_eq!(diff["changes"][0]["path"], "/instructions");
    let package = AgentPackage::parse(&proposed.to_string(), Format::Json).unwrap();
    let imported = server
        .request_raw(
            Method::POST,
            "/v1/agents/import?target=package-test&format=zip",
            vec![("Content-Type", "application/zip")],
            package.to_zip().unwrap(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(imported["id"], agent["id"]);
    let diff = server
        .post(
            "/v1/agents/diff",
            json!({"content":proposed.to_string(),"format":"json","target":"package-test"}),
        )
        .await
        .assert_success()
        .json_value();
    assert_eq!(diff["changed"], false, "{diff}");
}

#[tokio::test]
async fn validation_and_dependency_failure_never_create_an_agent() {
    let server = TestServer::in_memory().await;
    let mut value = definition("invalid-package");
    value["capabilities"] = json!(["missing-capability"]);
    let body = json!({"content":value.to_string(),"format":"json"});
    assert_eq!(
        server
            .post("/v1/agents/validate", &body)
            .await
            .assert_success()
            .json_value()["valid"],
        false
    );
    server
        .post("/v1/agents/import", &body)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .get("/v1/agents/invalid-package")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    value = definition("invalid-package");
    value["channels"]["chat"]["type"] = json!("webhook");
    server
        .post("/v1/agents/import", json!({"content":value.to_string()}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    server
        .get("/v1/agents/invalid-package")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_markdown_is_accepted_and_versioned_ids_are_rejected() {
    let server = TestServer::in_memory().await;
    server
        .request_raw(
            Method::POST,
            "/v1/agents/import",
            vec![("Content-Type", "text/markdown")],
            b"---\nname: Simple Agent\n---\nBe helpful.".to_vec(),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let exported = server
        .get("/v1/agents/simple-agent/export")
        .await
        .assert_success()
        .text();
    assert!(exported.contains("Be helpful."));
    assert!(!exported.contains("harness_id"));
    let mut bad = definition("versioned-id");
    bad["id"] = json!("agent_01933b5a000070008000000000000001");
    server
        .post("/v1/agents/import", json!({"content":bad.to_string()}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_import_and_export_use_the_shared_codec() {
    let server = TestServer::in_memory().await;
    for command in [
        format!(
            "everruns agents import --content '{}' --format json",
            definition("mcp-package")
        ),
        "everruns agents export mcp-package --format toml".to_string(),
    ] {
        let response = server
            .post(
                "/mcp",
                json!({
                    "jsonrpc":"2.0", "id":1, "method":"tools/call",
                    "params":{"name":"execute", "arguments":{"commands":command}}
                }),
            )
            .await
            .assert_success()
            .json_value();
        assert_ne!(response["result"]["isError"], true, "{response}");
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("mcp-package"),
            "{response}"
        );
    }
    server.get("/v1/agents/mcp-package").await.assert_success();
}

#[tokio::test]
async fn platform_chat_cli_resolves_workspace_folders_and_exports_zip_artifacts() {
    use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
    use everruns_server::domains::common::{Command, Ctx};
    use everruns_server::domains::session_files::{CreateWorkspaceFile, types::CreateFileRequest};
    use everruns_server::services::platform_command_surface::{CatalogContext, Operation, invoke};
    let server = TestServer::in_memory().await;
    let session = server
        .post(
            "/v1/sessions",
            json!({"harness_id":server.seed_base_harness_id}),
        )
        .await
        .assert_success()
        .json_value();
    let id = session["id"].as_str().unwrap();
    let ctx = Ctx::minimal(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        server.db.clone(),
        server.encryption.clone(),
        std::sync::Arc::new(everruns_core::DefaultPermissionResolver),
    )
    .with_feature_flags(everruns_server::records::FeatureFlags {
        skills: true,
        ..Default::default()
    })
    .acting_for_session(id.parse().unwrap());
    for (path, content) in [
        (
            "/agent/agent.toml",
            "schema_version = 1\nname = 'chat-package'\nharness = 'base'\ninstructions = 'Read the runbook.'\nfiles = ['runbook.md']",
        ),
        ("/agent/instructions.md", "Read the runbook."),
        ("/agent/runbook.md", "Workspace runbook"),
        ("/agent/unselected.txt", "Do not package this project file"),
        (
            "/agent/.agents/skills/check/SKILL.md",
            "---\nname: check\ndescription: Check the runbook\n---\nUse the script.",
        ),
        (
            "/agent/.agents/skills/check/scripts/check.py",
            "print('check')",
        ),
    ] {
        CreateWorkspaceFile {
            session_id: id.into(),
            path: path.into(),
            req: CreateFileRequest {
                content: Some(content.into()),
                encoding: Some("text".into()),
                is_readonly: Some(false),
                is_directory: Some(false),
            },
        }
        .run(&ctx)
        .await
        .unwrap();
    }
    let context = CatalogContext {
        domain_ctx: ctx,
        link_builder: everruns_server::api::common::UrlBuilder::new(
            "http://localhost",
            "http://localhost",
        ),
    };
    for (operation, commands) in [
        (Operation::Query, "everruns agents validate /agent"),
        (
            Operation::Query,
            "everruns agents validate /agent/agent.toml",
        ),
        (
            Operation::Execute,
            "everruns agents import /agent/agent.toml",
        ),
        (
            Operation::Execute,
            "everruns agents export chat-package --format zip --out /exports/chat.zip",
        ),
    ] {
        let result = invoke(operation, &json!({"commands":commands}), context.clone())
            .await
            .unwrap();
        assert!(!result.contains("\"valid\":false"), "{result}");
    }
    let response = server
        .get(&format!("/v1/sessions/{id}/fs/exports/chat.zip"))
        .await
        .assert_success()
        .json_value();
    let bytes = everruns_core::SessionFile::decode_content(
        response["content"].as_str().unwrap(),
        response["encoding"].as_str().unwrap(),
    )
    .unwrap();
    let package = AgentPackage::from_zip(&bytes).unwrap();
    assert_eq!(package.files().unwrap().len(), 3);
    assert!(
        package
            .files()
            .unwrap()
            .iter()
            .any(|f| f.path.ends_with("scripts/check.py"))
    );
    assert!(
        invoke(
            Operation::Query,
            &json!({"commands":"everruns agents export chat-package --out /blocked.json"}),
            context
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn full_replacement_removes_old_capabilities_and_reports_field_diagnostics() {
    let server = TestServer::in_memory().await;
    let mut definition = definition("replace-package");
    server
        .post(
            "/v1/agents/import",
            json!({"content":definition.to_string()}),
        )
        .await
        .assert_success();
    definition["capabilities"] = json!([]);
    definition["initial_files"] = json!([]);
    server
        .post(
            "/v1/agents/import?target=replace-package",
            json!({"content":definition.to_string()}),
        )
        .await
        .assert_success();
    let exported = server
        .get("/v1/agents/replace-package/export?format=json")
        .await
        .assert_success()
        .json_value();
    assert!(exported.get("capabilities").is_none(), "{exported}");
    definition["name"] = json!("Bad Name");
    definition["max_iterations"] = json!(0);
    let validation = server
        .post(
            "/v1/agents/validate",
            json!({"content":definition.to_string()}),
        )
        .await
        .assert_success()
        .json_value();
    let paths: Vec<_> = validation["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"name") && paths.contains(&"max_iterations"),
        "{validation}"
    );
}

#[tokio::test]
async fn importing_configuration_preserves_existing_channel_activation() {
    let server = TestServer::in_memory().await;
    let definition = definition("channel-package");
    let body = json!({"content":definition.to_string()});
    server
        .post("/v1/agents/import", &body)
        .await
        .assert_success();
    let channels = server
        .get("/v1/agents/channel-package/channels")
        .await
        .assert_success()
        .json_value();
    let id = channels[0]["id"].as_str().unwrap();
    server
        .patch(
            &format!("/v1/agents/channel-package/channels/{id}"),
            json!({"enabled":true}),
        )
        .await
        .assert_success();
    server
        .post("/v1/agents/import?target=channel-package", &body)
        .await
        .assert_success();
    let channels = server
        .get("/v1/agents/channel-package/channels")
        .await
        .assert_success()
        .json_value();
    assert_eq!(channels[0]["enabled"], true);
}

#[tokio::test]
async fn channel_enablement_is_opt_in_and_requires_publication() {
    let server = TestServer::in_memory().await;
    let mut definition = definition("enabled-package");
    definition["channels"]["chat"]["enabled"] = json!(true);
    server
        .post(
            "/v1/agents/import",
            json!({"content":definition.to_string()}),
        )
        .await
        .assert_success();
    let channels = server
        .get("/v1/agents/enabled-package/channels")
        .await
        .assert_success()
        .json_value();
    assert_eq!(channels[0]["enabled"], true);
    assert_eq!(channels[0]["status"], "draft");
}

#[tokio::test]
async fn denied_live_channel_update_does_not_partially_import_agent() {
    use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
    use everruns_server::domains::agents::packages::{ImportAgent, PackageInput};
    use everruns_server::domains::common::{Command, Ctx};
    let server = TestServer::in_memory().await;
    let mut manifest = definition("live-package");
    manifest["channels"]["chat"]["enabled"] = json!(true);
    server
        .post("/v1/agents/import", json!({"content":manifest.to_string()}))
        .await
        .assert_success();
    let channels = server
        .get("/v1/agents/live-package/channels")
        .await
        .assert_success()
        .json_value();
    let id = channels[0]["id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/agents/live-package/channels/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    let member = Ctx::minimal(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        },
        server.db.clone(),
        server.encryption.clone(),
        std::sync::Arc::new(everruns_core::DefaultPermissionResolver),
    );
    manifest["instructions"] = json!("This must not be saved");
    manifest["channels"]["chat"]["config"] = json!({"rate_limit_per_minute":42});
    let error = ImportAgent::Package(PackageInput {
        content: manifest.to_string(),
        file: None,
        format: Some("json".into()),
        target: Some("live-package".into()),
    })
    .run(&member)
    .await
    .expect_err("live change requires publication permission");
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    let unchanged = server
        .get("/v1/agents/live-package")
        .await
        .assert_success()
        .json_value();
    assert_eq!(unchanged["system_prompt"], "Read the runbook.");
}
