// OpenAI Responses API WebSocket transport (behind `responses-websocket`).
//
// Policy (when a call uses it) lives in `websocket.rs`; the wire contract is
// pinned in knowledge/foundations/openai-responses-websocket.md.
//
// The transport produces the same `SseStream` the HTTP path does: every server
// text frame becomes one event whose `data` is the frame, so the existing
// Responses event parser maps WebSocket and SSE events identically.
//
// Decisions:
// - Commit point. `open_stream` sends `response.create` and waits for the first
//   server frame. A failed dial, a socket that closes, stalls, or answers with
//   an `error` event before that frame returns `None`, and the caller sends the
//   same request over SSE. Once a response event has been forwarded the stream
//   is committed: a later drop surfaces as a stream error, exactly like a
//   mid-stream SSE failure, because the consumer may already have acted on it.
// - Reuse across a tool loop. A socket whose response completed is parked in a
//   process-wide pool keyed by (connection identity, response id). The next
//   call that continues from that response (`previous_response_id`) takes it
//   back, which is the case OpenAI's connection-local cache speeds up. Any other
//   call dials a fresh socket. Keying on the exact response id means two
//   sessions never share a socket, and a checkout is exclusive, so one socket
//   never carries two requests at once (we use only the default lane, no
//   `stream_id`).
// - Identity. The pool key hashes the socket URL and every handshake header,
//   credentials included, so a socket opened with one key is never reused for
//   another, and the pool is process-wide because drivers are rebuilt per step.
// - Idle sockets. A parked socket is owned by a keeper task that keeps reading
//   it (answering pings) until it is checked out, the server closes it, or it
//   has been idle for `IDLE_TTL`; then it sends a close frame. Sockets older
//   than `MAX_CONNECTION_AGE` are not reused (OpenAI ends a connection at 60
//   minutes). A stream dropped mid-response closes its socket in the
//   background instead of parking it.
// - SSRF. The base URL is org-configurable, so the dial applies the same guard
//   the shared HTTP clients do (EVE-623): resolve the host, refuse it if any
//   address is private or internal, then connect to the resolved addresses.
//   No redirects are followed (a non-101 handshake is a failed dial).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use eventsource_stream::Event;
use futures::{SinkExt, StreamExt, stream};
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::{self, Message as Frame};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::driver_registry::LlmCallConfig;
use crate::runtime_provider::ProviderEndpoint;
use crate::stream_reconnect::{SseItem, SseStream};
use crate::url_validation::is_blocked_ip;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// TCP + TLS + upgrade handshake budget, matching the HTTP clients.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Wait for the first server frame. OpenAI acknowledges `response.create`
/// with `response.created` before generating, so this only bounds a dead
/// socket; past it the call falls back to SSE.
const FIRST_EVENT_TIMEOUT: Duration = Duration::from_secs(20);
/// Per-frame inactivity bound once committed, matching the streaming HTTP
/// client's read timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a completed socket waits for the next turn of its tool loop.
const IDLE_TTL: Duration = Duration::from_secs(120);
/// OpenAI ends a connection at 60 minutes; stop reusing one well before.
const MAX_CONNECTION_AGE: Duration = Duration::from_secs(55 * 60);
/// Upper bound on parked sockets per process.
const MAX_IDLE: usize = 256;
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(1);

struct Conn {
    socket: Socket,
    opened_at: Instant,
}

/// Send `request_body` as a `response.create` event and return the response
/// stream once its first event has arrived. `None` means "use SSE": nothing
/// was committed and the caller sends the request over HTTP.
pub(super) async fn open_stream(
    endpoint: &ProviderEndpoint,
    api_url: &str,
    request_body: &Value,
    extension_headers: &HeaderMap,
    config: &LlmCallConfig,
) -> Option<SseStream> {
    crate::install_default_crypto_provider();
    let model = config.model.as_str();
    // The handshake has no body; an auth failure here is reported by the SSE
    // attempt that follows, with its usual classification.
    let resolved = endpoint.resolve("GET", api_url, &[]).await.ok()?;
    let Some(ws_url) = websocket_url(&resolved.url) else {
        tracing::warn!(
            model,
            "Responses WebSocket: unsupported URL scheme; using SSE"
        );
        return None;
    };
    // Same precedence as the HTTP request: provider decoration, then auth,
    // then the caller's per-request headers.
    let mut headers: Vec<(String, String)> = extension_headers
        .iter()
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
        .collect();
    headers.extend(resolved.headers);
    let headers = crate::driver_helpers::merge_request_headers(headers, &config.extra_headers);
    let identity = connection_identity(&ws_url, &headers);
    let frame = create_event(request_body);

    if let Some(previous) = request_body
        .get("previous_response_id")
        .and_then(Value::as_str)
        && let Some(conn) = pool().checkout(&identity, previous).await
    {
        match start(conn, &frame).await {
            Ok((conn, first)) => {
                tracing::debug!(
                    model,
                    "Responses WebSocket: continuing on the open connection"
                );
                return Some(response_stream(conn, first, identity));
            }
            // A parked socket the server already closed; dial a fresh one.
            Err(reason) => {
                tracing::debug!(model, %reason, "Responses WebSocket: reused connection failed")
            }
        }
    }

    let conn = match timeout(CONNECT_TIMEOUT, dial(&ws_url, &headers)).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(reason)) => {
            tracing::warn!(model, %reason, "Responses WebSocket: connect failed; using SSE");
            return None;
        }
        Err(_) => {
            tracing::warn!(model, "Responses WebSocket: connect timed out; using SSE");
            return None;
        }
    };
    match start(conn, &frame).await {
        Ok((conn, first)) => Some(response_stream(conn, first, identity)),
        Err(reason) => {
            tracing::warn!(model, %reason, "Responses WebSocket: no response event; using SSE");
            None
        }
    }
}

/// `https://…/responses` → `wss://…/responses` (and `http` → `ws`).
fn websocket_url(http_url: &str) -> Option<String> {
    let mut url = url::Url::parse(http_url).ok()?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    Some(url.to_string())
}

fn connection_identity(ws_url: &str, headers: &[(String, String)]) -> String {
    let mut sorted: Vec<(String, &str)> = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.as_str()))
        .collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(ws_url.as_bytes());
    for (name, value) in sorted {
        hasher.update([0]);
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// The HTTP create body as a `response.create` client event: `stream` is
/// implicit and `background` is not accepted over the socket.
fn create_event(request_body: &Value) -> String {
    let mut body = request_body.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("stream");
        object.remove("background");
        object.insert("type".into(), "response.create".into());
    }
    body.to_string()
}

async fn dial(ws_url: &str, headers: &[(String, String)]) -> Result<Conn, String> {
    let url = url::Url::parse(ws_url).map_err(|e| e.to_string())?;
    let host = url.host_str().ok_or("URL has no host")?.to_string();
    let port = url.port_or_known_default().ok_or("URL has no port")?;
    let addrs = pinned_addrs(&host, port).await?;
    let mut last_error = String::from("no address");
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    let tcp = tcp.ok_or(last_error)?;
    let _ = tcp.set_nodelay(true);

    let mut request = ws_url
        .into_client_request()
        .map_err(|e| format!("invalid request: {e}"))?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| e.to_string())?;
        let mut value = HeaderValue::from_str(value).map_err(|e| e.to_string())?;
        value.set_sensitive(true);
        request.headers_mut().insert(name, value);
    }
    let (socket, _) = tokio_tungstenite::client_async_tls_with_config(request, tcp, None, None)
        .await
        .map_err(|e| match e {
            tungstenite::Error::Http(response) => {
                format!("handshake rejected with HTTP {}", response.status())
            }
            other => other.to_string(),
        })?;
    Ok(Conn {
        socket,
        opened_at: Instant::now(),
    })
}

/// Resolve and SSRF-check the host (EVE-623). An IP literal is used as is,
/// like the shared HTTP clients, whose DNS guard only sees hostnames.
async fn pinned_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let addrs: Vec<SocketAddr> = timeout(DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| "DNS lookup timed out".to_string())?
        .map_err(|e| e.to_string())?
        .collect();
    if let Some(blocked) = addrs.iter().find(|addr| is_blocked_ip(addr.ip())) {
        return Err(format!(
            "host {host} resolves to blocked address {} (private/internal)",
            blocked.ip()
        ));
    }
    Ok(addrs)
}

/// Send the create event and wait for the first response event.
async fn start(mut conn: Conn, frame: &str) -> Result<(Conn, String), String> {
    if let Err(e) = conn.socket.send(Frame::text(frame)).await {
        close_in_background(conn);
        return Err(format!("send failed: {e}"));
    }
    match read_text(&mut conn.socket, FIRST_EVENT_TIMEOUT).await {
        Ok(text) if frame_type(&text).kind == "error" => {
            // A request-level rejection (bad request, rate limit, unknown
            // previous response). Nothing ran; HTTP reproduces it with its
            // full classification and retry policy.
            close_in_background(conn);
            Err(format!("error event before the response started: {text}"))
        }
        Ok(text) => Ok((conn, text)),
        Err(reason) => {
            close_in_background(conn);
            Err(reason)
        }
    }
}

/// Next text frame, answering pings in passing.
async fn read_text(socket: &mut Socket, limit: Duration) -> Result<String, String> {
    let deadline = Instant::now() + limit;
    loop {
        let next = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| format!("no frame for {}s", limit.as_secs()))?;
        match next {
            Some(Ok(Frame::Text(text))) => return Ok(text.to_string()),
            Some(Ok(Frame::Close(close))) => {
                return Err(match close {
                    Some(close) => format!("closed by server ({}: {})", close.code, close.reason),
                    None => "closed by server".to_string(),
                });
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("connection ended".to_string()),
        }
    }
}

#[derive(Deserialize, Default)]
struct FrameHead {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    response: Option<ResponseHead>,
}

#[derive(Deserialize)]
struct ResponseHead {
    #[serde(default)]
    id: Option<String>,
}

fn frame_type(text: &str) -> FrameHead {
    serde_json::from_str(text).unwrap_or_default()
}

fn sse_event(data: String) -> SseItem {
    Ok(Event {
        event: "message".to_string(),
        data,
        id: String::new(),
        retry: None,
    })
}

/// A committed stream that loses its socket reports it the way a mid-stream
/// SSE failure does: an `error` event the parser turns into a stream error.
fn transport_error(reason: &str) -> SseItem {
    sse_event(
        json!({
            "type": "error",
            "error": {
                "message": format!(
                    "Stream error: Responses WebSocket ended before the response finished: {reason}"
                ),
            },
        })
        .to_string(),
    )
}

/// Owns the socket while a response streams. Dropping it before the terminal
/// event (turn cancellation, a stall) closes the socket instead of leaking it.
struct Reader {
    conn: Option<Conn>,
    first: Option<String>,
    identity: String,
    finished: bool,
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            close_in_background(conn);
        }
    }
}

fn response_stream(conn: Conn, first: String, identity: String) -> SseStream {
    let reader = Reader {
        conn: Some(conn),
        first: Some(first),
        identity,
        finished: false,
    };
    Box::pin(stream::unfold(reader, |mut reader| async move {
        if reader.finished {
            return None;
        }
        let text = match reader.first.take() {
            Some(text) => text,
            None => {
                let conn = reader.conn.as_mut()?;
                match read_text(&mut conn.socket, READ_TIMEOUT).await {
                    Ok(text) => text,
                    Err(reason) => {
                        reader.finished = true;
                        if let Some(conn) = reader.conn.take() {
                            close_in_background(conn);
                        }
                        return Some((transport_error(&reason), reader));
                    }
                }
            }
        };
        let head = frame_type(&text);
        match head.kind.as_str() {
            // The response is over and the socket is idle: keep it for the
            // next turn of this tool loop, which continues from this id.
            "response.completed" | "response.incomplete" => {
                reader.finished = true;
                let id = head.response.and_then(|response| response.id);
                if let (Some(conn), Some(id)) = (reader.conn.take(), id) {
                    pool().park(reader.identity.clone(), id, conn);
                }
            }
            // Nothing will continue from a failed or rejected request.
            "response.failed" | "response.cancelled" | "error" => {
                reader.finished = true;
                if let Some(conn) = reader.conn.take() {
                    close_in_background(conn);
                }
            }
            _ => {}
        }
        Some((sse_event(text), reader))
    }))
}

fn close_in_background(mut conn: Conn) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            let _ = timeout(CLOSE_TIMEOUT, conn.socket.close(None)).await;
        });
    }
}

// ----------------------------------------------------------------------------
// Idle pool
// ----------------------------------------------------------------------------

type Checkout = oneshot::Sender<Conn>;

struct Parked {
    generation: u64,
    take: oneshot::Sender<Checkout>,
}

#[derive(Default)]
struct Pool {
    idle: Mutex<HashMap<(String, String), Parked>>,
    generation: AtomicU64,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::default)
}

impl Pool {
    #[expect(
        clippy::unwrap_used,
        reason = "Idle connection locks contain only internal pool state and never invoke user code; retain fail-closed poison handling"
    )]
    fn park(&'static self, identity: String, response_id: String, conn: Conn) {
        if conn.opened_at.elapsed() >= MAX_CONNECTION_AGE {
            close_in_background(conn);
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let (take, take_rx) = oneshot::channel();
        let key = (identity, response_id);
        {
            let mut idle = self.idle.lock().unwrap();
            if idle.len() >= MAX_IDLE {
                drop(idle);
                close_in_background(conn);
                return;
            }
            idle.insert(key.clone(), Parked { generation, take });
        }
        runtime.spawn(self.keep(key, generation, conn, take_rx));
    }

    /// Hold an idle socket: answer pings until it is checked out, closed by
    /// the server, or idle for `IDLE_TTL`.
    #[expect(
        clippy::unwrap_used,
        reason = "Idle connection locks contain only internal pool state and never invoke user code; retain fail-closed poison handling"
    )]
    async fn keep(
        &'static self,
        key: (String, String),
        generation: u64,
        mut conn: Conn,
        mut take_rx: oneshot::Receiver<Checkout>,
    ) {
        let expires = Instant::now() + IDLE_TTL;
        let handed_over = loop {
            tokio::select! {
                request = &mut take_rx => {
                    if let Ok(reply) = request {
                        // A failed send drops the socket, which only means the
                        // caller stopped waiting and dials its own.
                        let _ = reply.send(conn);
                    } else {
                        close_in_background(conn);
                    }
                    break true;
                }
                frame = conn.socket.next() => match frame {
                    Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => continue,
                    // Unsolicited data or a close: the socket is not reusable.
                    _ => {
                        close_in_background(conn);
                        break false;
                    }
                },
                _ = tokio::time::sleep_until(expires) => {
                    close_in_background(conn);
                    break false;
                }
            }
        };
        if !handed_over {
            let mut idle = self.idle.lock().unwrap();
            if idle
                .get(&key)
                .is_some_and(|parked| parked.generation == generation)
            {
                idle.remove(&key);
            }
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "Idle connection locks contain only internal pool state and never invoke user code; retain fail-closed poison handling"
    )]
    async fn checkout(&self, identity: &str, response_id: &str) -> Option<Conn> {
        let parked = self
            .idle
            .lock()
            .unwrap()
            .remove(&(identity.to_string(), response_id.to_string()))?;
        let (reply, reply_rx) = oneshot::channel();
        parked.take.send(reply).ok()?;
        let conn = timeout(CHECKOUT_TIMEOUT, reply_rx).await.ok()?.ok()?;
        if conn.opened_at.elapsed() >= MAX_CONNECTION_AGE {
            close_in_background(conn);
            return None;
        }
        Some(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_maps_scheme_and_keeps_path_and_query() {
        assert_eq!(
            websocket_url("https://api.openai.com/v1/responses").as_deref(),
            Some("wss://api.openai.com/v1/responses")
        );
        assert_eq!(
            websocket_url("http://127.0.0.1:8080/v1/responses?api-version=1").as_deref(),
            Some("ws://127.0.0.1:8080/v1/responses?api-version=1")
        );
        assert_eq!(websocket_url("ftp://example.com/responses"), None);
    }

    #[test]
    fn create_event_drops_http_only_fields() {
        let event: Value = serde_json::from_str(&create_event(&json!({
            "model": "gpt-6-astra",
            "stream": true,
            "background": false,
            "service_tier": "ultrafast",
            "input": [],
        })))
        .unwrap();
        assert_eq!(
            event,
            json!({
                "type": "response.create",
                "model": "gpt-6-astra",
                "service_tier": "ultrafast",
                "input": [],
            })
        );
    }

    #[test]
    fn identity_separates_credentials_and_ignores_header_order() {
        let a = vec![
            ("Authorization".to_string(), "Bearer one".to_string()),
            ("x-extra".to_string(), "1".to_string()),
        ];
        let reordered = vec![a[1].clone(), a[0].clone()];
        let other_key = vec![
            ("Authorization".to_string(), "Bearer two".to_string()),
            ("x-extra".to_string(), "1".to_string()),
        ];
        let url = "wss://api.openai.com/v1/responses";
        assert_eq!(
            connection_identity(url, &a),
            connection_identity(url, &reordered)
        );
        assert_ne!(
            connection_identity(url, &a),
            connection_identity(url, &other_key)
        );
        assert_ne!(
            connection_identity(url, &a),
            connection_identity("wss://other.example/v1/responses", &a)
        );
    }

    #[tokio::test]
    async fn blocked_hostnames_are_refused_before_connecting() {
        let error = pinned_addrs("localhost", 443).await.unwrap_err();
        assert!(error.contains("blocked address"), "{error}");
    }
}
