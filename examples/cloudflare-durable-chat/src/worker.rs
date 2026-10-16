//! Cloudflare host: hibernatable sockets, SQLite, and one durable alarm.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result as EngineResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::send::SendFuture;
use worker::*;

use crate::chat::{Command, Data, Database, Room, now_ms};
use crate::openai::{ChatCompletions, Transport};

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

struct SqlDatabase {
    state: send::SendWrapper<Rc<State>>,
}
#[derive(Deserialize)]
struct Row {
    payload: String,
}
impl SqlDatabase {
    fn new(state: &Rc<State>) -> Self {
        state.storage().sql().exec(
            "CREATE TABLE IF NOT EXISTS chat (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL)",None
        ).expect("chat schema");
        Self {
            state: send::SendWrapper::new(state.clone()),
        }
    }
}
impl Database for SqlDatabase {
    fn load(&self) -> Data {
        let rows = self
            .state
            .storage()
            .sql()
            .exec("SELECT payload FROM chat WHERE id=1", None)
            .expect("read chat")
            .to_array::<Row>()
            .expect("chat row");
        rows.first()
            .map(|r| serde_json::from_str(&r.payload).expect("chat data"))
            .unwrap_or_default()
    }
    fn save(&self, data: &Data) {
        // One statement is the commit boundary, including receipts/inbox or
        // canonical events/next engine position. Never keep a cached copy
        // across an await: another conversation may have committed meanwhile.
        let mut next = data.clone();
        next.revision += 1;
        let payload = serde_json::to_string(&next).expect("chat JSON");
        self.state.storage().sql().exec(
            "INSERT INTO chat(id,payload) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
            vec![SqlStorageValue::from(payload)],
        ).expect("commit chat");
        for ws in self.state.get_websockets() {
            let _ = ws.send_with_str(r#"{"type":"changed"}"#);
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Participant {
    username: String,
    conversation: Option<String>,
}

#[durable_object]
pub struct ChatRoom {
    state: Rc<State>,
    env: Env,
    running: Rc<std::cell::Cell<bool>>,
    alarm_updates: futures::lock::Mutex<()>,
}
impl ChatRoom {
    fn var(&self, key: &str) -> Option<String> {
        self.env
            .secret(key)
            .map(|v| v.to_string())
            .or_else(|_| self.env.var(key).map(|v| v.to_string()))
            .ok()
    }
    fn room(&self) -> Room {
        Room {
            db: Arc::new(SqlDatabase::new(&self.state)),
            model: self
                .var("MODEL")
                .unwrap_or_else(|| "meta/muse-spark-1.3-contributor".into()),
            driver: Arc::new(ChatCompletions {
                base_url: self
                    .var("OPENROUTER_BASE_URL")
                    .unwrap_or_else(|| "https://openrouter.ai/api/v1".into()),
                api_key: self.var("OPENROUTER_API_KEY").unwrap_or_default(),
                transport: Arc::new(Fetch),
            }),
        }
    }
    fn lease_ms(&self) -> u64 {
        self.var("LEASE_MS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30_000)
            .max(100)
    }
    fn snapshot(&self) -> Value {
        let mut snapshot = self.room().snapshot();
        snapshot["participants"] = json!(
            self.state
                .get_websockets()
                .iter()
                .filter_map(|ws| ws.deserialize_attachment::<Participant>().ok().flatten())
                .collect::<Vec<_>>()
        );
        snapshot
    }
    fn broadcast(&self) {
        let text = self.snapshot().to_string();
        for ws in self.state.get_websockets() {
            let _ = ws.send_with_str(&text);
        }
    }
    async fn arm_recovery(&self, room: &Room) -> Result<()> {
        let _guard = self.alarm_updates.lock().await;
        let now = now_ms();
        let deadline = room
            .next_deadline()
            .unwrap_or(u64::MAX)
            .min(now + self.lease_ms());
        self.state
            .storage()
            .set_alarm(Duration::from_millis(deadline.saturating_sub(now)))
            .await
    }
    async fn drive(&self) -> Result<()> {
        let room = self.room();
        loop {
            room.deliver_due(now_ms());
            let ids = room.runnable();
            if ids.is_empty() {
                let _guard = self.alarm_updates.lock().await;
                // Serialize the idle projection with command acceptance. No
                // later alarm write may overwrite an accepted command's wake.
                if !room.runnable().is_empty() {
                    continue;
                }
                if let Some(deadline) = room.next_deadline() {
                    self.state
                        .storage()
                        .set_alarm(Duration::from_millis(deadline.saturating_sub(now_ms())))
                        .await?;
                } else {
                    self.state.storage().delete_alarm().await?;
                }
                self.broadcast();
                return Ok(());
            }
            // Persist recovery *before* starting any work that can yield.
            self.arm_recovery(&room).await?;
            let cells: Vec<_> = ids.iter().map(|id| room.engine(id)).collect();
            let results = futures::future::join_all(cells.iter().map(|c| c.step())).await;
            self.broadcast();
            for result in results {
                if let Err(error) = result {
                    console_error!("chat step failed: {error}");
                    // Leave the pre-armed alarm to retry only the lost step.
                    return Ok(());
                }
            }
        }
    }
    async fn command(&self, command: Command) -> Result<Value> {
        // A successful receipt never precedes its durable wake-up. The alarm
        // may arrive during another turn; the running guard then re-arms it.
        let _guard = self.alarm_updates.lock().await;
        self.state.storage().set_alarm(Duration::ZERO).await?;
        let receipt = self.room().command(command).map_err(Error::from)?;
        self.broadcast();
        Ok(receipt)
    }
}
impl DurableObject for ChatRoom {
    fn new(state: State, env: Env) -> Self {
        Self {
            state: Rc::new(state),
            env,
            running: Rc::new(std::cell::Cell::new(false)),
            alarm_updates: futures::lock::Mutex::new(()),
        }
    }
    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let route = req.path().split('/').nth(3).unwrap_or("").to_string();
        match (req.method(), route.as_str()) {
            (Method::Get, "socket") => {
                if req.headers().get("Upgrade")?.as_deref() != Some("websocket") {
                    return Response::error("WebSocket required", 426);
                }
                let pair = WebSocketPair::new()?;
                pair.server.serialize_attachment(Participant::default())?;
                self.state.accept_web_socket(&pair.server);
                self.broadcast();
                Response::from_websocket(pair.client)
            }
            (Method::Get, "state") => Response::from_json(&self.snapshot()),
            (Method::Post, "commands") => {
                if req
                    .headers()
                    .get("Content-Length")?
                    .and_then(|v| v.parse::<usize>().ok())
                    .is_some_and(|n| n > 16000)
                {
                    return Response::error("command too large", 413);
                }
                let raw = req.text().await?;
                if raw.len() > 16000 {
                    return Response::error("command too large", 413);
                }
                let command = match serde_json::from_str(&raw) {
                    Ok(c) => c,
                    Err(_) => return Response::error("invalid command", 400),
                };
                match self.command(command).await {
                    Ok(receipt) => Response::from_json(&receipt),
                    Err(e) => Response::error(e.to_string(), 400),
                }
            }
            _ => Response::error("not found", 404),
        }
    }
    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        let WebSocketIncomingMessage::String(raw) = message else {
            return Ok(());
        };
        if raw.len() > 16000 {
            return ws.send_with_str(r#"{"type":"error","error":"command too large"}"#);
        }
        let value: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return ws.send_with_str(r#"{"type":"error","error":"invalid JSON"}"#),
        };
        if value["type"] == "select" {
            ws.serialize_attachment(Participant {
                username: value["username"]
                    .as_str()
                    .unwrap_or("Guest")
                    .chars()
                    .take(40)
                    .collect(),
                conversation: value["conversation"].as_str().map(Into::into),
            })?;
            self.broadcast();
            return Ok(());
        }
        let request_id = value["request_id"].clone();
        let result = match serde_json::from_value::<Command>(value) {
            Ok(command) => match self.command(command).await {
                Ok(receipt) => json!({"type":"ack","request_id":request_id,"receipt":receipt}),
                Err(e) => json!({"type":"error","request_id":request_id,"error":e.to_string()}),
            },
            Err(_) => json!({"type":"error","request_id":request_id,"error":"invalid command"}),
        };
        ws.send_with_str(result.to_string())
    }
    async fn websocket_close(
        &self,
        ws: WebSocket,
        code: usize,
        _reason: String,
        _clean: bool,
    ) -> Result<()> {
        let _ = ws.close(Some(code as u16), None::<&str>);
        self.broadcast();
        Ok(())
    }
    async fn websocket_error(&self, ws: WebSocket, _error: Error) -> Result<()> {
        let _ = ws.close(Some(1011), Some("socket error"));
        Ok(())
    }
    async fn alarm(&self) -> Result<Response> {
        if self.running.get() {
            self.arm_recovery(&self.room()).await?;
            return Response::ok("busy");
        }
        self.running.set(true);
        let result = self.drive().await;
        self.running.set(false);
        result?;
        Response::ok("idle")
    }
}

/// Optional shared demo access. Deployment can protect every route,  without placing the model key in a browser or URL.
pub(crate) fn authorize(req: &Request, env: &Env) -> Result<Option<Response>> {
    let expected = env
        .secret("CHAT_ACCESS_TOKEN")
        .map(|v| v.to_string())
        .or_else(|_| env.var("CHAT_ACCESS_TOKEN").map(|v| v.to_string()))
        .ok();
    let Some(expected) = expected.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let query = req
        .url()?
        .query_pairs()
        .any(|(k, v)| k == "access" && v == expected);
    let bearer = req
        .headers()
        .get("Authorization")?
        .is_some_and(|v| v == format!("Bearer {expected}"));
    let cookie = req.headers().get("Cookie")?.is_some_and(|v| {
        v.split(';')
            .any(|c| c.trim() == format!("everruns_chat_access={expected}"))
    });
    if query || bearer || cookie {
        return Ok(None);
    }
    if req.path() == "/" {
        let mut response = Response::from_html(
            r#"<!doctype html><html lang="en"><meta name="viewport" content="width=device-width"><title>Everruns chat access</title><main style="font:16px system-ui;max-width:400px;margin:15vh auto;padding:24px"><h1>Durable chat</h1><p>Enter the shared access key, or open your invitation link.</p><form><label>Access key <input type="password" name="access" required autocomplete="off"></label><button>Open chat</button></form></main></html>"#,
        )?;
        response.headers_mut().set("Cache-Control", "no-store")?;
        return Ok(Some(response));
    }
    Ok(Some(Response::error("access key required", 401)?))
}

pub async fn route(req: Request, env: Env) -> Result<Response> {
    if req.path() == "/" {
        let mut response = Response::from_html(include_str!("chat.html"))?;
        response.headers_mut().set("Cache-Control", "no-store")?;
        response
            .headers_mut()
            .set("Referrer-Policy", "no-referrer")?;
        if let Ok(token) = env.secret("CHAT_ACCESS_TOKEN") {
            // `authorize` checked the request already. The cookie lets native
            // WebSockets authenticate while the copied invitation URL grants
            // access to another browser.
            response.headers_mut().set(
                "Set-Cookie",
                &format!("everruns_chat_access={token}; HttpOnly; Secure; SameSite=Strict; Path=/"),
            )?;
        }
        response.headers_mut().set("Content-Security-Policy","default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'")?;
        return Ok(response);
    }
    let path = req.path();
    if !path.starts_with("/rooms/") {
        return Response::error("not found", 404);
    }
    let name = path
        .strip_prefix("/rooms/")
        .and_then(|p| p.split('/').next())
        .unwrap_or("");
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Response::error("invalid room id", 400);
    }
    env.durable_object("ROOMS")?
        .id_from_name(name)?
        .get_stub()?
        .fetch_with_request(req)
        .await
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if let Some(response) = authorize(&req, &env)? {
        return Ok(response);
    }
    route(req, env).await
}
