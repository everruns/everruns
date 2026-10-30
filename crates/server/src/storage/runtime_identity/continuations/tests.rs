use super::*;
use crate::kernel_imports::DEFAULT_ORG_ID;
use crate::storage::{CreateEventRow, CreateSessionScheduleRow};
use chrono::Utc;
use everruns_contracts::typed_id::{MessageId, PrincipalId};
use serde_json::json;

struct Fixture {
    db: StorageBackend,
    schedule: SessionScheduleRow,
    original: Uuid,
    person: Uuid,
    subject: VirtualUserId,
    responder: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let db = StorageBackend::test_database();
        let person = db.create_test_user(Uuid::now_v7()).await;
        db.ensure_membership(person, DEFAULT_ORG_ID, "member")
            .await
            .unwrap();
        let subject = db
            .default_virtual_user(DEFAULT_ORG_ID, person)
            .await
            .unwrap()
            .id;
        let responder = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
        let session = db.create_test_session().await;
        let original = Uuid::now_v7();
        db.record_runtime_invocation(
            DEFAULT_ORG_ID,
            session,
            original,
            Some(subject),
            Some(person),
            Some(responder),
        )
        .await
        .unwrap();
        let schedule = db.create_session_schedule(CreateSessionScheduleRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            description: "Continue the interrupted work (automatic retry after an AI provider rate limit).".into(),
            cron_expression: None,
            scheduled_at: Some(Utc::now() + chrono::Duration::seconds(120)),
            timezone: "UTC".into(),
            next_trigger_at: None,
        }).await.unwrap();
        Self {
            db,
            schedule,
            original,
            person,
            subject,
            responder,
        }
    }

    async fn proof(&self, event_type: &str, session: SessionId) {
        self.proof_for(event_type, session, self.original, false)
            .await;
    }

    async fn proof_for(&self, event_type: &str, session: SessionId, source: Uuid, success: bool) {
        self.db
            .create_event(CreateEventRow {
                session_id: session,
                event_type: event_type.into(),
                ts: Utc::now(),
                context: json!({"input_message_id": MessageId::from_uuid(source)}),
                data: json!({"success":success,"error":"Provider rate limited"}),
                metadata: Some(json!({"platform_auto_retry":{"schedule_id":self.schedule.id}})),
                tags: None,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_provider_retry_preserves_its_original_verified_invocation() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    let retry = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, retry, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, retry)
            .await
            .unwrap(),
        Some(f.subject)
    );
    assert_eq!(
        f.db.runtime_invocation_management_user(f.schedule.session_id, retry)
            .await
            .unwrap(),
        Some(f.person)
    );
    assert_eq!(
        f.db.runtime_invocation_responder(f.schedule.session_id, retry)
            .await
            .unwrap(),
        Some(f.responder)
    );
}

#[tokio::test]
async fn matching_a_retry_prompt_does_not_authorize_an_ordinary_schedule() {
    let f = Fixture::new().await;
    let wake = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        f.db.runtime_invocation_management_user(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_retry_cannot_take_proof_from_a_user_message_or_successful_reason() {
    for event_type in ["input.message", "reason.completed"] {
        let f = Fixture::new().await;
        f.proof_for(
            event_type,
            f.schedule.session_id,
            f.original,
            event_type == "reason.completed",
        )
        .await;
        let wake = Uuid::now_v7();
        f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
            .await
            .unwrap();
        assert_eq!(
            f.db.runtime_invocation_subject(f.schedule.session_id, wake)
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test]
async fn a_retry_cannot_use_another_sessions_proof_or_invocation() {
    let f = Fixture::new().await;
    let other = f.db.create_test_session().await;
    f.proof("reason.completed", other).await;
    for session in [f.schedule.session_id, other] {
        let mut timer = f.schedule.clone();
        timer.session_id = session;
        let wake = Uuid::now_v7();
        f.db.record_scheduled_runtime_invocation(&timer, wake, None)
            .await
            .unwrap();
        assert_eq!(
            f.db.runtime_invocation_subject(session, wake)
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test]
async fn a_retry_cannot_cross_organizations() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    let mut timer = f.schedule.clone();
    timer.org_id += 1;
    let wake = Uuid::now_v7();
    assert!(
        f.db.record_scheduled_runtime_invocation(&timer, wake, None)
            .await
            .is_err()
    );
    assert!(
        !f.db
            .runtime_invocation_exists(f.schedule.session_id, wake)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_retry_must_be_a_recent_short_first_fire() {
    for case in [
        "recurring",
        "replayed",
        "stale",
        "long",
        "past",
        "disabled",
        "deleted",
        "no_time",
    ] {
        let f = Fixture::new().await;
        f.proof("reason.completed", f.schedule.session_id).await;
        let mut timer = f.schedule.clone();
        match case {
            "recurring" => timer.cron_expression = Some("0 * * * *".into()),
            "replayed" => timer.trigger_count = 1,
            "stale" => {
                timer.created_at = Utc::now() - chrono::Duration::minutes(20);
                timer.scheduled_at = Some(timer.created_at + chrono::Duration::seconds(120));
            }
            "long" => timer.scheduled_at = Some(timer.created_at + chrono::Duration::hours(1)),
            "past" => timer.scheduled_at = Some(timer.created_at - chrono::Duration::minutes(1)),
            "disabled" => timer.enabled = false,
            "deleted" => timer.deleted_at = Some(Utc::now()),
            "no_time" => timer.scheduled_at = None,
            _ => unreachable!(),
        }
        let wake = Uuid::now_v7();
        f.db.record_scheduled_runtime_invocation(&timer, wake, None)
            .await
            .unwrap();
        assert_eq!(
            f.db.runtime_invocation_subject(timer.session_id, wake)
                .await
                .unwrap(),
            None,
            "{case}"
        );
    }
}

#[tokio::test]
async fn a_retry_cannot_substitute_a_schedule_or_unknown_source() {
    for case in ["schedule", "source", "old_event"] {
        let f = Fixture::new().await;
        let source = if case == "source" {
            Uuid::now_v7()
        } else {
            f.original
        };
        f.proof_for("reason.completed", f.schedule.session_id, source, false)
            .await;
        let mut timer = f.schedule.clone();
        match case {
            "schedule" => timer.id = everruns_contracts::typed_id::ScheduleId::new(),
            // A proof older than the claimed schedule cannot authorize it.
            "old_event" => timer.created_at = Utc::now() + chrono::Duration::seconds(1),
            "source" => {}
            _ => unreachable!(),
        }
        let wake = Uuid::now_v7();
        f.db.record_scheduled_runtime_invocation(&timer, wake, None)
            .await
            .unwrap();
        assert_eq!(
            f.db.runtime_invocation_subject(timer.session_id, wake)
                .await
                .unwrap(),
            None,
            "{case}"
        );
    }
}

#[tokio::test]
async fn a_retry_does_not_borrow_a_newer_persons_identity() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    let other = f.db.create_test_user(Uuid::now_v7()).await;
    f.db.ensure_membership(other, DEFAULT_ORG_ID, "member")
        .await
        .unwrap();
    let subject =
        f.db.default_virtual_user(DEFAULT_ORG_ID, other)
            .await
            .unwrap()
            .id;
    f.db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        f.schedule.session_id,
        Uuid::now_v7(),
        Some(subject),
        Some(other),
        Some(f.responder),
    )
    .await
    .unwrap();
    let wake = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        f.db.runtime_invocation_management_user(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_retry_allows_a_newer_invocation_of_the_same_person() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    f.db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        f.schedule.session_id,
        Uuid::now_v7(),
        Some(f.subject),
        Some(f.person),
        Some(f.responder),
    )
    .await
    .unwrap();
    let wake = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, wake)
            .await
            .unwrap(),
        Some(f.subject)
    );
}

#[tokio::test]
async fn a_retry_revalidates_management_membership() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    f.db.remove_organization_member(DEFAULT_ORG_ID, f.person)
        .await
        .unwrap();
    let wake = Uuid::now_v7();
    assert!(
        f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
            .await
            .is_err()
    );
    assert!(
        !f.db
            .runtime_invocation_exists(f.schedule.session_id, wake)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_retry_cannot_gain_a_person_from_an_anonymous_source() {
    let f = Fixture::new().await;
    let source = Uuid::now_v7();
    f.db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        f.schedule.session_id,
        source,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    f.proof_for("reason.completed", f.schedule.session_id, source, false)
        .await;
    let wake = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        f.db.runtime_invocation_management_user(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_retry_fails_closed_beyond_the_bounded_proof_window() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    let noise = (0..128)
        .map(|_| CreateEventRow {
            session_id: f.schedule.session_id,
            event_type: "reason.completed".into(),
            ts: Utc::now(),
            context: json!({}),
            data: json!({"success":true}),
            metadata: None,
            tags: None,
        })
        .collect();
    f.db.create_events(noise).await.unwrap();
    let wake = Uuid::now_v7();
    f.db.record_scheduled_runtime_invocation(&f.schedule, wake, None)
        .await
        .unwrap();
    assert_eq!(
        f.db.runtime_invocation_subject(f.schedule.session_id, wake)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_retry_atomic_insert_rejects_an_intervening_identity_switch() {
    let f = Fixture::new().await;
    f.proof("reason.completed", f.schedule.session_id).await;
    let captured =
        f.db.attested_retry_identity(&f.schedule)
            .await
            .unwrap()
            .unwrap();
    f.db.validate_runtime_invocation_identity(
        DEFAULT_ORG_ID,
        captured.identity.0,
        captured.identity.1,
        captured.identity.2,
        InvocationRows::default(),
    )
    .await
    .unwrap();
    // Another invocation commits after proof/permission validation, before insertion.
    let other = f.db.create_test_user(Uuid::now_v7()).await;
    f.db.ensure_membership(other, DEFAULT_ORG_ID, "member")
        .await
        .unwrap();
    let subject =
        f.db.default_virtual_user(DEFAULT_ORG_ID, other)
            .await
            .unwrap()
            .id;
    f.db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        f.schedule.session_id,
        Uuid::now_v7(),
        Some(subject),
        Some(other),
        Some(f.responder),
    )
    .await
    .unwrap();
    let wake = Uuid::now_v7();
    assert!(
        !f.db
            .insert_retry_invocation_if_current(&f.schedule, wake, captured)
            .await
            .unwrap()
    );
    assert!(
        !f.db
            .runtime_invocation_exists(f.schedule.session_id, wake)
            .await
            .unwrap()
    );
}
