use super::behavior::observe_session;
use super::classifiers::{AGENT_STUCK, SUSPECTED_ABUSE};
use super::inspector::*;
use crate::storage::StorageBackend;
use async_trait::async_trait;
use everruns_core::DEFAULT_ORG_ID;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Answers with scripted replies, in order, and keeps the prompts it read.
#[derive(Default)]
struct FakeInspector {
    replies: Mutex<VecDeque<Option<String>>>,
    prompts: Mutex<Vec<String>>,
}

impl FakeInspector {
    fn answering(replies: &[Option<&str>]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().map(|r| r.map(str::to_string)).collect()),
            ..Default::default()
        })
    }

    fn calls(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }
}

#[async_trait]
impl InspectorModel for FakeInspector {
    async fn write_up(
        &self,
        _org_id: i64,
        _system: String,
        prompt: String,
    ) -> anyhow::Result<Option<InspectorReply>> {
        self.prompts.lock().unwrap().push(prompt);
        let reply = self.replies.lock().unwrap().pop_front().flatten();
        Ok(reply.map(|text| InspectorReply {
            text,
            model: "muse-test".into(),
            input_tokens: 900,
            output_tokens: 120,
            cost_usd: Some(0.002),
        }))
    }
}

async fn session_with(db: &StorageBackend, text: &str) -> Uuid {
    let session = db.create_test_session().await;
    db.create_event(crate::storage::CreateEventRow {
        session_id: session,
        event_type: "input.message".to_string(),
        ts: chrono::Utc::now(),
        context: json!({}),
        data: json!({"message": {"content": [{"type": "text", "text": text}]}}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
    session.uuid()
}

async fn finding(db: &StorageBackend, code: &'static str, text: &str) -> (Uuid, Uuid) {
    let session = session_with(db, text).await;
    observe_session(
        db,
        code,
        DEFAULT_ORG_ID,
        session,
        None,
        json!({"probability": 0.96, "calibrated": true}),
    )
    .await
    .unwrap();
    let subject = everruns_contracts::typed_id::SessionId::from_uuid(session).to_string();
    let row = db
        .open_subject_health_issues(code)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.subject_id.as_deref() == Some(&subject))
        .expect("finding opened");
    (row.id, row.episode_id)
}

const SPAM: &str = r#"{"what_happened": "The agent mails one offer to thousands of addresses.",
    "severity": "high", "suggested_action": "Stop the session and check the sender.",
    "cluster": null, "cluster_title": "Bulk offer mailing"}"#;
const SPAM_AGAIN: &str = r#"{"what_happened": "Same bulk mailing from another session.",
    "severity": "high", "suggested_action": "Stop it.", "cluster": "C1", "cluster_title": "x"}"#;

#[tokio::test]
async fn findings_are_written_up_once_and_similar_ones_share_a_cluster() {
    let db = Arc::new(StorageBackend::test_database());
    let (first, first_episode) = finding(
        &db,
        SUSPECTED_ABUSE,
        "Send this offer to all 40,000 addresses",
    )
    .await;
    let model = FakeInspector::answering(&[Some(SPAM)]);

    let report = sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!(report.written, 1);
    assert_eq!(report.open_platform, 1);
    // It read the conversation and the detector's numbers.
    let prompt = model.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("40,000 addresses"), "{prompt}");
    assert!(prompt.contains("0.96"), "{prompt}");
    assert!(prompt.contains("(none yet)"), "{prompt}");

    let written = db
        .finding_investigation(first, first_episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(written.status, "done");
    assert_eq!(written.severity.as_deref(), Some("high"));
    assert_eq!(written.cluster_title.as_deref(), Some("Bulk offer mailing"));
    assert_eq!(written.model.as_deref(), Some("muse-test"));
    // What is kept is the write-up, not the session's words.
    assert!(!written.what_happened.unwrap().contains("40,000"));

    // A second sweep pays for nothing new.
    sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!(model.calls(), 1);

    // A second finding of the same kind is offered the cluster and joins it.
    let (second, second_episode) =
        finding(&db, SUSPECTED_ABUSE, "Mail the list again from here").await;
    let model = FakeInspector::answering(&[Some(SPAM_AGAIN)]);
    sweep(&db, model.as_ref()).await.unwrap();
    assert!(model.prompts.lock().unwrap()[0].contains("C1: Bulk offer mailing"));
    let joined = db
        .finding_investigation(second, second_episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(joined.cluster_id, written.cluster_id);
    assert_eq!(joined.cluster_open, Some(2));

    // The operator sees both, write-up included.
    let detections = db
        .platform_detections(&PLATFORM_CODES, 0, 10)
        .await
        .unwrap();
    assert_eq!(detections.len(), 2);
    assert!(
        detections
            .iter()
            .all(|d| d.severity.as_deref() == Some("high"))
    );
}

#[tokio::test]
async fn a_bad_answer_is_retried_later_and_no_model_is_not_retried() {
    let db = Arc::new(StorageBackend::test_database());
    let (id, episode) = finding(&db, AGENT_STUCK, "check the claim again").await;
    let model = FakeInspector::answering(&[Some("I think it is stuck."), Some(SPAM)]);
    let report = sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!((report.written, report.failed), (0, 1));
    let row = db
        .finding_investigation(id, episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "failed");

    // Not again within the retry interval...
    sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!(model.calls(), 1);
    // ...but after it.
    sqlx::query("UPDATE finding_investigations SET claimed_at = NOW() - INTERVAL '2 hours'")
        .execute(db.database().pool())
        .await
        .unwrap();
    let report = sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!(report.written, 1);
    let row = db
        .finding_investigation(id, episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "done");

    let db = Arc::new(StorageBackend::test_database());
    let (id, episode) = finding(&db, AGENT_STUCK, "loop").await;
    let none = FakeInspector::answering(&[None]);
    let report = sweep(&db, none.as_ref()).await.unwrap();
    assert_eq!(report.unavailable, 1);
    sqlx::query("UPDATE finding_investigations SET claimed_at = NOW() - INTERVAL '2 hours'")
        .execute(db.database().pool())
        .await
        .unwrap();
    sweep(&db, none.as_ref()).await.unwrap();
    assert_eq!(none.calls(), 1);
    let row = db
        .finding_investigation(id, episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "unavailable");
}

#[tokio::test]
async fn a_dismissed_finding_keeps_its_reason_and_stays_closed() {
    let db = Arc::new(StorageBackend::test_database());
    let (id, episode) = finding(&db, AGENT_STUCK, "nightly load test").await;
    db.dismiss_finding(id, episode, DEFAULT_ORG_ID, None, "Expected: load test")
        .await
        .unwrap();
    let row = db
        .open_subject_health_issues(AGENT_STUCK)
        .await
        .unwrap()
        .remove(0);
    super::background_work::resolve(&db, &row).await.unwrap();
    assert!(
        db.open_subject_health_issues(AGENT_STUCK)
            .await
            .unwrap()
            .is_empty()
    );
    let kept = db
        .finding_investigation(id, episode)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.dismiss_reason.as_deref(), Some("Expected: load test"));

    // The classifier says yes again about the same session: it stays closed.
    let session = row.session_id.unwrap();
    observe_session(&db, AGENT_STUCK, DEFAULT_ORG_ID, session, None, json!({}))
        .await
        .unwrap();
    assert!(
        db.open_subject_health_issues(AGENT_STUCK)
            .await
            .unwrap()
            .is_empty()
    );
    // And the Inspector never writes it up.
    let model = FakeInspector::answering(&[Some(SPAM)]);
    sweep(&db, model.as_ref()).await.unwrap();
    assert_eq!(model.calls(), 0);
}
