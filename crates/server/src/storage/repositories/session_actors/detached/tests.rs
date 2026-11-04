//! Detached tool call records on a real database.

use super::*;
use crate::storage::test_database::test_session_row;
use crate::storage::{ActorCancel, CreateEventRow, DEFAULT_ACTOR_LEASES_PER_ORG, StorageBackend};

const TTL: Duration = Duration::from_secs(10);
const ORG: i64 = 2;

/// A session whose actor `holder` leases, with its fence.
async fn leased(db: &StorageBackend, holder: &str) -> (SessionId, i64) {
    let id = db
        .create_session(test_session_row(ORG))
        .await
        .expect("create session")
        .id;
    db.mark_session_actor_runnable(id, ORG, 1).await.unwrap();
    let leased = db
        .lease_session_actors(holder, 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert_eq!(leased.len(), 1);
    (id, leased[0].fence)
}

/// The holder's turn parked: it releases the actor with nothing new.
async fn park(db: &StorageBackend, id: SessionId, holder: &str, fence: i64) {
    let inbox = db.get_session_actor(id).await.unwrap().unwrap().inbox_seq;
    assert_eq!(
        db.release_session_actor(id, holder, fence, inbox)
            .await
            .unwrap(),
        Some(false)
    );
}

async fn lease_any(db: &StorageBackend) -> Vec<SessionId> {
    db.lease_session_actors("other", 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.session_id)
        .collect()
}

async fn expire(db: &StorageBackend, id: SessionId) {
    sqlx::query(
        "UPDATE detached_tool_calls SET expires_at = NOW() - INTERVAL '1 second' \
         WHERE session_id = $1",
    )
    .bind(id)
    .execute(&db.database().pool)
    .await
    .unwrap();
}

fn row(session_id: SessionId, event_type: &str) -> CreateEventRow {
    CreateEventRow {
        session_id,
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({}),
        metadata: None,
        tags: None,
    }
}

fn call(id: SessionId, tool_call_id: &str) -> (SessionId, String) {
    (id, tool_call_id.to_string())
}

#[tokio::test]
async fn only_the_live_holder_detaches_a_call() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    let turn = Uuid::now_v7();

    assert!(
        !db.detach_tool_call(id, "c1", turn, "w2", fence, TTL)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_tool_call(id, "c1", turn, "w1", fence + 1, TTL)
            .await
            .unwrap()
    );
    assert!(db.list_detached_tool_calls(id).await.unwrap().is_empty());

    assert!(
        db.detach_tool_call(id, "c1", turn, "w1", fence, TTL)
            .await
            .unwrap()
    );
    assert_eq!(db.list_detached_tool_calls(id).await.unwrap(), ["c1"]);
}

#[tokio::test]
async fn a_parked_turn_is_not_leased_until_its_last_call_completes() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    let turn = Uuid::now_v7();
    for call_id in ["c1", "c2"] {
        assert!(
            db.detach_tool_call(id, call_id, turn, "w1", fence, TTL)
                .await
                .unwrap()
        );
    }
    park(&db, id, "w1", fence).await;

    // Steering input arrives: the actor is runnable, but its calls still run.
    db.mark_session_actor_runnable(id, ORG, 7).await.unwrap();
    assert!(lease_any(&db).await.is_empty());

    assert_eq!(
        db.complete_detached_tool_call(id, "c1", "w2")
            .await
            .unwrap(),
        None,
        "only the holder completes its call"
    );
    assert_eq!(
        db.complete_detached_tool_call(id, "c1", "w1")
            .await
            .unwrap(),
        Some(false)
    );
    assert!(lease_any(&db).await.is_empty(), "c2 still runs");
    assert_eq!(
        db.complete_detached_tool_call(id, "c2", "w1")
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(lease_any(&db).await, [id]);
}

#[tokio::test]
async fn the_last_completion_makes_a_quiet_actor_runnable_at_the_log_head() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(id, "c1", Uuid::now_v7(), "w1", fence, TTL)
            .await
            .unwrap()
    );
    park(&db, id, "w1", fence).await;
    assert!(lease_any(&db).await.is_empty());

    let stored = db
        .create_events_detached(vec![row(id, "tool.completed")], "w1", "c1")
        .await
        .expect("the call's holder appends its completion");
    assert_eq!(
        db.complete_detached_tool_call(id, "c1", "w1")
            .await
            .unwrap(),
        Some(true)
    );
    let actor = db.get_session_actor(id).await.unwrap().unwrap();
    assert!(actor.runnable_since.is_some());
    assert_eq!(actor.inbox_seq, i64::from(stored[0].sequence));
    assert_eq!(lease_any(&db).await, [id]);
}

#[tokio::test]
async fn a_detached_append_lands_only_under_the_calls_live_record() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(id, "c1", Uuid::now_v7(), "w1", fence, TTL)
            .await
            .unwrap()
    );
    park(&db, id, "w1", fence).await;

    for (holder, call_id) in [("w2", "c1"), ("w1", "c2")] {
        let error = db
            .create_events_detached(vec![row(id, "tool.progress")], holder, call_id)
            .await
            .expect_err("not this holder's call");
        assert!(
            error
                .downcast_ref::<crate::storage::ActorLeaseFenceRejected>()
                .is_some()
        );
    }
    db.create_events_detached(vec![row(id, "tool.progress")], "w1", "c1")
        .await
        .expect("the holder's call appends");

    expire(&db, id).await;
    let error = db
        .create_events_detached(vec![row(id, "tool.completed")], "w1", "c1")
        .await
        .expect_err("an expired record appends nothing");
    assert!(
        error
            .downcast_ref::<crate::storage::ActorLeaseFenceRejected>()
            .is_some()
    );
}

#[tokio::test]
async fn renew_extends_live_records_and_reports_the_rest_gone() {
    let db = StorageBackend::test_database();
    let (a, fa) = leased(&db, "w1").await;
    let (b, fb) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(a, "c1", Uuid::now_v7(), "w1", fa, TTL)
            .await
            .unwrap()
    );
    assert!(
        db.detach_tool_call(b, "c1", Uuid::now_v7(), "w1", fb, TTL)
            .await
            .unwrap()
    );
    expire(&db, b).await;

    let live = db
        .renew_detached_tool_calls("w1", TTL, &[call(a, "c1"), call(b, "c1"), call(a, "c9")])
        .await
        .unwrap();
    assert_eq!(live, [call(a, "c1")], "an expired record is not revived");
    assert!(
        db.renew_detached_tool_calls("w2", TTL, &[call(a, "c1")])
            .await
            .unwrap()
            .is_empty(),
        "another holder renews nothing"
    );
}

#[tokio::test]
async fn an_expired_record_resumes_its_turn_on_the_next_lease() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(id, "c1", Uuid::now_v7(), "w1", fence, TTL)
            .await
            .unwrap()
    );
    park(&db, id, "w1", fence).await;
    assert!(lease_any(&db).await.is_empty());

    // The holder died: renewals stop and the record runs out.
    expire(&db, id).await;
    assert_eq!(lease_any(&db).await, [id]);
    assert!(db.list_detached_tool_calls(id).await.unwrap().is_empty());
    assert_eq!(
        db.complete_detached_tool_call(id, "c1", "w1")
            .await
            .unwrap(),
        None,
        "a late completion of the expired call is refused"
    );
}

#[tokio::test]
async fn expiring_on_purpose_resumes_without_waiting_out_the_ttl() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(id, "c1", Uuid::now_v7(), "w1", fence, TTL)
            .await
            .unwrap()
    );
    park(&db, id, "w1", fence).await;

    db.expire_detached_tool_calls("w2", &[call(id, "c1")])
        .await
        .unwrap();
    assert!(
        lease_any(&db).await.is_empty(),
        "only the holder expires its call"
    );
    db.expire_detached_tool_calls("w1", &[call(id, "c1")])
        .await
        .unwrap();
    assert_eq!(lease_any(&db).await, [id]);
}

#[tokio::test]
async fn a_cancel_drops_the_turns_detached_calls() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, "w1").await;
    assert!(
        db.detach_tool_call(id, "c1", Uuid::now_v7(), "w1", fence, TTL)
            .await
            .unwrap()
    );
    park(&db, id, "w1", fence).await;

    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Idle,
        "nothing holds the parked turn, so the caller records the cancel"
    );
    assert!(db.list_detached_tool_calls(id).await.unwrap().is_empty());
    assert!(
        db.renew_detached_tool_calls("w1", TTL, &[call(id, "c1")])
            .await
            .unwrap()
            .is_empty(),
        "the holder learns the call is gone and stops it"
    );
    assert_eq!(
        db.complete_detached_tool_call(id, "c1", "w1")
            .await
            .unwrap(),
        None
    );
    assert!(
        lease_any(&db).await.is_empty(),
        "a late completion resumes nothing"
    );
}
