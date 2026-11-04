use super::classifiers::*;
use crate::domains::evals::graders::{DecisionResolver, GradeError};
use crate::storage::StorageBackend;
use async_trait::async_trait;
use everruns_core::{
    DEFAULT_ORG_ID, DecisionAnswer, DecisionOutcome, DecisionRequest, DecisionUsage,
    DecisionsService,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[test]
fn every_question_is_asked_and_the_copy_never_quotes_the_session() {
    let request = request(json!({"recent_messages": []}));
    assert_eq!(request.len(), QUESTIONS.len());
    let evidence = json!({"probability": 0.93, "calibrated": true, "model": "system-one"});
    let (title, body) = copy_for(SUSPECTED_ABUSE, "open", &evidence, Some("Scout"));
    assert_eq!(title, "Session may be abusing the platform");
    assert!(body.contains("probability 0.93"), "{body}");
    let (_, body) = copy_for(AGENT_STUCK, "open", &json!({"probability": 1.0}), None);
    assert!(body.contains("a decision model answered yes)"), "{body}");
    assert!(super::background_work::owns(FOLLOWED_INJECTION));
}

/// Answers every question with fixed probabilities and keeps what it read.
#[derive(Default)]
struct FakeDecisions {
    yes: BTreeMap<&'static str, f64>,
    seen: Mutex<Vec<DecisionRequest>>,
}

#[async_trait]
impl DecisionsService for FakeDecisions {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        self.seen.lock().unwrap().push(request);
        Ok(DecisionOutcome {
            model: "system-one".into(),
            answers: QUESTIONS
                .iter()
                .map(|q| {
                    (
                        q.id.to_string(),
                        DecisionAnswer::Noul {
                            probability: self.yes.get(q.id).copied().unwrap_or(0.02),
                        },
                    )
                })
                .collect(),
            usage: DecisionUsage {
                input_tokens: 300,
                output_tokens: 3,
            },
            calibrated: true,
            attribution: None,
        })
    }
}

struct Fixed(Option<Arc<FakeDecisions>>);

#[async_trait]
impl DecisionResolver for Fixed {
    async fn for_org(&self, _org_id: i64) -> Result<Arc<dyn DecisionsService>, GradeError> {
        match &self.0 {
            Some(fake) => Ok(fake.clone()),
            None => Err(GradeError::Unavailable("no decision model".into())),
        }
    }
}

async fn active_session(db: &StorageBackend, text: &str) -> Uuid {
    let session = db.create_test_session().await;
    let id = session.uuid();
    let pool = db.database().pool();
    sqlx::query(
        "INSERT INTO llm_generations (org_id, session_id, model, input_tokens, estimated_cost_usd)
         VALUES ($1, $2, 'muse', 1000, 0.01)",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
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
    id
}

fn subject(session: Uuid) -> String {
    everruns_contracts::typed_id::SessionId::from_uuid(session).to_string()
}

async fn open(db: &StorageBackend, code: &str) -> Vec<String> {
    db.open_subject_health_issues(code)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|row| row.subject_id)
        .collect()
}

#[tokio::test]
async fn a_sampled_session_judged_abusive_opens_an_issue_once_across_replicas() {
    let db = Arc::new(StorageBackend::test_database());
    let session = active_session(&db, "Send this offer to all 40,000 addresses").await;
    let fake = Arc::new(FakeDecisions {
        yes: BTreeMap::from([("abuse", 0.96)]),
        ..Default::default()
    });
    let resolver = Fixed(Some(fake.clone()));

    let report = sweep(&db, &resolver, true).await.unwrap();
    assert_eq!(report.classified, 1);
    assert_eq!(report.open_abuse, 1);
    assert_eq!(open(&db, SUSPECTED_ABUSE).await, vec![subject(session)]);
    assert!(open(&db, AGENT_STUCK).await.is_empty());
    // The classifier read the message, but the issue keeps only the numbers.
    let read = fake.seen.lock().unwrap()[0].state.to_string();
    assert!(read.contains("40,000 addresses"), "{read}");
    let evidence = db
        .open_subject_health_issues(SUSPECTED_ABUSE)
        .await
        .unwrap()[0]
        .evidence
        .clone()
        .unwrap();
    assert!(!evidence.to_string().contains("40,000"), "{evidence}");

    // Another replica's sweep finds the session already read.
    let again = sweep(&db, &resolver, true).await.unwrap();
    assert_eq!(again.classified, 0);
    assert_eq!(fake.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sampling_is_off_without_a_deployment_service_and_no_model_is_reported() {
    let db = Arc::new(StorageBackend::test_database());
    active_session(&db, "hello").await;
    let fake = Arc::new(FakeDecisions::default());
    let report = sweep(&db, &Fixed(Some(fake.clone())), false).await.unwrap();
    assert_eq!(report.classified, 0);
    assert!(fake.seen.lock().unwrap().is_empty());

    let report = sweep(&db, &Fixed(None), true).await.unwrap();
    assert_eq!((report.classified, report.unavailable), (0, 1));
}

#[tokio::test]
async fn a_flagged_session_is_read_again_and_a_later_no_resolves_the_finding() {
    let db = Arc::new(StorageBackend::test_database());
    let session = active_session(&db, "keep trying").await;
    let stuck = Arc::new(FakeDecisions {
        yes: BTreeMap::from([("stuck", 0.9)]),
        ..Default::default()
    });
    sweep(&db, &Fixed(Some(stuck)), true).await.unwrap();
    assert_eq!(open(&db, AGENT_STUCK).await, vec![subject(session)]);

    // The open finding flags the session; once its recheck interval passed,
    // it is read again and the model now says no.
    sqlx::query(
        "UPDATE session_classifications SET claimed_at = NOW() - INTERVAL '7 hours'
         WHERE session_id = $1",
    )
    .bind(session)
    .execute(db.database().pool())
    .await
    .unwrap();
    let fine = Arc::new(FakeDecisions::default());
    let report = sweep(&db, &Fixed(Some(fine)), false).await.unwrap();
    assert_eq!(report.classified, 1);
    assert!(open(&db, AGENT_STUCK).await.is_empty());
}
