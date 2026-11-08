use super::*;

pub(super) fn refresh_grant(provider: &str, refresh_token: &str) -> String {
    let host = if provider == "linear" {
        "linear.app"
    } else {
        "resend.com"
    };
    let server_url = format!("https://mcp.{host}/mcp");
    let oauth: McpServerOAuthSettings = serde_json::from_value(serde_json::json!({
        "token_endpoint": format!("https://api.{host}/oauth/token"),
        "client_id": "test-client",
    }))
    .unwrap();
    McpRefreshGrant {
        refresh_token: refresh_token.into(),
        resource: server_url.clone(),
        config_hash: mcp_oauth_config_hash(&server_url, &oauth),
    }
    .encode()
}

#[tokio::test]
async fn legacy_and_retargeted_grants_require_reauthorization_without_network() {
    for stored in ["old-refresh".to_string(), {
        let mut grant: McpRefreshGrant =
            serde_json::from_str(&refresh_grant("resend", "old-refresh")).unwrap();
        grant.config_hash = "previous-configuration".into();
        grant.encode()
    }] {
        let user_id = Uuid::now_v7();
        let (db, encryption, session_id, _, provider) = setup(Some(user_id)).await;
        db.upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.clone(),
            connection_type: "oauth".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(encryption.encrypt_string("old-access").unwrap()),
            refresh_token_encrypted: Some(encryption.encrypt_string(&stored).unwrap()),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
        let exchange = Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            result: FakeRefreshResult::Success,
        });
        let resolver =
            DbConnectionResolver::with_oauth_refresh(db, encryption, None, exchange.clone())
                .bound_to_input_message(INPUT_MESSAGE);
        assert_eq!(
            resolver
                .get_connection_token(session_id, &provider)
                .await
                .unwrap(),
            None
        );
        assert_eq!(exchange.calls.load(Ordering::SeqCst), 0);
    }
}
