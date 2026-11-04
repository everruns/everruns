//! Fixers on health issues (knowledge/operations/health-issues.md, "Fixers").
use super::fixers::*;
use super::tests::{fixture, observation};
use super::*;
use crate::domains::agent_channels::record::slack_provisioning::*;
use crate::domains::common::*;
use crate::storage::{SlackGrantKind, UpsertOrgSlackConnection};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

/// One `update_permissions` call: workspace, app id, scopes.
type Update = (Option<String>, String, Vec<String>);

#[derive(Default)]
struct Provisioner {
    updates: Mutex<Vec<Update>>,
    refuse: Option<&'static str>,
}
#[async_trait]
impl SlackAppProvisioner for Provisioner {
    async fn update_permissions(
        &self,
        _: i64,
        team: Option<&str>,
        app: &str,
        scopes: &[&str],
    ) -> SlackProvisioningResult<()> {
        if let Some(code) = self.refuse {
            return Err(SlackProvisioningError::Rejected(code.into()));
        }
        self.updates.lock().unwrap().push((
            team.map(str::to_owned),
            app.into(),
            scopes.iter().map(|s| s.to_string()).collect(),
        ));
        Ok(())
    }
    async fn create_app(
        &self,
        _: i64,
        _: Option<&str>,
        _: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        unreachable!("a fixer never creates an app")
    }
    async fn delete_app(&self, _: i64, _: Option<&str>, _: &str) -> SlackProvisioningResult<()> {
        unreachable!("a fixer never deletes an app")
    }
    async fn connection_status(
        &self,
        _: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        unreachable!()
    }
}

async fn connect_workspace(db: &crate::storage::StorageBackend, org_id: i64) {
    db.upsert_org_slack_connection(UpsertOrgSlackConnection {
        org_id,
        team_id: "T1".into(),
        team_name: Some("Everruns".into()),
        access_token_encrypted: b"access".to_vec(),
        refresh_token_encrypted: b"refresh".to_vec(),
        access_token_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        grant_kind: SlackGrantKind::ConfigToken,
    })
    .await
    .unwrap();
}

async fn slack_api() -> MockServer {
    let slack = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"team_id":"T1","bot_id":"B0BOT"})),
        )
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/bots.info"))
        .and(body_string_contains("bot=B0BOT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"ok":true,"bot":{"id":"B0BOT","app_id":"A0HANDMADE"}}),
        ))
        .mount(&slack)
        .await;
    slack
}

#[tokio::test]
async fn hand_configured_app_gets_its_permissions_added_and_an_install_page() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    connect_workspace(&db, ctx.org_id()).await;
    let provisioner = Arc::new(Provisioner::default());
    let ctx = ctx.with_slack_provisioner(Some(provisioner.clone()));
    let issue = ListHealthIssues::default()
        .run(&ctx)
        .await
        .unwrap()
        .data
        .remove(0);
    let [fixer] = issue.fixers.as_slice() else {
        panic!("one fixer expected, got {:?}", issue.fixers);
    };
    assert_eq!(fixer.kind, SLACK_ADD_PERMISSIONS);
    assert_eq!(fixer.effect, "consent");
    assert_eq!(fixer.unavailable_reason, None);
    assert!(fixer.steps[0].contains("reactions:write"));

    let slack = slack_api().await;
    let outcome = fixers::run_fixer(&ctx, issue.id, SLACK_ADD_PERMISSIONS, &slack.uri())
        .await
        .unwrap();
    assert_eq!(outcome.status, "consent");
    assert!(!outcome.returns_automatically);
    assert_eq!(
        outcome.consent_url.as_deref(),
        Some("https://api.slack.com/apps/A0HANDMADE/install-on-team")
    );
    let updates = provisioner.updates.lock().unwrap().clone();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0.as_deref(), Some("T1"));
    assert_eq!(updates[0].1, "A0HANDMADE");
    assert!(updates[0].2.iter().any(|s| s == "reactions:write"));
    // The fix does not resolve the issue; only a fresh check does.
    assert_eq!(outcome.issue.status, "open");
    let recorded = db
        .list_finding_actions(ctx.org_id(), issue.id)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].kind, SLACK_ADD_PERMISSIONS);
}

#[tokio::test]
async fn fix_is_unavailable_until_the_workspace_is_connected() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    let ctx = ctx.with_slack_provisioner(Some(Arc::new(Provisioner::default())));
    let issue = ListHealthIssues::default()
        .run(&ctx)
        .await
        .unwrap()
        .data
        .remove(0);
    let reason = issue.fixers[0]
        .unavailable_reason
        .clone()
        .expect("unavailable");
    assert!(reason.contains("Connect this Slack workspace"), "{reason}");
    let refused = fixers::run_fixer(&ctx, issue.id, SLACK_ADD_PERMISSIONS, "http://127.0.0.1:9")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("Connect this Slack workspace"),
        "{refused}"
    );
}

#[tokio::test]
async fn deployment_without_provisioning_offers_manual_steps_only() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    let issue = ListHealthIssues::default()
        .run(&ctx)
        .await
        .unwrap()
        .data
        .remove(0);
    assert!(issue.fixers[0].unavailable_reason.is_some());
}

#[tokio::test]
async fn slack_refusing_the_edit_points_to_the_manual_steps() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    connect_workspace(&db, ctx.org_id()).await;
    let ctx = ctx.with_slack_provisioner(Some(Arc::new(Provisioner {
        refuse: Some("app_not_found"),
        ..Default::default()
    })));
    let issue = ListHealthIssues::default()
        .run(&ctx)
        .await
        .unwrap()
        .data
        .remove(0);
    let slack = slack_api().await;
    let refused = fixers::run_fixer(&ctx, issue.id, SLACK_ADD_PERMISSIONS, &slack.uri())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("manual steps"), "{refused}");
}

#[tokio::test]
async fn unknown_or_resolved_fixes_are_refused() {
    let (db, ctx, row) = fixture().await;
    db.observe_health_issue(observation(&row, "open"))
        .await
        .unwrap();
    connect_workspace(&db, ctx.org_id()).await;
    let ctx = ctx.with_slack_provisioner(Some(Arc::new(Provisioner::default())));
    let issue = ListHealthIssues::default()
        .run(&ctx)
        .await
        .unwrap()
        .data
        .remove(0);
    assert!(
        fixers::run_fixer(&ctx, issue.id, "shell.run", "http://127.0.0.1:9")
            .await
            .is_err()
    );
    db.observe_health_issue(observation(&row, "resolved"))
        .await
        .unwrap();
    let resolved = GetHealthIssue { issue_id: issue.id }
        .run(&ctx)
        .await
        .unwrap();
    assert!(resolved.fixers.is_empty());
    assert!(
        fixers::run_fixer(&ctx, issue.id, SLACK_ADD_PERMISSIONS, "http://127.0.0.1:9")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_lookup_reports_slack_refusals() {
    let slack = MockServer::start().await;
    Mock::given(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":false,"error":"invalid_auth"})),
        )
        .mount(&slack)
        .await;
    assert_eq!(
        slack_app_id(&slack.uri(), "xoxb-x").await,
        Err(AppLookupError::Rejected("invalid_auth".into()))
    );
}

#[test]
fn install_page_accepts_only_slack_app_ids() {
    assert_eq!(
        install_page("A0123ABC").unwrap(),
        "https://api.slack.com/apps/A0123ABC/install-on-team"
    );
    for bad in ["", "A", "B0123", "A01/../x", "a0123", "A0123?x=1"] {
        assert!(install_page(bad).is_err(), "{bad}");
    }
}
