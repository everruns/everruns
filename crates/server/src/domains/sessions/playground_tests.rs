use super::playground::*;
use super::{CreateSession, ListSessions, SessionFilterArgs, SessionService};
use crate::api::sessions::CreateSessionRequest;
use crate::domains::common::{Command, Ctx};
use crate::storage::{StorageBackend, models::CreateUserRow};
use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
use everruns_platform::{FeatureFlags, SessionSource};
use std::sync::Arc;
use uuid::Uuid;

async fn fixture(role: OrgRole) -> Ctx {
    let db = Arc::new(StorageBackend::in_memory());
    crate::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
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
    caller.role = role;
    let service = Arc::new(SessionService::new(db.clone()));
    Ctx::minimal_for_test(caller, db, None).with_session_service(service)
}

fn request() -> CreateSessionRequest {
    CreateSessionRequest {
        source: Some(SessionSource::Playground),
        harness_name: Some("generic".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn playground_is_flag_gated_and_defaults_to_the_operator() {
    let mut ctx = fixture(OrgRole::Member).await;
    let mut req = request();
    req.harness_name = Some("base".into());
    ctx.feature_flags = FeatureFlags::default();
    let error = CreateSession(req.clone()).run(&ctx).await.unwrap_err();
    assert!(error.to_string().contains("playground"));
    ctx.feature_flags.playground = true;
    let mut inherited = req.clone();
    inherited.budget_root_session_id = Some(everruns_contracts::typed_id::SessionId::new());
    assert!(CreateSession(inherited).run(&ctx).await.is_err());
    let session = CreateSession(req).run(&ctx).await.unwrap();
    assert_eq!(
        session.playground_user_id,
        Some(
            ctx.db
                .default_virtual_user(ctx.org_id(), ctx.caller.user_id.unwrap())
                .await
                .unwrap()
                .id
        )
    );
    assert_eq!(session.source, SessionSource::Playground);
    assert!(!session.tags.iter().any(|tag| tag == "chat"));
}

#[tokio::test]
async fn another_subject_requires_admin_and_must_be_active_in_org() {
    let mut ctx = fixture(OrgRole::Member).await;
    let other = ctx
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id: ctx.org_id(),
            provider: "test".into(),
            realm: "test".into(),
            subject: "customer".into(),
            name: "Customer".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await
        .unwrap();
    assert!(validate_subject(&ctx, Some(other.id)).await.is_err());
    ctx.caller.role = OrgRole::Admin;
    assert_eq!(
        validate_subject(&ctx, Some(other.id)).await.unwrap(),
        other.id
    );
    assert!(
        validate_subject(
            &ctx,
            Some(everruns_contracts::typed_id::VirtualUserId::new())
        )
        .await
        .is_err()
    );
    for (org_id, usage) in [(ctx.org_id(), "service"), (ctx.org_id() + 1, "end_user")] {
        let id = everruns_contracts::typed_id::VirtualUserId::new();
        ctx.db
            .create_virtual_user(crate::storage::models::CreateVirtualUserRow {
                org_id,
                id,
                usage: usage.into(),
                name: "Excluded subject".into(),
                description: None,
                avatar_url: None,
                locale: None,
                timezone: None,
            })
            .await
            .unwrap();
        assert!(validate_subject(&ctx, Some(id)).await.is_err());
    }
    ctx.db
        .delete_virtual_user(ctx.org_id(), other.id)
        .await
        .unwrap();
    assert!(validate_subject(&ctx, Some(other.id)).await.is_err());
}

#[tokio::test]
async fn playground_is_shared_and_subject_filter_is_server_side() {
    let mut ctx = fixture(OrgRole::Admin).await;
    let session = CreateSession(request()).run(&ctx).await.unwrap();
    let selected = session.playground_user_id.unwrap();
    ctx.caller.user_id = None;
    ctx.caller.role = OrgRole::Member;
    let page = ListSessions {
        filters: SessionFilterArgs {
            source: Some("playground".into()),
            playground_user_id: Some(selected),
            ..Default::default()
        },
        ..Default::default()
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.data[0].id, session.id);
    let page = ListSessions {
        filters: SessionFilterArgs {
            source: Some("playground".into()),
            playground_user_id: Some(everruns_contracts::typed_id::VirtualUserId::new()),
            ..Default::default()
        },
        ..Default::default()
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(page.total, 0);
}

#[tokio::test]
async fn shared_playground_cannot_use_the_personal_operator_harness() {
    let ctx = fixture(OrgRole::Admin).await;
    let mut req = request();
    req.harness_name = Some("platform-chat".into());
    assert!(CreateSession(req).run(&ctx).await.is_err());
    let mut req = request();
    req.source = Some(SessionSource::Chat);
    req.playground_user_id = Some(
        ctx.db
            .default_virtual_user(ctx.org_id(), ctx.caller.user_id.unwrap())
            .await
            .unwrap()
            .id,
    );
    assert!(CreateSession(req).run(&ctx).await.is_err());
}

struct NoopRunner;
#[async_trait::async_trait]
impl everruns_worker::AgentRunner for NoopRunner {
    async fn start_run(
        &self,
        _: i64,
        _: everruns_contracts::typed_id::SessionId,
        _: everruns_contracts::typed_id::HarnessId,
        _: Option<everruns_contracts::typed_id::AgentId>,
        _: everruns_contracts::typed_id::MessageId,
        _: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn resume_after_tool_results(
        &self,
        _: everruns_contracts::typed_id::SessionId,
        _: Uuid,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn cancel_run(&self, _: everruns_contracts::typed_id::SessionId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn is_running(&self, _: everruns_contracts::typed_id::SessionId) -> bool {
        false
    }
    async fn active_count(&self) -> usize {
        0
    }
}

#[tokio::test]
async fn playground_input_records_subject_and_operator_without_management_authority() {
    let mut ctx = fixture(OrgRole::Admin).await;
    let other = ctx
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id: ctx.org_id(),
            provider: "test".into(),
            realm: "test".into(),
            subject: "customer".into(),
            name: "Customer".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await
        .unwrap();
    let mut req = request();
    req.playground_user_id = Some(other.id);
    let session = CreateSession(req).run(&ctx).await.unwrap();
    let service = Arc::new(crate::domains::messages::MessageService::new(
        ctx.db.clone(),
        Arc::new(NoopRunner),
        false,
        crate::event_delivery::EventDelivery::in_memory(),
    ));
    ctx = ctx.with_message_service(service.clone());
    let command = || {
        serde_json::from_value::<crate::domains::messages::CreateMessage>(serde_json::json!({
        "session_id": session.id, "message": {"role":"user", "content":[{"type":"text","text":"Hello"}]}
    })).unwrap()
    };
    let message = command().run(&ctx).await.unwrap();
    assert_eq!(
        ctx.db
            .runtime_invocation_subject(session.id, message.id.uuid())
            .await
            .unwrap(),
        Some(other.id)
    );
    assert_eq!(
        ctx.db
            .runtime_invocation_management_user(session.id, message.id.uuid())
            .await
            .unwrap(),
        None
    );
    assert!(
        ctx.db
            .list_session_participants(ctx.org_id(), session.id)
            .await
            .unwrap()
            .iter()
            .any(|p| p.display_name.as_deref() == Some("Customer"))
    );
    // An ingress adapter with the same subject still cannot bypass the command policy.
    let principal = subject_principal(&ctx, other.id).await.unwrap();
    let input = serde_json::from_value::<crate::api::messages::CreateMessageRequest>(
        serde_json::json!({"message": {"role":"user", "content":[{"type":"text","text":"Bypass"}]}}),
    ).unwrap();
    assert!(
        service
            .create(
                crate::domains::messages::CreateMessageContext {
                    runtime_subject_principal_id: Some(principal),
                    org_id: ctx.org_id(),
                    user_id: ctx.caller.user_id,
                    harness_id: session.harness_id.uuid(),
                    agent_id: session.agent_id.map(|id| id.uuid()),
                    session_id: session.id.uuid(),
                    event_metadata: None,
                    request_id: None,
                },
                input,
            )
            .await
            .is_err()
    );
    ctx.feature_flags.playground = false;
    assert!(command().run(&ctx).await.is_err());
    ctx.feature_flags.playground = true;
    ctx.caller.role = OrgRole::Member;
    assert!(command().run(&ctx).await.is_err());
}
