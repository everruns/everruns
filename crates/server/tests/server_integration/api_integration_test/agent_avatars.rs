//! API integration tests: agent avatars (upload, public presets, Agent Card).

use crate::test_harness::TestServer;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use std::io::Cursor;

const BOUNDARY: &str = "----everruns-avatar-upload";

fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let image =
        image::RgbaImage::from_pixel(width, height, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut buffer = Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, image::ImageFormat::Png)
        .unwrap();
    buffer.into_inner()
}

fn multipart(content_type: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"avatar\"\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload(
    server: &TestServer,
    agent_id: &str,
    content_type: &str,
    data: &[u8],
) -> crate::test_harness::TestResponse {
    let header = format!("multipart/form-data; boundary={BOUNDARY}");
    server
        .request_raw(
            Method::PUT,
            &format!("/v1/agents/{agent_id}/avatar"),
            vec![("content-type", header.as_str())],
            multipart(content_type, data),
        )
        .await
}

async fn create_agent(server: &TestServer, name: &str) -> Value {
    server
        .post(
            "/v1/agents",
            json!({ "name": name, "system_prompt": "You help." }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

#[tokio::test]
async fn uploaded_avatar_is_served_as_immutable_square_and_circle_presets() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "avatar-agent").await;
    let agent_id = agent["id"].as_str().unwrap();
    assert!(
        agent.get("avatar").is_none(),
        "no avatar until one is uploaded"
    );

    let avatar: Value = upload(
        &server,
        agent_id,
        "image/png",
        &png(300, 200, [200, 10, 10]),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let avatar_id = avatar["id"].as_str().unwrap();
    assert!(avatar_id.starts_with("avatar_"), "{avatar_id}");
    assert_eq!(
        avatar["url"],
        format!("/v1/avatars/{avatar_id}/square-256.png")
    );
    assert_eq!(
        avatar["circle_url"],
        format!("/v1/avatars/{avatar_id}/circle-256.png")
    );
    assert_eq!(avatar["sizes"], json!([32, 64, 128, 256, 512]));

    // The agent carries it, both alone and in the list.
    let fetched: Value = server
        .get(&format!("/v1/agents/{agent_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched["avatar"], avatar);
    let listed: Value = server
        .get("/v1/agents")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let entry = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == agent["id"])
        .unwrap();
    assert_eq!(entry["avatar"]["id"], avatar["id"]);

    for (variant, edge) in [
        ("square-32.png", 32),
        ("circle-128.png", 128),
        ("source.png", 200),
    ] {
        let response = server
            .request_raw(
                Method::GET,
                &format!("/v1/avatars/{avatar_id}/{variant}"),
                vec![],
                vec![],
            )
            .await
            .assert_status(StatusCode::OK);
        let headers = response.headers();
        assert_eq!(headers["content-type"], "image/png");
        assert_eq!(
            headers["cache-control"],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(headers["access-control-allow-origin"], "*");
        let image = image::load_from_memory(response.bytes()).unwrap();
        assert_eq!((image.width(), image.height()), (edge, edge), "{variant}");
    }
    let circle = server
        .request_raw(
            Method::GET,
            &format!("/v1/avatars/{avatar_id}/circle-64.png"),
            vec![],
            vec![],
        )
        .await;
    let circle = image::load_from_memory(circle.bytes()).unwrap().to_rgba8();
    assert_eq!(
        circle.get_pixel(0, 0).0[3],
        0,
        "circle corners are transparent"
    );

    // Unknown variants and ids are 404s.
    for path in [
        format!("/v1/avatars/{avatar_id}/square-100.png"),
        format!("/v1/avatars/{avatar_id}/square-64.jpg"),
        "/v1/avatars/avatar_01933b5a000070008000000000000001/square-64.png".to_string(),
        "/v1/avatars/not-an-id/square-64.png".to_string(),
    ] {
        server
            .request_raw(Method::GET, &path, vec![], vec![])
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    // A replacement gets a new id and retires the old URLs.
    let replaced: Value = upload(
        &server,
        agent_id,
        "image/png",
        &png(128, 128, [10, 10, 200]),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_ne!(replaced["id"], avatar["id"]);
    server
        .request_raw(
            Method::GET,
            &format!("/v1/avatars/{avatar_id}/square-64.png"),
            vec![],
            vec![],
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Editing the agent keeps the avatar.
    let patched: Value = server
        .patch(
            &format!("/v1/agents/{agent_id}"),
            json!({ "description": "Still me" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(patched["avatar"]["id"], replaced["id"]);

    server
        .delete(&format!("/v1/agents/{agent_id}/avatar"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let fetched: Value = server
        .get(&format!("/v1/agents/{agent_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(fetched.get("avatar").is_none());
}

#[tokio::test]
async fn invalid_avatar_uploads_are_rejected() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "avatar-rejects").await;
    let agent_id = agent["id"].as_str().unwrap();

    for (content_type, data, needle) in [
        (
            "image/svg+xml",
            b"<svg/>".to_vec(),
            "PNG, JPEG, GIF or WebP",
        ),
        ("image/png", b"not an image".to_vec(), "could not be read"),
        ("image/jpeg", png(100, 100, [0, 0, 0]), "could not be read"),
        ("image/png", png(40, 40, [0, 0, 0]), "at least 64x64"),
    ] {
        let response = upload(&server, agent_id, content_type, &data)
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        assert!(response.text().contains(needle), "{}", response.text());
    }

    upload(
        &server,
        "agent_01933b5a000070008000000000000099",
        "image/png",
        &png(100, 100, [0, 0, 0]),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a2a_agent_card_advertises_the_avatar_as_icon_url() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "avatar-a2a").await;
    let agent_id = agent["id"].as_str().unwrap();
    let app = server
        .seed_app_channel(
            "avatar-a2a",
            agent_id,
            "a2a",
            json!({
                "session_mode": "shared_session",
                "message": "{{a2a.text}}",
                "api_key_hash": "0".repeat(64),
                "api_key_prefix": "evr_app_0000",
            }),
        )
        .await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    server.set_app_channels_live(app_id, true).await;
    let card_path = format!("/v1/channels/{channel_id}/a2a/.well-known/agent-card.json");

    let card: Value = server
        .request_raw(
            Method::GET,
            &card_path,
            vec![("host", "agents.example.com")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(card.get("iconUrl").is_none());

    let avatar: Value = upload(&server, agent_id, "image/png", &png(128, 128, [0, 120, 0]))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let card: Value = server
        .request_raw(
            Method::GET,
            &card_path,
            vec![("host", "agents.example.com")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let icon = card["iconUrl"].as_str().unwrap();
    assert!(icon.starts_with("https://agents.example.com/"), "{icon}");
    assert!(
        icon.ends_with(&format!(
            "/v1/avatars/{}/square-256.png",
            avatar["id"].as_str().unwrap()
        )),
        "{icon}"
    );
}

#[tokio::test]
async fn curated_preset_selection_persists_and_shares_upload_pipeline() {
    let server = TestServer::in_memory().await;
    let catalog: Vec<Value> = server
        .get("/v1/avatar-presets")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(catalog.len(), 25);
    let agent = create_agent(&server, "preset-avatar").await;
    let id = agent["id"].as_str().unwrap();
    let path = format!("/v1/agents/{id}/avatar/preset");
    for bad in [
        "unknown",
        "../watchers-bracket",
        "https://example.com/a.png",
        "WATCHERS-BRACKET",
    ] {
        server
            .put(&path, json!({"preset_id": bad}))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
    let preset: Value = server
        .put(&path, json!({"preset_id": "familiars-patch"}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let current: Value = server.get(&path).await.assert_status(StatusCode::OK).json();
    assert_eq!(current["preset_id"], "familiars-patch");
    let fetched: Value = server
        .get(&format!("/v1/agents/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched["avatar"], preset);
    for shape in ["square", "circle"] {
        let stored = server
            .request_raw(
                Method::GET,
                &format!(
                    "/v1/avatars/{}/{shape}-64.png",
                    preset["id"].as_str().unwrap()
                ),
                vec![],
                vec![],
            )
            .await
            .assert_status(StatusCode::OK);
        let preview = server
            .request_raw(
                Method::GET,
                &format!("/v1/avatar-presets/familiars-patch/{shape}-64.png"),
                vec![],
                vec![],
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(stored.bytes(), preview.bytes());
    }
    let custom: Value = upload(&server, id, "image/png", &png(128, 128, [30, 40, 50]))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_ne!(custom["id"], preset["id"]);
    let current: Value = server.get(&path).await.json();
    assert!(current["preset_id"].is_null());
    server
        .request_raw(Method::GET, preset["url"].as_str().unwrap(), vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .put(&path, json!({"preset_id": "bloom-mint"}))
        .await
        .assert_status(StatusCode::OK);
    server
        .delete(&format!("/v1/agents/{id}/avatar"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let current: Value = server.get(&path).await.json();
    assert!(current["preset_id"].is_null());
    server
        .put(
            "/v1/agents/agent_01933b5a000070008000000000000099/avatar/preset",
            json!({"preset_id":"bloom-mint"}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    for path in [
        "/v1/avatar-presets/missing/square-64.png",
        "/v1/avatar-presets/bloom-mint/oval-64.png",
    ] {
        server
            .request_raw(Method::GET, path, vec![], vec![])
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
}

mod authorization {
    use super::*;
    use async_trait::async_trait;
    use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId};
    use everruns_core::{Caller, Permission, PermissionResolver};
    use everruns_worker::AgentRunner;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct IdleRunner;

    #[async_trait]
    impl AgentRunner for IdleRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: SessionId,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _input_message_id: MessageId,
            _request_id: Option<String>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn resume_after_tool_results(
            &self,
            _session_id: SessionId,
            _resolution_id: uuid::Uuid,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn cancel_run(&self, _run_id: SessionId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_running(&self, _run_id: SessionId) -> bool {
            false
        }

        async fn active_count(&self) -> usize {
            0
        }
    }

    #[derive(Default)]
    struct DenyAvatarWrites {
        armed: AtomicBool,
    }
    impl PermissionResolver for DenyAvatarWrites {
        fn has_permission(&self, _: &Caller, permission: &Permission) -> bool {
            !(self.armed.load(Ordering::SeqCst) && permission == &Permission::OrgAgentsManage)
        }
        fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
            Permission::ALL
                .iter()
                .copied()
                .filter(|p| self.has_permission(caller, p))
                .collect()
        }
    }
    #[tokio::test]
    async fn preset_mutation_honors_custom_permission_resolver() {
        let resolver = Arc::new(DenyAvatarWrites::default());
        let server = TestServer::in_memory_with_runner_and_permission_resolver(
            Arc::new(IdleRunner),
            resolver.clone(),
        )
        .await;
        let agent = create_agent(&server, "avatar-authz").await;
        let id = agent["id"].as_str().unwrap();
        resolver.armed.store(true, Ordering::SeqCst);
        server
            .put(
                &format!("/v1/agents/{id}/avatar/preset"),
                json!({"preset_id": "bloom-mint"}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        server
            .get(&format!("/v1/agents/{id}/avatar/preset"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        upload(&server, id, "image/png", &png(128, 128, [0, 0, 0]))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        server
            .delete(&format!("/v1/agents/{id}/avatar"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
}
