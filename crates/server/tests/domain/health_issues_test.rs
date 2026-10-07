use crate::test_harness::TestServer;
use axum::http::StatusCode;
use chrono::Utc;
use everruns_server::storage::ObserveHealthIssue;
use serde_json::{Value, json};

#[tokio::test]
async fn health_lifecycle_postgres() {
    lifecycle(TestServer::new().await).await;
}

#[tokio::test]
async fn health_lifecycle_memory() {
    lifecycle(TestServer::in_memory().await).await;
}

async fn lifecycle(server: TestServer) {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({"name":format!("health-{}", uuid::Uuid::now_v7()),"system_prompt":"test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let channel: Value = server.post(&format!("/v1/agents/{agent_id}/channels"),
        json!({"channel_type":"slack","channel_config":{"bot_token":"xoxb-test","signing_secret":"test","team_id":"T1"}}))
        .await.assert_status(StatusCode::CREATED).json();
    let channel_id = channel["id"].as_str().unwrap();
    let row = server
        .db
        .get_ingress_channel_by_public_id(channel_id)
        .await
        .unwrap()
        .unwrap();
    let observe = |status: &str| ObserveHealthIssue {
        org_id: row.org_id,
        channel_id: row.channel_id,
        channel_revision: row.updated_at,
        status: status.into(),
        missing_scopes: if status == "open" {
            vec!["reactions:write".into()]
        } else {
            vec![]
        },
        error_code: None,
        checked_at: Utc::now(),
    };
    for _ in 0..3 {
        server
            .db
            .observe_health_issue(observe("open"))
            .await
            .unwrap();
    }
    let url = format!("/v1/health-issues?channel_id={channel_id}");
    let list: Value = server.get(&url).await.assert_status(StatusCode::OK).json();
    assert_eq!(list["total"], 1);
    assert_eq!(list["data"][0]["agent_id"], agent_id);
    let id = list["data"][0]["id"].as_str().unwrap();
    server
        .get(&format!("/v1/health-issues/{id}"))
        .await
        .assert_status(StatusCode::OK);
    server
        .db
        .observe_health_issue(observe("needs_check"))
        .await
        .unwrap();
    let list: Value = server.get(&url).await.assert_status(StatusCode::OK).json();
    assert_eq!(list["data"][0]["status"], "open");
    assert_eq!(
        list["data"][0]["missing_scopes"],
        json!(["reactions:write"])
    );
    let user = server
        .db
        .create_user(everruns_server::storage::CreateUserRow {
            email: format!("health-{}@example.test", uuid::Uuid::now_v7()),
            name: "Health test".into(),
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
    let canonical = server
        .db
        .get_health_issue(row.org_id, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let notification = || everruns_server::storage::CreateNotificationRow {
        org_id: row.org_id,
        user_id: user.id,
        kind: "health.issue".into(),
        title: "Slack permissions".into(),
        body: "Missing reactions".into(),
        target_type: Some("health_issue".into()),
        target_id: Some(id.into()),
        href: Some(format!("/settings/health?issue={id}")),
        payload: json!({"episode_id":canonical.episode_id}),
        dedupe_key: Some(format!("health:{id}:{}", canonical.episode_id)),
        source: None,
    };
    let alert = server
        .db
        .health_notification(notification())
        .await
        .unwrap()
        .unwrap();
    server
        .db
        .mark_notification_viewed(row.org_id, user.id, alert.id)
        .await
        .unwrap();
    assert_eq!(
        server
            .db
            .health_notification(notification())
            .await
            .unwrap()
            .unwrap()
            .id,
        alert.id
    );
    assert_eq!(
        server
            .db
            .count_unviewed_notifications(row.org_id, user.id)
            .await
            .unwrap(),
        0
    );
    server
        .db
        .health_issue_snooze(
            canonical.id,
            user.id,
            canonical.episode_id,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
    server.db.health_notification(notification()).await.unwrap();
    assert_eq!(
        server
            .db
            .count_unviewed_notifications(row.org_id, user.id)
            .await
            .unwrap(),
        1
    );
    server
        .db
        .update_channel_config_by_id(
            row.channel_id,
            json!({"bot_token":"replacement","signing_secret":"test","team_id":"T1"}),
            None,
        )
        .await
        .unwrap();
    server
        .db
        .observe_health_issue(observe("resolved"))
        .await
        .unwrap();
    let list: Value = server.get(&url).await.assert_status(StatusCode::OK).json();
    assert_eq!(list["total"], 1);
    assert_eq!(list["data"][0]["stale"], true);
    let current = server
        .db
        .get_ingress_channel_by_public_id(channel_id)
        .await
        .unwrap()
        .unwrap();
    let mut fresh = observe("resolved");
    fresh.channel_revision = current.updated_at;
    server.db.observe_health_issue(fresh).await.unwrap();
    let list: Value = server.get(&url).await.assert_status(StatusCode::OK).json();
    assert_eq!(list["total"], 0);
    assert!(
        server
            .db
            .health_notification(notification())
            .await
            .unwrap()
            .is_none()
    );
    let restored = server
        .db
        .get_notification(row.org_id, user.id, alert.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.payload["status"], "resolved");
    assert!(restored.viewed_at.is_some());
    server
        .delete(&format!("/v1/agents/{agent_id}/channels/{channel_id}"))
        .await
        .assert_status(StatusCode::OK);
    server
        .get(&format!("/v1/health-issues/{id}"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
