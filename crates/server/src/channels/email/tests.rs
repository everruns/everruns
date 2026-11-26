use super::queue::*;
use crate::domains::organizations::record::generate_org_public_id;
use crate::storage::{CreateOrganizationConnectionRow, CreateOrganizationRow, StorageBackend};
use channelkit_agentmail::{EmailEvent, EmailMessage};
use uuid::Uuid;

pub(super) async fn fixture() -> (StorageBackend, i64, Uuid) {
    let db = StorageBackend::test_database();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: generate_org_public_id(),
            name: "Email queue test".into(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    let account = db
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: org,
            name: "Test".into(),
            provider: "agentmail_inbox".into(),
            access_token_encrypted: Some(vec![1]),
            ..Default::default()
        })
        .await
        .unwrap();
    let inbox = Uuid::now_v7();
    sqlx::query("INSERT INTO email_inboxes (id,org_id,account_id,provider,provider_inbox_id,address,name,signing_secret_encrypted,enabled) VALUES ($1,$2,$3,'agentmail','support@agentmail.to','support@agentmail.to','test',$4,true)").bind(inbox).bind(org).bind(account.id).bind(vec![1u8]).execute(db.pool()).await.unwrap();
    (db, org, inbox)
}
fn event(i: usize) -> EmailEvent {
    EmailEvent {
        event_id: format!("event{i}"),
        event_type: "message.received".into(),
        message: EmailMessage {
            inbox_id: "support@agentmail.to".into(),
            message_id: format!("message{i}"),
            thread_id: format!("thread{i}"),
            from: "alice@example.com".into(),
            subject: "Support".into(),
            preview: "Please help".into(),
            text: Some("Body".into()),
            ..Default::default()
        },
    }
}

#[tokio::test]
async fn burst_is_durable_bounded_deduplicated_and_tenant_scoped() {
    let (db, org, inbox) = fixture().await;
    let q = EmailQueue::new(db.pool().clone());
    // Concurrent arrivals use the same database quota fence as other replicas.
    for chunk in (0..1000).collect::<Vec<_>>().chunks(50) {
        let work = chunk.iter().map(|i| {
            let q = q.clone();
            let event = event(*i);
            async move { q.ingest(org, inbox, &event).await.unwrap() }
        });
        assert!(
            futures::future::join_all(work)
                .await
                .into_iter()
                .all(|inserted| inserted)
        );
    }
    assert!(!q.ingest(org, inbox, &event(0)).await.unwrap());
    let mut replay = event(0);
    replay.event_id = "another_delivery".into();
    assert!(!q.ingest(org, inbox, &replay).await.unwrap());
    assert!(q.list(org + 1, inbox, None).await.unwrap().is_empty());
    let batch = q.claim(org, inbox).await.unwrap().unwrap();
    assert!(batch.messages.len() <= 25);
    assert!(
        batch
            .messages
            .iter()
            .map(|m| m.sender.len() + m.subject.len() + m.preview.len() + 256)
            .sum::<usize>()
            <= BATCH_BYTES as usize
    );
    assert!(q.claim(org, inbox).await.unwrap().is_none());
    let first = batch.messages[0].id;
    assert!(q.message(org + 1, inbox, first).await.unwrap().is_none());
    assert!(
        !q.dispose(
            org + 1,
            inbox,
            first,
            batch.lease_id,
            Disposition::Ignore,
            "wrong tenant"
        )
        .await
        .unwrap()
    );
    assert!(
        !q.dispose(
            org,
            inbox,
            first,
            Uuid::new_v4(),
            Disposition::Ignore,
            "stale lease"
        )
        .await
        .unwrap()
    );
    assert!(
        q.dispose(
            org,
            inbox,
            first,
            batch.lease_id,
            Disposition::Ignore,
            "newsletter"
        )
        .await
        .unwrap()
    );
    // Crash after one disposition: the remainder is recovered, never silently acked.
    sqlx::query("UPDATE email_intake SET lease_until=now()-interval '1 second' WHERE lease_id=$1")
        .bind(batch.lease_id)
        .execute(db.pool())
        .await
        .unwrap();
    let recovered = EmailQueue::new(db.pool().clone())
        .claim(org, inbox)
        .await
        .unwrap()
        .unwrap();
    assert!(!recovered.messages.iter().any(|m| m.id == first));
    assert!(
        !q.dispose(
            org,
            inbox,
            recovered.messages[0].id,
            batch.lease_id,
            Disposition::Ignore,
            "expired"
        )
        .await
        .unwrap()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_intake WHERE org_id=$1")
        .bind(org)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1000);
    // Pagination remains complete when timestamps tie during a burst.
    let mut before = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = q.list(org, inbox, before).await.unwrap();
        if page.is_empty() {
            break;
        }
        before = page.last().map(|m| m.id);
        for row in page {
            assert!(seen.insert(row.id));
        }
    }
    assert_eq!(seen.len(), 1000);
}

#[tokio::test]
async fn poison_mail_is_held_after_finite_retries_and_disabled_inboxes_do_not_run() {
    let (db, org, inbox) = fixture().await;
    let q = EmailQueue::new(db.pool().clone());
    for i in 0..25 {
        q.ingest(org, inbox, &event(i)).await.unwrap();
    }
    for _ in 0..3 {
        let batch = q.claim(org, inbox).await.unwrap().unwrap();
        sqlx::query(
            "UPDATE email_intake SET lease_until=now()-interval '1 second' WHERE lease_id=$1",
        )
        .bind(batch.lease_id)
        .execute(db.pool())
        .await
        .unwrap();
    }
    assert!(q.claim(org, inbox).await.unwrap().is_none());
    assert!(
        q.list(org, inbox, None)
            .await
            .unwrap()
            .iter()
            .all(|m| m.state == "held")
    );
    for i in 25..50 {
        q.ingest(org, inbox, &event(i)).await.unwrap();
    }
    sqlx::query("UPDATE email_inboxes SET enabled=false WHERE id=$1")
        .bind(inbox)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(q.claim(org, inbox).await.unwrap().is_none());
}

#[test]
fn preview_removes_controls_without_splitting_unicode() {
    assert_eq!(excerpt("Привіт", 5), "Пр");
    assert_eq!(excerpt("hello\0\nworld", 100), "hello world");
}

#[tokio::test]
async fn email_webhook_verifies_raw_bytes_before_persisting_and_acknowledging() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::Engine;
    use hmac::{Hmac, KeyInit, Mac};
    use std::sync::Arc;
    use tower::ServiceExt;
    let (db, org, inbox) = fixture().await;
    let encryption = Arc::new(
        crate::storage::EncryptionService::new(
            "test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .unwrap(),
    );
    let key = [7u8; 32];
    let secret = format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode(key)
    );
    sqlx::query("UPDATE email_inboxes SET signing_secret_encrypted=$2 WHERE id=$1")
        .bind(inbox)
        .bind(encryption.encrypt_string(&secret).unwrap())
        .execute(db.pool())
        .await
        .unwrap();
    let db = Arc::new(db);
    let auth = crate::auth::middleware::AuthState::builtin(
        crate::auth::AuthConfig {
            mode: crate::auth::config::AuthMode::Full,
            ..Default::default()
        },
        db.clone(),
    );
    let app = super::routes::routes(super::routes::AppState::new(
        db.clone(),
        Some(encryption),
        auth,
        everruns_contracts::connector::ConnectorRegistry::default(),
    ));
    let event = event(1);
    let body = serde_json::to_vec(&event).unwrap();
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let mut hmac = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
    hmac.update(format!("delivery.{timestamp}.").as_bytes());
    hmac.update(&body);
    let signature = format!(
        "v1,{}",
        base64::engine::general_purpose::STANDARD.encode(hmac.finalize().into_bytes())
    );
    for payload in [body.clone(), body.clone()] {
        let request = Request::post(format!("/v1/email/inboxes/{inbox}/webhook"))
            .header("svix-id", "delivery")
            .header("svix-timestamp", &timestamp)
            .header("svix-signature", &signature)
            .body(Body::from(payload))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_intake WHERE org_id=$1")
        .bind(org)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let request = Request::post(format!("/v1/email/inboxes/{inbox}/webhook"))
        .header("svix-id", "delivery")
        .header("svix-timestamp", timestamp)
        .header("svix-signature", signature)
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let request = Request::get("/v1/platform/email/inboxes")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn email_run_claim_is_bounded_and_recovers_the_same_identity() {
    let (db, org, inbox) = fixture().await;
    let queue = EmailQueue::new(db.pool().clone());
    for i in 0..2 {
        queue.ingest(org, inbox, &event(i)).await.unwrap();
    }
    sqlx::query("UPDATE email_intake SET state='ready' WHERE org_id=$1")
        .bind(org)
        .execute(db.pool())
        .await
        .unwrap();
    let (first, lease) = queue.claim_run(org, inbox).await.unwrap().unwrap();
    assert!(queue.claim_run(org, inbox).await.unwrap().is_none());
    sqlx::query(
        "UPDATE email_intake SET session_id=$2,lease_until=now()-interval '1 second' WHERE id=$1",
    )
    .bind(first)
    .bind(Uuid::now_v7())
    .execute(db.pool())
    .await
    .unwrap();
    let (recovered, new_lease) = queue.claim_run(org, inbox).await.unwrap().unwrap();
    assert_eq!(first, recovered);
    assert_ne!(lease, new_lease);
}

#[test]
fn email_admission_requires_authenticated_identity_and_rejects_loops() {
    let mut mail = event(1).message;
    assert!(super::provider::admission(&mail).is_err());
    mail.authentication_results = Some(channelkit_agentmail::AuthenticationResults {
        dmarc: Some("pass".into()),
        dmarc_domain: Some("example.com".into()),
        spf: None,
        dkim: None,
        spf_domain: None,
        dkim_domain: None,
    });
    assert!(super::provider::admission(&mail).is_ok());
    mail.headers
        .insert("Auto-Submitted".into(), "auto-replied".into());
    assert!(super::provider::admission(&mail).is_err());
    mail.headers.clear();
    mail.from = "attacker@other.example".into();
    assert!(super::provider::admission(&mail).is_err());
    mail.from = "Alice <alice@example.com>".into();
    assert!(super::provider::admission(&mail).is_ok());
    mail.from = "Alice\r\nBcc: attacker@example.com <alice@example.com>".into();
    assert!(super::provider::sender(&mail).is_err());
}

#[tokio::test]
async fn email_delivery_stages_one_immutable_reply_after_successful_turn() {
    use everruns_contracts::{
        runtime::channel::DeliveryContext,
        typed_id::{PrincipalId, SessionId},
    };
    use everruns_core::channel_runtime::{DeliveryEvent, DeliveryOptions, TurnDelivery};
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};
    let (db, org, inbox) = fixture().await;
    let harness = Uuid::now_v7();
    db.create_test_harness(org, harness).await;
    let agent = db.create_test_agent(org, Uuid::now_v7()).await;
    sqlx::query("UPDATE agents SET harness_id=$1 WHERE id=$2")
        .bind(harness)
        .bind(agent)
        .execute(db.pool())
        .await
        .unwrap();
    let config = super::EmailChannelConfig {
        inbox_id: inbox,
        purpose: "Support".into(),
        auto_dispatch: false,
        auto_reply: false,
        reply_template_id: None,
    };
    let row = db
        .create_agent_channel(
            org,
            crate::storage::CreateAgentChannelRow {
                agent_id: agent,
                public_id: format!("appchan_{}", Uuid::now_v7().simple()),
                channel_type: "email".into(),
                channel_config: json!(config),
                channel_config_encrypted: None,
                auth: None,
                auth_encrypted: None,
                enabled: true,
                status: "live".into(),
                virtual_user_id: None,
                owner_principal_id: PrincipalId::from_seed(1).uuid(),
                resolved_owner_user_id: None,
            },
        )
        .await
        .unwrap();
    let (context, channel) =
        crate::domains::agent_channels::ingress::row_to_ingress(None, row).unwrap();
    let q = EmailQueue::new(db.pool().clone());
    q.ingest(org, inbox, &event(91)).await.unwrap();
    let id = q.list(org, inbox, None).await.unwrap()[0].id;
    let session = SessionId::from_uuid(Uuid::now_v7());
    sqlx::query("UPDATE email_intake SET state='running',session_id=$2 WHERE id=$1")
        .bind(id)
        .bind(session.uuid())
        .execute(db.pool())
        .await
        .unwrap();
    let db = Arc::new(db);
    let driver = Arc::new(super::driver::EmailDriver {
        db: db.clone(),
        scope: crate::channels::host::ServerChannelScope::new(context, channel),
        config,
    });
    let context = DeliveryContext {
        auth_token: String::new(),
        channel_id: inbox.to_string(),
        thread_ref: "thread91".into(),
        extra: HashMap::from([
            ("email_intake_id".into(), id.to_string()),
            ("session_id".into(), session.to_string()),
        ]),
        receipts: None,
    };
    let event = |kind: &str, data| DeliveryEvent {
        sequence: Some(1),
        event_type: kind.into(),
        data,
        input_message_id: Some(id.to_string()),
        turn_id: None,
        turn_traceparent: None,
    };
    let completed = event(
        "output.message.completed",
        json!({"message":{"id":"reply","content":[{"type":"text","text":"We can help <safely>."}]}}),
    );
    for _ in 0..2 {
        let mut delivery = TurnDelivery::new(
            driver.clone(),
            context.clone(),
            session,
            id.to_string(),
            DeliveryOptions::default(),
        );
        delivery.observe(&completed).await;
        // A reply is staged only at the successful turn boundary.
        delivery.observe(&event("turn.completed", json!({}))).await;
        assert!(delivery.is_finished());
    }
    let rows: Vec<(serde_json::Value, String)> =
        sqlx::query_as("SELECT body,state FROM email_outbox WHERE intake_id=$1")
            .bind(id)
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "held");
    assert_eq!(rows[0].0["to"], json!(["alice@example.com"]));
    assert_eq!(rows[0].0["text"], "We can help <safely>.");
    assert!(
        rows[0].0["html"]
            .as_str()
            .unwrap()
            .contains("&lt;safely&gt;")
    );
    let state: String = sqlx::query_scalar("SELECT state FROM email_intake WHERE id=$1")
        .bind(id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(state, "completed");
}

#[tokio::test]
async fn email_retention_preserves_receipts_and_held_work() {
    let (db, org, inbox) = fixture().await;
    let q = EmailQueue::new(db.pool().clone());
    q.ingest(org, inbox, &event(1)).await.unwrap();
    q.ingest(org, inbox, &event(2)).await.unwrap();
    sqlx::query("UPDATE email_intake SET state=CASE WHEN message_id='message1' THEN 'completed' ELSE 'held' END,updated_at=now()-interval '31 days'").execute(db.pool()).await.unwrap();
    retain(db.pool()).await.unwrap();
    assert_eq!(q.list(org, inbox, None).await.unwrap().len(), 1);
    assert!(!q.ingest(org, inbox, &event(1)).await.unwrap());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_intake WHERE org_id=$1")
        .bind(org)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 2);
}
