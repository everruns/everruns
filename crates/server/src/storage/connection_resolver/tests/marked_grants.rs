// Grants that can no longer produce a token are marked, not dropped or deleted.
//
// Spec: knowledge/integrations/mcp-oauth-grant-health.md.
use super::*;

async fn service_grant(
    fixture: &McpFixture,
    refresh: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
) {
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
            provider: fixture.provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-agent".to_string()),
            access_token_encrypted: Some(
                fixture.encryption.encrypt_string("stale-access").unwrap(),
            ),
            refresh_token_encrypted: refresh
                .map(|value| fixture.encryption.encrypt_string(&value).unwrap()),
            scopes: None,
            expires_at,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
}

fn fixture_resolver(
    fixture: &McpFixture,
    result: FakeRefreshResult,
) -> (DbConnectionResolver, Arc<FakeRefreshExchange>) {
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result,
    });
    let resolver = DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        None,
        exchange.clone(),
    )
    .bound_to_input_message(INPUT_MESSAGE);
    (resolver, exchange)
}

/// The Kids Duties case: a grant saved before refresh tokens were bound to
/// their server keeps working until its access token expires, then is marked
/// as needing sign-in, without its refresh token ever being sent.
#[tokio::test]
async fn an_expired_unbound_service_grant_is_marked_without_a_refresh_request() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    service_grant(
        &fixture,
        Some("legacy-plain-refresh".into()),
        Some(Utc::now() + Duration::days(2)),
    )
    .await;
    let (resolver, exchange) = fixture_resolver(&fixture, FakeRefreshResult::Success);
    let server = DbConnectionResolver::parse_mcp_oauth_provider(&fixture.provider).unwrap();
    let row = || async {
        fixture
            .db
            .get_virtual_user_connection(fixture.identity_id, &fixture.provider)
            .await
            .unwrap()
            .unwrap()
    };

    // Still valid: it works, and Health warns ahead of the expiry.
    assert_eq!(
        resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service
            )
            .await
            .unwrap()
            .as_deref(),
        Some("stale-access")
    );
    assert!(matches!(
        resolver.mcp_grant_health(DEFAULT_ORG_ID, server, &row().await).await.unwrap(),
        GrantHealth::ExpiresSoon { ref reason, .. } if reason == grant_health::reason::UNBOUND
    ));

    // Expired: no token, no refresh request, and the grant says why.
    service_grant(
        &fixture,
        Some("legacy-plain-refresh".into()),
        Some(Utc::now() - Duration::minutes(5)),
    )
    .await;
    assert_eq!(
        resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 0);
    let marked = row().await;
    assert_eq!(
        grant_health::reconnect_mark(&marked).map(|(reason, _)| reason),
        Some(grant_health::reason::UNBOUND.to_string())
    );
    assert!(matches!(
        resolver.mcp_grant_health(DEFAULT_ORG_ID, server, &marked).await.unwrap(),
        GrantHealth::NeedsSignIn { ref reason, .. } if reason == grant_health::reason::UNBOUND
    ));

    // Signing in again replaces the row and clears the mark.
    service_grant(
        &fixture,
        Some(refresh_grant("linear", "old-refresh")),
        Some(Utc::now() + Duration::days(30)),
    )
    .await;
    let fresh = row().await;
    assert!(grant_health::reconnect_mark(&fresh).is_none());
    assert_eq!(
        resolver
            .mcp_grant_health(DEFAULT_ORG_ID, server, &fresh)
            .await
            .unwrap(),
        GrantHealth::Usable
    );
}

/// A server rejecting the token first forces a refresh; rejecting the
/// refreshed token too marks the grant. The row is never deleted.
#[tokio::test]
async fn repeated_rejections_mark_the_grant_and_keep_it() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    service_grant(
        &fixture,
        Some(refresh_grant("linear", "old-refresh")),
        Some(Utc::now() + Duration::days(1)),
    )
    .await;
    let (resolver, exchange) = fixture_resolver(&fixture, FakeRefreshResult::Success);
    let fingerprint = everruns_internal_protocol::credential_fingerprint("stale-access");
    resolver
        .invalidate_mcp_connection(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
            &fingerprint,
        )
        .await
        .unwrap();
    // Forced refresh on the next use.
    assert_eq!(
        resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service
            )
            .await
            .unwrap()
            .as_deref(),
        Some("fresh-access")
    );
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    // The refreshed token is rejected as well: sign in again.
    resolver
        .invalidate_mcp_connection(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
            &everruns_internal_protocol::credential_fingerprint("fresh-access"),
        )
        .await
        .unwrap();
    let row = fixture
        .db
        .get_virtual_user_connection(fixture.identity_id, &fixture.provider)
        .await
        .unwrap()
        .expect("kept");
    assert_eq!(
        grant_health::reconnect_mark(&row).map(|(reason, _)| reason),
        Some(grant_health::reason::REJECTED.to_string())
    );
    assert_eq!(
        resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
}
