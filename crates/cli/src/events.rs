//! A session's event stream over SSE, with reconnection.
//!
//! Decision: the CLI reads `GET /v1/sessions/{id}/sse` itself instead of
//! through the SDK, so the binary depends on the API contract alone. The
//! behaviour is the one the server's stream is designed for:
//!
//! - every reconnect resumes from the last event seen (`since_id`), so nothing
//!   is replayed or skipped across a drop;
//! - a `disconnecting` event is the server cycling the connection, so it
//!   reconnects after the hinted delay and never counts against the retries;
//! - any other drop, error or stall backs off exponentially (1s doubling to
//!   30s), and a `connected` event or a delivered event resets the backoff;
//! - silence longer than 1.5 server heartbeats (45s) is a half-open
//!   connection, and is treated as a drop.
//!
//! It is a pull API (`next().await`) rather than a `Stream`: both callers
//! drive it from a loop with their own timeout or Ctrl+C, and a pull keeps
//! the reconnect logic in straight-line code.

use std::pin::Pin;
use std::time::Duration;

use anyhow::{Result, anyhow};
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;

use crate::commands::api::{ApiClient, with_query};

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);

/// Streamed text deltas. A terminal shows the completed message, so asking
/// the server to leave these out saves most of a turn's bytes.
pub const DELTA_EVENTS: &[&str] = &["output.message.delta", "reason.thinking.delta"];

/// One session event, as the server sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub ts: String,
    pub session_id: String,
    #[serde(default)]
    pub data: Value,
}

impl Event {
    /// The fields the CLI prints in JSON and YAML output.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "type": self.event_type,
            "ts": self.ts,
            "session_id": self.session_id,
            "data": self.data,
        })
    }
}

#[derive(Debug, Deserialize)]
struct Disconnecting {
    #[serde(default = "default_retry_ms")]
    retry_ms: u64,
}

fn default_retry_ms() -> u64 {
    100
}

type Messages = Pin<Box<dyn Stream<Item = Result<eventsource_stream::Event>> + Send>>;

pub struct EventStream<'a> {
    client: ApiClient<'a>,
    session_id: String,
    exclude: Vec<String>,
    last_id: Option<String>,
    max_retries: Option<u32>,
    retries: u32,
    backoff: Duration,
    messages: Option<Messages>,
}

impl<'a> EventStream<'a> {
    /// Follow a session's events after `since_id` (all retained events when
    /// `None`).
    pub fn new(client: ApiClient<'a>, session_id: &str, since_id: Option<String>) -> Self {
        Self {
            client,
            session_id: session_id.to_string(),
            exclude: Vec::new(),
            last_id: since_id,
            max_retries: None,
            retries: 0,
            backoff: INITIAL_BACKOFF,
            messages: None,
        }
    }

    pub fn excluding(mut self, types: &[&str]) -> Self {
        self.exclude = types.iter().map(|t| t.to_string()).collect();
        self
    }

    /// Give up after this many consecutive failed reconnects (unlimited by
    /// default). Server-initiated cycling never counts.
    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries = Some(max);
        self
    }

    /// The next event, reconnecting as needed. `None` once retries are spent
    /// after a clean end of stream; an error once they are spent after a
    /// failure.
    pub async fn next(&mut self) -> Option<Result<Event>> {
        loop {
            if self.messages.is_none() {
                match self.connect().await {
                    Ok(messages) => self.messages = Some(messages),
                    Err(error) => match self.back_off().await {
                        true => continue,
                        false => return Some(Err(error)),
                    },
                }
            }
            let messages = self.messages.as_mut()?;
            let failure = match tokio::time::timeout(IDLE_TIMEOUT, messages.next()).await {
                Ok(Some(Ok(message))) => match message.event.as_str() {
                    "connected" => {
                        self.reset();
                        continue;
                    }
                    "disconnecting" => {
                        let retry_ms = serde_json::from_str::<Disconnecting>(&message.data)
                            .map(|d| d.retry_ms)
                            .unwrap_or_else(|_| default_retry_ms());
                        self.messages = None;
                        tokio::time::sleep(Duration::from_millis(retry_ms)).await;
                        continue;
                    }
                    _ => match serde_json::from_str::<Event>(&message.data) {
                        Ok(event) => {
                            self.reset();
                            self.last_id = Some(event.id.clone());
                            return Some(Ok(event));
                        }
                        // Comments, heartbeats and lifecycle frames that are
                        // not events.
                        Err(_) => continue,
                    },
                },
                Ok(Some(Err(error))) => Some(error),
                Ok(None) => None,
                Err(_) => Some(anyhow!(
                    "no data for {}s, reconnecting",
                    IDLE_TIMEOUT.as_secs()
                )),
            };
            self.messages = None;
            if !self.back_off().await {
                return failure.map(Err);
            }
        }
    }

    async fn connect(&self) -> Result<Messages> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(id) = &self.last_id {
            query.push(("since_id", id));
        }
        for event_type in &self.exclude {
            query.push(("exclude", event_type));
        }
        let path = format!("/v1/sessions/{}/sse", urlencoding::encode(&self.session_id));
        let response = self
            .client
            .request(reqwest::Method::GET, &with_query(&path, &query))
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|error| anyhow!("could not open the event stream: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("event stream refused ({status}): {body}"));
        }
        Ok(Box::pin(response.bytes_stream().eventsource().map(
            |message| message.map_err(|error| anyhow!("event stream failed: {error}")),
        )))
    }

    /// Wait before the next reconnect. `false` when retries are spent.
    async fn back_off(&mut self) -> bool {
        if self.max_retries.is_some_and(|max| self.retries >= max) {
            return false;
        }
        self.retries += 1;
        tokio::time::sleep(self.backoff).await;
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        true
    }

    fn reset(&mut self) {
        self.retries = 0;
        self.backoff = INITIAL_BACKOFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serve each scripted SSE body to one connection in turn, and report the
    /// request line each connection asked for.
    async fn serve(bodies: Vec<&'static str>) -> (String, tokio::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (requests, received) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                requests.send(line).await.ok();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (url, received)
    }

    fn event(id: &str, event_type: &str) -> String {
        format!(
            "event: {event_type}\ndata: {{\"id\":\"{id}\",\"type\":\"{event_type}\",\"ts\":\"t\",\"session_id\":\"s\",\"data\":{{}}}}\n\n"
        )
    }

    #[tokio::test]
    async fn a_dropped_stream_resumes_after_the_last_event_and_skips_lifecycle_frames() {
        let first: &'static str = Box::leak(
            format!(
                "event: connected\ndata: {{}}\n\n{}event: disconnecting\ndata: {{\"reason\":\"connection_cycle\",\"retry_ms\":1}}\n\n",
                event("evt_1", "turn.started")
            )
            .into_boxed_str(),
        );
        let second: &'static str = Box::leak(event("evt_2", "turn.completed").into_boxed_str());
        let (url, mut requests) = serve(vec![first, second]).await;

        let client = ApiClient::new(&url, "key", None);
        let mut stream = EventStream::new(client, "s", Some("evt_0".into()))
            .excluding(DELTA_EVENTS)
            .with_max_retries(0);

        let one = stream.next().await.unwrap().unwrap();
        assert_eq!(
            (one.id.as_str(), one.event_type.as_str()),
            ("evt_1", "turn.started")
        );
        let two = stream.next().await.unwrap().unwrap();
        assert_eq!(two.id, "evt_2");

        let opened = requests.recv().await.unwrap();
        assert!(opened.contains("since_id=evt_0"), "{opened}");
        assert!(opened.contains("exclude=output.message.delta"), "{opened}");
        // The server cycled the connection: the retry budget of zero is not
        // spent on it, and the new connection picks up after evt_1.
        let resumed = requests.recv().await.unwrap();
        assert!(resumed.contains("since_id=evt_1"), "{resumed}");
    }

    #[tokio::test]
    async fn a_refused_stream_is_an_error_once_retries_are_spent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            socket
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 7\r\n\r\nmissing")
                .await
                .unwrap();
        });
        let mut stream =
            EventStream::new(ApiClient::new(&url, "key", None), "s", None).with_max_retries(0);
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(error.to_string().contains("404"), "{error}");
    }
}
