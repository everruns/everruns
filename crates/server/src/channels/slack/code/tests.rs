use super::*;
use crate::channels::slack::host_tests::{Slack, slack_config};
use everruns_core::channel::SessionBinding;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path, query_param},
};

struct Fixture {
    slack: Slack,
    config: SlackChannelConfig,
    api: MockServer,
}

impl Fixture {
    async fn new() -> Self {
        let api = MockServer::start().await;
        let mut config = slack_config(SessionBinding::Thread);
        config.code_channels_enabled = true;
        config.channel_id = Some("C_SOURCE".into());
        let mut slack = Slack::new(config.clone()).await;
        slack.state.code_api_base = api.uri();
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"team_id":"T1","user_id":"B_AGENT"})),
            )
            .mount(&api)
            .await;
        Mock::given(method("GET")).and(path("/conversations.info")).and(query_param("channel", "C_SOURCE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"channel":{"id":"C_SOURCE","is_private":true,"is_member":true,"properties":{}}}))).mount(&api).await;
        Mock::given(method("GET"))
            .and(path("/conversations.info"))
            .and(query_param("channel", "C_CODE"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"channel":code_info("B_AGENT")})),
            )
            .mount(&api)
            .await;
        Mock::given(method("POST"))
            .and(path("/agents.conversations.create"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"channel_id":"C_CODE"})),
            )
            .mount(&api)
            .await;
        for endpoint in [
            "agents.sessions.setStatus",
            "agents.sessions.rename",
            "agents.conversations.setProperties",
            "agents.conversations.setView",
            "agents.conversations.archive",
            "chat.update",
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/{endpoint}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
                .mount(&api)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"9.0"})))
            .mount(&api)
            .await;
        Mock::given(method("POST"))
            .and(path("/agents.conversations.setView"))
            .and(body_partial_json(json!({"type":"diff"})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"view_id":"V_OWN_DIFF"})),
            )
            .with_priority(2)
            .mount(&api)
            .await;
        Mock::given(method("POST"))
            .and(path("/agents.conversations.removeView"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .mount(&api)
            .await;
        Self { slack, config, api }
    }
    async fn enqueue(&self, body: Value) {
        assert!(
            enqueue(&self.slack.state, &self.slack.channel, &self.config, &body)
                .await
                .unwrap()
        );
    }
    async fn inputs(&self) {
        for _ in 0..12 {
            if !worker::inbox_step(&self.slack.state).await.unwrap() {
                break;
            }
        }
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM slack_code_inbox WHERE NOT done")
                .fetch_one(self.slack.db.pool())
                .await
                .unwrap();
        assert_eq!(pending, 0, "all inputs must be durably admitted or refused");
    }
    async fn task(&self) -> Task {
        sqlx::query_as("SELECT * FROM slack_code_tasks LIMIT 1")
            .fetch_one(self.slack.db.pool())
            .await
            .unwrap()
    }
    async fn start(&self) -> Task {
        self.enqueue(event("app_mention", "C_SOURCE", "1.0", None, "U1"))
            .await;
        self.inputs().await;
        let task = self.task().await;
        assert!(task.session_id.is_some());
        task
    }
    async fn requests(&self, name: &str) -> Vec<Value> {
        self.api
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == format!("/{name}"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    }
    async fn emit(&self, task: &Task, kind: &str, data: Value) {
        self.slack
            .db
            .create_event(crate::storage::CreateEventRow {
                session_id: SessionId::from_uuid(task.session_id.unwrap()),
                event_type: kind.into(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
    }
    async fn project(&self) {
        for _ in 0..40 {
            sqlx::query("UPDATE slack_code_tasks SET next_attempt=NOW()")
                .execute(self.slack.db.pool())
                .await
                .unwrap();
            projection::step(&self.slack.state).await.unwrap();
            let task = self.task().await;
            let latest: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(sequence),0) FROM events WHERE session_id=$1",
            )
            .bind(task.session_id)
            .fetch_one(self.slack.db.pool())
            .await
            .unwrap();
            if task.cursor == i64::from(latest) && task.version == task.published {
                break;
            }
        }
    }
}

fn event(kind: &str, channel: &str, ts: &str, thread: Option<&str>, user: &str) -> Value {
    json!({"type":"event_callback","team_id":"T1","event_id":format!("{kind}:{channel}:{ts}"),"event":{"type":kind,"channel":channel,"ts":ts,"event_ts":ts,"thread_ts":thread,"user":user,"text":"Fix the failing test"}})
}
fn code_info(bot: &str) -> Value {
    json!({"id":"C_CODE","is_member":true,"is_archived":false,"properties":{"record_channel":{"record_type":"agent_channel"},"agent_session":{"encoded_agent_bot_user_ids":[bot],"origin_link":{"channel_id":"C_SOURCE","ts":"1.0"}}}})
}

#[tokio::test]
async fn mention_duplicates_reverse_order_and_crash_replay_create_one_private_task() {
    let f = Fixture::new().await;
    f.enqueue(event("message", "C_SOURCE", "1.0", None, "U1"))
        .await;
    f.inputs().await;
    let task = f.start().await;
    f.enqueue(event("app_mention", "C_SOURCE", "1.0", None, "U1"))
        .await;
    f.inputs().await;
    assert_eq!(f.requests("agents.conversations.create").await.len(), 1);
    // Simulate a crash after remote creation but before its local commit. The
    // session binding and inbox survive independently of that transaction.
    sqlx::query("UPDATE slack_code_tasks SET slack_channel=NULL,session_id=NULL")
        .execute(f.slack.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE slack_code_inbox SET done=FALSE,next_attempt=NOW()")
        .execute(f.slack.db.pool())
        .await
        .unwrap();
    f.inputs().await;
    let recovered = f.task().await;
    assert_eq!(recovered.session_id, task.session_id);
    let creates = f.requests("agents.conversations.create").await;
    assert_eq!(creates.len(), 2);
    assert_eq!(creates[0]["session_id"], creates[1]["session_id"]);
    assert!(creates.iter().all(|r| r.get("is_private").is_none()));
    assert_eq!(creates[0]["origin_channel_id"], "C_SOURCE");
    let inputs = f
        .slack
        .db
        .list_events(
            SessionId::from_uuid(task.session_id.unwrap()),
            None,
            None,
            &["input.message".into()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        inputs.len(),
        1,
        "crash replay must not execute a second input"
    );
}

#[tokio::test]
async fn teammates_and_thread_replies_use_the_same_session_and_keep_identity() {
    let f = Fixture::new().await;
    let task = f.start().await;
    f.enqueue(event("message", "C_CODE", "2.0", None, "U2"))
        .await;
    f.enqueue(event("message", "C_CODE", "3.0", Some("2.0"), "U3"))
        .await;
    f.inputs().await;
    assert_eq!(f.task().await.session_id, task.session_id);
    assert_eq!(f.requests("agents.conversations.create").await.len(), 1);
    let events = f
        .slack
        .db
        .list_events(
            SessionId::from_uuid(task.session_id.unwrap()),
            None,
            None,
            &["input.message".into()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let actors: Vec<_> = events
        .iter()
        .map(|e| {
            e.data["message"]["external_actor"]["actor_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(actors, vec!["U1", "U2", "U3"]);
    assert_eq!(
        f.task().await.requester,
        "U1",
        "steering never transfers approval authority"
    );
}

#[tokio::test]
async fn assignment_and_source_restrictions_fail_closed() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", "C_OTHER"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"channel":{"id":"C_OTHER","properties":{}}})),
        )
        .mount(&f.api)
        .await;
    f.enqueue(event("app_mention", "C_OTHER", "1.0", None, "U1"))
        .await;
    f.inputs().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM slack_code_tasks")
        .fetch_one(f.slack.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(verify_assignment(&json!({"user_id":"B_AGENT"}), &code_info("B_OTHER")).is_err());
    assert!(verify_assignment(&json!({"user_id":"B_AGENT"}), &code_info("B_AGENT")).is_ok());
    f.enqueue(event("app_mention", "C_CODE", "2.0", None, "U1"))
        .await;
    f.inputs().await;
    assert_eq!(f.task().await.slack_channel.as_deref(), Some("C_CODE"));
    assert!(
        f.requests("agents.conversations.create").await.is_empty(),
        "Slack-created channels are adopted"
    );
}

#[tokio::test]
async fn stopped_delivery_survives_restart_and_duplicate_stops_do_not_stop_a_new_input() {
    let f = Fixture::new().await;
    let task = f.start().await;
    f.emit(&task, "turn.started", json!({})).await;
    f.project().await;
    f.emit(
        &task,
        "output.message.completed",
        json!({"message":{"id":"m1","content":[{"type":"text","text":"Old output"}]}}),
    )
    .await;
    let stop = event("agent_session_stopped", "C_CODE", "2.0", None, "U1");
    f.enqueue(stop.clone()).await;
    assert!(
        store::stopped_at(&f.slack.state, &f.task().await)
            .await
            .unwrap()
            .is_some(),
        "stop must persist before the webhook ack"
    );
    f.inputs().await;
    f.project().await;
    assert!(f.requests("chat.postMessage").await.is_empty());
    f.enqueue(event("message", "C_CODE", "3.0", Some("9.0"), "U2"))
        .await;
    f.inputs().await;
    assert!(!f.task().await.stopped);
    f.enqueue(stop).await;
    f.inputs().await;
    assert!(
        !f.task().await.stopped,
        "a replayed stop cannot cancel newer work"
    );
    f.emit(&task, "turn.started", json!({})).await;
    f.project().await;
    assert_eq!(
        f.requests("agents.sessions.setStatus")
            .await
            .iter()
            .filter(|r| r["status"] == "processing")
            .count(),
        2,
        "resumption must publish processing after Slack automatically clears the stopped status"
    );
    assert!(
        f.requests("chat.postMessage").await.is_empty(),
        "old output must not resurrect"
    );
}

#[tokio::test]
async fn projections_are_bounded_revision_checked_and_recover_after_rate_limits() {
    let f = Fixture::new().await;
    let task = f.start().await;
    let session = SessionId::from_uuid(task.session_id.unwrap());
    let snapshot=json!({"expected_revision":0,"context":[{"key":"repo","label":"Repository","url":"https://github.com/everruns/everruns"}],"diff":"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n","base_branch":"main","head_branch":"fix","summary":"Tests pass"}).to_string();
    assert_eq!(
        request(&f.slack.db, task.org_id, session, &snapshot)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        request(&f.slack.db, task.org_id, session, &snapshot)
            .await
            .unwrap(),
        1,
        "identical retry returns its receipt"
    );
    let stale = json!({"expected_revision":0,"summary":"Stale","context":[]}).to_string();
    assert!(
        request(&f.slack.db, task.org_id, session, &stale)
            .await
            .is_err()
    );
    let mut big: Value = serde_json::from_str(&snapshot).unwrap();
    big["diff"] = json!("x".repeat(1_000_001));
    assert!(
        request(&f.slack.db, task.org_id, session, &big.to_string())
            .await
            .is_err()
    );
    Mock::given(method("POST"))
        .and(path("/agents.conversations.setProperties"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "60")
                .set_body_json(json!({"ok":false,"error":"ratelimited"})),
        )
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&f.api)
        .await;
    f.project().await;
    let props = f.requests("agents.conversations.setProperties").await;
    assert!(
        props.last().unwrap()["code_channel"]["context_bar_items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["key"] == "repo")
    );
    assert_eq!(f.task().await.published, 1);
    let views = f.requests("agents.conversations.setView").await;
    assert!(
        views
            .iter()
            .any(|v| v["type"] == "diff" && v["content"].as_str().unwrap().contains("+new"))
    );
}

#[tokio::test]
async fn final_summary_archives_and_reopen_reuses_the_session() {
    let f = Fixture::new().await;
    let task = f.start().await;
    request(
        &f.slack.db,
        task.org_id,
        SessionId::from_uuid(task.session_id.unwrap()),
        &json!({"expected_revision":0,"context":[],"summary":"Fixed and tested","complete":true})
            .to_string(),
    )
    .await
    .unwrap();
    f.emit(&task, "turn.completed", json!({})).await;
    // Once events and publication settle, completion archives on the next pass.
    f.project().await;
    f.project().await;
    assert!(f.task().await.archived);
    let archive = f.requests("agents.conversations.archive").await;
    assert_eq!(archive[0]["summary_message_ts"], "9.0");
    f.enqueue(event("channel_unarchive", "C_CODE", "4.0", None, "U1"))
        .await;
    f.inputs().await;
    assert!(!f.task().await.archived);
    assert_eq!(f.task().await.session_id, task.session_id);
}

#[tokio::test]
async fn uninstall_revocation_and_workspace_mismatch_cannot_resume_tasks() {
    let f = Fixture::new().await;
    f.start().await;
    let mut wrong = event("message", "C_CODE", "4.0", None, "U2");
    wrong["team_id"] = json!("T_OTHER");
    assert!(
        !enqueue(&f.slack.state, &f.slack.channel, &f.config, &wrong)
            .await
            .unwrap()
    );
    f.enqueue(json!({"type":"event_callback","team_id":"T1","event_id":"uninstall","event":{"type":"app_uninstalled"}})).await;
    assert!(
        store::stopped_at(&f.slack.state, &f.task().await)
            .await
            .unwrap()
            .is_some()
    );
    f.inputs().await;
    assert!(f.task().await.disabled);
    f.enqueue(event("message", "C_CODE", "5.0", None, "U2"))
        .await;
    f.inputs().await;
    assert!(f.task().await.disabled);
    f.project().await;
    assert!(f.requests("chat.postMessage").await.is_empty());
}

async fn gated(
    f: &Fixture,
    task: &Task,
    arguments: Value,
) -> everruns_core::tool_hooks::PreToolUseDecision {
    use everruns_contracts::tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolHints};
    use everruns_core::capabilities::Capability;
    let definition = ToolDefinition::Builtin(BuiltinTool {
        name: "send_email".into(),
        display_name: None,
        description: "Send".into(),
        parameters: json!({"type":"object"}),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: ToolHints::default().with_open_world(true),
        full_parameters: None,
    });
    let context = everruns_core::tool_context::ToolContext::new(SessionId::from_uuid(
        task.session_id.unwrap(),
    ))
    .with_storage_store_arc(std::sync::Arc::new(
        crate::storage::DbSessionStorageStore::new_without_encryption(
            f.slack.db.database().clone(),
        ),
    ));
    let cap = everruns_core::builtins::ToolApprovalCapability::new(std::sync::Arc::new(
        everruns_core::builtins::DurableToolApprover,
    ));
    cap.pre_tool_use_hooks_with_config(&json!({"mode":"normal"}))[0]
        .before_exec(
            ToolCall {
                id: "call1".into(),
                name: "send_email".into(),
                arguments,
            },
            &definition,
            &context,
        )
        .await
}

async fn park(f: &Fixture, task: &Task, expired: bool) -> Value {
    use everruns_core::tool_hooks::PreToolUseDecision;
    let PreToolUseDecision::Defer { result, .. } =
        gated(f, task, json!({"to":"first@example.com"})).await
    else {
        panic!("must defer an unapproved outward action")
    };
    let mut request =
        everruns_contracts::tool_types::ToolApprovalRequired::from_tool_result(&result).unwrap();
    if expired {
        request.expires_at = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
    }
    let call = request.request_call();
    f.emit(task, "tool.call_requested", json!({"tool_calls":[call]}))
        .await;
    f.slack
        .db
        .update_session(
            task.org_id,
            SessionId::from_uuid(task.session_id.unwrap()),
            crate::storage::UpdateSession {
                status: Some("waiting_for_tool_results".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    json!({"type":"block_actions","team":{"id":task.team_id},"channel":{"id":task.slack_channel},"user":{"id":"U1"},"message":{"ts":"9.0"},"actions":[{"action_id":"everruns_code_allow","value":json!({"task":task.id,"request":call.id,"fingerprint":request.fingerprint}).to_string()}]})
}

#[tokio::test]
async fn hard_approvals_bind_requester_fingerprint_workspace_and_reject_replay() {
    use everruns_core::tool_hooks::PreToolUseDecision;
    let f = Fixture::new().await;
    let task = f.start().await;
    let payload = park(&f, &task, false).await;
    for (field, value) in [
        ("user", json!({"id":"U2"})),
        ("team", json!({"id":"T_OTHER"})),
        ("channel", json!({"id":"C_SOURCE"})),
    ] {
        let mut forged = payload.clone();
        forged[field] = value;
        assert!(
            interaction::handle(&f.slack.state, &f.slack.channel, &f.config, &forged)
                .await
                .unwrap()
        );
        assert!(matches!(
            gated(&f, &task, json!({"to":"first@example.com"})).await,
            PreToolUseDecision::Defer { .. }
        ));
    }
    let mut stale = payload.clone();
    let mut binding: Value =
        serde_json::from_str(stale["actions"][0]["value"].as_str().unwrap()).unwrap();
    binding["fingerprint"] = json!("changed");
    stale["actions"][0]["value"] = json!(binding.to_string());
    interaction::handle(&f.slack.state, &f.slack.channel, &f.config, &stale)
        .await
        .unwrap();
    assert!(matches!(
        gated(&f, &task, json!({"to":"first@example.com"})).await,
        PreToolUseDecision::Defer { .. }
    ));
    interaction::handle(&f.slack.state, &f.slack.channel, &f.config, &payload)
        .await
        .unwrap();
    assert!(matches!(
        gated(&f, &task, json!({"to":"first@example.com"})).await,
        PreToolUseDecision::Continue(_)
    ));
    assert!(matches!(
        gated(&f, &task, json!({"to":"changed@example.com"})).await,
        PreToolUseDecision::Defer { .. }
    ));
    interaction::handle(&f.slack.state, &f.slack.channel, &f.config, &payload)
        .await
        .unwrap();
    let events = f
        .slack
        .db
        .list_events(
            SessionId::from_uuid(task.session_id.unwrap()),
            None,
            None,
            &["tool.completed".into()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.data["tool_name"] == "approve_tool_call")
            .count(),
        1
    );
    let inputs = f
        .slack
        .db
        .list_events(
            SessionId::from_uuid(task.session_id.unwrap()),
            None,
            None,
            &["input.message".into()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        inputs.last().unwrap().data["message"]["external_actor"]["actor_id"],
        "U1"
    );
}

#[tokio::test]
async fn expired_hard_approval_never_releases_execution() {
    use everruns_core::tool_hooks::PreToolUseDecision;
    let f = Fixture::new().await;
    let task = f.start().await;
    let payload = park(&f, &task, true).await;
    interaction::handle(&f.slack.state, &f.slack.channel, &f.config, &payload)
        .await
        .unwrap();
    assert!(matches!(
        gated(&f, &task, json!({"to":"first@example.com"})).await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn stop_ingress_does_not_wait_for_a_projection_lock() {
    let f = Fixture::new().await;
    let task = f.start().await;
    let mut in_flight = everruns_durable::begin_transaction(f.slack.db.pool())
        .await
        .unwrap();
    let _: Uuid = sqlx::query_scalar("SELECT id FROM slack_code_tasks WHERE id=$1 FOR UPDATE")
        .bind(task.id)
        .fetch_one(&mut *in_flight)
        .await
        .unwrap();
    let stopped = tokio::time::timeout(
        Duration::from_secs(2),
        f.enqueue(event(
            "agent_session_stopped",
            "C_SOURCE",
            "7.0",
            Some("1.0"),
            "U1",
        )),
    )
    .await;
    in_flight.rollback().await.unwrap();
    assert!(
        stopped.is_ok(),
        "stop acknowledgment must not wait for the network transaction"
    );
    assert!(
        store::stopped_at(&f.slack.state, &task)
            .await
            .unwrap()
            .is_some()
    );
    f.inputs().await;
}

#[tokio::test]
async fn retry_after_is_persisted_and_no_call_is_made_before_it() {
    let f = Fixture::new().await;
    let task = f.start().await;
    f.project().await;
    request(
        &f.slack.db,
        task.org_id,
        SessionId::from_uuid(task.session_id.unwrap()),
        &json!({"expected_revision":0,"context":[],"summary":"Updated"}).to_string(),
    )
    .await
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/agents.conversations.setProperties"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "60")
                .set_body_json(json!({"ok":false,"error":"ratelimited"})),
        )
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&f.api)
        .await;
    sqlx::query("UPDATE slack_code_tasks SET next_attempt=NOW() WHERE id=$1")
        .bind(task.id)
        .execute(f.slack.db.pool())
        .await
        .unwrap();
    projection::step(&f.slack.state).await.unwrap();
    let delay:f64=sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM (next_attempt-NOW()))::double precision FROM slack_code_tasks WHERE id=$1")
        .bind(task.id).fetch_one(f.slack.db.pool()).await.unwrap();
    assert!(delay > 55.0);
    // A newer desired snapshot must not erase Slack's persisted Retry-After.
    request(
        &f.slack.db,
        task.org_id,
        SessionId::from_uuid(task.session_id.unwrap()),
        &json!({"expected_revision":1,"context":[],"summary":"Newer update during backoff"})
            .to_string(),
    )
    .await
    .unwrap();
    let before = f.requests("agents.conversations.setProperties").await.len();
    assert!(!projection::step(&f.slack.state).await.unwrap());
    assert_eq!(
        f.requests("agents.conversations.setProperties").await.len(),
        before
    );
    f.project().await;
    assert_eq!(f.task().await.published, 2);
}

#[tokio::test]
async fn view_cleanup_removes_only_the_view_our_upsert_created() {
    let f = Fixture::new().await;
    let task = f.start().await;
    let session = SessionId::from_uuid(task.session_id.unwrap());
    request(&f.slack.db,task.org_id,session,&json!({"expected_revision":0,"context":[],"summary":"Diff","diff":"diff --git a/a b/a\n--- a/a\n+++ b/a\n"}).to_string()).await.unwrap();
    f.project().await;
    assert_eq!(f.task().await.diff_view_id.as_deref(), Some("V_OWN_DIFF"));
    request(
        &f.slack.db,
        task.org_id,
        session,
        &json!({"expected_revision":1,"context":[],"summary":"Review link only"}).to_string(),
    )
    .await
    .unwrap();
    f.project().await;
    let removals = f.requests("agents.conversations.removeView").await;
    assert_eq!(removals.len(), 1);
    assert_eq!(removals[0]["view_id"], "V_OWN_DIFF");
    assert!(
        f.requests("agents.conversations.listViews")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn builtin_diff_clears_only_the_exact_snapshot_we_published() {
    for (current_file, current_version, should_clear) in [
        ("F_OWN", 1, true),
        ("F_OTHER", 1, false),
        ("F_OWN", 2, false),
    ] {
        let f = Fixture::new().await;
        let task = f.start().await;
        let session = SessionId::from_uuid(task.session_id.unwrap());
        Mock::given(method("POST"))
            .and(path("/agents.conversations.setView"))
            .and(body_partial_json(json!({"type":"diff"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok":true,"view_id":"code","file_id":"F_OWN","content_version":1
            })))
            .with_priority(1)
            .mount(&f.api)
            .await;
        request(
            &f.slack.db,
            task.org_id,
            session,
            &json!({
                "expected_revision":0,"context":[],"summary":"Diff",
                "diff":"diff --git a/a b/a\n--- a/a\n+++ b/a\n"
            })
            .to_string(),
        )
        .await
        .unwrap();
        f.project().await;
        assert_eq!(f.task().await.diff_file_id.as_deref(), Some("F_OWN"));
        let mut info = code_info("B_AGENT");
        info["properties"]["code_channel"] = json!({
            "diff_file_id":current_file,"diff_file_version":current_version
        });
        Mock::given(method("GET"))
            .and(path("/conversations.info"))
            .and(query_param("channel", "C_CODE"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"channel":info})),
            )
            .with_priority(1)
            .mount(&f.api)
            .await;
        request(
            &f.slack.db,
            task.org_id,
            session,
            &json!({
                "expected_revision":1,"context":[],"summary":"No diff"
            })
            .to_string(),
        )
        .await
        .unwrap();
        f.project().await;
        let cleared = f
            .requests("agents.conversations.setView")
            .await
            .iter()
            .any(|v| v["type"] == "diff" && v["content"] == "\n");
        assert_eq!(cleared, should_clear);
        assert!(
            f.requests("agents.conversations.removeView")
                .await
                .is_empty()
        );
        let task = f.task().await;
        assert_eq!(task.published, 2);
        assert!(task.diff_view_id.is_none());
        assert!(!task.disabled);
    }
}

#[tokio::test]
async fn empty_git_diff_is_published_as_an_empty_code_tab() {
    let f = Fixture::new().await;
    let task = f.start().await;
    let session = SessionId::from_uuid(task.session_id.unwrap());
    request(
        &f.slack.db,
        task.org_id,
        session,
        &json!({
            "expected_revision":0,"context":[],"summary":"No changes","diff":""
        })
        .to_string(),
    )
    .await
    .unwrap();
    f.project().await;
    assert!(
        f.requests("agents.conversations.setView")
            .await
            .iter()
            .any(|v| v["type"] == "diff" && v["content"] == "\n")
    );
    assert_eq!(f.task().await.published, 1);
}

#[tokio::test]
async fn the_code_webhook_retains_verified_input_before_returning_success() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let f = Fixture::new().await;
    let body = event("app_mention", "C_SOURCE", "1.0", None, "U1").to_string();
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature =
        crate::channels::slack::events::tests_support::make_signature("secret", &timestamp, &body);
    for (signed, status) in [("v0=invalid", 401), (signature.as_str(), 200)] {
        let request = Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/channels/{}/slack/events",
                f.slack.channel.public_id
            ))
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", signed)
            .header("Content-Type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();
        let response = crate::channels::slack::events::routes(f.slack.state.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM slack_code_inbox WHERE NOT done")
        .fetch_one(f.slack.db.pool())
        .await
        .unwrap();
    assert_eq!(queued, 1);
    assert!(f.requests("agents.conversations.create").await.is_empty());
    f.inputs().await;
    assert!(f.task().await.session_id.is_some());
}

#[tokio::test]
async fn approval_intake_is_durable_and_does_not_wait_for_the_task_lock() {
    use everruns_core::tool_hooks::PreToolUseDecision;
    let f = Fixture::new().await;
    let task = f.start().await;
    let payload = park(&f, &task, false).await;
    let mut locked = everruns_durable::begin_transaction(f.slack.db.pool())
        .await
        .unwrap();
    sqlx::query("SELECT id FROM slack_code_tasks WHERE id=$1 FOR UPDATE")
        .bind(task.id)
        .fetch_one(&mut *locked)
        .await
        .unwrap();
    let retained = tokio::time::timeout(
        Duration::from_secs(2),
        interaction::enqueue(&f.slack.state, &f.slack.channel, &f.config, &payload),
    )
    .await
    .expect("approval acknowledgment must not wait behind projection I/O")
    .unwrap();
    assert!(retained);
    assert!(
        interaction::enqueue(&f.slack.state, &f.slack.channel, &f.config, &payload)
            .await
            .unwrap()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM slack_code_inbox WHERE NOT done AND payload->>'type'='code_approval'",
    )
    .fetch_one(f.slack.db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1, "retried clicks share one durable input");
    locked.rollback().await.unwrap();
    f.inputs().await;
    assert!(matches!(
        gated(&f, &task, json!({"to":"first@example.com"})).await,
        PreToolUseDecision::Continue(_)
    ));
}
