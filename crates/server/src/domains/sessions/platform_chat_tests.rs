use super::{CreateSession, EnsurePlatformChat, SessionService};
use crate::domains::common::{Command, Ctx};
use crate::domains::sessions::record::SessionSource;
use crate::storage::{CreateUserRow, StorageBackend};
use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn fixture() -> Ctx {
    let db = Arc::new(StorageBackend::test_database());
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let user = Uuid::now_v7();
    db.create_user_with_id(
        user,
        CreateUserRow {
            email: format!("{user}@example.com"),
            name: "Operator".into(),
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
    db.add_organization_member(DEFAULT_ORG_ID, user, "member")
        .await
        .unwrap();
    let mut caller = Caller::internal(DEFAULT_ORG_ID);
    caller.is_internal = false;
    caller.user_id = Some(user);
    caller.role = OrgRole::Member;
    let service = Arc::new(SessionService::new(db.clone()));
    Ctx::minimal_for_test(caller, db, None).with_session_service(service)
}

#[tokio::test]
async fn permanent_chat_is_durable_and_cannot_be_removed_or_reassigned() {
    let ctx = fixture().await;
    let first = EnsurePlatformChat {}.run(&ctx).await.unwrap();
    let second = EnsurePlatformChat {}.run(&ctx).await.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.source, SessionSource::Chat);
    assert!(first.tags.iter().any(|t| t == "platform-chat-starter"));
    let service = SessionService::new(ctx.db.clone());
    assert!(service.archive(&ctx.caller, first.id.uuid()).await.is_err());
    assert!(service.delete(&ctx.caller, first.id.uuid()).await.is_err());
    assert!(
        service
            .unpin(&ctx.caller, ctx.caller.user_id.unwrap(), first.id.uuid())
            .await
            .is_err()
    );
    for patch in [
        json!({"title":"Renamed"}),
        json!({"tags":[]}),
        json!({"virtual_user_id":null}),
    ] {
        assert!(
            service
                .update(
                    &ctx.caller,
                    first.id.uuid(),
                    serde_json::from_value(patch).unwrap()
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn side_chat_starts_empty_on_the_same_agent_and_bashkit_worker() {
    let ctx = fixture().await;
    let main = EnsurePlatformChat {}.run(&ctx).await.unwrap();
    let side = CreateSession(
        serde_json::from_value(
            json!({"source":"chat", "agent_name":"platform-chat", "tags":["chat"]}),
        )
        .unwrap(),
    )
    .run(&ctx)
    .await
    .unwrap();
    assert_ne!(main.id, side.id);
    assert_eq!(main.agent_id, side.agent_id);
    assert_eq!(main.harness_id, side.harness_id);
    assert_eq!(main.resolved_owner_user_id, side.resolved_owner_user_id);
    assert!(!side.tags.iter().any(|t| t == "platform-chat-starter"));
    assert!(
        ctx.db
            .list_events(side.id, None, None, &[], &[], None, None)
            .await
            .unwrap()
            .is_empty()
    );
    for request in [
        json!({"source":"chat", "harness_name":"generic"}),
        json!({"source":"chat", "agent_name":"platform-chat", "capabilities":[{"ref":"platform"}]}),
        json!({"source":"chat", "agent_name":"platform-chat", "system_prompt":"Override"}),
        json!({"source":"chat", "agent_name":"platform-chat", "environment":{"target":{"kind":"vfs", "provider":"bashkit"}}}),
        json!({"source":"chat", "agent_name":"platform-chat", "harness_name":"base"}),
    ] {
        assert!(
            CreateSession(serde_json::from_value(request).unwrap())
                .run(&ctx)
                .await
                .is_err()
        );
    }
    assert!(
        CreateSession(
            serde_json::from_value(json!({"source":"playground", "agent_name":"platform-chat"}))
                .unwrap()
        )
        .run(&ctx)
        .await
        .is_err()
    );
}

#[tokio::test]
async fn platform_chat_forks_cannot_override_the_managed_prompt() {
    let ctx = fixture().await;
    let parent = EnsurePlatformChat {}.run(&ctx).await.unwrap();
    let service = SessionService::new(ctx.db.clone());
    assert!(
        service
            .fork(
                &ctx.caller,
                parent.id,
                super::ForkOverrides {
                    system_prompt: Some("Override".into()),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
}
