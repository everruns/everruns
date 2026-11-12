//! The session stream against a live fleet (design.md, step 8): every
//! session has one stream node, picked by rendezvous hash over its cell's
//! members, and deltas are fenced by the epoch that produced them.
//!
//! The four cases the step 8 plan names:
//! - a stale publisher (a superseded epoch) reaches no watcher;
//! - a publisher hashing with another view is redirected to the owner;
//! - a stream node that restarts and lags behind resumes the stream with
//!   no gap and no repeat;
//! - a move while the stream replays and then goes live keeps one stream.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use everruns_cell::stream::{Deliver, LiveBatch, LiveItem, Redirect, View};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::events::{Event, EventContext, OutputMessageDeltaData};
use serde_json::{Value, json};
use support::Fleet;

/// One frame of a session stream.
#[derive(Debug, Clone)]
struct Frame {
    event: String,
    id: Option<String>,
    data: Value,
}

/// A session stream as a client reads it.
struct Sse {
    response: reqwest::Response,
    buf: String,
}

impl Sse {
    async fn open(fleet: &Fleet, url: &str) -> Self {
        let response = fleet.http.get(url).send().await.unwrap();
        assert!(response.status().is_success(), "{url}: {}", response.status());
        Self {
            response,
            buf: String::new(),
        }
    }

    /// The next frame, or None once the stream ends or `wait` passes.
    async fn next(&mut self, wait: Duration) -> Option<Frame> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(at) = self.buf.find("\n\n") {
                let raw: String = self.buf.drain(..at + 2).collect();
                let mut frame = Frame {
                    event: String::new(),
                    id: None,
                    data: Value::Null,
                };
                for line in raw.lines() {
                    if let Some(v) = line.strip_prefix("event: ") {
                        frame.event = v.to_owned();
                    } else if let Some(v) = line.strip_prefix("id: ") {
                        frame.id = Some(v.to_owned());
                    } else if let Some(v) = line.strip_prefix("data: ") {
                        frame.data = serde_json::from_str(v).unwrap_or(Value::Null);
                    }
                }
                if frame.event.is_empty() {
                    continue; // keep-alive
                }
                return Some(frame);
            }
            match tokio::time::timeout_at(deadline, self.response.chunk()).await {
                Ok(Ok(Some(chunk))) => self.buf.push_str(&String::from_utf8_lossy(&chunk)),
                _ => return None,
            }
        }
    }

    /// Frames until `done` says stop (or 60 s pass).
    async fn until(&mut self, mut done: impl FnMut(&[Frame]) -> bool) -> Vec<Frame> {
        let mut frames = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done(&frames) && Instant::now() < deadline {
            match self.next(Duration::from_secs(30)).await {
                Some(frame) => frames.push(frame),
                None => break,
            }
        }
        frames
    }
}

/// The ids of `session`'s committed events, read from the API.
async fn all_ids(fleet: &Fleet, session: &str) -> Vec<String> {
    fleet
        .events(session)
        .await
        .iter()
        .filter_map(|e| e["id"].as_str().map(str::to_string))
        .collect()
}

/// Read on until the stream carried `last` (events committed after the
/// turn's end, such as `session.idled`, follow it).
async fn through(sse: &mut Sse, frames: &mut Vec<Frame>, last: &str) {
    if frames.iter().any(|f| f.id.as_deref() == Some(last)) {
        return;
    }
    frames.extend(sse.until(|f| f.iter().any(|f| f.id.as_deref() == Some(last))).await);
}

fn completed(frames: &[Frame]) -> usize {
    frames.iter().filter(|f| f.event == "turn.completed").count()
}

/// The ids of the committed events a stream carried, in order.
fn ids(frames: &[Frame]) -> Vec<String> {
    frames.iter().filter_map(|f| f.id.clone()).collect()
}

/// A delta carrying `text`, as an epoch publishes it.
fn marker(session: &str, seq: u64, text: &str) -> LiveItem {
    let session = SessionId::parse(session).unwrap();
    LiveItem {
        stream: format!("marker-{seq}"),
        seq,
        event: Event::new(
            session,
            EventContext::default(),
            OutputMessageDeltaData {
                turn_id: TurnId::new(),
                message_id: MessageId::new(),
                delta: text.into(),
                accumulated: text.into(),
                phase: None,
            },
        ),
    }
}

async fn post_live(fleet: &Fleet, addr: &str, batch: &LiveBatch) -> (u16, Value) {
    let reply = fleet
        .http
        .post(format!("http://{addr}/cell/exec/live"))
        .json(batch)
        .send()
        .await
        .unwrap();
    let status = reply.status().as_u16();
    (status, reply.json().await.unwrap_or(Value::Null))
}

async fn view(fleet: &Fleet, addr: &str) -> View {
    fleet
        .http
        .get(format!("http://{addr}/cell/view"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The current epoch's fence of `session`.
async fn fence(fleet: &Fleet, session: &str) -> u64 {
    fleet.held(session).await;
    fleet.get(&format!("/v1/sessions/{session}")).await["active"]["fence"]
        .as_u64()
        .unwrap()
}

/// Turns of the `slow` agent take long enough to publish into.
fn slow_turns() {
    // This binary's tests all want the same value.
    unsafe {
        std::env::set_var("CELL_MODEL_DELAY_MS", "8000");
    }
}

/// The gateway on cell 1's node, so cell 0's nodes can come and go under
/// a client's stream.
fn gateway(fleet: &Fleet) -> String {
    format!("http://{}", fleet.addrs(1)[0].1)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_publisher_reaches_no_watcher() {
    slow_turns();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 0).await;
    let mut sse = Sse::open(&fleet, &format!("{}/v1/sessions/{session}/sse", gateway(&fleet))).await;
    assert_eq!(sse.next(Duration::from_secs(5)).await.unwrap().event, "connected");
    fleet.say(&session, "go").await;
    let stale = fence(&fleet, &session).await;
    // Executor 1 stops answering; executor 2 takes the turn over under a
    // newer epoch.
    fleet.kill_executor(1);
    fleet.executor(2);
    fleet
        .until("a newer epoch holds the session", || async {
            fleet.get(&format!("/v1/sessions/{session}")).await["active"]["fence"]
                .as_u64()
                .is_some_and(|f| f > stale)
        })
        .await;
    // The old epoch's deltas land nowhere, on whichever node they reach.
    let batch = LiveBatch {
        session: SessionId::parse(&session).unwrap(),
        fence: stale,
        view: 0,
        deltas: vec![marker(&session, 1 << 40, "STALE-MARKER")],
    };
    for (_, addr) in fleet.addrs(0) {
        let (status, body) = post_live(&fleet, &addr, &batch).await;
        assert_eq!((status, &body["taken"]), (200, &json!(false)), "{addr}");
    }
    let frames = sse.until(|f| completed(f) >= 1).await;
    assert_eq!(completed(&frames), 1, "{frames:?}");
    assert!(
        !frames.iter().any(|f| f.data.to_string().contains("STALE-MARKER")),
        "a stale delta reached the stream"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_publisher_on_another_view_is_redirected_to_the_owner() {
    slow_turns();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 0).await;
    let id = SessionId::parse(&session).unwrap();
    let mut sse = Sse::open(&fleet, &format!("{}/v1/sessions/{session}/sse", gateway(&fleet))).await;
    assert_eq!(sse.next(Duration::from_secs(5)).await.unwrap().event, "connected");
    fleet.say(&session, "go").await;
    let current = fence(&fleet, &session).await;
    let addrs = fleet.addrs(0);
    let view = view(&fleet, &addrs[0].1).await;
    let owner = view.rank(id)[0].1.clone();

    // Every member refuses a batch hashed with another view, and names
    // the same view and owner.
    let mut batch = LiveBatch {
        session: id,
        fence: current,
        view: view.generation + 1,
        deltas: vec![marker(&session, 1 << 40, "WRONG-VIEW")],
    };
    for (_, addr) in &addrs {
        let (status, body) = post_live(&fleet, addr, &batch).await;
        assert_eq!(status, 409, "{addr}: {body}");
        let redirect: Redirect = serde_json::from_value(body).unwrap();
        assert_eq!(redirect.view.generation, view.generation);
        assert_eq!(redirect.owner, owner);
    }
    // A member that is not the owner refuses the right view too.
    batch.view = view.generation;
    for (_, addr) in addrs.iter().filter(|(_, a)| *a != owner) {
        let (status, _) = post_live(&fleet, addr, &batch).await;
        assert_eq!(status, 409, "{addr} is not the owner");
    }

    // A sender holding a stale view (an older generation, missing the
    // owner) learns the current one from the redirect and delivers.
    let deliver = Deliver::default();
    let mut stale = view.clone();
    stale.generation = view.generation.saturating_sub(1);
    stale.members.retain(|_, addr| *addr != owner);
    deliver.learn(0, stale);
    let seeds: Vec<String> = addrs.iter().map(|(_, a)| a.clone()).collect();
    batch.deltas = vec![marker(&session, 1 << 41, "REDIRECTED-MARKER")];
    assert!(deliver.send(&fleet.http, 0, &seeds, batch).await);

    let frames = sse
        .until(|f| f.iter().any(|f| f.data.to_string().contains("REDIRECTED-MARKER")))
        .await;
    assert!(
        frames.iter().any(|f| f.data.to_string().contains("REDIRECTED-MARKER")),
        "the redirected delta reached the stream: {frames:?}"
    );
    assert!(!frames.iter().any(|f| f.data.to_string().contains("WRONG-VIEW")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_stream_node_lags_and_the_stream_has_no_gap() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("chat", 0).await;
    let id = SessionId::parse(&session).unwrap();
    let addrs = fleet.addrs(0);
    let owner_addr = view(&fleet, &addrs[0].1).await.rank(id)[0].1.clone();
    let owner = addrs.iter().find(|(_, a)| *a == owner_addr).unwrap().0;

    let mut sse = Sse::open(&fleet, &format!("{}/v1/sessions/{session}/sse", gateway(&fleet))).await;
    let mut frames = vec![sse.next(Duration::from_secs(5)).await.unwrap()];
    fleet.say(&session, "one").await;
    frames.extend(sse.until(|f| completed(f) >= 1).await);
    // The stream node goes down: the stream carries on from the next node.
    fleet.kill_node(0, owner);
    fleet.say(&session, "two").await;
    frames.extend(sse.until(|f| completed(f) >= 1).await);
    // It comes back behind the others and catches up from its log.
    fleet.start_node(0, owner);
    fleet.say(&session, "three").await;
    frames.extend(sse.until(|f| completed(f) >= 1).await);
    fleet.turns(&session, 3).await;
    let all = all_ids(&fleet, &session).await;
    through(&mut sse, &mut frames, all.last().unwrap()).await;

    assert_eq!(
        frames.iter().filter(|f| f.event == "connected").count(),
        1,
        "one stream to the client"
    );
    assert_eq!(completed(&frames), 3, "{frames:?}");
    let streamed = ids(&frames);
    let from = all.iter().position(|id| *id == streamed[0]).unwrap();
    assert_eq!(streamed, all[from..], "every committed event once, in order");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_between_replay_and_live_keeps_one_stream() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("chat", 0).await;
    for (n, text) in ["one", "two", "three"].into_iter().enumerate() {
        fleet.say(&session, text).await;
        fleet.turns(&session, n as u64 + 1).await;
    }
    let before = fleet.events(&session).await;
    let since = before[1]["id"].as_str().unwrap().to_string();
    // The client resumes from early on (a replay), and the session moves
    // to the other cell while the stream replays and then goes live.
    let url = format!(
        "{}/v1/sessions/{session}/sse?since_id={since}",
        fleet.base()
    );
    let mut sse = Sse::open(&fleet, &url).await;
    let mut frames = vec![sse.next(Duration::from_secs(5)).await.unwrap()];
    fleet
        .post(&format!("/v1/sessions/{session}/move"), json!({ "cell": 1 }))
        .await;
    fleet.say(&session, "four").await;
    frames.extend(sse.until(|f| completed(f) >= 4).await);
    fleet.turns(&session, 4).await;
    let all = all_ids(&fleet, &session).await;
    through(&mut sse, &mut frames, all.last().unwrap()).await;

    assert!(
        frames.iter().all(|f| f.event != "moved"),
        "the gateway follows the move for the client"
    );
    assert_eq!(
        frames.iter().filter(|f| f.event == "connected").count(),
        1
    );
    assert_eq!(completed(&frames), 4, "{frames:?}");
    let streamed = ids(&frames);
    let from = all.iter().position(|id| *id == since).unwrap() + 1;
    assert_eq!(streamed, all[from..], "every event after since_id once, in order");
}
