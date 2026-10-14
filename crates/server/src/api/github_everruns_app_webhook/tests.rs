// The shared Everruns App webhook: signature first, then routing by the signed
// installation id to the one org that links it.

use super::*;
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::github_apps::{EVERRUNS_APP_CONNECTION_TYPE, GITHUB_PROVIDER};
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{
    CreateOrganizationConnectionRow, CreateOrganizationRow, CreateVirtualUserConnectionRow,
    CreateVirtualUserRow, StorageBackend,
};
use async_trait::async_trait;
use everruns_contracts::typed_id::{SessionId, VirtualUserId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::host::{TurnBackend, TurnRequest, TurnTicket};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::sync::Arc;

const SECRET: &str = "shared-app-secret";
const LINKED: i64 = 9100;

struct NoopRunner;

#[async_trait]
impl TurnBackend for NoopRunner {
    async fn start_turn(
        &self,
        request: TurnRequest,
    ) -> everruns_contracts::error::Result<TurnTicket> {
        Ok(TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

fn state(db: &Arc<StorageBackend>, secret: Option<&str>) -> ChannelWebhookState {
    let mut state = ChannelWebhookState::new(
        db.clone(),
        None,
        Arc::new(NoopRunner),
        EventDelivery::in_memory(),
        ChannelRateLimiter::in_memory("webhook"),
    );
    state.everruns_app = Some(EverrunsAppWebhook {
        secret: secret.map(str::to_string),
        app_slug: "everruns".to_string(),
    });
    state
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn headers(event: &str, signature: Option<String>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-github-event", event.parse().unwrap());
    headers.insert("x-github-delivery", "delivery-1".parse().unwrap());
    if let Some(signature) = signature {
        headers.insert("x-hub-signature-256", signature.parse().unwrap());
    }
    headers
}

async fn deliver(
    state: &ChannelWebhookState,
    event: &str,
    payload: serde_json::Value,
    signature: Option<String>,
) -> Result<(StatusCode, GitHubWebhookResponse), StatusCode> {
    let body = serde_json::to_vec(&payload).unwrap();
    let signature = signature.or_else(|| Some(sign(&body)));
    receive(
        State(state.clone()),
        None,
        headers(event, signature),
        Bytes::from(body),
    )
    .await
    .map(|(status, Json(response))| (status, response))
    .map_err(|(status, _)| status)
}

async fn link(db: &StorageBackend, org_id: i64, installation_id: i64) -> uuid::Uuid {
    db.create_organization_connection(CreateOrganizationConnectionRow {
        org_id,
        name: "acme".to_string(),
        provider: GITHUB_PROVIDER.to_string(),
        connection_type: Some(EVERRUNS_APP_CONNECTION_TYPE.to_string()),
        provider_username: Some("acme".to_string()),
        scopes: Some("contents: read".to_string()),
        installation_id: Some(installation_id),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

/// A service identity in `org_id` that chose the link for `installation_id`.
async fn agent_choice(db: &StorageBackend, org_id: i64, installation_id: i64) -> VirtualUserId {
    let identity_id = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        usage: "service".to_string(),
        org_id,
        id: identity_id,
        name: format!("agent-{}", identity_id.uuid().simple()),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
        virtual_user_id: identity_id,
        provider: GITHUB_PROVIDER.to_string(),
        connection_type: EVERRUNS_APP_CONNECTION_TYPE.to_string(),
        provider_user_id: None,
        provider_username: Some("acme".to_string()),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        scopes: Some("contents: read".to_string()),
        expires_at: None,
        installation_id: Some(installation_id),
        provider_metadata: None,
    })
    .await
    .unwrap();
    identity_id
}

fn installation_event(action: &str, installation_id: i64) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "installation": {
            "id": installation_id,
            "permissions": {"contents": "read", "pull_requests": "write"}
        }
    })
}

#[tokio::test]
async fn deliveries_without_a_valid_signature_are_refused() {
    let db = Arc::new(StorageBackend::test_database());
    link(&db, DEFAULT_ORG_ID, LINKED).await;
    let payload = installation_event("deleted", LINKED);

    // No secret configured: fail closed, even with a signature that would match.
    let unset = state(&db, None);
    assert_eq!(
        deliver(&unset, "installation", payload.clone(), None).await,
        Err(StatusCode::UNAUTHORIZED)
    );
    // No shared App configured at all.
    let mut no_app = state(&db, Some(SECRET));
    no_app.everruns_app = None;
    assert_eq!(
        deliver(&no_app, "installation", payload.clone(), None).await,
        Err(StatusCode::UNAUTHORIZED)
    );

    let configured = state(&db, Some(SECRET));
    for bad in ["sha256=00".to_string(), "sha1=abc".to_string()] {
        assert_eq!(
            deliver(&configured, "installation", payload.clone(), Some(bad)).await,
            Err(StatusCode::UNAUTHORIZED)
        );
    }
    // A signature over another body does not verify this one.
    let other = sign(b"{}");
    assert_eq!(
        deliver(&configured, "installation", payload, Some(other)).await,
        Err(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        db.list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .len(),
        1,
        "nothing changed"
    );

    let (status, _) = deliver(&configured, "ping", serde_json::json!({}), None)
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn uninstalling_removes_only_the_linking_orgs_rows() {
    let db = Arc::new(StorageBackend::test_database());
    let other_org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_000000000000000000000000000000c3".to_string(),
            name: "Other Org".to_string(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    link(&db, DEFAULT_ORG_ID, LINKED).await;
    link(&db, other_org, LINKED + 1).await;
    let chose = agent_choice(&db, DEFAULT_ORG_ID, LINKED).await;
    let elsewhere = agent_choice(&db, other_org, LINKED + 1).await;
    let state = state(&db, Some(SECRET));

    // An installation no org links is accepted and dropped.
    let (status, response) = deliver(
        &state,
        "installation",
        installation_event("deleted", 1),
        None,
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(response, GitHubWebhookResponse::default());

    deliver(
        &state,
        "installation",
        installation_event("deleted", LINKED),
        None,
    )
    .await
    .unwrap();
    assert!(
        db.list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db.get_virtual_user_connection(chose, GITHUB_PROVIDER)
            .await
            .unwrap()
            .is_none(),
        "the agent's choice goes with the link"
    );
    assert_eq!(
        db.list_everruns_app_links(other_org).await.unwrap().len(),
        1
    );
    assert!(
        db.get_virtual_user_connection(elsewhere, GITHUB_PROVIDER)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn accepted_permissions_refresh_the_link_and_its_agents() {
    let db = Arc::new(StorageBackend::test_database());
    link(&db, DEFAULT_ORG_ID, LINKED).await;
    let chose = agent_choice(&db, DEFAULT_ORG_ID, LINKED).await;
    let state = state(&db, Some(SECRET));

    deliver(
        &state,
        "installation",
        installation_event("new_permissions_accepted", LINKED),
        None,
    )
    .await
    .unwrap();
    let link = &db.list_everruns_app_links(DEFAULT_ORG_ID).await.unwrap()[0];
    let scopes = link.scopes.as_deref().unwrap();
    assert!(scopes.contains("pull_requests: write"), "{scopes}");
    let agent = db
        .get_virtual_user_connection(chose, GITHUB_PROVIDER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.scopes.as_deref(), Some(scopes));
}

#[tokio::test]
async fn events_route_by_installation_to_the_linking_org() {
    let db = Arc::new(StorageBackend::test_database());
    link(&db, DEFAULT_ORG_ID, LINKED).await;
    let chose = agent_choice(&db, DEFAULT_ORG_ID, LINKED).await;
    agent_choice(&db, DEFAULT_ORG_ID, LINKED + 7).await;

    let recipients =
        GitHubDeliveryRecipients::everruns_app(&db, DEFAULT_ORG_ID, LINKED, Some("everruns"))
            .await
            .unwrap();
    assert_eq!(recipients.identities, vec![chose]);
    assert_eq!(recipients.bot_login.as_deref(), Some("everruns[bot]"));

    // Through the route: no trigger listens yet, so nothing is dispatched,
    // but the delivery is accepted rather than refused.
    let state = state(&db, Some(SECRET));
    let (status, response) = deliver(
        &state,
        "pull_request",
        serde_json::json!({
            "action": "opened",
            "installation": {"id": LINKED},
            "repository": {"full_name": "acme/api"}
        }),
        None,
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(response, GitHubWebhookResponse::default());
}
