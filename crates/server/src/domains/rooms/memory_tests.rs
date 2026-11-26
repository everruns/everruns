// Room memory end to end against a real database: facts that replace each
// other, recall, notes, forgetting, and memory at wake.

use super::rules::WakeMode;
use super::tests::{
    agent_member, caller, create, ctx, guest, members_by_handle, post, seed_agent, seed_person,
};
use super::*;
use crate::domains::common::*;
use crate::storage::StorageBackend;
use everruns_core::{DEFAULT_ORG_ID, OrgRole};
use std::sync::Arc;

// ============================================================================
// Room memory
// ============================================================================

fn decision(
    room_id: &str,
    subject: &str,
    statement: &str,
    message_id: Option<&str>,
    quote: Option<&str>,
    as_member: &str,
) -> RememberRoomFact {
    RememberRoomFact {
        room_id: room_id.into(),
        kind: "decision".into(),
        subject: subject.into(),
        statement: statement.into(),
        message_id: message_id.map(Into::into),
        quote: quote.map(Into::into),
        status: None,
        occurred_at: None,
        as_member: Some(as_member.into()),
    }
}

async fn facts(
    ctx: &Ctx,
    room_id: &str,
    include_replaced: bool,
    as_of: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<RoomMemoryEntry> {
    ListRoomFacts {
        room_id: room_id.into(),
        kind: None,
        subject: None,
        as_of,
        include_replaced,
        limit: None,
    }
    .run(ctx)
    .await
    .unwrap()
    .data
}

async fn recall_hits(ctx: &Ctx, room_id: &str, q: &str) -> Vec<RoomRecallHit> {
    RecallRoom {
        room_id: room_id.into(),
        q: q.into(),
        as_of: None,
        limit: None,
    }
    .run(ctx)
    .await
    .unwrap()
    .data
}

#[tokio::test]
async fn a_newer_decision_replaces_the_older_one_and_recall_finds_both() {
    let db = Arc::new(StorageBackend::test_database());
    let app = ctx(&db, caller(None, OrgRole::Admin));
    let room = create("Release", vec![guest("Ana"), guest("Bo")])
        .run(&app)
        .await
        .unwrap();
    let handles = members_by_handle(&app, &room.id).await;
    let (ana, bo) = (handles["ana"].as_str(), handles["bo"].as_str());

    let first = post(
        &app,
        &room.id,
        "Final call: we deploy on Mondays at 10am.",
        None,
        Some(ana),
    )
    .await
    .message;
    // A quote must be the message's own words.
    let error = decision(
        &room.id,
        "Deploy window",
        "Deploys are on Mondays.",
        Some(&first.id),
        Some("we deploy on Fridays"),
        ana,
    )
    .run(&app)
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("quote is not in the message"),
        "{error}"
    );
    let monday = decision(
        &room.id,
        "Deploy window",
        "Deploys happen on Mondays at 10am.",
        Some(&first.id),
        Some("we deploy on Mondays at 10am"),
        ana,
    )
    .run(&app)
    .await
    .unwrap();
    assert!(monday.replaced.is_empty());
    assert_eq!(monday.entry.subject.as_deref(), Some("deploy window"));
    assert_eq!(monday.entry.occurred_at, first.created_at);
    let between = chrono::Utc::now();

    let second = post(
        &app,
        &room.id,
        "Change of plan: deploys move to Tuesdays at 3pm.",
        None,
        Some(bo),
    )
    .await
    .message;
    let tuesday = decision(
        &room.id,
        "deploy  window.",
        "Deploys happen on Tuesdays at 3pm.",
        Some(&second.id),
        Some("deploys move to Tuesdays at 3pm"),
        bo,
    )
    .run(&app)
    .await
    .unwrap();
    assert_eq!(tuesday.replaced, vec![monday.entry.id.clone()]);

    // Now: only Tuesday is current. Replaced facts stay, and say what replaced them.
    let current = facts(&app, &room.id, false, None).await;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].id, tuesday.entry.id);
    let all = facts(&app, &room.id, true, None).await;
    assert_eq!(all.len(), 2);
    let old = all.iter().find(|fact| fact.id == monday.entry.id).unwrap();
    assert!(!old.current);
    assert_eq!(old.replaced_by.as_deref(), Some(tuesday.entry.id.as_str()));
    // As of before the change, Monday was the rule.
    let then = facts(&app, &room.id, false, Some(between)).await;
    assert_eq!(then.len(), 1);
    assert_eq!(then[0].id, monday.entry.id);
    assert!(then[0].current);

    // Recall finds the current fact first, then the talk around it.
    let hits = recall_hits(&app, &room.id, "when do we deploy?").await;
    assert!(hits.len() >= 3, "{hits:?}");
    assert_eq!(hits[0].kind, "entry");
    assert_eq!(hits[0].entry.as_ref().unwrap().id, tuesday.entry.id);
    assert!(
        hits.iter()
            .any(|hit| hit.message.as_ref().is_some_and(|m| m.id == first.id))
    );
    let excerpt = hits.iter().find_map(|hit| hit.excerpt.as_deref()).unwrap();
    assert!(excerpt.contains("**"), "{excerpt}");
    // Nothing matches words the room never used; syntax is never a query.
    assert!(
        recall_hits(&app, &room.id, "kubernetes & | !")
            .await
            .is_empty()
    );

    // The entry opens with its source message and its subject's history.
    let detail = GetRoomMemoryEntry {
        room_id: room.id.clone(),
        entry_id: tuesday.entry.id.clone(),
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(detail.source_message.unwrap().id, second.id);
    assert_eq!(
        detail
            .history
            .iter()
            .map(|fact| fact.id.as_str())
            .collect::<Vec<_>>(),
        vec![monday.entry.id.as_str(), tuesday.entry.id.as_str()]
    );

    // Ana cannot forget Bo's fact; Bo can, and Monday is current again.
    let person = seed_person(&db, "Cy Member", "member").await;
    let cy = ctx(&db, caller(Some(person), OrgRole::Member));
    post(&cy, &room.id, "hello", None, None).await;
    let error = ForgetRoomMemoryEntry {
        room_id: room.id.clone(),
        entry_id: tuesday.entry.id.clone(),
        as_member: None,
    }
    .run(&cy)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Only the member"), "{error}");
    let forgotten = ForgetRoomMemoryEntry {
        room_id: room.id.clone(),
        entry_id: tuesday.entry.id.clone(),
        as_member: Some(bo.into()),
    }
    .run(&app)
    .await
    .unwrap();
    assert!(forgotten.forgotten);
    let current = facts(&app, &room.id, false, None).await;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].id, monday.entry.id);
    assert!(
        !recall_hits(&app, &room.id, "deploy tuesdays")
            .await
            .iter()
            .any(|hit| hit.entry.as_ref().is_some_and(|e| e.id == tuesday.entry.id))
    );
    let events = ListRoomEvents {
        room_id: room.id.clone(),
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap()
    .data;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "memory.recorded")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "memory.forgotten")
            .count(),
        1
    );
}

#[tokio::test]
async fn ingested_notes_are_replaced_by_caller_id_and_recalled() {
    let db = Arc::new(StorageBackend::test_database());
    let app = ctx(&db, caller(None, OrgRole::Admin));
    let room = create("Standups", vec![guest("Ana")])
        .run(&app)
        .await
        .unwrap();
    let ana = members_by_handle(&app, &room.id).await["ana"].clone();
    let note = |text: &str| IngestRoomNote {
        room_id: room.id.clone(),
        text: text.into(),
        label: Some("Standup 2026-11-24".into()),
        source_ref: Some("standup-2026-11-24".into()),
        occurred_at: None,
        metadata: None,
        as_member: Some(ana.clone()),
    };
    let first = note("We agreed the importer ships behind a flag.")
        .run(&app)
        .await
        .unwrap();
    assert!(!first.unchanged);
    assert_eq!(first.entry.kind, "note");
    let again = note("We agreed the importer ships behind a flag.")
        .run(&app)
        .await
        .unwrap();
    assert!(again.unchanged);
    assert_eq!(again.entry.id, first.entry.id);
    let changed = note("We agreed the exporter ships first.")
        .run(&app)
        .await
        .unwrap();
    assert!(!changed.unchanged);
    assert_ne!(changed.entry.id, first.entry.id);

    let hits = recall_hits(&app, &room.id, "importer exporter").await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].entry.as_ref().unwrap().id, changed.entry.id);
}

#[tokio::test]
async fn an_agent_with_memory_recall_is_woken_with_what_the_room_remembers() {
    let db = Arc::new(StorageBackend::test_database());
    let app = ctx(&db, caller(None, OrgRole::Admin));
    let agent = seed_agent(&db, "helper").await;
    let mut member = agent_member(&agent, WakeMode::Mentions);
    member.memory_recall = Some(true);
    let room = create("Ops", vec![guest("Ana"), member])
        .run(&app)
        .await
        .unwrap();
    let handles = members_by_handle(&app, &room.id).await;
    let ana = handles["ana"].clone();
    decision(
        &room.id,
        "deploy window",
        "Deploys happen on Tuesdays at 3pm.",
        None,
        None,
        &ana,
    )
    .run(&app)
    .await
    .unwrap();
    let asked = post(
        &app,
        &room.id,
        "@helper which day do we deploy on?",
        None,
        Some(&ana),
    )
    .await
    .message;

    let room_id = room.id.parse().unwrap();
    let view = super::queries::RoomView::load(
        &db,
        db.get_room(DEFAULT_ORG_ID, room_id).await.unwrap().unwrap(),
    )
    .await
    .unwrap();
    let message = db
        .get_room_message(room_id, asked.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let block = super::memory::recall_for_wake(&db, &view, &message)
        .await
        .unwrap();
    assert!(block.starts_with("<room_memory>"), "{block}");
    assert!(
        block.contains("decision on \"deploy window\" (current, stated)"),
        "{block}"
    );
    assert!(block.contains("Tuesdays at 3pm"), "{block}");
    // The message itself is not recalled back to the agent.
    assert!(!block.contains(&asked.id), "{block}");

    let members = ListRoomMembers {
        room_id: room.id.clone(),
        include_left: None,
    }
    .run(&app)
    .await
    .unwrap()
    .data;
    assert!(
        members
            .iter()
            .any(|m| m.handle == "helper" && m.memory_recall)
    );
}

#[tokio::test]
async fn rooms_record_facts_on_their_own_until_turned_off() {
    let db = Arc::new(StorageBackend::test_database());
    let app = ctx(&db, caller(None, OrgRole::Admin));
    let room = create("Quiet", vec![]).run(&app).await.unwrap();
    assert!(room.auto_facts);
    let room = UpdateRoom {
        room_id: room.id.clone(),
        name: None,
        topic: None,
        metadata: None,
        link_template: None,
        loop_limits: None,
        auto_facts: Some(false),
    }
    .run(&app)
    .await
    .unwrap();
    assert!(!room.auto_facts);
    let row = db
        .get_room(DEFAULT_ORG_ID, room.id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let message = crate::storage::RoomMessageRow {
        id: everruns_contracts::typed_id::RoomMessageId::new(),
        room_id: row.id,
        org_id: row.org_id,
        author_member_id: Some(everruns_contracts::typed_id::RoomMemberId::new()),
        thread_id: None,
        text: "We decided to ship the importer behind a flag on Friday.".into(),
        parts: serde_json::json!([]),
        mentions: vec![],
        hops: 0,
        session_id: None,
        source_ref: None,
        reply_count: 0,
        last_reply_at: None,
        pinned_at: None,
        edited_at: None,
        deleted_at: None,
        created_at: chrono::Utc::now(),
    };
    assert!(!super::memory::worth_reading(&row, &message));
}
