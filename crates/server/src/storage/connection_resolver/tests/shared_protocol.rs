//! Refresh through the shared protocol while retaining the Server's replica lock and encrypted store.
use super::*;
use everruns_core::{EgressRequest, EgressResponse, EgressResult, EgressStreamResponse};

#[derive(Default)]
struct RotatingEgress {
    calls: AtomicUsize,
}
#[async_trait]
impl EgressService for RotatingEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let form: std::collections::BTreeMap<String, String> =
            serde_urlencoded::from_bytes(&request.body).unwrap();
        assert_eq!(request.url, "https://1.1.1.1/token");
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "old-refresh");
        assert_eq!(form["resource"], "https://mcp.resend.com/mcp");
        assert_eq!(
            request.scope.as_ref().and_then(|s| s.org_id),
            Some(
                crate::domains::mcp_servers::presentation::org_scope(DEFAULT_ORG_ID)
                    .org_id
                    .unwrap()
            )
        );
        assert!(request.pinned_addrs.is_some());
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(StdDuration::from_millis(20)).await;
        Ok(EgressResponse { status: 200, headers: Default::default(), body: br#"{"access_token":"shared-access","refresh_token":"shared-rotation","expires_in":3600}"#.to_vec() })
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unreachable!()
    }
}

#[tokio::test]
async fn shared_protocol_refresh_is_single_flight_across_server_replicas_and_persists_rotation() {
    let user_id = Uuid::now_v7();
    let (db, encryption, session, server, provider) = setup(Some(user_id)).await;
    let oauth =
        serde_json::json!({"token_endpoint": "https://1.1.1.1/token", "client_id": "test-client"});
    db.update_mcp_server_settings_any_owner(
        DEFAULT_ORG_ID,
        server,
        serde_json::json!({"auth_mode": "oauth", "oauth": oauth}),
    )
    .await
    .unwrap();
    let settings: McpServerOAuthSettings = serde_json::from_value(oauth).unwrap();
    let grant = McpRefreshGrant {
        refresh_token: "old-refresh".into(),
        resource: "https://mcp.resend.com/mcp".into(),
        config_hash: mcp_oauth_config_hash("https://mcp.resend.com/mcp", &settings),
    }
    .encode();
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id,
        provider: provider.clone(),
        connection_type: "oauth".into(),
        provider_user_id: None,
        provider_username: None,
        access_token_encrypted: Some(encryption.encrypt_string("stale-access").unwrap()),
        refresh_token_encrypted: Some(encryption.encrypt_string(&grant).unwrap()),
        scopes: None,
        expires_at: Some(Utc::now() - Duration::minutes(1)),
        installation_id: None,
        provider_metadata: None,
    })
    .await
    .unwrap();
    let egress = Arc::new(RotatingEgress::default());
    let replicas = [0, 1].map(|_| {
        Arc::new(
            DbConnectionResolver::new(db.clone(), encryption.clone(), None, egress.clone())
                .bound_to_input_message(INPUT_MESSAGE),
        )
    });
    let mut tasks = Vec::new();
    for index in 0..8 {
        let resolver = replicas[index % 2].clone();
        let provider = provider.clone();
        tasks.push(tokio::spawn(async move {
            resolver
                .get_connection_token(session, &provider)
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().as_deref(), Some("shared-access"));
    }
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
    let saved = db
        .get_user_connection(user_id, &provider)
        .await
        .unwrap()
        .unwrap();
    let encoded = encryption
        .decrypt_to_string(saved.refresh_token_encrypted.as_deref().unwrap())
        .unwrap();
    let persisted: McpRefreshGrant = serde_json::from_str(&encoded).unwrap();
    assert_eq!(persisted.refresh_token, "shared-rotation");
    assert_eq!(persisted.resource, "https://mcp.resend.com/mcp");
    // A new resolver reads the stored result; it cannot replay the old grant after a restart.
    let restarted = DbConnectionResolver::new(db, encryption, None, egress.clone())
        .bound_to_input_message(INPUT_MESSAGE);
    assert_eq!(
        restarted
            .get_connection_token(session, &provider)
            .await
            .unwrap()
            .as_deref(),
        Some("shared-access")
    );
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
}
