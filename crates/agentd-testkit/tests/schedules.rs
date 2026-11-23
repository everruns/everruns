//! A session's schedules on the data plane (data plane v2, step 11.4): they
//! live in the session's shard state and the shard's leader fires them. The
//! agent's own `create_schedule` tool and the console's schedule API reach
//! them, an occurrence arrives once whoever leads, and the bench holds the
//! step's gate: 10 000 schedules fire within 1 s of their time.
//!
//! The bench prints its numbers for the step's record (`cargo test -p
//! agentd-testkit --test schedules -- --nocapture`).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use everruns_agentd::platform::Op;
use everruns_agentd::state::Command;
use everruns_contracts::typed_id::{EventId, SessionId};
use futures::StreamExt;
use serde_json::{Value, json};
use support::{Fleet, answers, now_ms};

fn rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339()
}

async fn schedules(fleet: &Fleet, session: &str) -> Vec<Value> {
    fleet
        .get(&format!("/v1/sessions/{session}/schedules"))
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A request through the console's API; its status and body.
async fn call(fleet: &Fleet, method: reqwest::Method, path: &str, body: Value) -> (u16, Value) {
    let response = fleet
        .http
        .request(method, format!("{}{path}", fleet.base()))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_schedules_its_own_next_run() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("files").await;

    // The agent schedules a run with its own tool, far enough out that the
    // first turn (2.4 s in a debug build of the whole workspace) is over
    // before it is due.
    let due = now_ms() + 6_000;
    fleet
        .say(&session, &format!("at {} check the queue", rfc3339(due)))
        .await;
    fleet.turns(&session, 1).await;
    assert!(
        now_ms() < due,
        "the first turn ended after the schedule's time"
    );
    let answer = answers(&fleet.events(&session).await).remove(0);
    assert!(answer.contains("\"created\":true"), "{answer}");
    let listed = schedules(&fleet, &session).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["enabled"], true);
    let id = listed[0]["id"].as_str().unwrap().to_owned();

    // It arrives as the session's next turn, from the schedule.
    fleet.turns(&session, 2).await;
    let events = fleet.events(&session).await;
    let fired: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "input.message")
        .filter(|e| e["data"]["message"]["content"][0]["text"] == "check the queue")
        .collect();
    assert_eq!(fired.len(), 1, "one occurrence");
    let after = schedules(&fleet, &session).await;
    assert_eq!(after[0]["trigger_count"], 1);
    assert_eq!(after[0]["enabled"], false, "a one-shot is done");
    let fired_at = DateTime::parse_from_rfc3339(after[0]["last_triggered_at"].as_str().unwrap())
        .unwrap()
        .timestamp_millis();
    assert!(
        (due..due + 1_000).contains(&fired_at),
        "fired {} ms after its time",
        fired_at - due
    );

    // The console runs it again, turns it back on, and deletes it.
    let path = format!("/v1/sessions/{session}/schedules/{id}");
    let (status, run) = call(
        &fleet,
        reqwest::Method::POST,
        &format!("{path}/trigger"),
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{run}");
    assert_eq!(run["trigger_count"], 2);
    fleet.turns(&session, 3).await;
    let (status, patched) = call(
        &fleet,
        reqwest::Method::PATCH,
        &path,
        json!({ "max_runs": 5, "ends_at": null }),
    )
    .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["max_runs"], 5);
    // A spend cap is kept with the schedule; a cap that is no amount is
    // refused.
    let (status, capped) = call(
        &fleet,
        reqwest::Method::PATCH,
        &path,
        json!({ "budget_usd": 1.0 }),
    )
    .await;
    assert_eq!(status, 200, "{capped}");
    assert_eq!(capped["budget"]["limit_usd"], 1.0, "{capped}");
    assert_eq!(capped["budget"]["status"], "active", "{capped}");
    let (status, refused) = call(
        &fleet,
        reqwest::Method::PATCH,
        &path,
        json!({ "budget_usd": -1.0 }),
    )
    .await;
    assert_eq!(status, 400, "{refused}");
    let (status, _) = call(&fleet, reqwest::Method::DELETE, &path, Value::Null).await;
    assert_eq!(status, 204);
    assert!(schedules(&fleet, &session).await.is_empty());
    let (status, _) = call(&fleet, reqwest::Method::GET, &path, Value::Null).await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &fleet,
        reqwest::Method::POST,
        &format!("{path}/trigger"),
        json!({}),
    )
    .await;
    assert_eq!(status, 404);
}

/// A replica that has not applied a committed delete must not answer for it:
/// after the 204 a read on any node shows the schedule gone or answers 503,
/// never the deleted schedule.
#[tokio::test(flavor = "multi_thread")]
async fn a_lagging_replica_never_reads_a_deleted_schedule() {
    let mut fleet = Fleet::start().await;
    let session = fleet.create_in("files", 0).await;
    let id = SessionId::parse(&session).unwrap();
    let schedule = everruns_agentd::platform_store::build(
        id,
        "tick".into(),
        None,
        DateTime::from_timestamp_millis(now_ms() + 3_600_000),
        "UTC".into(),
        Utc::now(),
    )
    .unwrap();
    let schedule_id = schedule.id.to_string();
    let command = Command::Platform {
        session: id,
        action: Op::CreateSchedule {
            schedule: Box::new(schedule),
        },
        now_ms: now_ms(),
        seed: EventId::new(),
    };
    let created = fleet
        .propose(0, serde_json::to_value(command).unwrap())
        .await;
    assert_eq!(created["result"], "value", "{created}");

    // A follower misses the delete: down while it commits, then back with
    // its state from before it.
    let leader = fleet.leader(0).await;
    let follower = (1..=3).find(|n| *n != leader).unwrap();
    let (_, addr) = fleet
        .addrs(0)
        .into_iter()
        .find(|(node, _)| *node == follower)
        .unwrap();
    fleet.kill_node(0, follower);
    let path = format!("/v1/sessions/{session}/schedules/{schedule_id}");
    let (status, _) = call(&fleet, reqwest::Method::DELETE, &path, Value::Null).await;
    assert_eq!(status, 204);
    fleet.start_node(0, follower);

    // Every answer the follower gives is fresh: 503 while it cannot sync,
    // then the list without the schedule. Only non-answers are waited out:
    // no connection yet, or 421 while it knows it is catching up
    // (`catchup.rs` `guard`).
    let url = format!("http://{addr}/shard/sessions/{session}/schedules");
    let deadline = Instant::now() + Duration::from_secs(30);
    let listed = loop {
        assert!(Instant::now() < deadline, "the follower never answered");
        let Ok(response) = fleet.http.post(&url).json(&json!({})).send().await else {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        };
        match response.status().as_u16() {
            421 | 503 => tokio::time::sleep(Duration::from_millis(20)).await,
            200 => break response.json::<Value>().await.unwrap(),
            other => panic!("the follower answered {other}"),
        }
    };
    assert_eq!(listed["reply"], "ok", "{listed}");
    let live = listed["value"].as_array().unwrap();
    assert!(
        live.iter().all(|s| s["id"] != schedule_id.as_str()),
        "a deleted schedule read back from a lagging replica: {listed}"
    );
    fleet.check();
}

/// Sessions in the bench, five schedules each.
const SESSIONS: usize = 2_000;

/// Put five one-shot schedules due at `due_ms` on each of `sessions`.
async fn plant(fleet: &Fleet, sessions: &[String], due_ms: i64) {
    futures::stream::iter(sessions.iter().cloned())
        .map(|session| async move {
            let id = SessionId::parse(&session).unwrap();
            for _ in 0..5 {
                let schedule = everruns_agentd::platform_store::build(
                    id,
                    "tick".into(),
                    None,
                    DateTime::from_timestamp_millis(due_ms),
                    "UTC".into(),
                    Utc::now(),
                )
                .unwrap();
                let command = Command::Platform {
                    session: id,
                    action: Op::CreateSchedule {
                        schedule: Box::new(schedule),
                    },
                    now_ms: now_ms(),
                    seed: EventId::new(),
                };
                // The session's shard, following a move (shards balance load)
                // and waiting out one in flight.
                let command = serde_json::to_value(command).unwrap();
                let mut shard = 0;
                let response = loop {
                    let response = fleet.propose(shard, command.clone()).await;
                    match response["shard"].as_u64() {
                        Some(to) if response["result"] == "moved" => shard = to as u32,
                        _ if response["result"] == "frozen" => {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        _ => break response,
                    }
                };
                assert_eq!(response["result"], "value", "{response}");
            }
        })
        .buffer_unordered(32)
        .collect::<Vec<_>>()
        .await;
}

/// Each schedule's lag past `due_ms`, once all `5 * sessions` fired once.
async fn lags(fleet: &Fleet, sessions: &[String], due_ms: i64) -> Vec<i64> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let all: Vec<Vec<Value>> = futures::stream::iter(sessions.iter().cloned())
            .map(|session| async move { schedules(fleet, &session).await })
            .buffer_unordered(32)
            .collect()
            .await;
        let round: Vec<&Value> = all
            .iter()
            .flatten()
            .filter(|s| {
                DateTime::parse_from_rfc3339(s["scheduled_at"].as_str().unwrap_or_default())
                    .is_ok_and(|t| t.timestamp_millis() == due_ms)
            })
            .collect();
        let fired = round.iter().filter(|s| s["trigger_count"] == 1).count();
        assert!(
            round.iter().all(|s| s["trigger_count"].as_u64() <= Some(1)),
            "a schedule fired twice"
        );
        if fired == sessions.len() * 5 {
            let mut lags: Vec<i64> = round
                .iter()
                .map(|s| {
                    DateTime::parse_from_rfc3339(s["last_triggered_at"].as_str().unwrap())
                        .unwrap()
                        .timestamp_millis()
                        - due_ms
                })
                .collect();
            lags.sort_unstable();
            return lags;
        }
        assert!(
            Instant::now() < deadline,
            "{fired} of {} fired",
            sessions.len() * 5
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn report(name: &str, lags: &[i64]) {
    let at = |p: f64| lags[((lags.len() - 1) as f64 * p).round() as usize];
    println!(
        "{name}: {} schedules, lag p50 {} ms, p99 {} ms, max {} ms",
        lags.len(),
        at(0.5),
        at(0.99),
        lags[lags.len() - 1]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ten_thousand_schedules_fire_on_time() {
    let mut fleet = Fleet::start().await;
    // No executor: the bench measures the fire (the occurrence admitted),
    // not the turns it starts.
    let started = Instant::now();
    let sessions: Vec<String> = futures::stream::iter(0..SESSIONS)
        .map(|_| fleet.create_in("echo", 0))
        .buffer_unordered(32)
        .collect()
        .await;
    println!("{SESSIONS} sessions in {:?}", started.elapsed());

    // All 10 000 due at the same instant.
    let due = now_ms() + 30_000;
    let started = Instant::now();
    plant(&fleet, &sessions, due).await;
    println!("10 000 schedules planted in {:?}", started.elapsed());
    assert!(now_ms() < due, "planting took past the due time");
    let steady = lags(&fleet, &sessions, due).await;
    report("steady", &steady);
    assert!(
        steady[steady.len() - 1] < 1_000,
        "{:?}",
        &steady[steady.len() - 10..]
    );

    // The leader dies just before the next 10 000 are due: the new leader
    // fires each once, as soon as it leads.
    let due = now_ms() + 30_000;
    plant(&fleet, &sessions, due).await;
    let leader = fleet.leader(0).await;
    while now_ms() < due - 200 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fleet.kill_node(0, leader);
    let killed = now_ms();
    let next = fleet.leader(0).await;
    let elected = now_ms() - killed;
    let failover = lags(&fleet, &sessions, due).await;
    report(
        &format!("leader {leader} killed, {next} elected after {elected} ms"),
        &failover,
    );
    // Late by the election (1.5 to 3 s without a leader), then on time.
    let after_election = (killed - due).max(0) + elected;
    assert!(
        failover[failover.len() - 1] < after_election + 1_000,
        "max lag {} ms, election done {after_election} ms after the due time",
        failover[failover.len() - 1]
    );
}
