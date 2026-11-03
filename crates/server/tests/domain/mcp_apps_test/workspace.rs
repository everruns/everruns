use super::*;
use base64::Engine as _;

fn api_body(response: &Value) -> Value {
    let result = structured(response);
    assert_eq!(result["status"], 200, "{result}");
    serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(result["body"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn shared_workspace_reads_the_same_resources_and_opens_each_entity() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, &unique("workspace")).await;
    let session = start_session(&server, &agent, "workspace trace").await;
    let collection: Value = server
        .post(
            "/v1/knowledge",
            json!({"name":unique("workspace knowledge"),"indexing":"off"}),
        )
        .await
        .assert_success()
        .json();
    let capabilities: Value = server.get("/v1/capabilities").await.assert_success().json();
    let capability = capabilities["data"][0]["id"].as_str().unwrap();
    for (tool, field, id, kind) in [
        (
            "harness_get_card",
            "harness_id",
            server.seed_base_harness_id.to_string(),
            "harness",
        ),
        (
            "capability_get_card",
            "capability_id",
            capability.to_string(),
            "capability",
        ),
        (
            "knowledge_get_card",
            "knowledge_id",
            collection["id"].as_str().unwrap().to_string(),
            "knowledge",
        ),
        ("session_open", "session_id", session.clone(), "session"),
    ] {
        let result = structured(&call(&server, tool, json!({field:id})).await);
        assert_eq!(result["view"]["kind"], kind);
        assert_eq!(result["view"]["id"], id);
        assert!(
            result["organization_id"]
                .as_str()
                .unwrap()
                .starts_with("org_")
        );
    }
    let result = call(
        &server,
        "app_read",
        json!({"path":format!("/v1/agents/{agent}")}),
    )
    .await;
    assert_eq!(api_body(&result)["id"], agent);
    for path in [
        format!("/v1/sessions/{session}/trace"),
        format!("/v1/sessions/{session}/events?after_sequence=0&limit=20"),
        format!("/v1/sessions/{session}/storage/keys"),
        format!("/v1/sessions/{session}/storage/secrets"),
    ] {
        let result = call(&server, "app_read", json!({"path":path})).await;
        api_body(&result);
    }
    let recording: Value = server
        .get(&format!("/v1/sessions/{session}"))
        .await
        .assert_success()
        .json();
    let file_path = format!(
        "/v1/workspaces/{}/fs/workspace/bridge.txt",
        recording["workspace_id"].as_str().unwrap()
    );
    server
        .post(
            &file_path,
            json!({"content":"Bridge file bytes", "encoding":"text"}),
        )
        .await
        .assert_success();
    let file = api_body(&call(&server, "app_read", json!({"path":file_path})).await);
    assert_eq!(file["content"], "Bridge file bytes");
    let download_path = format!(
        "/v1/workspaces/{}/fs/_/download/workspace/bridge.txt",
        recording["workspace_id"].as_str().unwrap()
    );
    let downloaded = structured(&call(&server, "app_read", json!({"path":download_path})).await);
    assert_eq!(downloaded["status"], 200);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(downloaded["body"].as_str().unwrap())
            .unwrap(),
        b"Bridge file bytes"
    );
    let image = server
        .db
        .create_image(
            DEFAULT_ORG_ID,
            everruns_server::storage::CreateImageRow {
                org_id: DEFAULT_ORG_ID,
                filename: "proof.png".into(),
                content_type: "image/png".into(),
                size_bytes: 3,
                data: vec![0, 255, 128],
                thumbnail_data: Some(vec![0, 255, 128]),
                thumbnail_content_type: Some("image/png".into()),
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    for path in [
        format!("/v1/images/{}", image.id),
        format!("/v1/images/{}/thumbnail", image.id),
    ] {
        let result = structured(&call(&server, "app_read", json!({"path":path})).await);
        assert_eq!(result["status"], 200, "{result}");
        assert_eq!(result["headers"]["content-type"], "image/png");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(result["body"].as_str().unwrap())
                .unwrap(),
            vec![0, 255, 128]
        );
        assert!(is_error(
            &call(&server, "app_write", json!({"path":path,"method":"DELETE"})).await
        ));
    }
    assert!(is_error(
        &call(
            &server,
            "app_write",
            json!({"path":file_path,"method":"DELETE"})
        )
        .await
    ));
    let changed = call(&server, "app_write", json!({"path":format!("/v1/agents/{agent}"),"method":"PATCH","body":{"description":"Changed through the workspace"}})).await;
    assert_eq!(
        api_body(&changed)["description"],
        "Changed through the workspace"
    );
    for path in [
        "https://example.com/v1/agents",
        "/v1/auth/me",
        "/v1/commands/get_agent",
        "/v1/agents/../auth/me",
        "/v1/sessions/example/storage/secrets/name",
    ] {
        assert!(
            is_error(&call(&server, "app_read", json!({"path":path})).await),
            "{path}"
        );
    }
}

#[tokio::test]
async fn workspace_keeps_pat_scope_and_organization_boundaries() {
    let server = TestServer::in_memory_authenticated().await;
    let default_org = server
        .db
        .get_organization(DEFAULT_ORG_ID)
        .await
        .unwrap()
        .unwrap();
    let full =
        crate::agent_api_spend_members_test::member_token_with_scopes(&server, true, &["*"]).await;
    let full_authorization = format!("Bearer {full}");
    let full_call = |name, args| {
        rpc(
            &server,
            "tools/call",
            json!({"name":name,"arguments":args}),
            LATEST,
            vec![
                ("authorization", full_authorization.as_str()),
                ("x-org-id", default_org.public_id.as_str()),
            ],
        )
    };
    let created = full_call("execute",json!({"commands": format!("create_agent --name '{}' --system_prompt 'Test'", unique("workspace-scope"))})).await;
    assert!(!is_error(&created), "{}", text(&created));
    let agent: Value = serde_json::from_str(&text(&created)).unwrap();
    let agent = agent["id"].as_str().unwrap();
    let token =
        crate::agent_api_spend_members_test::member_token_with_scopes(&server, true, &["read"])
            .await;
    let authorization = format!("Bearer {token}");
    let invoke = |name, args| {
        rpc(
            &server,
            "tools/call",
            json!({"name":name,"arguments":args}),
            LATEST,
            vec![
                ("authorization", authorization.as_str()),
                ("x-org-id", default_org.public_id.as_str()),
            ],
        )
    };
    let context = structured(
        &invoke(
            "app_bootstrap",
            json!({"organization_id":default_org.public_id}),
        )
        .await,
    );
    assert_eq!(context["read_only"], true);
    let result = invoke(
        "app_read",
        json!({"path":format!("/v1/agents/{agent}"),"organization_id":default_org.public_id}),
    )
    .await;
    assert_eq!(api_body(&result)["id"], agent);
    assert!(is_error(&invoke("app_write",json!({"path":format!("/v1/agents/{agent}"),"method":"PATCH","body":{"description":"forbidden"}})).await));
    let other: Value = server
        .request_raw(
            Method::POST,
            "/v1/orgs",
            vec![
                ("content-type", "application/json"),
                ("authorization", full_authorization.as_str()),
                (
                    "x-org-id",
                    context["organization"]["public_id"].as_str().unwrap(),
                ),
            ],
            serde_json::to_vec(&json!({"name":unique("Workspace other")})).unwrap(),
        )
        .await
        .assert_success()
        .json();
    assert!(is_error(
        &invoke("app_bootstrap", json!({"organization_id":other["id"]})).await
    ));
    assert!(is_error(
        &full_call("app_bootstrap", json!({"organization_id":other["id"]})).await
    ));
    let result = rpc(
        &server, "tools/call", json!({"name":"app_read","arguments":{"path":format!("/v1/agents/{agent}"),"organization_id":other["id"]}}), LATEST,
        vec![("authorization",full_authorization.as_str()),("x-org-id",other["id"].as_str().unwrap())],
    ).await;
    assert_eq!(structured(&result)["status"], 404);
}
