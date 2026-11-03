use chrono::{TimeZone, Utc};
use everruns_contracts::model_profiles::CostBreakdown;
use everruns_contracts::typed_id::TurnId;
use uuid::Uuid;

use super::*;
use crate::storage::backend::session_cost::{
    CostGenerationRow, CostScheduleRow, CostToolRow, CostTurnOriginRow, CostTurnRow,
};

fn generation(turn: Uuid, minute: u32, actual: Option<f64>) -> CostGenerationRow {
    CostGenerationRow {
        turn_id: Some(turn),
        created_at: Utc.with_ymd_and_hms(2026, 10, 31, 22, minute, 0).unwrap(),
        model: Some("m".into()),
        provider: Some("openrouter".into()),
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        cache_read_tokens: 1_000_000,
        cache_creation_tokens: 0,
        actual_cost_usd: actual,
        estimated_cost_usd: None,
    }
}

/// $1/M uncached, $0.1/M cache read, $2/M output: 1M of each is $3.10.
fn priced(_: Option<&str>, model: Option<&str>, t: CallTokens) -> Option<CostBreakdown> {
    (model == Some("m")).then(|| CostBreakdown {
        uncached_input: t.uncached_input as f64 / 1e6,
        cache_read: t.cache_read as f64 * 0.1 / 1e6,
        cache_write: 0.0,
        output: t.output as f64 * 2.0 / 1e6,
    })
}

fn turn(no: i32, id: Uuid) -> CostTurnRow {
    CostTurnRow {
        turn_no: no,
        turn_id: id,
        started_at: Utc.with_ymd_and_hms(2026, 10, 31, 22, 0, 0).unwrap(),
        status: "completed".into(),
    }
}

fn usd(cost: &SessionCost, key: &str) -> f64 {
    cost.categories.iter().find(|c| c.key == key).unwrap().usd
}

#[test]
fn category_split_scales_to_the_reported_cost() {
    let t = Uuid::now_v7();
    // Provider billed $6.20, twice the price table: every category doubles.
    let cost = aggregate(
        "s".into(),
        &[generation(t, 0, Some(6.2))],
        &[],
        &[],
        &[],
        vec![],
        &priced,
    );
    assert!((cost.total_usd - 6.2).abs() < 1e-9);
    assert!((usd(&cost, UNCACHED_INPUT) - 2.0).abs() < 1e-9);
    assert!((usd(&cost, CACHE_READ) - 0.2).abs() < 1e-9);
    assert!((usd(&cost, OUTPUT) - 4.0).abs() < 1e-9);
    assert_eq!(cost.unattributed_usd, 0.0);
    let keys: Vec<_> = cost.categories.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, [UNCACHED_INPUT, CACHE_READ, CACHE_WRITE, OUTPUT]);
}

#[test]
fn price_table_fills_in_when_nothing_was_recorded() {
    let cost = aggregate(
        "s".into(),
        &[generation(Uuid::now_v7(), 0, None)],
        &[],
        &[],
        &[],
        vec![],
        &priced,
    );
    assert!((cost.total_usd - 3.1).abs() < 1e-9);
    assert_eq!(cost.unpriced_calls, 0);
}

#[test]
fn a_reported_cost_without_a_price_table_stays_unattributed() {
    let mut row = generation(Uuid::now_v7(), 0, Some(1.5));
    row.model = Some("unknown-model".into());
    let cost = aggregate("s".into(), &[row], &[], &[], &[], vec![], &priced);
    assert!((cost.total_usd - 1.5).abs() < 1e-9);
    assert!((cost.unattributed_usd - 1.5).abs() < 1e-9);
    assert_eq!(usd(&cost, UNCACHED_INPUT), 0.0);
    assert!((cost.hours[0].unattributed_usd - 1.5).abs() < 1e-9);
}

#[test]
fn a_call_with_no_cost_at_all_is_counted_as_unpriced() {
    let mut row = generation(Uuid::now_v7(), 0, None);
    row.model = Some("unknown-model".into());
    let cost = aggregate("s".into(), &[row], &[], &[], &[], vec![], &priced);
    assert_eq!(cost.total_usd, 0.0);
    assert_eq!(cost.unpriced_calls, 1);
    assert_eq!(cost.categories[0].tokens, 1_000_000);
}

#[test]
fn calls_are_bucketed_by_utc_hour() {
    let t = Uuid::now_v7();
    let mut late = generation(t, 0, Some(1.0));
    late.created_at = Utc.with_ymd_and_hms(2026, 10, 31, 23, 59, 0).unwrap();
    let rows = [
        generation(t, 5, Some(1.0)),
        generation(t, 50, Some(1.0)),
        late,
    ];
    let cost = aggregate("s".into(), &rows, &[], &[], &[], vec![], &priced);
    assert_eq!(cost.hours.len(), 2);
    assert_eq!(
        cost.hours[0].hour,
        Utc.with_ymd_and_hms(2026, 10, 31, 22, 0, 0).unwrap()
    );
    assert_eq!(cost.hours[0].model_calls, 2);
    assert_eq!(cost.hours[1].model_calls, 1);
}

#[test]
fn turns_are_grouped_by_what_started_them() {
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let origin = |id: Uuid, kind: Option<&str>, source: Option<&str>| CostTurnOriginRow {
        turn_id: TurnId::from_uuid(id).to_string(),
        kind: kind.map(Into::into),
        source_id: source.map(Into::into),
    };
    let sched = "sched_01a1510ebffc7370a495ed68cc7f9ab3";
    let cost = aggregate(
        "s".into(),
        &[
            generation(a, 0, Some(1.0)),
            generation(b, 1, Some(2.0)),
            generation(c, 2, Some(3.0)),
        ],
        &[turn(1, a), turn(2, b), turn(3, c)],
        &[
            origin(a, None, None),
            origin(b, Some("schedule"), Some(sched)),
            origin(c, Some("schedule"), Some(sched)),
        ],
        &[CostScheduleRow {
            public_id: sched.into(),
            description: "Shipmail claim poll".into(),
            cron_expression: Some("*/5 * * * *".into()),
            enabled: false,
            deleted: false,
        }],
        vec![],
        &priced,
    );
    assert_eq!(cost.sources.len(), 2);
    let schedule = &cost.sources[0];
    assert_eq!(schedule.kind, "schedule");
    assert_eq!(schedule.turns, 2);
    assert!((schedule.usd - 5.0).abs() < 1e-9);
    assert_eq!(schedule.name.as_deref(), Some("Shipmail claim poll"));
    assert_eq!(schedule.active, Some(false));
    assert_eq!(cost.sources[1].kind, USER_SOURCE);
    assert_eq!(cost.sources[1].active, None);
    assert_eq!(cost.turns[1].source_kind, "schedule");
    assert_eq!(cost.turns[1].prompt_tokens, 2_000_000);
}

#[test]
fn a_long_session_returns_its_newest_turns() {
    let turns: Vec<_> = (1..=(MAX_TURNS as i32 + 3))
        .map(|n| turn(n, Uuid::now_v7()))
        .collect();
    let cost = aggregate("s".into(), &[], &turns, &[], &[], vec![], &priced);
    assert!(cost.turns_truncated);
    assert_eq!(cost.turns.len(), MAX_TURNS);
    assert_eq!(cost.turns[0].turn, 4);
}

#[test]
fn tool_totals_add_up_the_items() {
    let tool = |name: &str, calls, failed, duration_ms| CostToolRow {
        name: name.into(),
        calls,
        failed,
        duration_ms,
    };
    let cost = aggregate(
        "s".into(),
        &[],
        &[],
        &[],
        &[],
        vec![tool("web_fetch", 10, 2, 5000), tool("read_file", 4, 0, 40)],
        &priced,
    );
    assert_eq!(
        (cost.tools.calls, cost.tools.failed, cost.tools.duration_ms),
        (14, 2, 5040)
    );
    assert_eq!(cost.tools.items[0].name, "web_fetch");
}

// ---------------------------------------------------------------------------
// The reads against a real database: event payloads as the runtime writes them.
// ---------------------------------------------------------------------------

mod reads {
    use chrono::Utc;
    use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
    use serde_json::{Value, json};

    use crate::storage::{CreateEventRow, StorageBackend};

    fn event(
        session: SessionId,
        turn: TurnId,
        kind: &str,
        data: Value,
        meta: Option<Value>,
    ) -> CreateEventRow {
        CreateEventRow {
            session_id: session,
            event_type: kind.to_string(),
            ts: Utc::now(),
            context: json!({"turn_id": turn.to_string()}),
            data,
            metadata: meta,
            tags: None,
        }
    }

    /// A turn: its input message (with `initiator`), turn start, a tool call, turn end.
    fn turn(
        session: SessionId,
        initiator: Option<Value>,
        tool_ok: bool,
    ) -> (TurnId, Vec<CreateEventRow>) {
        let turn = TurnId::new();
        let message = MessageId::new();
        let call = format!("call_{}", turn.uuid().simple());
        let rows = vec![
            event(
                session,
                turn,
                "input.message",
                json!({"message": {"id": message.to_string(), "role": "user",
                    "content": [{"type": "text", "text": "poll"}], "created_at": Utc::now()}}),
                initiator.map(|i| json!({"initiator": i})),
            ),
            event(
                session,
                turn,
                "turn.started",
                json!({"turn_id": turn.to_string(), "input_message_id": message.to_string()}),
                None,
            ),
            event(
                session,
                turn,
                "tool.started",
                json!({"tool_call": {"id": call, "name": "web_fetch", "arguments": {}}}),
                None,
            ),
            event(
                session,
                turn,
                "tool.completed",
                json!({"tool_call_id": call, "tool_name": "web_fetch", "success": tool_ok,
                    "status": if tool_ok { "success" } else { "error" }, "duration_ms": 250}),
                None,
            ),
            event(
                session,
                turn,
                "turn.completed",
                json!({"turn_id": turn.to_string(), "iterations": 1}),
                None,
            ),
        ];
        (turn, rows)
    }

    #[tokio::test]
    async fn reads_turn_origins_tools_and_generations() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await;
        let org = everruns_core::DEFAULT_ORG_ID;
        let sched = "sched_01a1510ebffc7370a495ed68cc7f9ab3";

        let (person, rows) = turn(session, Some(json!({"type": "user", "user_id": "u"})), true);
        db.create_events(rows).await.unwrap();
        let (scheduled, rows) = turn(
            session,
            Some(json!({"type": "schedule", "schedule_id": sched})),
            false,
        );
        db.create_events(rows).await.unwrap();
        db.database()
            .catch_up_session_trace_fully(session.uuid())
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO llm_generations (org_id, session_id, turn_id, model, provider, \
                input_tokens, output_tokens, cache_read_tokens, actual_cost_usd) \
             VALUES ($1, $2, $3, 'm', 'openrouter', 100, 10, 50, 0.5), \
                    ($1, $2, $4, 'm', 'openrouter', 200, 20, 0, 1.5)",
        )
        .bind(org)
        .bind(session.uuid())
        .bind(person.uuid())
        .bind(scheduled.uuid())
        .execute(db.database().pool())
        .await
        .unwrap();

        let generations = db
            .session_cost_generations(org, session.uuid())
            .await
            .unwrap();
        assert_eq!(generations.len(), 2);
        // Another org sees nothing.
        assert!(
            db.session_cost_generations(org + 1, session.uuid())
                .await
                .unwrap()
                .is_empty()
        );

        let turns = db.session_cost_turns(session.uuid()).await.unwrap();
        assert_eq!(turns.len(), 2);

        let origins = db.session_cost_turn_origins(session.uuid()).await.unwrap();
        let of = |t: TurnId| origins.iter().find(|o| o.turn_id == t.to_string()).unwrap();
        assert_eq!(of(person).kind, None);
        assert_eq!(of(scheduled).kind.as_deref(), Some("schedule"));
        assert_eq!(of(scheduled).source_id.as_deref(), Some(sched));

        let tools = db.session_cost_tools(session.uuid()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(
            (tools[0].calls, tools[0].failed, tools[0].duration_ms),
            (2, 1, 500)
        );

        assert!(
            db.session_cost_schedules(org, session.uuid())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            db.session_cost_sandboxes(org, session.uuid())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            db.session_cost_sandbox_transitions(org, session.uuid())
                .await
                .unwrap()
                .is_empty()
        );

        let cost = super::aggregate(
            session.to_string(),
            &generations,
            &turns,
            &origins,
            &[],
            tools,
            &|_, _, _| None,
        );
        assert!((cost.total_usd - 2.0).abs() < 1e-9);
        assert_eq!(cost.sources[0].kind, "schedule");
        assert!((cost.sources[0].usd - 1.5).abs() < 1e-9);
        assert_eq!(cost.sources[1].kind, super::USER_SOURCE);
        assert_eq!(cost.turns.len(), 2);
    }

    /// A run is the first session and every session started under it, read
    /// from any of them, with turns from the trace index and spend per turn.
    #[tokio::test]
    async fn a_run_is_read_from_any_of_its_sessions() {
        let db = StorageBackend::test_database();
        let org = everruns_core::DEFAULT_ORG_ID;
        let root = db.create_test_session().await;
        let mut child = crate::storage::test_database::test_session_row(org);
        child.parent_session_id = Some(root);
        let child = db.create_session(child).await.unwrap().id;
        let other = db.create_test_session().await;

        let (handed_off, rows) = turn(child, None, true);
        db.create_events(rows).await.unwrap();
        sqlx::query(
            "INSERT INTO llm_generations (org_id, session_id, turn_id, model, provider, \
                input_tokens, output_tokens, cache_read_tokens, actual_cost_usd) \
             VALUES ($1, $2, $3, 'm', 'openrouter', 100, 10, 0, 0.25), \
                    ($1, $2, $3, 'm', 'openrouter', 100, 10, 0, 0.5)",
        )
        .bind(org)
        .bind(child.uuid())
        .bind(handed_off.uuid())
        .execute(db.database().pool())
        .await
        .unwrap();

        // The session itself comes first and carries the run's root.
        let first = db.list_run_sessions(org, child, 1).await.unwrap();
        assert_eq!(first[0].id, child);
        assert_eq!(first[0].root_session_id, Some(root));

        let sessions = db.list_run_sessions(org, root, 10).await.unwrap();
        let ids: Vec<_> = sessions.iter().map(|s| s.id).collect();
        assert_eq!(ids, [root, child], "first session first, never another run");
        assert!(!ids.contains(&other));
        assert!(db.list_run_sessions(org + 1, root, 10).await.unwrap().is_empty());

        let uuids: Vec<_> = ids.iter().map(|id| id.uuid()).collect();
        for id in &uuids {
            db.database().catch_up_session_trace_fully(*id).await.unwrap();
        }
        let turns = db.run_turns(&uuids, 10).await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].session_id, child.uuid());
        assert!(turns[0].ended_at.is_some());

        let costs = db.run_turn_costs(org, &uuids).await.unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].turn_id, Some(handed_off.uuid()));
        assert!((costs[0].cost_usd - 0.75).abs() < 1e-9);
        assert!(db.run_turn_costs(org + 1, &uuids).await.unwrap().is_empty());
    }
}
