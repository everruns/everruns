//! Manual fire with an event: the event takes its trigger's real path.

use super::*;
use crate::domains::agent_triggers::manual::{ManualFireStatus, ManualTriggerEvent};
use crate::domains::agent_triggers::record::TriggerDeliveryStatus;
use crate::github_apps::{GitHubAppApi, GitHubAppContext, GitHubEndpoints};
use crate::storage::CreateGitHubAppRow;
use everruns_core::connection_services::UserConnectionResolver;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Hands out a fixed token for `acme/api` pull request reads only.
struct PullRequestReader;

#[async_trait]
impl UserConnectionResolver for PullRequestReader {
    async fn get_connection_token(
        &self,
        _: SessionId,
        _: &str,
    ) -> crate::kernel_imports::Result<Option<String>> {
        Ok(None)
    }

    async fn get_github_token_for_owner(
        &self,
        _: i64,
        _: uuid::Uuid,
        access: &everruns_contracts::runtime::GitHubAccess,
    ) -> crate::kernel_imports::Result<Option<String>> {
        let reads_pulls =
            access.permissions.get("pull_requests").map(String::as_str) == Some("read");
        Ok((reads_pulls && access.repositories == ["acme/api"]).then(|| "pr-token".to_string()))
    }
}

struct Fixture {
    ctx: Ctx,
    agent_id: String,
    runner: Arc<RecordingRunner>,
}

async fn fixture(github: Option<&MockServer>) -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let runner = Arc::new(RecordingRunner::default());
    let mut ctx = role_ctx(db.clone(), OrgRole::Owner)
        .with_session_service(Arc::new(SessionService::new(db.clone())))
        .with_message_service(Arc::new(MessageService::new(
            db.clone(),
            runner.clone(),
            EventDelivery::in_memory(),
        )))
        .with_connection_resolver(Some(Arc::new(PullRequestReader)))
        .with_workflow_store(Some(workflow_store(&db)));
    if let Some(github) = github {
        ctx = ctx.with_github_apps(GitHubAppContext {
            api: GitHubAppApi::new(GitHubEndpoints {
                api_url: github.uri(),
                web_url: "https://github.com".into(),
            }),
            api_base_url: "https://api.example".into(),
            frontend_url: "https://app.example".into(),
            shared_app_slug: None,
        });
    }
    Fixture {
        ctx,
        agent_id,
        runner,
    }
}

impl Fixture {
    /// Connect GitHub on the agent and create a GitHub trigger for `acme/api`.
    async fn github_trigger(&self) -> (String, crate::storage::GitHubAppRow) {
        let agent = self
            .ctx
            .db
            .get_agent_by_public_id(DEFAULT_ORG_ID, &self.agent_id)
            .await
            .unwrap()
            .unwrap();
        let (identity_id, _) = ensure_identity_for_agent(&self.ctx.db, DEFAULT_ORG_ID, &agent)
            .await
            .unwrap();
        let app = self
            .ctx
            .db
            .create_github_app(CreateGitHubAppRow {
                id: uuid::Uuid::new_v4(),
                org_id: DEFAULT_ORG_ID,
                virtual_user_id: identity_id,
                app_id: 42,
                slug: "reviewer".to_string(),
                name: "reviewer".to_string(),
                html_url: "https://github.com/apps/reviewer".to_string(),
                owner_login: Some("acme".to_string()),
                client_id: None,
                client_secret_encrypted: None,
                private_key_encrypted: vec![1],
                webhook_secret_encrypted: Some(vec![1]),
                created_by_user_id: None,
            })
            .await
            .unwrap();
        let trigger = CreateAgentTrigger {
            agent_id: self.agent_id.clone(),
            req: CreateAgentTriggerRequest {
                filter: Some(crate::domains::agent_triggers::record::TriggerEventFilter {
                    changes_only: None,
                    conditions: vec![
                        crate::domains::agent_triggers::record::TriggerFilterCondition {
                            path: "payload.pull_request.draft".to_string(),
                            any_of: vec![json!(false)],
                        },
                    ],
                }),
                ..github_req(Some(vec!["acme/api".to_string()]))
            },
        }
        .run(&self.ctx)
        .await
        .expect("create GitHub trigger");
        (trigger.id.to_string(), app)
    }

    async fn fire(
        &self,
        trigger_id: &str,
        event: Option<ManualTriggerEvent>,
    ) -> Result<TriggerAgentTriggerOutput, CommandError> {
        TriggerAgentTriggerNow {
            agent_id: self.agent_id.clone(),
            trigger_id: trigger_id.to_string(),
            event,
        }
        .run(&self.ctx)
        .await
    }

    async fn deliveries(&self, trigger_id: &str) -> Vec<(String, TriggerDeliveryStatus, String)> {
        ListAgentTriggerDeliveries {
            agent_id: self.agent_id.clone(),
            trigger_id: trigger_id.to_string(),
            limit: None,
        }
        .run(&self.ctx)
        .await
        .expect("list deliveries")
        .into_iter()
        .rev()
        .map(|d| (d.source, d.status, d.subject.unwrap_or_default()))
        .collect()
    }
}

fn pull_request(number: u64, draft: bool) -> serde_json::Value {
    json!({
        "number": number,
        "title": "Fix",
        "html_url": format!("https://github.com/acme/api/pull/{number}"),
        "draft": draft,
        "user": {"login": "octo"},
        "base": {"repo": {"full_name": "acme/api"}},
    })
}

#[tokio::test]
async fn a_pull_request_reference_lands_in_that_pull_requests_session() {
    let github = MockServer::start().await;
    for (number, draft) in [(7, false), (8, true)] {
        Mock::given(method("GET"))
            .and(path(format!("/repos/acme/api/pulls/{number}")))
            .and(header("authorization", "Bearer pr-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pull_request(number, draft)))
            .mount(&github)
            .await;
    }
    let fx = fixture(Some(&github)).await;
    let (trigger_id, app) = fx.github_trigger().await;

    // A real delivery for #7 opens the pull request's session.
    let delivered = crate::domains::agent_triggers::github::dispatch_github_delivery(
        &fx.ctx.db,
        fx.ctx.session_service.as_ref().unwrap(),
        fx.ctx.message_service.as_ref().unwrap(),
        &crate::domains::agent_triggers::github::GitHubDeliveryRecipients::agent_app(&app),
        &crate::domains::agent_triggers::github::GitHubDelivery {
            event: "pull_request".to_string(),
            delivery_id: "d1".to_string(),
            payload: json!({
                "action": "opened",
                "number": 7,
                "pull_request": pull_request(7, false),
                "repository": {"full_name": "acme/api"},
                "sender": {"login": "octo"},
            }),
        },
        None,
    )
    .await
    .unwrap();
    let Ok(events::TriggerEventOutcome::Dispatched(real)) = &delivered[0].outcome else {
        panic!("real delivery must dispatch");
    };

    let reference = |number| ManualTriggerEvent {
        repository: Some("Acme/API".to_string()),
        pull_request: Some(number),
        ..Default::default()
    };
    let manual = fx.fire(&trigger_id, Some(reference(7))).await.unwrap();
    assert_eq!(manual.status, ManualFireStatus::Dispatched);
    assert_eq!(
        manual.session_id,
        Some(real.session_id),
        "manual review continues the pull request's own session"
    );
    assert!(!manual.created_session);

    // The trigger's filter still applies: a draft is filtered, with the reason.
    let draft = fx.fire(&trigger_id, Some(reference(8))).await.unwrap();
    assert_eq!(draft.status, ManualFireStatus::Filtered);
    assert_eq!(draft.session_id, None);
    assert_eq!(
        draft.reason.as_deref(),
        Some("payload.pull_request.draft is true")
    );

    assert_eq!(
        fx.deliveries(&trigger_id).await,
        vec![
            (
                "github".to_string(),
                TriggerDeliveryStatus::Dispatched,
                "acme/api#7".to_string()
            ),
            (
                "manual".to_string(),
                TriggerDeliveryStatus::Dispatched,
                "acme/api#7".to_string()
            ),
            (
                "manual".to_string(),
                TriggerDeliveryStatus::Filtered,
                "acme/api#8".to_string()
            ),
        ]
    );
    assert_eq!(fx.runner.harness_ids.lock().unwrap().len(), 2);

    // A repository the agent's connection cannot read is refused, not run.
    let err = fx
        .fire(
            &trigger_id,
            Some(ManualTriggerEvent {
                repository: Some("acme/web".to_string()),
                pull_request: Some(1),
                ..Default::default()
            }),
        )
        .await
        .expect_err("no access to acme/web");
    assert!(err.message().contains("acme/web"), "got: {err}");
}

#[tokio::test]
async fn raw_github_events_take_the_same_path_and_unsubscribed_ones_are_refused() {
    let fx = fixture(None).await;
    let (trigger_id, _) = fx.github_trigger().await;
    let event = |action: &str, repo: &str| ManualTriggerEvent {
        name: Some("pull_request".to_string()),
        payload: Some(json!({
            "action": action,
            "number": 3,
            "pull_request": pull_request(3, false),
            "repository": {"full_name": repo},
        })),
        ..Default::default()
    };

    let opened = fx
        .fire(&trigger_id, Some(event("opened", "acme/api")))
        .await
        .unwrap();
    assert_eq!(opened.status, ManualFireStatus::Dispatched);
    assert!(opened.created_session);

    let other = fx
        .fire(&trigger_id, Some(event("opened", "acme/web")))
        .await
        .unwrap();
    assert_eq!(other.status, ManualFireStatus::Filtered);
    assert_eq!(
        other.reason.as_deref(),
        Some("github.repository_key is acme/web")
    );

    let err = fx
        .fire(&trigger_id, Some(event("closed", "acme/api")))
        .await
        .expect_err("unsubscribed action");
    assert!(err.message().contains("pull_request.closed"), "got: {err}");

    let err = fx
        .fire(&trigger_id, None)
        .await
        .expect_err("event triggers need an event");
    assert!(err.message().contains("event"), "got: {err}");
}

#[tokio::test]
async fn webhook_and_schedule_triggers_read_the_manual_event() {
    let fx = fixture(None).await;
    let webhook = CreateAgentTrigger {
        agent_id: fx.agent_id.clone(),
        req: CreateAgentTriggerRequest {
            message: "Ticket {{payload.ticket}} from {{webhook.headers.x-source}}".to_string(),
            event_id_template: Some("{{payload.ticket}}".to_string()),
            ..webhook_req(true)
        },
    }
    .run(&fx.ctx)
    .await
    .unwrap()
    .id
    .to_string();
    let ticket = || ManualTriggerEvent {
        payload: Some(json!({"ticket": 7})),
        headers: Some([("X-Source".to_string(), "test".to_string())].into()),
        ..Default::default()
    };
    let first = fx.fire(&webhook, Some(ticket())).await.unwrap();
    assert_eq!(first.status, ManualFireStatus::Dispatched);
    // The trigger's id template still applies, so the same ticket dedupes.
    let again = fx.fire(&webhook, Some(ticket())).await.unwrap();
    assert_eq!(again.status, ManualFireStatus::Duplicate);
    // An explicit id wins over the template.
    let named = fx
        .fire(
            &webhook,
            Some(ManualTriggerEvent {
                id: Some("retry-1".to_string()),
                ..ticket()
            }),
        )
        .await
        .unwrap();
    assert_eq!(named.status, ManualFireStatus::Dispatched);
    let err = fx
        .fire(
            &webhook,
            Some(ManualTriggerEvent {
                repository: Some("acme/api".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect_err("references are GitHub only");
    assert!(err.message().contains("repository"), "got: {err}");

    let schedule = CreateAgentTrigger {
        agent_id: fx.agent_id.clone(),
        req: create_req("0 9 * * *", "{{payload.team}}", true),
    }
    .run(&fx.ctx)
    .await
    .unwrap()
    .id
    .to_string();
    // The message is only the payload's field: empty without the event's
    // payload, so a dispatch proves the schedule message reads it.
    let run = fx
        .fire(
            &schedule,
            Some(ManualTriggerEvent {
                payload: Some(json!({"team": "core"})),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(run.status, ManualFireStatus::Dispatched);
    assert!(run.session_id.is_some());
    let err = fx
        .fire(&schedule, Some(ManualTriggerEvent::default()))
        .await
        .expect_err("no payload renders an empty message");
    assert!(err.message().contains("empty"), "got: {err}");
    assert_eq!(
        fx.deliveries(&schedule).await,
        vec![
            (
                "manual".to_string(),
                TriggerDeliveryStatus::Dispatched,
                String::new()
            ),
            (
                "manual".to_string(),
                TriggerDeliveryStatus::Failed,
                String::new()
            ),
        ]
    );
}
