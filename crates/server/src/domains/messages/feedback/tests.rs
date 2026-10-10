use super::*;
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::SessionSource;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::storage::{CreateEventRow, CreateUserRow, StorageBackend};
use everruns_core::{Caller, OrgRole};
use axum::http::StatusCode;
use std::sync::Arc;
use uuid::Uuid;

const ORG: i64 = 1;

async fn user(db: &StorageBackend) -> Caller {
    let id = Uuid::now_v7();
    db.create_user_with_id(
        id,
        CreateUserRow {
            email: format!("{id}@example.com"),
            name: "Rater".into(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        },
    )
    .await
    .unwrap();
    db.add_organization_member(ORG, id, "owner").await.unwrap();
    Caller {
        org_id: ORG,
        org_public_id: everruns_core::organization::org_public_id_from_internal(ORG),
        user_id: Some(id),
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
    }
}

fn ctx(db: &Arc<StorageBackend>, caller: Caller) -> Ctx {
    Ctx::minimal(
        caller,
        db.clone(),
        None,
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
    .with_session_service(Arc::new(SessionService::new(db.clone())))
}

/// A session holding one user message `msg_user` and one reply `msg_agent`.
async fn session_with_reply(db: &Arc<StorageBackend>, caller: &Caller) -> String {
    crate::setup::org_init::initialize_org_harnesses(db, ORG)
        .await
        .unwrap();
    let harness_id = crate::setup::org_init::base_harness_id(db, ORG)
        .await
        .unwrap();
    let session = SessionService::new(db.clone())
        .create(
            caller,
            harness_id.uuid(),
            None,
            None,
            SessionSource::Api,
            CreateSessionRequest {
                harness_id: Some(harness_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for (event_type, id) in [
        ("input.message", "msg_user"),
        ("output.message.completed", "msg_agent"),
    ] {
        db.create_event(CreateEventRow {
            session_id: session.id,
            event_type: event_type.to_string(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({ "message": { "id": id } }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }
    session.id.to_string()
}

fn rate(session_id: &str, message_id: &str, rating: Option<MessageRating>) -> SetMessageFeedback {
    SetMessageFeedback {
        session_id: session_id.to_string(),
        message_id: message_id.to_string(),
        rating,
        comment: None,
    }
}

#[tokio::test]
async fn rating_replaces_and_clearing_removes_only_your_own() {
    let db = Arc::new(StorageBackend::test_database());
    let alice = user(&db).await;
    let bob = user(&db).await;
    let session_id = session_with_reply(&db, &alice).await;
    let as_alice = ctx(&db, alice);
    let as_bob = ctx(&db, bob);

    rate(&session_id, "msg_agent", Some(MessageRating::Good))
        .execute(&as_alice)
        .await
        .unwrap();
    let mut bad = rate(&session_id, "msg_agent", Some(MessageRating::Bad));
    bad.comment = Some("  Ignored the failing test  ".into());
    let saved = bad.execute(&as_alice).await.unwrap();
    assert_eq!(saved.rating, Some(MessageRating::Bad));
    assert_eq!(saved.comment.as_deref(), Some("Ignored the failing test"));
    rate(&session_id, "msg_agent", Some(MessageRating::Good))
        .execute(&as_bob)
        .await
        .unwrap();

    let list = |ctx: &Ctx| {
        let session_id = session_id.clone();
        let ctx = ctx.clone();
        async move {
            ListMessageFeedback { session_id }
                .execute(&ctx)
                .await
                .unwrap()
        }
    };
    let mine = list(&as_alice).await;
    assert_eq!(mine.len(), 1, "one row per person and message");
    assert_eq!(mine[0].rating, Some(MessageRating::Bad));

    let cleared = rate(&session_id, "msg_agent", None)
        .execute(&as_alice)
        .await
        .unwrap();
    assert_eq!(cleared.rating, None);
    assert!(list(&as_alice).await.is_empty());
    assert_eq!(list(&as_bob).await[0].rating, Some(MessageRating::Good));
}

#[tokio::test]
async fn rating_needs_a_message_of_the_session_and_a_person() {
    let db = Arc::new(StorageBackend::test_database());
    let alice = user(&db).await;
    let session_id = session_with_reply(&db, &alice).await;

    let missing = rate(&session_id, "msg_elsewhere", Some(MessageRating::Good))
        .execute(&ctx(&db, alice.clone()))
        .await
        .unwrap_err();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let mut too_long = rate(&session_id, "msg_user", Some(MessageRating::Bad));
    too_long.comment = Some("x".repeat(MAX_COMMENT_CHARS + 1));
    let error = too_long.execute(&ctx(&db, alice)).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let error = rate(&session_id, "msg_user", Some(MessageRating::Good))
        .execute(&ctx(&db, Caller::internal(ORG)))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}
