use super::*;
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::agent_channels::{CreateAgentChannel, types::CreateAgentChannelRequest};
use crate::domains::common::*;
use crate::storage::{CreateAgentRow, CreateHarnessRow, ObserveHealthIssue, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::AgentId;
use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole, Permission, PermissionResolver};
use serde_json::json;
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn seed_agent(db: &StorageBackend) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "channel-harness".to_string(),
                display_name: Some("Channel Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create harness");
    let public_id = AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: public_id.clone(),
            name: "channel-agent".to_string(),
            display_name: Some("Channel Agent".to_string()),
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    public_id
}

async fn fixture() -> (Arc<StorageBackend>, Ctx, crate::storage::IngressChannelRow) {
    let db = Arc::new(StorageBackend::test_database());
    let agent = seed_agent(&db).await;
    let internal = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
    let channel = CreateAgentChannel {
        agent_id: agent,
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Slack,
            channel_config: json!({"bot_token":"xoxb-test","signing_secret":"test","team_id":"T1"}),
            enabled: true,
        },
    }
    .run(&internal)
    .await
    .unwrap();
    let row = db
        .get_ingress_channel_by_public_id(&channel.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let user = db
        .create_user(crate::storage::CreateUserRow {
            email: "health@example.com".into(),
            name: "Health user".into(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    db.add_organization_member(DEFAULT_ORG_ID, user.id, "owner")
        .await
        .unwrap();
    let ctx = Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_test".into(),
            user_id: Some(user.id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        db.clone(),
        None,
    );
    (db, ctx, row)
}
fn observation(row: &crate::storage::IngressChannelRow, status: &str) -> ObserveHealthIssue {
    ObserveHealthIssue {
        org_id: row.org_id,
        channel_id: row.channel_id,
        channel_revision: row.updated_at,
        status: status.into(),
        missing_scopes: if status == "open" {
            vec!["reactions:write".into()]
        } else {
            vec![]
        },
        error_code: (status == "open").then(|| "missing_scope".into()),
        checked_at: Utc::now(),
    }
}
async fn list(ctx: &Ctx) -> types::HealthIssueList {
    ListHealthIssues::default().run(ctx).await.unwrap()
}

#[tokio::test]
async fn reading_and_repeated_failures_do_not_resolve_or_duplicate_an_issue() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    let first = list(&ctx).await;
    assert_eq!(first.total, 1);
    let notification_id = first.data[0]
        .notification_id
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    db.mark_notification_viewed(ctx.org_id(), ctx.caller.user_id.unwrap(), notification_id)
        .await
        .unwrap();
    for _ in 0..10 {
        db.observe_health_issue(observation(&row, "open"))
            .await
            .unwrap();
    }
    let repeated = list(&ctx).await;
    assert_eq!(repeated.total, 1);
    assert_eq!(repeated.data[0].id, first.data[0].id);
    assert_eq!(
        db.list_notifications(ctx.org_id(), ctx.caller.user_id.unwrap(), 100)
            .await
            .unwrap()
            .len(),
        1
    );
    db.observe_health_issue(observation(&row, "resolved"))
        .await
        .unwrap();
    assert_eq!(list(&ctx).await.total, 0);
    assert_eq!(
        GetHealthIssue {
            issue_id: first.data[0].id
        }
        .run(&ctx)
        .await
        .unwrap()
        .status,
        "resolved"
    );
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    list(&ctx).await;
    assert_eq!(
        db.list_notifications(ctx.org_id(), ctx.caller.user_id.unwrap(), 100)
            .await
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
async fn stale_check_cannot_resolve_replaced_credentials() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    db.update_channel_config_by_id(
        row.channel_id,
        json!({"bot_token":"replacement","signing_secret":"test","team_id":"T1"}),
        None,
    )
    .await
    .unwrap();
    db.observe_health_issue(observation(&row, "resolved"))
        .await
        .unwrap();
    let result = list(&ctx).await;
    assert_eq!(result.total, 1);
    assert!(result.data[0].stale);
}
#[tokio::test]
async fn unknown_probe_preserves_confirmed_failure_and_snooze_is_user_scoped() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    let first = list(&ctx).await;
    let id = first.data[0].id;
    let snoozed = SnoozeHealthIssue { issue_id: id }.run(&ctx).await.unwrap();
    assert!(snoozed.snoozed_until.is_some());
    let mut unknown = observation(&row, "needs_check");
    unknown.error_code = Some("verification_unavailable".into());
    db.observe_health_issue(unknown).await.unwrap();
    let result = list(&ctx).await;
    assert_eq!(result.total, 1);
    assert_eq!(result.data[0].missing_scopes, vec!["reactions:write"]);
    assert!(result.data[0].stale);
    assert!(
        db.health_issue_snooze(
            id,
            uuid::Uuid::now_v7(),
            db.get_health_issue(ctx.org_id(), id)
                .await
                .unwrap()
                .unwrap()
                .episode_id,
            None
        )
        .await
        .unwrap()
        .is_none()
    );
}
#[tokio::test]
async fn unavailable_verification_does_not_turn_credentials_failure_into_missing_permissions() {
    let (db, ctx, row) = fixture().await;
    let mut rejected = observation(&row, "open");
    rejected.missing_scopes.clear();
    rejected.error_code = Some("token_revoked".into());
    db.observe_health_issue(rejected).await.unwrap();
    let first = list(&ctx).await;
    assert_eq!(first.data[0].title, "Slack credentials need to be renewed");
    let mut unknown = observation(&row, "needs_check");
    unknown.error_code = Some("verification_unavailable".into());
    db.observe_health_issue(unknown).await.unwrap();
    let result = list(&ctx).await;
    assert_eq!(result.data[0].id, first.data[0].id);
    assert_eq!(result.data[0].status, "open");
    assert!(result.data[0].missing_scopes.is_empty());
    assert!(result.data[0].stale);
    assert_eq!(result.data[0].title, "Slack installation needs attention");
    assert!(result.data[0].body.contains("still unresolved"));
}

struct DenyAgents;
impl PermissionResolver for DenyAgents {
    fn has_permission(&self, _: &Caller, permission: &Permission) -> bool {
        permission != &Permission::OrgAgentsManage
    }
    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|p| self.has_permission(caller, p))
            .collect()
    }
}
#[tokio::test]
async fn denied_policy_revoked_membership_and_other_org_cannot_read_health() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    let first = list(&ctx).await;
    let notification = crate::domains::notifications::ListNotifications { limit: None }
        .run(&ctx)
        .await
        .unwrap();
    assert_eq!(notification.unviewed_count, 1);
    let mut other = Ctx::minimal_for_test(Caller::internal(999), db.clone(), None);
    assert!(
        GetHealthIssue {
            issue_id: first.data[0].id
        }
        .run(&other)
        .await
        .is_err()
    );
    other.caller = ctx.caller.clone();
    other.permission_resolver = Arc::new(DenyAgents);
    assert!(ListHealthIssues::default().run(&other).await.is_err());
    let hidden = crate::domains::notifications::ListNotifications { limit: None }
        .run(&other)
        .await
        .unwrap();
    assert!(hidden.data.is_empty());
    assert_eq!(hidden.unviewed_count, 0);
    db.remove_organization_member(ctx.org_id(), ctx.caller.user_id.unwrap())
        .await
        .unwrap();
    assert!(ListHealthIssues::default().run(&ctx).await.is_err());
}
#[tokio::test]
async fn granted_scopes_verify_without_mutating_slack() {
    let (db, ctx, row) = fixture().await;
    let mock = MockServer::start().await;
    let scopes = crate::domains::agent_channels::record::slack_channel::slack_bot_scopes(false);
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-oauth-scopes", scopes.join(","))
                .set_body_json(json!({"ok":true,"team_id":"T1"})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    service::SlackHealthService::new(db.clone(), None)
        .with_api_base(mock.uri())
        .check(&row.channel_public_id)
        .await
        .unwrap();
    assert_eq!(list(&ctx).await.total, 0);
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn absent_scope_header_and_wrong_workspace_never_report_healthy() {
    for body in [
        json!({"ok":true,"team_id":"T1"}),
        json!({"ok":true,"team_id":"T2"}),
    ] {
        let (db, ctx, row) = fixture().await;
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;
        db.observe_health_issue(observation(&row, "open"))
            .await
            .unwrap();
        service::SlackHealthService::new(db.clone(), None)
            .with_api_base(mock.uri())
            .check(&row.channel_public_id)
            .await
            .unwrap();
        let result = list(&ctx).await;
        assert_eq!(result.total, 1);
        assert!(result.data[0].stale);
    }
}

#[tokio::test]
async fn notification_flag_does_not_disable_health_and_due_reminder_is_once() {
    let (db, mut ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    ctx.feature_flags.notifications = false;
    let first = list(&ctx).await;
    assert_eq!(first.total, 1);
    assert!(first.data[0].notification_id.is_none());
    ctx.feature_flags.notifications = true;
    let issue = list(&ctx).await.data.remove(0);
    let notification = issue.notification_id.unwrap().parse().unwrap();
    let user = ctx.caller.user_id.unwrap();
    db.mark_notification_viewed(ctx.org_id(), user, notification)
        .await
        .unwrap();
    let episode = db
        .get_health_issue(ctx.org_id(), issue.id)
        .await
        .unwrap()
        .unwrap()
        .episode_id;
    db.health_issue_snooze(
        issue.id,
        user,
        episode,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await
    .unwrap();
    list(&ctx).await;
    assert_eq!(
        db.count_unviewed_notifications(ctx.org_id(), user)
            .await
            .unwrap(),
        1
    );
    db.mark_notification_viewed(ctx.org_id(), user, notification)
        .await
        .unwrap();
    list(&ctx).await;
    assert_eq!(
        db.count_unviewed_notifications(ctx.org_id(), user)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_manual_checks_share_cooldown() {
    let (db, _ctx, row) = fixture().await;
    let mut input = observation(&row, "open");
    input.checked_at = Utc::now() - chrono::Duration::minutes(1);
    db.observe_health_issue(input).await.unwrap();
    let id = db.list_health_issues(row.org_id, 0, 1, None).await.unwrap()[0].id;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"invalid_auth"})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let service = service::SlackHealthService::new(db.clone(), None).with_api_base(mock.uri());
    let (a, b) = tokio::join!(
        service.check_requested(&row.channel_public_id, id),
        service.check_requested(&row.channel_public_id, id)
    );
    assert_ne!(a.unwrap(), b.unwrap());
}

/// Poll for the issue a detached `record_limit_reached` writes.
async fn org_issue(db: &StorageBackend, org_id: i64) -> crate::storage::HealthIssueRow {
    for _ in 0..100 {
        let rows = db.list_health_issues(org_id, 0, 10, None).await.unwrap();
        if let Some(row) = rows
            .into_iter()
            .find(|row| row.code == active_turns::ACTIVE_TURN_LIMIT)
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no active-turn limit issue was recorded");
}

#[tokio::test]
async fn active_turn_limit_issue_opens_reopens_and_resolves_below_the_limit() {
    use crate::domains::sessions::limits::OrgCaps;
    let (db, ctx, _row) = fixture().await;
    let org_id = ctx.org_id();
    let at_limit = OrgCaps {
        max_concurrent_sessions: 10,
        max_active_turns: 0,
    };
    let under_limit = OrgCaps {
        max_concurrent_sessions: 10,
        max_active_turns: 1,
    };

    active_turns::record_limit_reached(db.clone(), org_id);
    let opened = org_issue(&db, org_id).await;
    assert_eq!(opened.status, "open");
    assert_eq!(opened.channel_id, None);

    // Members see it with no agent or channel attached.
    let listed = list(&ctx).await;
    let issue = listed
        .data
        .iter()
        .find(|issue| issue.code == active_turns::ACTIVE_TURN_LIMIT)
        .expect("org issue listed");
    assert_eq!(issue.title, "Active turn limit reached");
    assert!(issue.agent_id.is_none() && issue.channel_id.is_none());
    assert!(!issue.stale);

    // "Check again" right after detection hits the shared cooldown.
    let error = CheckHealthIssue {
        issue_id: opened.id,
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Wait a few seconds"), "{error}");

    // A refusal burst does not rewrite the still-open issue.
    active_turns::recheck(&db, org_id, at_limit).await.unwrap();
    let same = db
        .get_health_issue(org_id, opened.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same.last_checked_at, opened.last_checked_at);

    active_turns::recheck(&db, org_id, under_limit)
        .await
        .unwrap();
    let resolved = db
        .get_health_issue(org_id, opened.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.status, "resolved");
    assert!(
        !list(&ctx)
            .await
            .data
            .iter()
            .any(|issue| issue.code == active_turns::ACTIVE_TURN_LIMIT)
    );

    // Hitting the limit again reopens the same issue as a new episode.
    active_turns::recheck(&db, org_id, at_limit).await.unwrap();
    let reopened = db
        .get_health_issue(org_id, opened.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, "open");
    assert_ne!(reopened.episode_id, opened.episode_id);
}
