#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The step machine against an in-memory store. Dropping a step's future
//! mid-flight is what losing the isolate looks like to the log.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::events::EventData;
use everruns_provider::error::Result;
use serde_json::{Value, json};

use crate::cell::{Cell, Next, Progress, Store};
use crate::openai::{ChatCompletions, Transport};

#[derive(Default)]
struct Rows {
    events: Vec<(i64, String, bool)>,
    meta: std::collections::BTreeMap<String, String>,
    turn: Option<String>,
    inbox: Vec<(i64, String)>,
    inbox_seq: i64,
}

#[derive(Default)]
struct MemStore(Mutex<Rows>);

impl Store for MemStore {
    fn last_seq(&self) -> i64 {
        self.0.lock().unwrap().events.last().map_or(0, |row| row.0)
    }
    fn insert(&self, seq: i64, _event_type: &str, json: &str, committed: bool) {
        self.0
            .lock()
            .unwrap()
            .events
            .push((seq, json.into(), committed));
    }
    fn committed(&self) -> Vec<String> {
        let rows = self.0.lock().unwrap();
        rows.events
            .iter()
            .filter(|r| r.2)
            .map(|r| r.1.clone())
            .collect()
    }
    fn discard_uncommitted(&self) -> usize {
        let mut rows = self.0.lock().unwrap();
        let before = rows.events.len();
        rows.events.retain(|r| r.2);
        before - rows.events.len()
    }
    fn meta(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().meta.get(key).cloned()
    }
    fn set_meta(&self, key: &str, value: &str) {
        self.0.lock().unwrap().meta.insert(key.into(), value.into());
    }
    fn turn(&self) -> Option<String> {
        self.0.lock().unwrap().turn.clone()
    }
    fn set_turn(&self, turn: &str) {
        self.0.lock().unwrap().turn = Some(turn.into());
    }
    fn push_inbox(&self, message: &str) {
        let mut rows = self.0.lock().unwrap();
        rows.inbox_seq += 1;
        let id = rows.inbox_seq;
        rows.inbox.push((id, message.into()));
    }
    fn peek_inbox(&self) -> Option<(i64, String)> {
        self.0.lock().unwrap().inbox.first().cloned()
    }
    fn inbox_len(&self) -> usize {
        self.0.lock().unwrap().inbox.len()
    }
    fn commit(&self, turn: Option<&str>, meta: &[(&str, String)], consumed: Option<i64>) {
        let mut rows = self.0.lock().unwrap();
        rows.inbox.retain(|entry| Some(entry.0) != consumed);
        for row in &mut rows.events {
            row.2 = true;
        }
        rows.turn = turn.map(Into::into);
        for (k, v) in meta {
            rows.meta.insert((*k).into(), v.clone());
        }
    }
}

/// First call asks for `look_up`; once a tool result is in the history,
/// answers in text.
struct Scripted {
    calls: AtomicU32,
    delay: Duration,
}

#[async_trait]
impl Transport for Scripted {
    async fn post_json(&self, url: &str, bearer: &str, body: Value) -> Result<Value> {
        assert!(url.ends_with("/chat/completions"));
        assert_eq!(bearer, "test-key");
        self.calls.fetch_add(1, Ordering::SeqCst);
        everruns_provider::rt::sleep(self.delay).await;
        let messages = body["messages"].as_array().unwrap();
        let answered = messages.iter().any(|m| m["role"] == "tool");
        if answered {
            Ok(
                json!({"choices":[{"message":{"role":"assistant","content":"celld runs Durable Objects."}}]}),
            )
        } else {
            assert_eq!(body["tools"][0]["function"]["name"], "look_up");
            Ok(
                json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[
                    {"id":"call_1","type":"function","function":{"name":"look_up","arguments":"{\"topic\":\"celld\"}"}}
                ]}}]}),
            )
        }
    }
}

fn cell(store: &Arc<MemStore>, transport: &Arc<Scripted>) -> Cell<MemStore> {
    Cell {
        store: store.clone(),
        driver: Arc::new(ChatCompletions {
            base_url: "http://model.test/v1/".into(),
            api_key: "test-key".into(),
            transport: transport.clone(),
        }),
        model: "test-model".into(),
        tool_delay: Duration::from_millis(200),
    }
}

async fn run(cell: &Cell<MemStore>) {
    while cell.step().await.unwrap() == Progress::Stepped {}
}

fn types(cell: &Cell<MemStore>) -> Vec<String> {
    cell.events()
        .unwrap()
        .into_iter()
        .map(|e| e.event_type)
        .collect()
}

fn count(cell: &Cell<MemStore>, event_type: &str) -> usize {
    types(cell).iter().filter(|t| *t == event_type).count()
}

fn meta(store: &MemStore, key: &str) -> u64 {
    store.meta(key).map_or(0, |v| v.parse().unwrap())
}

#[tokio::test]
async fn a_turn_calls_the_tool_and_answers() {
    let store = Arc::new(MemStore::default());
    let transport = Arc::new(Scripted {
        calls: AtomicU32::new(0),
        delay: Duration::ZERO,
    });
    let cell = cell(&store, &transport);
    cell.post_message("what is celld?").unwrap();
    run(&cell).await;

    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(count(&cell, "tool.completed"), 1);
    assert_eq!(count(&cell, "turn.completed"), 1);
    let history = crate::cell::project(&cell.events().unwrap());
    let last = history.last().unwrap();
    assert_eq!(last.text(), Some("celld runs Durable Objects."));
    assert!(cell.open_turn().is_none());
    assert!(types(&cell).iter().all(|t| !t.ends_with(".delta")));
    // Nothing left to do.
    assert_eq!(cell.step().await.unwrap(), Progress::Idle);
}

#[tokio::test]
async fn losing_the_act_step_reruns_only_the_act_step() {
    let store = Arc::new(MemStore::default());
    let transport = Arc::new(Scripted {
        calls: AtomicU32::new(0),
        delay: Duration::ZERO,
    });
    let first = cell(&store, &transport);
    first.post_message("what is celld?").unwrap();
    assert_eq!(first.step().await.unwrap(), Progress::Stepped); // turn.started
    assert_eq!(first.step().await.unwrap(), Progress::Stepped); // reason
    assert!(matches!(first.open_turn().unwrap().next, Next::Act { .. }));

    // The isolate dies 50ms into a 200ms tool call.
    let lost = tokio::time::timeout(Duration::from_millis(50), first.step()).await;
    assert!(lost.is_err(), "the act step must still be running");
    drop(first);
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .turn
            .as_deref()
            .map(|t| t.contains("\"attempt\":1")),
        Some(true)
    );

    // A new isolate over the same storage resumes the turn.
    let second = cell(&store, &transport);
    run(&second).await;
    assert_eq!(meta(&store, "resumed_steps"), 1);
    assert_eq!(
        meta(&store, "act_steps"),
        1,
        "counters count committed steps"
    );
    assert_eq!(meta(&store, "reason_steps"), 2);
    assert_eq!(
        transport.calls.load(Ordering::SeqCst),
        2,
        "the committed model call is not repeated"
    );
    assert_eq!(
        count(&second, "tool.completed"),
        1,
        "one tool result in the log"
    );
    assert_eq!(count(&second, "turn.completed"), 1);
    let users: Vec<_> = second
        .events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.data, EventData::InputMessage(_)))
        .collect();
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn losing_the_reason_step_repeats_one_model_call() {
    let store = Arc::new(MemStore::default());
    let slow = Arc::new(Scripted {
        calls: AtomicU32::new(0),
        delay: Duration::from_millis(200),
    });
    let first = cell(&store, &slow);
    first.post_message("what is celld?").unwrap();
    first.step().await.unwrap(); // turn.started
    let lost = tokio::time::timeout(Duration::from_millis(50), first.step()).await;
    assert!(lost.is_err());
    drop(first);

    let second = cell(&store, &slow);
    run(&second).await;
    assert_eq!(meta(&store, "resumed_steps"), 1);
    assert_eq!(
        slow.calls.load(Ordering::SeqCst),
        3,
        "only the lost call repeats"
    );
    assert_eq!(count(&second, "output.message.completed"), 2);
    assert_eq!(count(&second, "turn.completed"), 1);
}

#[tokio::test]
async fn messages_queue_behind_the_open_turn() {
    let store = Arc::new(MemStore::default());
    let transport = Arc::new(Scripted {
        calls: AtomicU32::new(0),
        delay: Duration::ZERO,
    });
    let cell = cell(&store, &transport);
    cell.post_message("one").unwrap();
    cell.step().await.unwrap(); // turn.started
    cell.step().await.unwrap(); // reason: a tool call is now open
    cell.post_message("two").unwrap();
    assert_eq!(cell.store.inbox_len(), 1);
    run(&cell).await;
    assert_eq!(cell.store.inbox_len(), 0);
    let types = types(&cell);
    let first_done = types.iter().position(|t| t == "turn.completed").unwrap();
    let second_input = types.iter().rposition(|t| t == "input.message").unwrap();
    assert!(second_input > first_done, "{types:?}");
    assert_eq!(count(&cell, "turn.completed"), 2);
    assert_eq!(count(&cell, "turn.started"), 2);
}
