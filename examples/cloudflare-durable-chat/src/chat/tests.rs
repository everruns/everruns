#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::openai::{ChatCompletions, Transport};
use everruns_contracts::error::Result;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Default)]
struct Memory(Mutex<Data>);
impl Database for Memory {
    fn load(&self) -> Data {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, data: &Data) {
        let mut next = data.clone();
        next.revision += 1;
        *self.0.lock().unwrap() = next;
    }
}
struct Model {
    calls: AtomicUsize,
    child_delay: Duration,
}
#[async_trait]
impl Transport for Model {
    async fn post_json(&self, _url: &str, _key: &str, body: Value) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let messages = body["messages"].as_array().unwrap();
        let last = messages.last().unwrap();
        let names: Vec<_> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        let tool = |name: &str, args: Value| json!({"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":args.to_string()}}]});
        let text = |s: &str| json!({"role":"assistant","content":s});
        let message = if last["role"] == "tool" {
            text("The event store is PostgreSQL (see cited conversation).")
        } else if names == ["search_conversations"] {
            everruns_contracts::rt::sleep(self.child_delay).await;
            tool("search_conversations", json!({"query":"PostgreSQL"}))
        } else if last["content"].as_str().unwrap_or("").contains("What did") {
            tool(
                "search_history",
                json!({"question":"What did we decide about the event store?"}),
            )
        } else {
            text("Remembered.")
        };
        Ok(json!({"choices":[{"message":message}]}))
    }
}
fn room(db: Arc<Memory>, child_delay: Duration) -> (Room, Arc<Model>) {
    let model = Arc::new(Model {
        calls: AtomicUsize::new(0),
        child_delay,
    });
    (
        Room {
            db,
            driver: Arc::new(ChatCompletions {
                base_url: "http://test/v1".into(),
                api_key: "test".into(),
                transport: model.clone(),
            }),
            model: "test".into(),
        },
        model,
    )
}
fn create(room: &Room, key: &str) -> String {
    room.command(Command {
        request_id: key.into(),
        action: Action::Create { title: key.into() },
    })
    .unwrap()["conversation"]
        .as_str()
        .unwrap()
        .into()
}
fn send(room: &Room, id: &str, key: &str, text: &str) -> Value {
    room.command(Command {
        request_id: key.into(),
        action: Action::Send {
            conversation: id.into(),
            username: "Alice".into(),
            text: text.into(),
        },
    })
    .unwrap()
}
async fn settle(room: &Room, id: &str) {
    while room.engine(id).step().await.unwrap() == Progress::Stepped {}
}

#[tokio::test]
async fn retry_receipts_queue_once_and_survive_reopening() {
    let db = Arc::new(Memory::default());
    let (first, _) = room(db.clone(), Duration::ZERO);
    let id = create(&first, "create");
    send(&first, &id, "send", "hello");
    let (reopened, _) = room(db.clone(), Duration::ZERO);
    assert_eq!(create(&reopened, "create"), id);
    send(&reopened, &id, "send", "hello");
    assert_eq!(db.load().conversations[&id].inbox.len(), 1);
    assert!(
        reopened
            .command(Command {
                request_id: "send".into(),
                action: Action::Create {
                    title: "conflict".into()
                }
            })
            .is_err()
    );
    settle(&reopened, &id).await;
    assert_eq!(project(&reopened.engine(&id).events().unwrap()).len(), 2);
    let (other, _) = room(Arc::new(Memory::default()), Duration::ZERO);
    assert!(other.db.load().conversations.is_empty());
}

#[tokio::test]
async fn forks_copy_only_a_valid_committed_prefix() {
    let (room, _) = room(Arc::new(Memory::default()), Duration::ZERO);
    let parent = create(&room, "create");
    send(&room, &parent, "one", "hello");
    settle(&room, &parent).await;
    let events = room.engine(&parent).events().unwrap();
    let sequence = i64::from(
        events
            .iter()
            .find(|e| e.event_type == "output.message.completed")
            .unwrap()
            .sequence
            .unwrap(),
    );
    let cmd = Command {
        request_id: "fork".into(),
        action: Action::Fork {
            conversation: parent.clone(),
            sequence,
        },
    };
    let fork = room.command(cmd.clone()).unwrap()["conversation"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(room.command(cmd).unwrap()["conversation"], fork);
    send(&room, &parent, "two", "only the parent");
    settle(&room, &parent).await;
    assert_eq!(project(&room.engine(&fork).events().unwrap()).len(), 2);
    assert!(room.engine(&fork).open_turn().is_none());
    assert_eq!(room.engine(&fork).store.inbox_len(), 0);
    assert!(
        room.command(Command {
            request_id: "bad".into(),
            action: Action::Fork {
                conversation: parent,
                sequence: 999
            }
        })
        .is_err()
    );
}

#[tokio::test]
async fn schedule_replay_and_duplicate_alarms_deliver_once() {
    let db = Arc::new(Memory::default());
    let (first, _) = room(db.clone(), Duration::ZERO);
    let id = create(&first, "create");
    let tool = ChatTool {
        behavior: ChatBehavior {
            room: first.clone(),
            id: id.clone(),
        },
        name: "schedule",
    };
    let mut ctx = ToolContext::new(first.engine(&id).session_id());
    ctx.tool_call_id = Some("schedule-call".into());
    ctx.idempotency_key = Some("schedule-execution".into());
    let args = json!({"instruction":"review launch","delay_seconds":10});
    assert!(
        tool.execute_with_context(args.clone(), &ctx)
            .await
            .is_success()
    );
    assert!(tool.execute_with_context(args, &ctx).await.is_success());
    assert_eq!(db.load().schedules.len(), 1);
    let due = first.next_deadline().unwrap();
    first.deliver_due(due - 1);
    assert_eq!(first.engine(&id).store.inbox_len(), 0);
    let (restarted, _) = room(db.clone(), Duration::ZERO);
    restarted.deliver_due(due);
    restarted.deliver_due(due + 500);
    assert_eq!(restarted.engine(&id).store.inbox_len(), 1);
    assert!(restarted.next_deadline().is_none());
    settle(&restarted, &id).await;
    assert_eq!(project(&restarted.engine(&id).events().unwrap()).len(), 2);
}

#[tokio::test]
async fn recurring_schedules_collapse_missed_ticks_and_cancel() {
    let db = Arc::new(Memory::default());
    let (room, _) = room(db.clone(), Duration::ZERO);
    let id = create(&room, "create");
    let tool = ChatTool {
        behavior: ChatBehavior {
            room: room.clone(),
            id: id.clone(),
        },
        name: "schedule",
    };
    let mut ctx = ToolContext::new(room.engine(&id).session_id());
    ctx.tool_call_id = Some("tick".into());
    ctx.idempotency_key = Some("tick-execution".into());
    tool.execute_with_context(
        json!({"instruction":"tick","delay_seconds":1,"repeat_seconds":10}),
        &ctx,
    )
    .await;
    let due = room.next_deadline().unwrap();
    room.deliver_due(due + 100000);
    assert_eq!(room.engine(&id).store.inbox_len(), 1);
    assert_eq!(room.next_deadline(), Some(due + 110000));
    let key = db.load().schedules.keys().next().unwrap().clone();
    room.command(Command {
        request_id: "cancel".into(),
        action: Action::CancelSchedule { schedule: key },
    })
    .unwrap();
    room.deliver_due(due + 1000000);
    assert_eq!(room.engine(&id).store.inbox_len(), 1);
}

#[tokio::test]
async fn interrupted_search_reopens_the_same_child_and_keeps_matches_out_of_parent() {
    let db = Arc::new(Memory::default());
    let (first, model) = room(db.clone(), Duration::from_millis(100));
    let original = create(&first, "original");
    send(
        &first,
        &original,
        "fact",
        "We chose PostgreSQL for the event store.",
    );
    settle(&first, &original).await;
    let parent = create(&first, "question");
    send(
        &first,
        &parent,
        "ask",
        "What did we decide about the event store?",
    );
    let cell = first.engine(&parent);
    cell.step().await.unwrap();
    cell.step().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), cell.step())
            .await
            .is_err()
    );
    let child = db
        .load()
        .conversations
        .iter()
        .find(|(_, c)| c.owner.is_some())
        .unwrap()
        .0
        .clone();
    // Rebuild the host over persisted data. The child is already materialized
    // but its first model request was cut off.
    settle(&first, &parent).await;
    let data = db.load();
    assert_eq!(
        data.conversations
            .values()
            .filter(|c| c.owner.is_some())
            .count(),
        1
    );
    assert_eq!(
        committed(&data.conversations[&child])
            .iter()
            .filter(|e| e.event_type == "input.message")
            .count(),
        1
    );
    let snapshot = first.snapshot();
    let parent_view = snapshot["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == parent)
        .unwrap();
    assert!(parent_view["messages"].as_array().unwrap().iter().any(|m| {
        m["role"] == "tool_result"
            && m["text"]
                .as_str()
                .is_some_and(|text| text.contains("PostgreSQL"))
    }));
    let parent_events = cell.events().unwrap();
    assert_eq!(
        parent_events
            .iter()
            .filter(|e| e.event_type == "turn.completed")
            .count(),
        1
    );
    assert!(!parent_events.iter().any(
        |e| matches!(&e.data,EventData::ToolCompleted(d) if d.tool_name=="search_conversations")
    ));
    assert!(model.calls.load(Ordering::SeqCst) >= 5);
    let hits = first.search("PostgreSQL");
    assert!(
        hits.as_array()
            .unwrap()
            .iter()
            .all(|h| h["conversation"] != child)
    );
    // Search-generated assistant answers must not become future evidence.
    assert!(
        !first
            .search("cited conversation")
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["conversation"] == parent)
    );
}
