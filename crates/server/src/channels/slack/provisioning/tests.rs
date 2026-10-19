use super::manager_app::*;
use super::*;
use base64::Engine;
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .expect("test encryption"),
    )
}

pub(crate) fn provisioner(server: &MockServer) -> (SlackApiProvisioner, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::test_database());
    (
        SlackApiProvisioner::with_api_base(db.clone(), encryption(), &server.uri())
            .expect("provisioner"),
        db,
    )
}

pub(crate) fn stored(
    org_id: i64,
    team_id: &str,
    expires_in: chrono::Duration,
) -> UpsertOrgSlackConnection {
    let encryption = encryption();
    UpsertOrgSlackConnection {
        org_id,
        team_id: team_id.to_string(),
        team_name: None,
        access_token_encrypted: encryption.encrypt_string("access").unwrap(),
        refresh_token_encrypted: encryption.encrypt_string("refresh").unwrap(),
        access_token_expires_at: chrono::Utc::now() + expires_in,
        grant_kind: SlackGrantKind::ConfigToken,
    }
}

async fn mount_rotate(server: &MockServer, team_id: &str) {
    Mock::given(method("POST"))
        .and(path("/tooling.tokens.rotate"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "token": "new-access",
            "refresh_token": "new-refresh",
            "team_id": team_id,
            "user_id": "U1",
            "exp": chrono::Utc::now().timestamp() + 3600
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn permission_update_preserves_exported_manifest_and_unions_scopes() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();
    let manifest = serde_json::json!({
        "display_information":{"name":"Customized bot"},
        "settings":{"event_subscriptions":{"request_url":"https://example.test/events"}},
        "oauth_config":{"redirect_urls":["https://example.test/callback"],"scopes":{"bot":["chat:write","custom:read"]}}
    });
    Mock::given(method("POST"))
        .and(path("/apps.manifest.export"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"manifest":manifest})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apps.manifest.update"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    provisioner
        .update_permissions(1, Some("T1"), "A1", &["chat:write", "reactions:write"])
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(body["app_id"], "A1");
    let updated: serde_json::Value =
        serde_json::from_str(body["manifest"].as_str().unwrap()).unwrap();
    assert_eq!(
        updated["display_information"],
        manifest["display_information"]
    );
    assert_eq!(updated["settings"], manifest["settings"]);
    assert_eq!(
        updated["oauth_config"]["redirect_urls"],
        manifest["oauth_config"]["redirect_urls"]
    );
    assert_eq!(
        updated["oauth_config"]["scopes"]["bot"],
        serde_json::json!(["chat:write", "custom:read", "reactions:write"])
    );
}

#[tokio::test]
async fn app_icon_is_uploaded_as_a_multipart_file_with_the_workspace_token() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/apps.icon.set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    provisioner
        .set_app_icon(1, Some("T1"), "A1", b"\x89PNG-bytes".to_vec())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
    assert!(request.headers.get("authorization").is_some());
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("name=\"app_id\""));
    assert!(body.contains("A1"));
    assert!(body.contains("name=\"file\"; filename=\"avatar.png\""));
    assert!(body.contains("PNG-bytes"));
}

#[tokio::test]
async fn app_icon_can_be_set_from_a_public_url() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/apps.icon.set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    let icon_url = "https://api.example.com/v1/avatars/avatar_1/square-512.png";
    provisioner
        .set_app_icon_url(1, Some("T1"), "A1", icon_url)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("application/json"),
        "{content_type}"
    );
    assert!(request.headers.get("authorization").is_some());
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["app_id"], "A1");
    assert_eq!(body["url"], icon_url);
}

#[tokio::test]
async fn malformed_export_fails_without_mutating_manifest() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/apps.manifest.export"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok":true,"manifest":{"oauth_config":"invalid"}}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        provisioner
            .update_permissions(1, Some("T1"), "A1", &["reactions:write"])
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn connecting_records_the_workspace_and_never_stores_plaintext() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tooling.tokens.rotate"))
        // Slack ignores a JSON body here and answers `invalid_arguments`.
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string("refresh_token=input-refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "token": "new-access",
            "refresh_token": "new-refresh",
            "team_id": "T0123",
            "user_id": "U1",
            "exp": chrono::Utc::now().timestamp() + 3600
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true, "team": "Acme", "team_id": "T0123"})),
        )
        .mount(&server)
        .await;
    let (provisioner, _db) = provisioner(&server);

    let row = provisioner.connect(41, "input-refresh").await.unwrap();

    assert_eq!(row.team_id.as_deref(), Some("T0123"));
    assert_eq!(row.team_name.as_deref(), Some("Acme"));
    assert!(
        !row.refresh_token_encrypted
            .unwrap()
            .windows(b"new-refresh".len())
            .any(|part| part == b"new-refresh")
    );
}

#[tokio::test]
async fn a_workspace_name_slack_will_not_reveal_does_not_fail_the_connection() {
    let server = MockServer::start().await;
    mount_rotate(&server, "T0123").await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": false, "error": "not_allowed_token_type"})),
        )
        .mount(&server)
        .await;
    let (provisioner, _db) = provisioner(&server);

    let row = provisioner.connect(41, "input-refresh").await.unwrap();

    assert_eq!(row.team_id.as_deref(), Some("T0123"));
    assert_eq!(row.team_name, None);
}

#[tokio::test]
async fn invalid_refresh_token_marks_the_claimed_generation_for_reconnect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tooling.tokens.rotate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": false, "error": "invalid_refresh_token"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = provisioner(&server);
    let row = db
        .upsert_org_slack_connection(stored(41, "T1", -chrono::Duration::minutes(1)))
        .await
        .unwrap();

    assert!(matches!(
        provisioner.create_app(41, None, "{}").await,
        Err(SlackProvisioningError::ReconnectRequired)
    ));
    let row = db
        .get_org_slack_connection(41, row.id)
        .await
        .unwrap()
        .expect("connection retained");
    assert_eq!(row.state, "reconnect_required");
    assert!(row.access_token_encrypted.is_none());
    assert!(row.refresh_token_encrypted.is_none());
}

#[tokio::test]
async fn with_several_workspaces_the_caller_must_name_one() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();
    db.upsert_org_slack_connection(stored(41, "T2", chrono::Duration::hours(1)))
        .await
        .unwrap();

    assert!(matches!(
        provisioner.create_app(41, None, "{}").await,
        Err(SlackProvisioningError::WorkspaceRequired)
    ));
    assert!(matches!(
        provisioner.create_app(41, Some("T9"), "{}").await,
        Err(SlackProvisioningError::WorkspaceNotConnected)
    ));
}

#[tokio::test]
async fn app_creation_encodes_the_generated_yaml_as_json_for_slack() {
    let server = MockServer::start().await;
    let manifest = crate::channels::slack::events::build_manifest_yaml(
        "Support Agent",
        "Support Agent",
        Some("Answers questions"),
        "https://example.com/api/v1/channels/test/slack/events",
        "https://example.com/api/v1/channels/test/slack/interactivity",
        "https://example.com/api/v1/channels/test/slack/oauth/callback",
        true,
        &[],
    );
    let expected: serde_json::Value = serde_yaml::from_str(&manifest).unwrap();
    Mock::given(method("POST"))
        .and(path("/apps.manifest.create"))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "manifest": serde_json::to_string(&expected).unwrap()
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "app_id": "A1",
            "credentials": {
                "client_id": "c1",
                "client_secret": "s1",
                "signing_secret": "g1"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();

    let created = provisioner
        .create_app(41, Some("T1"), &manifest)
        .await
        .unwrap();
    assert_eq!(created.app_id, "A1");
}

#[tokio::test]
async fn an_app_is_created_with_the_named_workspaces_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/apps.manifest.create"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer access-T2",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "app_id": "A2",
            "credentials": {
                "client_id": "c2",
                "client_secret": "s2",
                "signing_secret": "g2"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = provisioner(&server);
    let encryption = encryption();
    for team in ["T1", "T2"] {
        db.upsert_org_slack_connection(UpsertOrgSlackConnection {
            access_token_encrypted: encryption
                .encrypt_string(&format!("access-{team}"))
                .unwrap(),
            ..stored(41, team, chrono::Duration::hours(1))
        })
        .await
        .unwrap();
    }

    let created = provisioner.create_app(41, Some("T2"), "{}").await.unwrap();

    assert_eq!(created.app_id, "A2");
}

#[tokio::test]
async fn connection_status_is_scoped_to_the_requested_organization() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
        .await
        .unwrap();

    assert!(provisioner.connection_status(41).await.unwrap().connected);
    assert!(!provisioner.connection_status(42).await.unwrap().connected);
}

fn manager_app() -> Option<SlackManagerApp> {
    Some(SlackManagerApp {
        client_id: "111.222".to_string(),
        client_secret: "manager-secret".to_string(),
    })
}

fn manager_provisioner(server: &MockServer) -> (SlackApiProvisioner, Arc<StorageBackend>) {
    let (provisioner, db) = provisioner(server);
    (provisioner.with_manager_app(manager_app()), db)
}

fn query_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("{name} missing from {url}"))
}

fn manager_grant(scope: &str) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "app_id": "AMANAGER",
        "authed_user": {
            "id": "U1",
            "scope": scope,
            "access_token": "xoxe.xoxp-grant",
            "refresh_token": "xoxe-1-grant-refresh",
            "expires_in": 43200,
            "token_type": "user"
        },
        "team": {"id": "T0123", "name": "Acme"}
    })
}

#[tokio::test]
async fn manager_consent_asks_only_for_app_configuration_and_names_the_org() {
    let server = MockServer::start().await;
    let (provisioner, _db) = manager_provisioner(&server);

    let url = provisioner
        .manager_authorize_url(
            41,
            "https://api.example.com/v1/slack/workspaces/oauth/callback",
            "browser-secret",
        )
        .await
        .unwrap()
        .0;

    assert!(url.starts_with(SLACK_OAUTH_AUTHORIZE_URL));
    assert_eq!(query_param(&url, "client_id"), "111.222");
    assert_eq!(query_param(&url, "scope"), "");
    assert_eq!(
        query_param(&url, "user_scope"),
        "app_configurations:read,app_configurations:write"
    );
    assert_eq!(
        query_param(&url, "redirect_uri"),
        "https://api.example.com/v1/slack/workspaces/oauth/callback"
    );
    let state = query_param(&url, "state");
    assert_eq!(
        provisioner
            .open_manager_state(&state)
            .map(|state| state.org_id),
        Some(41)
    );
}

#[tokio::test]
async fn a_forged_altered_foreign_or_expired_connect_state_names_no_org() {
    let server = MockServer::start().await;
    let (provisioner, _db) = manager_provisioner(&server);
    let seal = |state: &ManagerConnectState| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            encryption()
                .encrypt(&serde_json::to_vec(state).unwrap())
                .unwrap(),
        )
    };
    let valid = seal(&ManagerConnectState {
        purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
        org_id: 41,
        transaction_id: uuid::Uuid::new_v4(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
    });
    assert_eq!(
        provisioner
            .open_manager_state(&valid)
            .map(|state| state.org_id),
        Some(41)
    );

    let mut altered = valid.into_bytes();
    let last = altered.len() - 2;
    altered[last] = if altered[last] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        provisioner
            .open_manager_state(&String::from_utf8(altered).unwrap())
            .map(|state| state.org_id),
        None
    );
    assert_eq!(
        provisioner
            .open_manager_state("not-a-state")
            .map(|state| state.org_id),
        None
    );
    assert_eq!(
        provisioner
            .open_manager_state(&seal(&ManagerConnectState {
                purpose: "github_setup".to_string(),
                org_id: 41,
                transaction_id: uuid::Uuid::new_v4(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
            }))
            .map(|state| state.org_id),
        None
    );
    assert_eq!(
        provisioner
            .open_manager_state(&seal(&ManagerConnectState {
                purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
                org_id: 41,
                transaction_id: uuid::Uuid::new_v4(),
                expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
            }))
            .map(|state| state.org_id),
        None
    );
}

#[tokio::test]
async fn manager_transactions_bind_the_browser_and_org_and_survive_api_recreation() {
    let server = MockServer::start().await;
    let (provisioner, db) = manager_provisioner(&server);
    let (url, _) = provisioner
        .manager_authorize_url(41, "https://example.test/cb", "browser-one")
        .await
        .unwrap();
    let state = query_param(&url, "state");
    let mut pending = provisioner.open_manager_state(&state).unwrap();
    assert!(
        !provisioner
            .consume_manager_state(&pending, "browser-two")
            .await
            .unwrap()
    );
    pending.org_id = 42;
    assert!(
        !provisioner
            .consume_manager_state(&pending, "browser-one")
            .await
            .unwrap()
    );
    pending.org_id = 41;
    let restarted = SlackApiProvisioner::with_api_base(db, encryption(), &server.uri()).unwrap();
    let (first, second) = tokio::join!(
        provisioner.consume_manager_state(&pending, "browser-one"),
        restarted.consume_manager_state(&pending, "browser-one"),
    );
    assert_eq!(
        usize::from(first.unwrap()) + usize::from(second.unwrap()),
        1
    );
    assert!(
        !restarted
            .consume_manager_state(&pending, "browser-one")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn without_a_manager_app_there_is_no_consent_to_start() {
    let server = MockServer::start().await;
    let (provisioner, _db) = provisioner(&server);
    assert!(!provisioner.manager_app_configured());
    assert!(matches!(
        provisioner
            .manager_authorize_url(41, "https://example.test/cb", "browser-secret")
            .await,
        Err(SlackProvisioningError::Unavailable)
    ));
}

#[tokio::test]
async fn a_manager_grant_connects_the_workspace_it_was_granted_for() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string(
            "client_id=111.222&client_secret=manager-secret&code=consent-code&redirect_uri=https%3A%2F%2Fexample.test%2Fcb",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(manager_grant(
            "app_configurations:read,app_configurations:write",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = manager_provisioner(&server);

    let row = provisioner
        .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
        .await
        .unwrap();

    assert_eq!(row.org_id, 41);
    assert_eq!(row.team_id.as_deref(), Some("T0123"));
    assert_eq!(row.team_name.as_deref(), Some("Acme"));
    assert_eq!(row.grant_kind(), SlackGrantKind::ManagerApp);
    assert_eq!(row.state, "connected");
    let stored = row.refresh_token_encrypted.clone().unwrap();
    assert!(
        !stored
            .windows(b"grant-refresh".len())
            .any(|part| part == b"grant-refresh")
    );
    assert!(
        row.access_token_expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::hours(11)
    );
    assert!(provisioner.connection_status(41).await.unwrap().connected);
    assert_eq!(db.list_org_slack_connections(41).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_grant_that_cannot_create_apps_is_refused_and_not_stored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(manager_grant("app_configurations:read")),
        )
        .mount(&server)
        .await;
    let (provisioner, db) = manager_provisioner(&server);

    let result = provisioner
        .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
        .await;

    assert!(
        matches!(result, Err(SlackProvisioningError::Rejected(code)) if code == "missing_scope")
    );
    assert!(db.list_org_slack_connections(41).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_non_rotating_grant_is_refused_with_its_own_code() {
    let server = MockServer::start().await;
    let mut grant = manager_grant("app_configurations:read,app_configurations:write");
    grant["authed_user"]
        .as_object_mut()
        .unwrap()
        .retain(|key, _| key != "refresh_token" && key != "expires_in");
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(grant))
        .mount(&server)
        .await;
    let (provisioner, db) = manager_provisioner(&server);

    let result = provisioner
        .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
        .await;

    assert!(
        matches!(result, Err(SlackProvisioningError::Rejected(code)) if code == "token_rotation_disabled")
    );
    assert!(db.list_org_slack_connections(41).await.unwrap().is_empty());
}

#[tokio::test]
async fn slack_refusing_the_consent_code_surfaces_its_error_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": false, "error": "invalid_code"})),
        )
        .mount(&server)
        .await;
    let (provisioner, _db) = manager_provisioner(&server);

    assert!(matches!(
        provisioner
            .connect_with_manager_code(41, "stale", "https://example.test/cb")
            .await,
        Err(SlackProvisioningError::Rejected(code)) if code == "invalid_code"
    ));
}

fn stored_manager_grant(org_id: i64, expires_in: chrono::Duration) -> UpsertOrgSlackConnection {
    UpsertOrgSlackConnection {
        grant_kind: SlackGrantKind::ManagerApp,
        ..stored(org_id, "T1", expires_in)
    }
}

#[tokio::test]
async fn an_expiring_manager_grant_refreshes_through_oauth_not_token_rotation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(body_string(
            "client_id=111.222&client_secret=manager-secret&grant_type=refresh_token&refresh_token=refresh",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "access_token": "refreshed-access",
            "refresh_token": "refreshed-refresh",
            "expires_in": 43200,
            "token_type": "user",
            "team": {"id": "T1"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/tooling.tokens.rotate"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apps.manifest.create"))
        .and(header("authorization", "Bearer refreshed-access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "app_id": "A1",
            "credentials": {"client_id": "c1", "client_secret": "s1", "signing_secret": "g1"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = manager_provisioner(&server);
    let row = db
        .upsert_org_slack_connection(stored_manager_grant(41, chrono::Duration::minutes(1)))
        .await
        .unwrap();

    let created = provisioner.create_app(41, None, "{}").await.unwrap();

    assert_eq!(created.app_id, "A1");
    let row = db
        .get_org_slack_connection(41, row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "connected");
    assert_eq!(row.grant_kind(), SlackGrantKind::ManagerApp);
    assert!(
        row.access_token_expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::hours(11)
    );
}

#[tokio::test]
async fn a_revoked_manager_grant_asks_for_reconnect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": false, "error": "invalid_refresh_token"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (provisioner, db) = manager_provisioner(&server);
    let row = db
        .upsert_org_slack_connection(stored_manager_grant(41, -chrono::Duration::minutes(1)))
        .await
        .unwrap();

    assert!(matches!(
        provisioner.create_app(41, None, "{}").await,
        Err(SlackProvisioningError::ReconnectRequired)
    ));
    let row = db
        .get_org_slack_connection(41, row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "reconnect_required");
    assert!(
        provisioner
            .connection_status(41)
            .await
            .unwrap()
            .reconnect_required
    );
}

#[tokio::test]
async fn a_manager_grant_is_kept_when_the_deployment_drops_its_manager_app() {
    let server = MockServer::start().await;
    let (provisioner, db) = provisioner(&server);
    let row = db
        .upsert_org_slack_connection(stored_manager_grant(41, -chrono::Duration::minutes(1)))
        .await
        .unwrap();

    assert!(matches!(
        provisioner.create_app(41, None, "{}").await,
        Err(SlackProvisioningError::Rejected(code)) if code == "manager_app_not_configured"
    ));
    let row = db
        .get_org_slack_connection(41, row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "connected");
    assert!(row.refresh_token_encrypted.is_some());
    assert!(server.received_requests().await.unwrap().is_empty());
}
