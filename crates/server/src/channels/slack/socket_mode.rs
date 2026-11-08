//! Slack Socket Mode: Slack channels served over an outgoing WebSocket.
//!
//! Decision: a self-hosted Everruns on a laptop or behind a firewall has no
//! public HTTPS address, and Slack's Request URL needs one. In Socket Mode
//! this server calls Slack instead: `apps.connections.open` with the channel's
//! app-level token (`xapp-`) returns a WebSocket URL, and Slack delivers the
//! channel's events and button clicks down that socket. Only the transport is
//! new. Each envelope is acknowledged at once (Slack's three-second rule) and
//! then handed to the same code the HTTP routes run after their signature
//! check: the channel host for events, the approval handler for clicks.
//!
//! Decision: every server replica keeps its own socket per channel, with no
//! leader election. Slack allows up to ten connections per app and delivers
//! each envelope to one of them, and a redelivery after a reconnect is dropped
//! by the channel host's existing event dedup.
//!
//! The supervisor reconciles every few seconds against the enabled, live Slack
//! channels set to Socket Mode, so publishing, pausing, deleting a channel or
//! changing its token starts or stops its socket without a restart.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use everruns_contracts::runtime::channel_driver::ChannelRequest;

use super::events::{SlackState, SlackTarget, resolve_slack_channel};
use crate::background::supervised_task::TaskSupervisor;
use crate::channels::host::ServerChannelScope;
use crate::domains::agent_channels::ingress::{channel_liveness, row_to_ingress};
use crate::domains::agent_channels::record::slack_channel::{SlackChannelConfig, SlackConnection};
use crate::domains::agent_channels::slack_evidence::{self, DeliveryEvidence};

/// The request path a Socket Mode event carries into the Slack driver, which
/// skips the HTTP signature check for it. HTTP routes never set a path.
pub(crate) const SOCKET_MODE_PATH: &str = "slack-socket-mode";

const RECONCILE_EVERY: Duration = Duration::from_secs(10);
/// Send a ping this often, and give up on a socket silent for `IDLE_LIMIT`.
const PING_EVERY: Duration = Duration::from_secs(30);
const IDLE_LIMIT: Duration = Duration::from_secs(120);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// After Slack refuses the token, retry rarely: a person has to fix it.
const BACKOFF_AUTH: Duration = Duration::from_secs(600);

/// Start the supervisor for this server process.
pub(crate) fn spawn(supervisor: &mut TaskSupervisor, state: SlackState) {
    supervisor.track("slack_socket_mode", tokio::spawn(supervise(state)));
}

struct Running {
    token_hash: u64,
    handle: JoinHandle<()>,
}

async fn supervise(state: SlackState) {
    let mut running: HashMap<String, Running> = HashMap::new();
    loop {
        match wanted(&state).await {
            Ok(wanted) => reconcile(&state, &mut running, wanted),
            Err(error) => tracing::warn!(%error, "Slack Socket Mode: listing channels failed"),
        }
        tokio::time::sleep(RECONCILE_EVERY).await;
    }
}

/// Channel public id → app token, for every live Socket Mode channel.
async fn wanted(state: &SlackState) -> anyhow::Result<HashMap<String, String>> {
    let mut wanted = HashMap::new();
    for row in state.db.list_enabled_slack_ingress_channels().await? {
        let Ok((context, channel)) = row_to_ingress(state.encryption.as_ref(), row) else {
            continue;
        };
        if channel_liveness(&context, &channel).is_err() {
            continue;
        }
        if let Some(config) = channel.slack_config()
            && let Some(token) = socket_token(&config)
        {
            wanted.insert(channel.public_id.to_string(), token);
        }
    }
    Ok(wanted)
}

/// The app token when this channel should hold a socket.
fn socket_token(config: &SlackChannelConfig) -> Option<String> {
    (config.connection == SlackConnection::SocketMode && config.app_token.starts_with("xapp-"))
        .then(|| config.app_token.clone())
}

fn reconcile(
    state: &SlackState,
    running: &mut HashMap<String, Running>,
    wanted: HashMap<String, String>,
) {
    running.retain(|channel_id, conn| {
        let keep = !conn.handle.is_finished()
            && wanted
                .get(channel_id)
                .is_some_and(|token| hash(token) == conn.token_hash);
        if !keep {
            conn.handle.abort();
            tracing::info!(channel_id, "Slack Socket Mode: closing connection");
        }
        keep
    });
    for (channel_id, token) in wanted {
        if running.contains_key(&channel_id) {
            continue;
        }
        tracing::info!(channel_id, "Slack Socket Mode: opening connection");
        let token_hash = hash(&token);
        let handle = tokio::spawn(hold(state.clone(), channel_id.clone(), token));
        running.insert(channel_id, Running { token_hash, handle });
    }
}

fn hash(token: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

/// Keep one channel's socket open until the supervisor aborts it.
async fn hold(state: SlackState, channel_id: String, token: String) {
    let http = reqwest::Client::new();
    let api_base = everruns_integrations::slack_channel::web_api::SLACK_API_BASE;
    let mut backoff = Duration::from_secs(1);
    loop {
        let result = async {
            let url = open_connection(&http, api_base, &token).await?;
            let hello_state = state.clone();
            let hello_channel = channel_id.clone();
            let dispatch_state = state.clone();
            let dispatch_channel = channel_id.clone();
            serve(
                &url,
                move || {
                    tokio::spawn(record_connected(hello_state.clone(), hello_channel.clone()));
                },
                move |envelope| {
                    tokio::spawn(dispatch(
                        dispatch_state.clone(),
                        dispatch_channel.clone(),
                        envelope,
                    ));
                },
            )
            .await
        }
        .await;
        match result {
            Ok(()) => {
                // Slack asked for a reconnect (it does so every few hours).
                backoff = Duration::from_secs(1);
                continue;
            }
            Err(SocketError::Rejected(code)) if is_auth_error(&code) => {
                tracing::warn!(
                    channel_id,
                    code,
                    "Slack Socket Mode: Slack refused the app token"
                );
                tokio::time::sleep(BACKOFF_AUTH).await;
            }
            Err(error) => {
                tracing::warn!(channel_id, %error, "Slack Socket Mode: connection failed; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

fn is_auth_error(code: &str) -> bool {
    matches!(
        code,
        "invalid_auth"
            | "not_authed"
            | "token_revoked"
            | "account_inactive"
            | "not_allowed_token_type"
    )
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SocketError {
    /// Slack answered `ok: false` with this error code.
    #[error("Slack refused: {0}")]
    Rejected(String),
    #[error("{0}")]
    Transport(String),
}

/// `apps.connections.open`: the WebSocket URL for one connection.
pub(crate) async fn open_connection(
    http: &reqwest::Client,
    api_base: &str,
    app_token: &str,
) -> Result<String, SocketError> {
    let response: Value = http
        .post(format!(
            "{}/apps.connections.open",
            api_base.trim_end_matches('/')
        ))
        .bearer_auth(app_token)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| SocketError::Transport(error.to_string()))?
        .json()
        .await
        .map_err(|error| SocketError::Transport(error.to_string()))?;
    if response["ok"].as_bool() != Some(true) {
        return Err(SocketError::Rejected(
            response["error"]
                .as_str()
                .unwrap_or("unknown_error")
                .to_string(),
        ));
    }
    response["url"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| SocketError::Transport("apps.connections.open returned no url".into()))
}

/// One envelope Slack delivered: `events_api`, `interactive`, ...
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Envelope {
    pub kind: String,
    pub payload: Value,
}

/// Read one socket until Slack asks for a reconnect (`Ok`) or it fails.
/// Every envelope is acknowledged before it is handed on.
pub(crate) async fn serve(
    url: &str,
    mut on_hello: impl FnMut(),
    mut on_envelope: impl FnMut(Envelope),
) -> Result<(), SocketError> {
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|error| SocketError::Transport(error.to_string()))?;
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_seen = tokio::time::Instant::now();
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = ping.tick() => {
                if last_seen.elapsed() > IDLE_LIMIT {
                    return Err(SocketError::Transport("Slack socket went silent".into()));
                }
                sink.send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|error| SocketError::Transport(error.to_string()))?;
                continue;
            }
        };
        let Some(message) = message else {
            return Err(SocketError::Transport("Slack closed the socket".into()));
        };
        let message = message.map_err(|error| SocketError::Transport(error.to_string()))?;
        last_seen = tokio::time::Instant::now();
        let text = match message {
            Message::Text(text) => text,
            Message::Ping(data) => {
                let _ = sink.send(Message::Pong(data)).await;
                continue;
            }
            Message::Close(_) => return Ok(()),
            _ => continue,
        };
        let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        match frame["type"].as_str() {
            Some("hello") => on_hello(),
            Some("disconnect") => return Ok(()),
            _ => {}
        }
        if let Some(envelope_id) = frame["envelope_id"].as_str() {
            sink.send(Message::Text(
                json!({ "envelope_id": envelope_id }).to_string().into(),
            ))
            .await
            .map_err(|error| SocketError::Transport(error.to_string()))?;
            on_envelope(Envelope {
                kind: frame["type"].as_str().unwrap_or_default().to_string(),
                payload: frame["payload"].clone(),
            });
        }
    }
}

/// Slack reached this channel: the same evidence a verified Request URL gives.
async fn record_connected(state: SlackState, channel_id: String) {
    let Ok((_, channel)) = resolve_slack_channel(&state, SlackTarget::Channel(channel_id)).await
    else {
        return;
    };
    let Some(config) = channel.slack_config() else {
        return;
    };
    if config.webhook_verified_at.is_some() {
        return;
    }
    if let Err(error) = slack_evidence::record(
        &state.db,
        state.encryption.as_ref(),
        channel.internal_id,
        &channel.public_id.to_string(),
        &config.signing_secret,
        DeliveryEvidence::WebhookVerified,
    )
    .await
    {
        tracing::warn!(channel_id = %channel.public_id, %error, "Failed to record Slack Socket Mode connection");
    }
}

async fn dispatch(state: SlackState, channel_id: String, envelope: Envelope) {
    let Ok((app, channel)) =
        resolve_slack_channel(&state, SlackTarget::Channel(channel_id.clone())).await
    else {
        tracing::debug!(channel_id, "Slack Socket Mode: channel no longer live");
        return;
    };
    let Some(config) = channel.slack_config() else {
        return;
    };
    // A channel switched back to its Request URL keeps no socket path.
    if config.connection != SlackConnection::SocketMode {
        return;
    }
    match envelope.kind.as_str() {
        "events_api" => {
            let Some(channels) = state.channels.clone() else {
                return;
            };
            let Some(resolved) = channels
                .resolver
                .channel_for(ServerChannelScope::new(app, channel))
            else {
                return;
            };
            let request = ChannelRequest {
                path: SOCKET_MODE_PATH.to_string(),
                headers: Default::default(),
                body: envelope.payload.to_string().into_bytes(),
            };
            if let Err(error) = channels.host.try_handle(&resolved, &request).await {
                tracing::warn!(channel_id, %error, "Failed to handle Slack Socket Mode event");
            }
        }
        "interactive" => {
            super::events::handle_socket_interaction(
                &state,
                &app,
                &channel,
                &config,
                envelope.payload,
            )
            .await;
        }
        other => tracing::debug!(
            channel_id,
            kind = other,
            "Ignoring Slack Socket Mode envelope"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    fn config(connection: SlackConnection, app_token: &str) -> SlackChannelConfig {
        serde_json::from_value(json!({
            "connection": connection,
            "app_token": app_token,
        }))
        .unwrap()
    }

    #[test]
    fn only_socket_mode_channels_with_an_app_token_hold_a_socket() {
        assert_eq!(
            socket_token(&config(SlackConnection::SocketMode, "xapp-1")).as_deref(),
            Some("xapp-1")
        );
        assert_eq!(socket_token(&config(SlackConnection::SocketMode, "")), None);
        assert_eq!(
            socket_token(&config(SlackConnection::SocketMode, "xoxb-wrong")),
            None
        );
        assert_eq!(
            socket_token(&config(SlackConnection::RequestUrl, "xapp-1")),
            None
        );
    }

    #[test]
    fn config_without_a_connection_is_a_request_url_channel() {
        let config: SlackChannelConfig =
            serde_json::from_value(json!({ "bot_token": "xoxb-1" })).unwrap();
        assert_eq!(config.connection, SlackConnection::RequestUrl);
        assert!(config.app_token.is_empty());
    }

    /// A fake Slack socket: says hello, sends the given frames, then records
    /// what the client sends back until it closes.
    async fn fake_slack(frames: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let seen = received.clone();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text(json!({"type": "hello"}).to_string().into()))
                .await
                .unwrap();
            for frame in frames {
                ws.send(Message::Text(frame.to_string().into()))
                    .await
                    .unwrap();
            }
            while let Some(Ok(message)) = ws.next().await {
                if let Message::Text(text) = message {
                    seen.lock()
                        .unwrap()
                        .push(serde_json::from_str(text.as_str()).unwrap());
                }
            }
        });
        (url, received)
    }

    #[tokio::test]
    async fn envelopes_are_acknowledged_and_handed_on_until_disconnect() {
        let event =
            json!({"type": "event_callback", "event": {"type": "app_mention", "text": "hi"}});
        let (url, acks) = fake_slack(vec![
            json!({"envelope_id": "e1", "type": "events_api", "payload": event}),
            json!({"envelope_id": "e2", "type": "interactive", "payload": {"type": "block_actions"}}),
            json!({"type": "disconnect", "reason": "refresh_requested"}),
        ])
        .await;
        let mut hellos = 0;
        let mut envelopes = Vec::new();
        serve(&url, || hellos += 1, |envelope| envelopes.push(envelope))
            .await
            .expect("a disconnect ends the socket cleanly");
        assert_eq!(hellos, 1);
        assert_eq!(
            envelopes,
            vec![
                Envelope {
                    kind: "events_api".into(),
                    payload: event
                },
                Envelope {
                    kind: "interactive".into(),
                    payload: json!({"type": "block_actions"})
                },
            ]
        );
        // The server task records acks as they arrive; give it a moment.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            *acks.lock().unwrap(),
            vec![json!({"envelope_id": "e1"}), json!({"envelope_id": "e2"})]
        );
    }

    #[tokio::test]
    async fn a_closed_socket_is_an_error_so_the_caller_backs_off() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            drop(ws);
        });
        assert!(serve(&url, || {}, |_| {}).await.is_err());
    }

    #[tokio::test]
    async fn open_connection_returns_the_socket_url_or_slacks_error() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apps.connections.open"))
            .and(header("authorization", "Bearer xapp-good"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": true, "url": "wss://wss.slack.test/link"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apps.connections.open"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": false, "error": "invalid_auth"})),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        assert_eq!(
            open_connection(&http, &server.uri(), "xapp-good")
                .await
                .unwrap(),
            "wss://wss.slack.test/link"
        );
        match open_connection(&http, &server.uri(), "xapp-bad").await {
            Err(SocketError::Rejected(code)) => assert!(is_auth_error(&code)),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
