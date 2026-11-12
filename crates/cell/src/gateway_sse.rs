//! A session's SSE stream through the gateway: the session's stream node
//! (`stream.rs`) serves it, and the gateway relays it to the client.
//!
//! Decisions:
//! - The gateway follows the session for the client. When the stream node
//!   says the session `moved`, or the node goes away mid-stream, the
//!   gateway reconnects (to the new cell, or the next node in rank order)
//!   from the last event it relayed. The client sees one stream with no
//!   gap and no repeat, in the control plane's shape: `connected` once,
//!   then events, then `disconnecting` when the connection cycles.
//! - Always over HTTP, even when the gateway runs on the stream node: one
//!   loopback hop keeps a single path to test and lets the relay follow a
//!   move the same way everywhere.
//! - The cursor is the last relayed event's id (`since_id`). Before any
//!   event, it is the sequence the stream node started from, which it
//!   reports in `connected`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use bytes::Bytes;
use everruns_contracts::typed_id::SessionId;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::gateway::{Gateway, parse};
use crate::stream::{Redirect, StreamQuery};

type ApiError = (StatusCode, String);

/// How long the relay keeps trying to reach the session again before it
/// ends the stream (the client then reconnects itself).
const FOLLOW_FOR: Duration = Duration::from_secs(30);

/// Where a reconnect resumes.
#[derive(Debug, Clone)]
enum Cursor {
    SinceId(String),
    After(i32),
}

/// The session's stream, relayed. Shared with the edge router.
pub async fn stream(
    gateway: &Arc<Gateway>,
    session: &str,
    raw: Option<&str>,
) -> Result<Response, ApiError> {
    let session = parse(session)?;
    let query = StreamQuery::parse(raw)?;
    let cursor = match (&query.since_id, query.after_sequence) {
        (Some(id), _) => Some(Cursor::SinceId(id.clone())),
        (None, Some(after)) => Some(Cursor::After(after)),
        (None, None) => None,
    };
    let first = connect(gateway, session, &query_string(&query, cursor.as_ref())).await?;
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(64);
    tokio::spawn(relay(gateway.clone(), session, query, cursor, first, tx));
    let mut response = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

/// The query a stream node gets: the client's filters and the cursor.
fn query_string(query: &StreamQuery, cursor: Option<&Cursor>) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    match cursor {
        Some(Cursor::SinceId(id)) => {
            out.append_pair("since_id", id);
        }
        Some(Cursor::After(after)) => {
            out.append_pair("after_sequence", &after.to_string());
        }
        None => {}
    }
    for kind in &query.types {
        out.append_pair("types", kind);
    }
    for kind in &query.exclude {
        out.append_pair("exclude", kind);
    }
    out.finish()
}

/// Open the session's stream on its stream node: the first in rank order
/// that answers, following redirects and moves.
async fn connect(
    gateway: &Arc<Gateway>,
    session: SessionId,
    query: &str,
) -> Result<reqwest::Response, ApiError> {
    for _ in 0..4 {
        let cell = gateway.locate(session).await?;
        let seeds = gateway.nodes(cell)?;
        let view = gateway
            .views
            .view(&gateway.http, cell, &seeds, false)
            .await
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("cell {cell} answers no view"),
            ))?;
        let mut again = false;
        for (_, addr) in view.rank(session) {
            let mut url = format!(
                "http://{addr}/cell/sessions/{session}/sse?view={}",
                view.generation
            );
            if !query.is_empty() {
                url.push('&');
                url.push_str(query);
            }
            let reply = match gateway
                .http
                .get(url)
                .timeout(crate::stream::CYCLE + Duration::from_secs(30))
                .send()
                .await
            {
                Ok(reply) => reply,
                // Down: the next node in rank order streams it.
                Err(_) => continue,
            };
            match reply.status() {
                StatusCode::OK => return Ok(reply),
                StatusCode::CONFLICT => {
                    if let Ok(redirect) = reply.json::<Redirect>().await {
                        gateway.views.learn(cell, redirect.view);
                    }
                    again = true;
                    break;
                }
                StatusCode::GONE => {
                    gateway.forget(session);
                    again = true;
                    break;
                }
                status => {
                    let text = reply.text().await.unwrap_or_default();
                    return Err((status, text));
                }
            }
        }
        if !again {
            // Nobody answered: read the view again before the next round.
            gateway.views.view(&gateway.http, cell, &seeds, true).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("no stream node of session {session} answered"),
    ))
}

/// One SSE frame's fields the relay looks at.
#[derive(Default)]
struct Frame {
    event: Option<String>,
    id: Option<String>,
    data: String,
}

fn fields(raw: &[u8]) -> Frame {
    let mut frame = Frame::default();
    for line in String::from_utf8_lossy(raw).lines() {
        if let Some(value) = line.strip_prefix("event:") {
            frame.event = Some(value.trim_start().to_owned());
        } else if let Some(value) = line.strip_prefix("id:") {
            frame.id = Some(value.trim_start().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            frame.data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    frame
}

/// Why one upstream connection ended.
enum End {
    /// The client went away, or the stream ended for good.
    Done,
    /// The session moved, or the stream node went away: reconnect.
    Follow,
}

async fn relay(
    gateway: Arc<Gateway>,
    session: SessionId,
    query: StreamQuery,
    mut cursor: Option<Cursor>,
    first: reqwest::Response,
    tx: mpsc::Sender<Result<Bytes, Infallible>>,
) {
    let mut upstream = first;
    let mut connected = false;
    loop {
        let end = pump(&mut upstream, &mut cursor, &mut connected, &tx).await;
        if matches!(end, End::Done) || tx.is_closed() {
            return;
        }
        gateway.forget(session);
        let started = tokio::time::Instant::now();
        let next = loop {
            let query = query_string(&query, cursor.as_ref());
            match connect(&gateway, session, &query).await {
                Ok(reply) => break Some(reply),
                Err(_) if started.elapsed() < FOLLOW_FOR && !tx.is_closed() => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(_) => break None,
            }
        };
        match next {
            Some(reply) => upstream = reply,
            None => return,
        }
    }
}

/// Relay one upstream connection's frames until it ends.
async fn pump(
    upstream: &mut reqwest::Response,
    cursor: &mut Option<Cursor>,
    connected: &mut bool,
    tx: &mpsc::Sender<Result<Bytes, Infallible>>,
) -> End {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = match upstream.chunk().await {
            Ok(Some(chunk)) => chunk,
            // Ended without `disconnecting`: the node went away.
            Ok(None) | Err(_) => return End::Follow,
        };
        buf.extend_from_slice(&chunk);
        while let Some(at) = buf.windows(2).position(|w| w == b"\n\n") {
            let raw: Vec<u8> = buf.drain(..at + 2).collect();
            let frame = fields(&raw);
            match frame.event.as_deref() {
                Some("moved") => return End::Follow,
                Some("connected") => {
                    if cursor.is_none()
                        && let Ok(data) = serde_json::from_str::<serde_json::Value>(&frame.data)
                        && let Some(after) = data["sequence"].as_i64()
                    {
                        *cursor = Some(Cursor::After(after as i32));
                    }
                    if std::mem::replace(connected, true) {
                        continue;
                    }
                }
                Some("disconnecting") => {
                    let _ = tx.send(Ok(Bytes::from(raw))).await;
                    return End::Done;
                }
                _ => {}
            }
            if let Some(id) = frame.id {
                *cursor = Some(Cursor::SinceId(id));
            }
            if tx.send(Ok(Bytes::from(raw))).await.is_err() {
                return End::Done;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_gives_its_event_id_and_data() {
        let frame = fields(b"event: output.message.completed\nid: event_1\ndata: {\"a\":1}\n\n");
        assert_eq!(frame.event.as_deref(), Some("output.message.completed"));
        assert_eq!(frame.id.as_deref(), Some("event_1"));
        assert_eq!(frame.data, "{\"a\":1}");
        let keepalive = fields(b":\n\n");
        assert!(keepalive.event.is_none() && keepalive.id.is_none());
    }

    #[test]
    fn a_reconnect_keeps_the_filters_and_resumes_from_the_cursor() {
        let query = StreamQuery::parse(Some("since_id=a&exclude=output.message.delta")).unwrap();
        let q = query_string(&query, Some(&Cursor::SinceId("b".into())));
        assert_eq!(q, "since_id=b&exclude=output.message.delta");
        let q = query_string(&query, Some(&Cursor::After(7)));
        assert_eq!(q, "after_sequence=7&exclude=output.message.delta");
    }
}
