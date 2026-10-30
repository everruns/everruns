use super::*;
use crate::storage::{EncryptionService, StorageBackend, encryption::generate_encryption_key};
use everruns_core::{
    Caller, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use serde_json::{Value, json};
use std::sync::Arc;
const SECRET: &str = "external_endpoint_test_secret_01234567890";
struct Endpoint;
#[async_trait::async_trait]
impl EgressService for Endpoint {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        unreachable!()
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["audience"], request.url);
        assert!(request.scope.unwrap().org_id.is_some());
        let proof = crate::worker_link::custom_workers::http::signature(
            SECRET,
            body["challenge"].as_str().unwrap().as_bytes(),
        );
        Ok(EgressStreamResponse {
            status: 200,
            headers: Default::default(),
            body: Box::pin(futures::stream::iter(vec![Ok(serde_json::to_vec(
                &json!({"proof":proof}),
            )
            .unwrap())])),
        })
    }
}
async fn setup() -> (Ctx, String, uuid::Uuid) {
    let db = Arc::new(StorageBackend::test_database());
    let id = db.create_test_agent(1, uuid::Uuid::now_v7()).await;
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("external-test"), &[]).unwrap());
    let mut ctx = Ctx::minimal_for_test(Caller::internal(1), db, Some(encryption));
    ctx.egress_service = Some(Arc::new(Endpoint));
    (ctx, format!("agent_{}", id.simple()), id)
}
fn register(agent_id: &str) -> RegisterExternalAgent {
    RegisterExternalAgent {
        agent_id: agent_id.into(),
        endpoint_url: "https://agent.example/".into(),
        endpoint_secret: SECRET.into(),
    }
}

#[tokio::test]
async fn protected_ingress_refuses_registration_but_allows_revocation() {
    let (mut ctx, agent_id, _) = setup().await;
    register(&agent_id).register(&ctx).await.unwrap();
    ctx.egress_service = None;
    for version in [1, 99] {
        sqlx::query("INSERT INTO worker_ingress_protection_floor (minimum_version) VALUES ($1) ON CONFLICT (singleton) DO UPDATE SET minimum_version=EXCLUDED.minimum_version")
            .bind(version).execute(ctx.db.pool()).await.unwrap();
        let error = register(&agent_id).register(&ctx).await.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.to_string().contains("protected worker ingress"));
    }
    DisableExternalAgent {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(
        !GetExternalAgent { agent_id }
            .run(&ctx)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn self_service_registration_encrypts_secrets_rotates_and_disables_without_fallback() {
    let (ctx, agent_id, id) = setup().await;
    let binding = register(&agent_id).register(&ctx).await.unwrap();
    assert!(binding.enabled);
    assert!(!serde_json::to_string(&binding).unwrap().contains(SECRET));
    let (hash, secret, token): (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT token_hash,endpoint_secret_encrypted,adapter_token_encrypted FROM custom_workers WHERE agent_id=$1",
    )
    .bind(id)
    .fetch_one(ctx.db.pool())
    .await
    .unwrap();
    assert!(
        !secret
            .windows(SECRET.len())
            .any(|bytes| bytes == SECRET.as_bytes())
    );
    assert_eq!(
        ctx.encryption
            .as_ref()
            .unwrap()
            .decrypt_to_string(&secret)
            .unwrap(),
        SECRET
    );
    let token = ctx
        .encryption
        .as_ref()
        .unwrap()
        .decrypt_to_string(&token)
        .unwrap();
    assert_eq!(hash, Sha256::digest(token.as_bytes()).to_vec());
    let invoked_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("UPDATE custom_workers SET last_invoked_at=NOW() WHERE agent_id=$1 RETURNING last_invoked_at")
        .bind(id).fetch_one(ctx.db.pool()).await.unwrap();
    let rotated = register(&agent_id).register(&ctx).await.unwrap();
    let stored = GetExternalAgent {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap()
    .unwrap();
    assert_eq!(rotated.last_invoked_at, Some(invoked_at));
    assert_eq!(rotated.last_invoked_at, stored.last_invoked_at);
    let current: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM custom_workers WHERE agent_id=$1")
            .bind(id)
            .fetch_one(ctx.db.pool())
            .await
            .unwrap();
    assert_ne!(hash, current);
    DisableExternalAgent {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(
        !GetExternalAgent { agent_id }
            .run(&ctx)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM custom_workers WHERE agent_id=$1)")
            .bind(id)
            .fetch_one(ctx.db.pool())
            .await
            .unwrap();
    assert!(exists, "Revocation must preserve routing exclusion");
}

#[tokio::test]
async fn registration_cannot_replace_pull_worker_or_cross_tenant() {
    let (mut ctx, agent_id, id) = setup().await;
    sqlx::query(
        "INSERT INTO custom_workers(org_id,agent_id,name,token_hash) VALUES(1,$1,'pull',$2)",
    )
    .bind(id)
    .bind(vec![1u8; 32])
    .execute(ctx.db.pool())
    .await
    .unwrap();
    assert_eq!(
        register(&agent_id)
            .register(&ctx)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::CONFLICT
    );
    ctx.caller.org_id = 2;
    assert_eq!(
        register(&agent_id)
            .register(&ctx)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn read_only_caller_cannot_register_or_revoke() {
    let (mut ctx, agent_id, _) = setup().await;
    ctx.caller.read_only = true;
    assert_eq!(
        register(&agent_id).run(&ctx).await.unwrap_err().status(),
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        DisableExternalAgent { agent_id }
            .run(&ctx)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::FORBIDDEN
    );
}

#[test]
fn endpoint_credentials_are_excluded_from_entity_history() {
    use crate::domains::change_history::Change;
    assert!(matches!(RegisterExternalAgent::change(), Change::Exempt(_)));
    assert!(matches!(DisableExternalAgent::change(), Change::Exempt(_)));
}

#[tokio::test]
async fn ordinary_member_cannot_change_execution_credentials() {
    let (mut ctx, agent_id, _) = setup().await;
    ctx.caller.is_internal = false;
    ctx.caller.role = everruns_core::OrgRole::Member;
    ctx.caller.user_id = Some(uuid::Uuid::nil());
    assert_eq!(
        register(&agent_id).run(&ctx).await.unwrap_err().status(),
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        DisableExternalAgent { agent_id }
            .run(&ctx)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::FORBIDDEN
    );
}
