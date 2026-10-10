//! Browser OAuth failures return to `return_to` with `connect_error`.

use super::*;
use crate::oauth_client::OAUTH_BLOCKED_BY_NETWORK_POLICY;

/// The organization's egress policy refuses every host.
struct PolicyDeniedEgress;

#[async_trait::async_trait]
impl EgressService for PolicyDeniedEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        Err(everruns_core::EgressError::NetworkAccessDenied { url: request.url })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by OAuth handlers")
    }
}

fn with_policy_denied_egress(mut state: AppState) -> AppState {
    state.mcp_service = Arc::new(McpServerService::with_egress_service(
        state.db.clone(),
        state.encryption.clone(),
        Arc::new(PolicyDeniedEgress),
    ));
    state
}

fn location(redirect: Redirect) -> String {
    redirect
        .into_response()
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

async fn begin_user_oauth_returning_to(
    state: AppState,
    org: ResolvedOrg,
    server_id: Uuid,
    return_to: Option<&str>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    authorize_connection(
        State(state),
        org.clone(),
        ConnectionUser {
            id: org.user_id.unwrap(),
            management_user_id: org.user_id.unwrap(),
            org_id: org.org_id,
        },
        CookieJar::new(),
        Path(mcp_oauth_provider_id_for_uuid(server_id)),
        Query(OAuthAuthorizeQuery {
            return_to: return_to.map(ToOwned::to_owned),
            mode: None,
            session_id: None,
            agent_id: None,
            popup: None,
        }),
    )
    .await
}

#[tokio::test]
async fn blocked_discovery_host_returns_to_the_page_with_a_policy_code() {
    let (state, org, server_id, _, _) = identity_oauth_fixture(false).await;
    let state = with_policy_denied_egress(state);
    let provider = mcp_oauth_provider_id_for_uuid(server_id);

    let (jar, redirect) =
        begin_user_oauth_returning_to(state, org, server_id, Some("/agents/agent_1?tab=mcp"))
            .await
            .expect("a safe return_to turns the failure into a redirect");

    let target = location(redirect);
    assert_eq!(
        target,
        format!(
            "{}/agents/agent_1?tab=mcp&connect_error=blocked_by_network_policy&provider={provider}",
            AuthConfig::default().frontend_url.trim_end_matches('/')
        )
    );
    // The blocked host stays in the server log, never in the browser URL.
    assert!(!target.contains("8.8.8.8"), "{target}");
    // No pending setup is planted for a flow that never reached the provider.
    assert!(jar.get(&oauth_state_cookie_name(&provider)).is_none());
}

#[tokio::test]
async fn blocked_host_without_a_safe_return_to_stays_an_error_response() {
    let (state, org, server_id, _, _) = identity_oauth_fixture(false).await;
    let state = with_policy_denied_egress(state);

    for return_to in [
        None,
        Some("//attacker.example/x"),
        Some("https://attacker.example"),
    ] {
        let error = begin_user_oauth_returning_to(state.clone(), org.clone(), server_id, return_to)
            .await
            .expect_err("no safe return_to means no redirect");
        assert_eq!(error.0, StatusCode::BAD_GATEWAY, "{return_to:?}");
        // The JSON/plain error names the policy instead of a bare "Bad gateway".
        assert_eq!(error.1, OAUTH_BLOCKED_BY_NETWORK_POLICY, "{return_to:?}");
    }
}

#[tokio::test]
async fn unreachable_provider_returns_with_provider_unreachable() {
    // FakeOAuthEgress answers discovery with a 502.
    let (state, org, server_id, _, _) = identity_oauth_fixture(false).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);

    let (_, redirect) =
        begin_user_oauth_returning_to(state, org, server_id, Some("/settings/agent-experience"))
            .await
            .unwrap();

    let target = location(redirect);
    assert!(
        target.ends_with(&format!(
            "/settings/agent-experience?connect_error=provider_unreachable&provider={provider}"
        )),
        "{target}"
    );
}

#[tokio::test]
async fn provider_refusal_at_callback_returns_with_provider_refused() {
    let (state, org, server_id, _, user_id) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_user_oauth_returning_to(
        state.clone(),
        org.clone(),
        server_id,
        Some("/agents/agent_1?tab=mcp"),
    )
    .await
    .unwrap();
    let pending = pending_state(&jar, &provider);

    let (jar, redirect) = connection_oauth_callback(
        State(state.clone()),
        Ok(org),
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: None,
            state: Some(pending.state),
            error: Some("access_denied".to_string()),
            error_description: Some("secret upstream detail".to_string()),
        }),
    )
    .await
    .expect("a valid state returns the browser to its page");

    let target = location(redirect);
    assert!(
        target.ends_with(&format!(
            "/agents/agent_1?tab=mcp&connect_error=provider_refused&provider={provider}"
        )),
        "{target}"
    );
    assert!(!target.contains("secret"), "{target}");
    // The state cookie is cleared and no grant is written.
    assert!(
        jar.get(&oauth_state_cookie_name(&provider))
            .is_none_or(|cookie| cookie.value().is_empty())
    );
    assert!(
        state
            .db
            .get_user_connection(user_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}
