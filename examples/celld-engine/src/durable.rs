//! The Durable Object: SQL storage for [`Cell`], a `fetch` model transport,
//! and the alarm that drives turns.
//!
//! Decisions:
//! - The alarm is the only thing that runs steps. A request only queues a
//!   message and arms the alarm, so a turn survives the request that started
//!   it, the isolate, and the node.
//! - Before every step the alarm is re-armed one lease ahead. If the node is
//!   lost mid-step, that alarm fires on whichever node owns the cell next and
//!   the turn resumes from its last committed step. A step that outlives its
//!   lease meets the in-memory `running` flag and the alarm just re-arms.
//! - [`Store::commit`] runs in `transactionSync`, which workers-rs 0.8 does
//!   not bind, so it is called on the raw storage object.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result as EngineResult};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use worker::send::SendFuture;
use worker::*;

use crate::cell::{Cell, Progress, Store};
use crate::openai::{ChatCompletions, Transport};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY, committed INTEGER NOT NULL, type TEXT NOT NULL, json TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS inbox (id INTEGER PRIMARY KEY AUTOINCREMENT, message TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS turn (id INTEGER PRIMARY KEY CHECK (id = 1), state TEXT NOT NULL)",
];

struct SqlStore {
    sql: SqlStorage,
    raw: send::SendWrapper<JsValue>,
}

#[derive(Deserialize)]
struct Num {
    n: Option<f64>,
}

#[derive(Deserialize)]
struct Text {
    t: String,
}

#[derive(Deserialize)]
struct Entry {
    id: f64,
    message: String,
}

impl SqlStore {
    fn open(state: &State) -> Result<Self> {
        let storage = state.storage();
        let sql = storage.sql();
        for statement in SCHEMA {
            sql.exec(statement, None)?;
        }
        let raw: &JsValue = storage.as_raw().as_ref();
        Ok(Self {
            sql,
            raw: send::SendWrapper::new(raw.clone()),
        })
    }

    fn run(&self, query: &str, bindings: Vec<SqlStorageValue>) -> SqlCursor {
        // A failing statement here is a bug in this file, not a runtime
        // condition the engine could act on.
        self.sql
            .exec(query, bindings)
            .unwrap_or_else(|e| panic!("{query}: {e}"))
    }

    fn number(&self, query: &str) -> i64 {
        self.run(query, vec![])
            .to_array::<Num>()
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|row| row.n)
            .unwrap_or(0.0) as i64
    }

    fn texts(&self, query: &str, bindings: Vec<SqlStorageValue>) -> Vec<String> {
        self.run(query, bindings)
            .to_array::<Text>()
            .unwrap_or_else(|e| panic!("{query}: {e}"))
            .into_iter()
            .map(|row| row.t)
            .collect()
    }
}

impl Store for SqlStore {
    fn last_seq(&self) -> i64 {
        self.number("SELECT max(seq) AS n FROM events")
    }

    fn insert(&self, seq: i64, event_type: &str, json: &str, committed: bool) {
        self.run(
            "INSERT INTO events (seq, committed, type, json) VALUES (?, ?, ?, ?)",
            vec![
                seq.into(),
                i64::from(committed).into(),
                event_type.into(),
                json.into(),
            ],
        );
    }

    fn committed(&self) -> Vec<String> {
        self.texts(
            "SELECT json AS t FROM events WHERE committed = 1 ORDER BY seq",
            vec![],
        )
    }

    fn discard_uncommitted(&self) -> usize {
        let n = self.number("SELECT count(*) AS n FROM events WHERE committed = 0");
        self.run("DELETE FROM events WHERE committed = 0", vec![]);
        n as usize
    }

    fn meta(&self, key: &str) -> Option<String> {
        self.texts("SELECT v AS t FROM meta WHERE k = ?", vec![key.into()])
            .into_iter()
            .next()
    }

    fn set_meta(&self, key: &str, value: &str) {
        self.run(
            "INSERT INTO meta (k, v) VALUES (?, ?) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
            vec![key.into(), value.into()],
        );
    }

    fn turn(&self) -> Option<String> {
        self.texts("SELECT state AS t FROM turn WHERE id = 1", vec![])
            .into_iter()
            .next()
    }

    fn set_turn(&self, turn: &str) {
        self.run(
            "INSERT INTO turn (id, state) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET state = excluded.state",
            vec![turn.into()],
        );
    }

    fn push_inbox(&self, message: &str) {
        self.run(
            "INSERT INTO inbox (message) VALUES (?)",
            vec![message.into()],
        );
    }

    fn peek_inbox(&self) -> Option<(i64, String)> {
        self.run("SELECT id, message FROM inbox ORDER BY id LIMIT 1", vec![])
            .to_array::<Entry>()
            .unwrap_or_default()
            .into_iter()
            .next()
            .map(|entry| (entry.id as i64, entry.message))
    }

    fn inbox_len(&self) -> usize {
        self.number("SELECT count(*) AS n FROM inbox") as usize
    }

    fn commit(&self, turn: Option<&str>, meta: &[(&str, String)], consumed: Option<i64>) {
        let this = SqlStore {
            sql: self.sql.clone(),
            raw: send::SendWrapper::new(self.raw.0.clone()),
        };
        let turn = turn.map(str::to_string);
        let meta: Vec<(String, String)> =
            meta.iter().map(|(k, v)| ((*k).into(), v.clone())).collect();
        let body = Closure::once_into_js(move || {
            this.run(
                "UPDATE events SET committed = 1 WHERE committed = 0",
                vec![],
            );
            match &turn {
                Some(turn) => this.set_turn(turn),
                None => {
                    this.run("DELETE FROM turn", vec![]);
                }
            }
            for (k, v) in &meta {
                this.set_meta(k, v);
            }
            if let Some(id) = consumed {
                this.run("DELETE FROM inbox WHERE id = ?", vec![id.into()]);
            }
        });
        let transaction: js_sys::Function =
            js_sys::Reflect::get(&self.raw, &"transactionSync".into())
                .and_then(|f| f.dyn_into())
                .expect("storage.transactionSync");
        transaction
            .call1(&self.raw, &body)
            .unwrap_or_else(|e| panic!("commit: {e:?}"));
    }
}

use wasm_bindgen::JsCast;

/// The model call, through the isolate's `fetch`.
struct Fetch;

#[async_trait]
impl Transport for Fetch {
    async fn post_json(&self, url: &str, bearer: &str, body: Value) -> EngineResult<Value> {
        let url = url.to_string();
        let bearer = bearer.to_string();
        SendFuture::new(async move {
            let headers = Headers::new();
            headers.set("content-type", "application/json")?;
            headers.set("authorization", &format!("Bearer {bearer}"))?;
            let mut init = RequestInit::new();
            init.with_method(Method::Post)
                .with_headers(headers)
                .with_body(Some(JsValue::from_str(&body.to_string())));
            let request = Request::new_with_init(&url, &init)?;
            let mut response = worker::Fetch::Request(request).send().await?;
            let status = response.status_code();
            let text = response.text().await?;
            Ok::<_, worker::Error>((status, text))
        })
        .await
        .map_err(|e| AgentLoopError::llm(e.to_string()))
        .and_then(|(status, text)| {
            if !(200..300).contains(&status) {
                return Err(AgentLoopError::llm_http(
                    status,
                    &text,
                    "chat completions failed",
                ));
            }
            serde_json::from_str(&text).map_err(|e| AgentLoopError::llm(e.to_string()))
        })
    }
}

#[durable_object]
pub struct AgentCell {
    state: State,
    env: Env,
    running: Rc<std::cell::Cell<bool>>,
}

impl AgentCell {
    fn var(&self, name: &str) -> Option<String> {
        self.env
            .secret(name)
            .map(|v| v.to_string())
            .or_else(|_| self.env.var(name).map(|v| v.to_string()))
            .ok()
    }

    fn millis(&self, name: &str, default: u64) -> Duration {
        Duration::from_millis(
            self.var(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(default),
        )
    }

    fn cell(&self) -> Result<Cell<SqlStore>> {
        Ok(Cell {
            store: Arc::new(SqlStore::open(&self.state)?),
            driver: Arc::new(ChatCompletions {
                base_url: self
                    .var("OPENAI_BASE_URL")
                    .unwrap_or_else(|| "https://api.openai.com/v1".into()),
                api_key: self.var("OPENAI_API_KEY").unwrap_or_default(),
                transport: Arc::new(Fetch),
            }),
            model: self.var("MODEL").unwrap_or_else(|| "gpt-5-mini".into()),
            tool_delay: self.millis("TOOL_DELAY_MS", 2000),
        })
    }

    async fn drive(&self) -> Result<()> {
        let cell = self.cell()?;
        let lease = self.millis("LEASE_MS", 30_000);
        loop {
            self.state.storage().set_alarm(lease).await?;
            match cell.step().await {
                Ok(Progress::Stepped) => continue,
                Ok(Progress::Idle) => break,
                Err(error) => {
                    // The step left nothing committed; the armed alarm
                    // retries it after the lease.
                    console_error!("step failed: {error}");
                    return Ok(());
                }
            }
        }
        self.state.storage().delete_alarm().await?;
        // A message queued while the last step committed.
        if cell.store.inbox_len() > 0 {
            self.state.storage().set_alarm(0).await?;
        }
        Ok(())
    }
}

impl DurableObject for AgentCell {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            running: Rc::new(std::cell::Cell::new(false)),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        // `/cells/<name>/<route>`
        let path = req.path();
        let route = path.splitn(4, '/').nth(3).unwrap_or("").to_string();
        let cell = self.cell()?;
        match (req.method(), route.as_str()) {
            (Method::Post, "messages") => {
                let body: Value = req.json().await?;
                let text = body["text"]
                    .as_str()
                    .ok_or_else(|| Error::from("body needs {\"text\": ...}"))?;
                let message = cell
                    .post_message(text)
                    .map_err(|e| Error::from(e.to_string()))?;
                if !self.running.get() {
                    self.state.storage().set_alarm(0).await?;
                }
                Ok(
                    Response::from_json(&json!({ "message_id": message.id, "queued": true }))?
                        .with_status(202),
                )
            }
            (Method::Get, "events") => {
                let events = cell.events().map_err(|e| Error::from(e.to_string()))?;
                Response::from_json(&json!({ "session_id": cell.session_id(), "data": events }))
            }
            (Method::Get, "state") => {
                let store = &cell.store;
                let counter = |key: &str| -> u64 {
                    store.meta(key).and_then(|v| v.parse().ok()).unwrap_or(0)
                };
                Response::from_json(&json!({
                    "running": self.running.get(),
                    "turn": cell.open_turn(),
                    "inbox": store.inbox_len(),
                    "events": store.last_seq(),
                    "reason_steps": counter("reason_steps"),
                    "act_steps": counter("act_steps"),
                    "resumed_steps": counter("resumed_steps"),
                }))
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        if self.running.get() {
            // The step outlived its lease; it re-arms when it commits.
            let lease = self.millis("LEASE_MS", 30_000);
            self.state.storage().set_alarm(lease).await?;
            return Response::ok("busy");
        }
        self.running.set(true);
        let result = self.drive().await;
        self.running.set(false);
        result?;
        Response::ok("idle")
    }
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = req.path();
    let name = path
        .strip_prefix("/cells/")
        .and_then(|rest| rest.split('/').next())
        .filter(|name| !name.is_empty());
    match name {
        Some(name) => {
            let stub = env.durable_object("CELL")?.id_from_name(name)?.get_stub()?;
            stub.fetch_with_request(req).await
        }
        None => Response::error(
            "everruns engine cell. Try POST /cells/<name>/messages {\"text\": \"hi\"}",
            404,
        ),
    }
}
