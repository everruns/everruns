//! Publication, resource ownership and consumer authority are one boundary.
use crate::{agent_api_channel_test::call, test_harness::TestServer};
use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::ApplicationId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::domains::applications::ApplicationInput;
use serde_json::{Value, json};

async fn chat(server: &TestServer) -> String {
    let agent:Value=server.post("/v1/platform/agents",json!({"name":format!("app-agent-{}",uuid::Uuid::new_v4().simple()),"system_prompt":"Help"})).await.assert_success().json();
    let agent = agent["id"].as_str().unwrap();
    let channel: Value = server
        .post(
            &format!("/v1/platform/agents/{agent}/channels"),
            json!({"channel_type":"work_chat","channel_config":{}}),
        )
        .await
        .assert_success()
        .json();
    let channel = channel["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/platform/agents/{agent}/channels/{channel}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    channel
}
fn input(resources: Value) -> Value {
    json!({"name":"Content studio","slug":format!("studio-{}",uuid::Uuid::new_v4().simple()),"description":"A shared workspace","config":{"membership":"open","show_in_sidebar":true},"resources":resources})
}
async fn publish_fixture(server: &TestServer, input: Value) -> Value {
    let app: Value = server
        .post("/v1/platform/applications", input)
        .await
        .assert_success()
        .json();
    let app = server
        .db
        .set_application_status(
            DEFAULT_ORG_ID,
            app["id"].as_str().unwrap().parse().unwrap(),
            1,
            "published",
        )
        .await
        .unwrap();
    serde_json::to_value(app).unwrap()
}
async fn token(server: &TestServer, app: &Value, subject: &str) -> String {
    let id: ApplicationId = app["id"].as_str().unwrap().parse().unwrap();
    let user = server
        .db
        .resolve_runtime_identity(
            everruns_server::storage::runtime_identity::VerifiedRuntimeIdentity {
                org_id: DEFAULT_ORG_ID,
                provider: "oidc".into(),
                realm: format!("application:{id}"),
                subject: subject.into(),
                name: subject.into(),
                avatar_url: None,
                management_user_id: None,
            },
        )
        .await
        .unwrap();
    let binding = server
        .db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, user.id)
        .await
        .unwrap();
    everruns_server::auth::jwt::JwtService::new(everruns_server::auth::AuthConfig::default().jwt)
        .generate_application_token(
            DEFAULT_ORG_ID,
            user.id,
            binding[0].id,
            everruns_server::auth::jwt::ApplicationIdentityClaims {
                id,
                version: app["version"].as_i64().unwrap(),
                subject: subject.into(),
                email: Some(format!("{subject}@example.com")),
            },
        )
        .unwrap()
}
#[tokio::test]
async fn application_stable_names_versions_and_resource_ownership() {
    let server = TestServer::in_memory().await;
    let first = chat(&server).await;
    let mut body =
        input(json!([{"kind":"chat","slug":"assistant","name":"Assistant","resource_id":first}]));
    let app: Value = server
        .post("/v1/platform/applications", body.clone())
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let path = format!("/v1/platform/applications/{}", app["id"].as_str().unwrap());
    assert_eq!(
        app["resources"][0]["resource_id"], first,
        "public and internal IDs differ"
    );
    server
        .post("/v1/platform/applications", body.clone())
        .await
        .assert_status(StatusCode::CONFLICT);
    let mut duplicate = body.clone();
    duplicate["slug"] = json!(format!("other-{}", uuid::Uuid::new_v4().simple()));
    server
        .post("/v1/platform/applications", duplicate)
        .await
        .assert_status(StatusCode::CONFLICT);
    body["version"] = json!(0);
    server
        .patch(&path, body.clone())
        .await
        .assert_status(StatusCode::CONFLICT);
    body["version"] = json!(1);
    body["slug"] = json!("changed-name");
    server
        .patch(&path, body.clone())
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    body["slug"] = app["slug"].clone();
    body["resources"] = json!([]);
    server.patch(&path, body.clone()).await.assert_success();
    server
        .post(
            &format!("{path}/status"),
            json!({"version":2,"status":"archived"}),
        )
        .await
        .assert_success();
    body.as_object_mut().unwrap().remove("version");
    server
        .post("/v1/platform/applications", body)
        .await
        .assert_status(StatusCode::CONFLICT);
    let hidden: Value = server
        .get("/v1/platform/applications")
        .await
        .assert_success()
        .json();
    assert!(
        !hidden
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == app["id"])
    );
    let archived: Value = server
        .get("/v1/platform/applications?include_archived=true")
        .await
        .assert_success()
        .json();
    assert!(
        archived
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == app["id"])
    );
    let restored: Value = server
        .post(
            &format!("{path}/status"),
            json!({"version":3,"status":"draft"}),
        )
        .await
        .assert_success()
        .json();
    assert_eq!(restored["status"], "draft");
    assert_eq!(restored["slug"], app["slug"]);
    assert_eq!(restored["version"], 4);
}
#[tokio::test]
async fn application_credential_spans_its_chats_but_not_other_users_or_applications() {
    let server = TestServer::in_memory().await;
    let a = chat(&server).await;
    let b = chat(&server).await;
    let foreign = chat(&server).await;
    let app = publish_fixture(
        &server,
        input(json!([
            {"kind":"chat","slug":"writer","name":"Writer","resource_id":a},
            {"kind":"chat","slug":"reviewer","name":"Reviewer","resource_id":b}
        ])),
    )
    .await;
    let alice = token(&server, &app, "alice").await;
    let bob = token(&server, &app, "bob").await;
    let jwt = everruns_server::auth::jwt::JwtService::new(
        everruns_server::auth::AuthConfig::default().jwt,
    );
    let claims = jwt.validate_runtime_token(&alice).unwrap();
    let standalone = jwt
        .generate_runtime_token(
            DEFAULT_ORG_ID,
            claims.sub.parse().unwrap(),
            a.clone(),
            claims.binding_id,
        )
        .unwrap();
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{a}/sessions"),
        Some(&standalone),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    call(
        &server,
        Method::POST,
        &format!("/v1/channels/{a}/runtime-auth"),
        Some(&standalone),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    for channel in [&a, &b] {
        let base = format!("/v1/channels/{channel}/sessions");
        let session: Value = call(
            &server,
            Method::POST,
            &base,
            Some(&alice),
            Some(json!({"title":"Private"})),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
        call(
            &server,
            Method::GET,
            &format!("{base}/{}", session["id"].as_str().unwrap()),
            Some(&bob),
            None,
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    }
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{foreign}/sessions"),
        Some(&alice),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    let other = publish_fixture(
        &server,
        input(json!([{ "kind":"chat","slug":"other-chat","name":"Other","resource_id":foreign }])),
    )
    .await;
    call(
        &server,
        Method::GET,
        &format!(
            "/v1/applications/{}/manifest",
            other["id"].as_str().unwrap()
        ),
        Some(&alice),
        None,
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    let path = format!("/v1/applications/{}/manifest", app["id"].as_str().unwrap());
    call(&server, Method::GET, &path, Some(&alice), None)
        .await
        .assert_success();
    server
        .post(
            &format!(
                "/v1/platform/applications/{}/status",
                app["id"].as_str().unwrap()
            ),
            json!({"version":2,"status":"draft"}),
        )
        .await
        .assert_success();
    call(&server, Method::GET, &path, Some(&alice), None)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{a}/sessions"),
        Some(&alice),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn application_rooms_and_memory_enforce_resource_access_and_attribution() {
    let server = TestServer::in_memory().await;
    let room: Value = server
        .post(
            "/v1/platform/rooms",
            json!({"name":"Editorial room","topic":"Work together"}),
        )
        .await
        .assert_success()
        .json();
    let memory: Value = server
        .post(
            "/v1/platform/memories",
            json!({"name":"application-handbook"}),
        )
        .await
        .assert_success()
        .json();
    let mut body = input(json!([
        {"kind":"room","slug":"editorial","name":"Editorial","resource_id":room["id"]},
        {"kind":"memory","slug":"handbook","name":"Handbook","resource_id":memory["id"],"writable":false}
    ]));
    let mut app = publish_fixture(&server, body.clone()).await;
    assert_eq!(app["resources"][1]["resource_id"], memory["id"]);
    let alice = token(&server, &app, "alice").await;
    let base = format!("/v1/applications/{}", app["id"].as_str().unwrap());
    let message: Value = call(
        &server,
        Method::POST,
        &format!("{base}/rooms/editorial/messages"),
        Some(&alice),
        Some(json!({"text":"Hello"})),
    )
    .await
    .assert_success()
    .json();
    assert_eq!(message["author"]["display_name"], "alice");
    assert!(message.get("session_id").is_none());
    call(
        &server,
        Method::POST,
        &format!("{base}/rooms/editorial/messages"),
        Some(&alice),
        Some(json!({"text":"Spoof","as_member":"someone-else"})),
    )
    .await
    .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    call(
        &server,
        Method::PUT,
        &format!("{base}/memory/handbook"),
        Some(&alice),
        Some(json!({"path":"/notes.md","content":"private","create":true})),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    body["resources"][1]["writable"] = json!(true);
    body["version"] = json!(2);
    app = server
        .patch(
            &format!("/v1/platform/applications/{}", app["id"].as_str().unwrap()),
            body.clone(),
        )
        .await
        .assert_success()
        .json();
    call(
        &server,
        Method::GET,
        &format!("{base}/manifest"),
        Some(&alice),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    let alice = token(&server, &app, "alice").await;
    call(
        &server,
        Method::PUT,
        &format!("{base}/memory/handbook"),
        Some(&alice),
        Some(json!({"path":"/notes.md","content":"shared","create":true})),
    )
    .await
    .assert_success();
    let read: Value = call(
        &server,
        Method::GET,
        &format!("{base}/memory/handbook?path=/notes.md&file=true"),
        Some(&alice),
        None,
    )
    .await
    .assert_success()
    .json();
    assert_eq!(read["content"], "shared");
    call(
        &server,
        Method::PUT,
        &format!("{base}/memory/handbook"),
        Some(&alice),
        Some(json!({"path":"/../escape","content":"x","create":true})),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    body["config"]["membership"] = json!("invites");
    body["config"]["invites"] = json!(["bob@example.com"]);
    body["version"] = json!(3);
    app = server
        .patch(
            &format!("/v1/platform/applications/{}", app["id"].as_str().unwrap()),
            body,
        )
        .await
        .assert_success()
        .json();
    let revoked = token(&server, &app, "alice").await;
    call(
        &server,
        Method::GET,
        &format!("{base}/manifest"),
        Some(&revoked),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    let bob = token(&server, &app, "bob").await;
    call(
        &server,
        Method::GET,
        &format!("{base}/manifest"),
        Some(&bob),
        None,
    )
    .await
    .assert_success();
    let info: Value = call(
        &server,
        Method::GET,
        &format!("{base}/rooms/editorial"),
        Some(&bob),
        None,
    )
    .await
    .assert_success()
    .json();
    let member = info["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["display_name"] == "bob")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    server
        .delete(&format!(
            "/v1/platform/rooms/{}/members/{member}",
            room["id"].as_str().unwrap()
        ))
        .await
        .assert_success();
    call(
        &server,
        Method::GET,
        &format!("{base}/rooms/editorial"),
        Some(&bob),
        None,
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
}
#[test]
fn application_configuration_rejects_unsafe_names_and_unimplemented_automation() {
    for value in [
        json!({"name":"Studio","slug":"content-studio"}),
        json!({"name":"Studio","slug":"content-studio","config":{}}),
    ] {
        let request: ApplicationInput = serde_json::from_value(value).unwrap();
        assert_eq!(
            request.config.membership,
            everruns_server::domains::agent_channels::record::work_chat::WorkChatMembership::OrgUsers
        );
    }
    assert_eq!(
        everruns_server::domains::applications::ApplicationConfig::default().membership,
        everruns_server::domains::agent_channels::record::work_chat::WorkChatMembership::OrgUsers
    );
    for slug in [
        "login",
        "api",
        "-bad",
        "example.com",
        "ab",
        "ab--",
        "abcdefghij",
    ] {
        let mut value = input(json!([]));
        value["slug"] = json!(slug);
        let mut request: ApplicationInput = serde_json::from_value(value).unwrap();
        assert!(request.prepare().is_err(), "{slug}");
    }
    let mut value = input(json!([]));
    value["config"]["automations"] = json!(true);
    let mut request: ApplicationInput = serde_json::from_value(value).unwrap();
    assert!(request.prepare().is_err());
}

#[tokio::test]
async fn application_oidc_exchange_isolated_identity_verified_email_and_no_self_renewal() {
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    let server = TestServer::in_memory().await;
    let issuer = "https://application-identity.example.com";
    server
        .application_verifier
        .prime_oidc(
            issuer,
            &format!("{issuer}/jwks"),
            serde_json::from_str(include_str!("fixtures/application-oidc-test-jwks.json")).unwrap(),
        )
        .await;
    // Publicly committed fixture key; never a deployment credential.
    let key = EncodingKey::from_rsa_pem(include_bytes!("fixtures/application-oidc-test-key.pem"))
        .unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("application-test".into());
    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({"iss":issuer,"sub":"alice","aud":"application-tests","iat":now,"exp":now+900,"email":"alice@example.com","email_verified":true});
    let identity = jsonwebtoken::encode(&header, &claims, &key).unwrap();
    let room: Value = server
        .post("/v1/platform/rooms", json!({"name":"Identity room"}))
        .await
        .assert_success()
        .json();
    let mut body =
        input(json!([{"kind":"room","slug":"general","name":"General","resource_id":room["id"]}]));
    body["config"]["membership"] = json!("invites");
    body["config"]["invites"] = json!(["alice@example.com"]);
    let app = publish_fixture(&server, body).await;
    let path = format!(
        "/v1/applications/{}/runtime-auth",
        app["id"].as_str().unwrap()
    );
    let first: Value = call(
        &server,
        Method::POST,
        &path,
        Some(&identity),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    let again: Value = call(
        &server,
        Method::POST,
        &path,
        Some(&identity),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    assert_eq!(first["virtual_user_id"], again["virtual_user_id"]);
    call(
        &server,
        Method::POST,
        &path,
        first["access_token"].as_str(),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    claims["email_verified"] = json!(false);
    let unverified = jsonwebtoken::encode(&header, &claims, &key).unwrap();
    call(
        &server,
        Method::POST,
        &path,
        Some(&unverified),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    claims["email_verified"] = json!(true);
    claims["aud"] = json!("other-client");
    let wrong = jsonwebtoken::encode(&header, &claims, &key).unwrap();
    call(&server, Method::POST, &path, Some(&wrong), Some(json!({})))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    let room: Value = server
        .post("/v1/platform/rooms", json!({"name":"Second identity room"}))
        .await
        .assert_success()
        .json();
    let other = publish_fixture(
        &server,
        input(json!([{"kind":"room","slug":"general","name":"General","resource_id":room["id"]}])),
    )
    .await;
    let second: Value = call(
        &server,
        Method::POST,
        &format!(
            "/v1/applications/{}/runtime-auth",
            other["id"].as_str().unwrap()
        ),
        Some(&identity),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    assert_ne!(
        first["virtual_user_id"], second["virtual_user_id"],
        "external audiences have separate private identity and memory"
    );
    let member = server
        .db
        .create_user(everruns_server::storage::CreateUserRow {
            email: "alice@example.com".into(),
            name: "Alice".into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: Some("alice".into()),
        })
        .await
        .unwrap();
    server
        .db
        .ensure_membership(member.id, DEFAULT_ORG_ID, "member")
        .await
        .unwrap();
    let expected = server
        .db
        .default_virtual_user(DEFAULT_ORG_ID, member.id)
        .await
        .unwrap();
    let mut org_input = json!({"name":app["name"],"slug":app["slug"],"description":app["description"],"config":app["config"],"resources":app["resources"],"version":app["version"]});
    org_input["config"]["membership"] = json!("org_users");
    server
        .patch(
            &format!("/v1/platform/applications/{}", app["id"].as_str().unwrap()),
            org_input,
        )
        .await
        .assert_success();
    let shared: Value = call(
        &server,
        Method::POST,
        &path,
        Some(&identity),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    assert_eq!(
        shared["virtual_user_id"],
        expected.id.to_string(),
        "organization mode reuses the member's account"
    );
    let member_claims = everruns_server::auth::jwt::JwtService::new(
        everruns_server::auth::AuthConfig::default().jwt,
    )
    .validate_runtime_token(shared["access_token"].as_str().unwrap())
    .unwrap();
    assert!(member_claims.application.is_some());
}

#[tokio::test]
async fn application_resource_creation_rolls_back_and_memory_mounts_follow_current_grants() {
    let server = TestServer::in_memory().await;
    let application: Value = server
        .post("/v1/platform/applications", input(json!([])))
        .await
        .assert_success()
        .json();
    let id = application["id"].as_str().unwrap();
    let agent:Value=server.post("/v1/platform/agents",json!({"name":format!("app-atomic-{}",uuid::Uuid::new_v4().simple()),"system_prompt":"Help"})).await.assert_success().json();
    let agent = agent["id"].as_str().unwrap();
    let path = format!("/v1/platform/applications/{id}/resources");
    server.post(&path,json!({"version":1,"kind":"chat","name":"Writer","slug":"invalid/path","agent_id":agent})).await.assert_status(StatusCode::BAD_REQUEST);
    let channels: Value = server
        .get(&format!("/v1/platform/agents/{agent}/channels"))
        .await
        .assert_success()
        .json();
    assert!(
        channels.as_array().unwrap().is_empty(),
        "a rejected attach cannot leave an independently published chat"
    );
    let app: Value = server
        .post(
            &path,
            json!({"version":1,"kind":"chat","name":"Writer","slug":"writer","agent_id":agent}),
        )
        .await
        .assert_success()
        .json();
    let channel = app["resources"][0]["resource_id"]
        .as_str()
        .unwrap()
        .to_string();
    let saved_chat: Value = server
        .get(&format!("/v1/platform/agents/{agent}/channels/{channel}"))
        .await
        .assert_success()
        .json();
    assert_eq!(
        saved_chat["channel_config"]["membership"], "org_users",
        "detaching a new Chat must not open its standalone audience"
    );
    let app: Value = server
        .post(
            &path,
            json!({"version":2,"kind":"memory","name":"Handbook","slug":"handbook"}),
        )
        .await
        .assert_success()
        .json();
    // A resource label ("Handbook") is not a valid memory store name, so the
    // created store is named for its path: /memory/handbook.
    let created_memory: Value = server
        .get(&format!(
            "/v1/platform/memories/{}",
            app["resources"][1]["resource_id"].as_str().unwrap()
        ))
        .await
        .assert_success()
        .json();
    assert_eq!(created_memory["name"], "handbook");
    let app = server
        .db
        .set_application_status(
            DEFAULT_ORG_ID,
            id.parse().unwrap(),
            app["version"].as_i64().unwrap(),
            "published",
        )
        .await
        .unwrap();
    let mut app = serde_json::to_value(app).unwrap();
    let credential = token(&server, &app, "alice").await;
    assert!(
        everruns_server::auth::jwt::JwtService::new(
            everruns_server::auth::AuthConfig::default().jwt
        )
        .validate_access_token(&credential)
        .is_err()
    );
    let session: Value = call(
        &server,
        Method::POST,
        &format!("/v1/channels/{channel}/sessions"),
        Some(&credential),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    let session: everruns_contracts::typed_id::SessionId =
        session["id"].as_str().unwrap().parse().unwrap();
    let router = everruns_server::domains::session_files::memory_mounts::MemoryMountRouter::new(
        server.db.clone(),
    );
    let (mount, inner) = router
        .route(session.uuid(), "/application/memory/handbook/notes.md")
        .await
        .expect("Application memory is a live agent mount");
    assert!(mount.readonly);
    assert_eq!(inner, "/notes.md");
    let ctx = everruns_server::domains::common::Ctx::new(
        everruns_core::Caller::internal(DEFAULT_ORG_ID),
        server.db.clone(),
        std::sync::Arc::new(everruns_server::services::CapabilityService::new(
            server.db.clone(),
            None,
        )),
        None,
        std::sync::Arc::new(everruns_core::DefaultPermissionResolver),
    );
    let write = json!({"session_id":session,"op":"write","path":"/application/memory/handbook/notes.md","content":"Shared knowledge"});
    let answer = everruns_server::domains::data_plane::memory::answer(&ctx, &write).await;
    assert!(
        answer.get("error").is_some(),
        "read-only applies on v2 too: {answer}"
    );

    app["resources"][1]["writable"] = json!(true);
    let body = json!({"name":app["name"],"slug":app["slug"],"description":app["description"],"config":app["config"],"resources":app["resources"],"version":app["version"]});
    let updated: Value = server
        .patch(&format!("/v1/platform/applications/{id}"), body)
        .await
        .assert_success()
        .json();
    let (mount, _) = router
        .route(session.uuid(), "/application/memory/handbook/notes.md")
        .await
        .unwrap();
    assert!(
        !mount.readonly,
        "an existing session sees current grants without a stale cache"
    );
    let answer = everruns_server::domains::data_plane::memory::answer(&ctx, &write).await;
    assert_eq!(answer["ok"]["path"], write["path"], "{answer}");
    let listing = everruns_server::domains::data_plane::memory::answer(
        &ctx,
        &json!({"session_id":session,"op":"list","path":"/application/memory"}),
    )
    .await;
    assert_eq!(
        listing["ok"][0]["path"], "/application/memory/handbook",
        "{listing}"
    );
    server
        .post(
            &format!("/v1/platform/applications/{id}/status"),
            json!({"version":updated["version"],"status":"draft"}),
        )
        .await
        .assert_success();
    assert!(
        router
            .route(session.uuid(), "/application/memory/handbook/notes.md")
            .await
            .is_none()
    );
    let denied = everruns_server::domains::data_plane::memory::answer(&ctx, &write).await;
    assert!(
        denied["error"]
            .as_str()
            .unwrap()
            .contains("not in this session's Memory")
    );
}

#[tokio::test]
async fn application_rejects_foreign_organization_resources_atomically() {
    let server = TestServer::in_memory().await;
    let room: Value = server
        .post("/v1/platform/rooms", json!({"name":"Private source room"}))
        .await
        .assert_success()
        .json();
    let other = server
        .db
        .create_organization(everruns_server::storage::CreateOrganizationRow {
            public_id: format!("org_{}", uuid::Uuid::new_v4().simple()),
            name: "Other tenant".into(),
            created_by: None,
        })
        .await
        .unwrap();
    let mut request: ApplicationInput = serde_json::from_value(input(
        json!([{"kind":"room","slug":"private","name":"Private","resource_id":room["id"]}]),
    ))
    .unwrap();
    request.prepare().unwrap();
    let id = ApplicationId::new();
    assert!(
        server
            .db
            .save_application(other.org_id, id, request, None)
            .await
            .is_err()
    );
    assert!(
        server
            .db
            .application(other.org_id, id)
            .await
            .unwrap()
            .is_none(),
        "failed binding rolls back the Application too"
    );
    assert!(
        server
            .db
            .list_applications(other.org_id)
            .await
            .unwrap()
            .is_empty()
    );
}
