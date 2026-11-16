// Unit tests for the one-click Slack install (`install.rs`), in its own file so
// that file stays under the size guard.

use super::super::manager_connect::{manager_redirect_uri, slack_error_code};
use super::*;
use crate::storage::StorageBackend;
fn resolved_org(role: everruns_core::OrgRole) -> ResolvedOrg {
    ResolvedOrg {
        org_id: 41,
        public_id: "org_00000000000000000000000000000029".to_string(),
        name: "Test".to_string(),
        user_id: Some(uuid::Uuid::nil()),
        role,
        is_platform_user: false,
        feature_flags: crate::records::FeatureFlags::default(),
        read_only: false,
    }
}

#[test]
fn slack_connection_mutations_require_org_admin() {
    assert!(require_org_admin(&resolved_org(everruns_core::OrgRole::Admin)).is_ok());
    assert!(require_org_admin(&resolved_org(everruns_core::OrgRole::Owner)).is_ok());
    let (status, _) = require_org_admin(&resolved_org(everruns_core::OrgRole::Member)).unwrap_err();
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn connection_row(team_id: Option<&str>) -> OrgSlackConnectionRow {
    OrgSlackConnectionRow {
        id: uuid::Uuid::now_v7(),
        org_id: 41,
        team_id: team_id.map(str::to_string),
        team_name: None,
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        access_token_expires_at: None,
        state: "connected".to_string(),
        token_generation: 1,
        grant_kind: "config_token".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn only_slack_shaped_error_codes_are_reflected_into_the_settings_url() {
    assert_eq!(slack_error_code("access_denied"), Some("access_denied"));
    assert_eq!(slack_error_code("missing_scope"), Some("missing_scope"));
    assert_eq!(slack_error_code(""), None);
    assert_eq!(slack_error_code("x&slack_connect=ok"), None);
    assert_eq!(slack_error_code("<script>"), None);
    assert_eq!(slack_error_code(&"a".repeat(65)), None);
}

#[test]
fn manager_consent_returns_to_a_fixed_callback_under_the_api_base() {
    assert_eq!(
        manager_redirect_uri("https://api.example.com/api/"),
        "https://api.example.com/api/v1/slack/workspaces/oauth/callback"
    );
}

#[test]
fn any_slack_refusal_while_connecting_is_reported_as_the_tokens_fault() {
    for code in ["invalid_refresh_token", "invalid_arguments", "invalid_auth"] {
        let (status, Json(body)) =
            connection_error_response(SlackProvisioningError::Rejected(code.to_string()));
        assert_eq!(status, StatusCode::BAD_REQUEST, "{code}");
        let rendered = serde_json::to_string(&body).unwrap();
        assert!(
            rendered.contains("configuration refresh token"),
            "{rendered}"
        );
        assert!(rendered.contains(code), "{rendered}");
        assert!(!rendered.contains("app creation"), "{rendered}");
    }
}

#[test]
fn a_requested_workspace_must_be_one_the_org_connected() {
    let rows = [connection_row(Some("T1")), connection_row(Some("T2"))];
    assert_eq!(
        pick_workspace(&rows, Some("T2".to_string())).unwrap(),
        Some("T2".to_string())
    );
    // Never stored or put on the consent URL on the caller's word alone.
    assert!(matches!(
        pick_workspace(&rows, Some("T_OTHER_ORG".to_string())),
        Err(SlackProvisioningError::WorkspaceNotConnected)
    ));
}

#[test]
fn with_no_choice_only_a_sole_workspace_is_assumed() {
    assert_eq!(
        pick_workspace(&[connection_row(Some("T1"))], None).unwrap(),
        Some("T1".to_string())
    );
    // Several connected: no guess here; the provisioner refuses later.
    assert_eq!(
        pick_workspace(
            &[connection_row(Some("T1")), connection_row(Some("T2"))],
            None
        )
        .unwrap(),
        None
    );
    assert_eq!(pick_workspace(&[], None).unwrap(), None);
}

#[test]
fn minted_state_is_long_random_hex() {
    let first = mint_install_state();
    let second = mint_install_state();
    assert_eq!(first.len(), 64, "256 bits, hex-encoded");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
    assert_ne!(first, second, "a fixed nonce would defeat the check");
}

#[test]
fn state_comparison_rejects_mismatch_and_prefix() {
    let expected = mint_install_state();
    assert!(constant_time_eq(&expected, &expected.clone()));
    assert!(!constant_time_eq(&expected, ""));
    // A prefix must not pass: a length-only check would let one through.
    assert!(!constant_time_eq(&expected, &expected[..32]));
    let mut flipped = expected.clone();
    flipped.replace_range(63..64, if expected.ends_with('a') { "b" } else { "a" });
    assert!(!constant_time_eq(&expected, &flipped));
}

#[test]
fn an_unparseable_config_does_not_panic() {
    let config = parse_config(&serde_json::json!("not an object"));
    assert!(config.provisioned_app.is_none());
}

#[tokio::test]
async fn exchange_rejects_a_body_slack_marked_not_ok() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/oauth.v2.access"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": false, "error": "invalid_code"})),
        )
        .mount(&server)
        .await;
    let result = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb").await;
    assert_eq!(result.err(), Some("oauth exchange refused"));
}

#[tokio::test]
async fn exchange_requires_both_a_token_and_a_team() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/oauth.v2.access"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok": true, "access_token": "xoxb-1", "team": {}}),
            ),
        )
        .mount(&server)
        .await;
    let result = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb").await;
    assert_eq!(result.err(), Some("oauth exchange returned no team id"));
}

#[tokio::test]
async fn exchange_takes_the_bot_token_and_workspace_from_a_good_response() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/oauth.v2.access"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "access_token": "xoxb-real-token",
                "team": {"id": "T0123", "name": "Acme"},
            })),
        )
        .mount(&server)
        .await;
    let exchanged = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb")
        .await
        .expect("exchange succeeds");
    assert_eq!(exchanged.bot_token, "xoxb-real-token");
    assert_eq!(exchanged.team_id, "T0123");
}

fn provisioned(state: Option<&str>, issued_minutes_ago: Option<i64>) -> ProvisionedSlackApp {
    ProvisionedSlackApp {
        app_id: "A0123".to_string(),
        client_id: "4567.89".to_string(),
        client_secret: "secret".to_string(),
        install_state: state.map(str::to_string),
        install_state_issued_at: issued_minutes_ago
            .map(|ago| chrono::Utc::now() - chrono::Duration::minutes(ago)),
        team_id: None,
        install_return_issue: None,
    }
}

#[test]
fn spending_a_matching_nonce_clears_it() {
    let nonce = mint_install_state();
    let mut app = provisioned(Some(&nonce), Some(0));
    assert!(spend_install_state(&mut app, &nonce, chrono::Utc::now()).is_ok());
    assert!(app.install_state.is_none(), "a replay must find nothing");
    assert!(app.install_state_issued_at.is_none());
}

#[test]
fn a_wrong_nonce_is_refused_and_still_burns_the_stored_one() {
    let nonce = mint_install_state();
    let mut app = provisioned(Some(&nonce), Some(0));
    assert_eq!(
        spend_install_state(&mut app, "not-the-nonce", chrono::Utc::now()),
        Err("state mismatch")
    );
    // Taken even on refusal: leaving it set would let an attacker who can
    // trigger the callback keep guessing against the same value.
    assert!(app.install_state.is_none());
}

#[test]
fn an_empty_nonce_cannot_stand_in_for_a_missing_one() {
    let mut app = provisioned(Some(&mint_install_state()), Some(0));
    assert_eq!(
        spend_install_state(&mut app, "", chrono::Utc::now()),
        Err("state mismatch")
    );
}

#[test]
fn a_callback_with_no_install_in_flight_is_refused() {
    let mut app = provisioned(None, None);
    assert_eq!(
        spend_install_state(&mut app, "anything", chrono::Utc::now()),
        Err("no install in flight")
    );
}

#[test]
fn a_nonce_past_its_window_is_refused_even_when_it_matches() {
    let nonce = mint_install_state();
    let mut app = provisioned(Some(&nonce), Some(INSTALL_STATE_TTL_MINUTES + 1));
    assert_eq!(
        spend_install_state(&mut app, &nonce, chrono::Utc::now()),
        Err("install state expired")
    );
}

#[test]
fn a_nonce_inside_its_window_is_accepted() {
    let nonce = mint_install_state();
    let mut app = provisioned(Some(&nonce), Some(INSTALL_STATE_TTL_MINUTES - 1));
    assert!(spend_install_state(&mut app, &nonce, chrono::Utc::now()).is_ok());
}

#[test]
fn consent_requests_manifest_scopes_for_both_slack_surfaces() {
    for agent_surface_enabled in [false, true] {
        let redirect = "https://example.com/api/v1/channels/test/slack/oauth/callback";
        let url = url::Url::parse(&authorize_url(
            "client-id",
            "nonce",
            redirect,
            Some("T1"),
            agent_surface_enabled,
        ))
        .unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params["client_id"], "client-id");
        assert_eq!(params["state"], "nonce");
        assert_eq!(params["redirect_uri"], redirect);
        assert_eq!(params["team"], "T1");
        let scopes: Vec<_> = params["scope"].split(',').collect();
        assert!(scopes.contains(&"chat:write"));
        assert!(scopes.contains(&"reactions:write"));
        assert_eq!(scopes.contains(&"assistant:write"), agent_surface_enabled);

        let yaml = super::super::events::build_manifest_yaml(
            "Agent",
            "Agent",
            None,
            "https://example.com/events",
            "https://example.com/actions",
            redirect,
            agent_surface_enabled,
            &[],
        );
        let manifest: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
        let declared: Vec<_> = manifest["oauth_config"]["scopes"]["bot"]
            .as_array()
            .unwrap()
            .iter()
            .map(|scope| scope.as_str().unwrap())
            .collect();
        assert_eq!(scopes, declared);
    }
}

#[test]
fn an_absent_provisioner_answers_not_implemented_rather_than_error() {
    let (status, _) = provisioning_error_response(SlackProvisioningError::Unavailable);
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    let (status, Json(problem)) =
        provisioning_error_response(SlackProvisioningError::Rejected("ratelimited".into()));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem.code.as_deref(), Some("ratelimited"));
    let (status, _) =
        provisioning_error_response(SlackProvisioningError::Unreachable("timeout".into()));
    assert_eq!(status, StatusCode::FAILED_DEPENDENCY);
}

#[tokio::test]
async fn concurrent_installs_for_one_channel_are_serialized() {
    let db = Arc::new(StorageBackend::test_database());
    let channel_id = uuid::Uuid::now_v7();
    let first = db.lock_slack_install(channel_id).await.expect("first lock");

    let waiting_db = db.clone();
    let waiting = tokio::spawn(async move {
        let _guard = waiting_db
            .lock_slack_install(channel_id)
            .await
            .expect("second lock");
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), waiting)
            .await
            .is_err(),
        "a concurrent install must wait for the endpoint lock"
    );

    drop(first);
    db.lock_slack_install(channel_id)
        .await
        .expect("lock released when guard drops");
}

#[test]
fn only_socket_mode_without_https_skips_oauth_consent() {
    use super::super::events::slack_oauth_redirect_available as available;
    assert!(available("https://app.example.com/api", true));
    assert!(available("https://app.example.com/api", false));
    // Request URL apps keep the redirect: they need public HTTPS regardless.
    assert!(available("http://localhost:9300/api", false));
    // Slack refuses http:// redirect URLs, localhost included.
    assert!(!available("http://localhost:9300/api", true));
    assert_eq!(
        slack_app_install_page("A0123"),
        "https://api.slack.com/apps/A0123/install-on-team"
    );
}
