//! THREAT[TM-BROWSER-005]: dedicated process and full session ownership checks.
//! THREAT[TM-BROWSER-006]: bounded protocol, redaction and terminal interruption.
//! THREAT[TM-BROWSER-007]: upstream handoff outcomes, never synthetic approval.
//! Persistent, host-owned upstream Browse subprocess. No browser extension is
//! installed here and no network listener is opened. One supervisor belongs to
//! exactly one Everruns session. It never restarts or replays a failed action.
//! Handoff awaits genuine upstream completion; durable client-tool persistence
//! and authenticated user-facing routing remain the embedding host's job.
//!
//! The host must cap the number of live supervisors, authorize the caller before
//! selecting a session, and persist pending/terminal handoff state through its
//! durable tool machinery. UUID comparison is routing defense, not authentication.
//! No reconnect is attempted: a restarted supervisor has a new upstream owner.
//! The upstream relay may be shared; shutdown kills/reaps only our MCP child,
//! leaving relay cleanup of that dead owner's work to the upstream protocol.
//! Safe profile/path/temp variables are inherited on Unix and Windows, never the
//! entire environment. Additional credentials require explicit host configuration.
//! `Ready` means MCP initialized, not extension paired or user consent granted.
//! Default configuration requests upstream's plain tool surface explicitly.
//! A separately running relay can override its own surface: hosts must verify
//! discovered handoff support rather than assume process initialization proves it.
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
};
use uuid::Uuid;

const MAX_FRAME: usize = 4 * 1024 * 1024;
const QUEUE: usize = 16;

/// Trusted host configuration, never populated from model tool arguments.
/// Supply a pinned, verified upstream executable. This library does not install it.
#[derive(Clone)]
pub struct BrowseProcessConfig {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub request_timeout: Duration,
}
impl BrowseProcessConfig {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: vec!["mcp".into()],
            environment: BTreeMap::from([("OPENCODE_BROWSER_TOOLS".into(), "plain".into())]),
            request_timeout: Duration::from_secs(60),
        }
    }
}
impl std::fmt::Debug for BrowseProcessConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowseProcessConfig")
            .field("process", &"[host-configured]")
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseState {
    /// MCP transport initialized. This does not mean an extension is paired,
    /// a tab is lent, or browser actions are currently permitted.
    Ready,
    /// The host explicitly ended this supervisor.
    Closed,
    /// Process/protocol continuity was lost; never automatically replay actions.
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseError {
    WrongOwner,
    Busy,
    InvalidRequest,
    Interrupted,
    RemoteError,
    ProtocolError,
}
impl std::fmt::Display for BrowseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BrowseError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffOutcome {
    Continued,
    Cancelled,
    TimedOut,
    TabClosed,
}
struct Request {
    method: &'static str,
    params: Value,
    timeout: Duration,
    reply: oneshot::Sender<Result<Value, BrowseError>>,
}

/// Non-cloneable session owner. Drop closes the queue and terminates the child;
/// `close` additionally waits for cleanup. Dropping an in-flight request future
/// interrupts the whole session, preventing unsafe continuation after ambiguity.
pub struct BrowseSupervisor {
    owner: Uuid,
    tx: mpsc::Sender<Request>,
    stop: Option<oneshot::Sender<()>>,
    state: watch::Receiver<BrowseState>,
    task: Option<tokio::task::JoinHandle<()>>,
    timeout: Duration,
}
impl BrowseSupervisor {
    pub async fn start(owner: Uuid, config: BrowseProcessConfig) -> Result<Self, BrowseError> {
        if !config.executable.is_absolute()
            || config.request_timeout.is_zero()
            || config.request_timeout > Duration::from_secs(3600)
        {
            return Err(BrowseError::InvalidRequest);
        }
        let mut command = Command::new(&config.executable);
        command.args(&config.args).env_clear();
        // Upstream needs its home pairing state. Do not inherit unrelated API keys.
        for key in [
            "PATH",
            "HOME",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .envs(&config.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| BrowseError::Interrupted)?;
        let mut input = child.stdin.take().ok_or(BrowseError::Interrupted)?;
        let mut output = BufReader::new(child.stdout.take().ok_or(BrowseError::Interrupted)?);
        let handshake = tokio::time::timeout(config.request_timeout, async {
            let response = exchange(&mut input, &mut output, 0, "initialize", json!({"protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":{"name":"everruns","version":env!("CARGO_PKG_VERSION")}})).await?;
            if response["protocolVersion"] != "2025-11-25" { return Err(BrowseError::ProtocolError); }
            write(&mut input, &json!({"jsonrpc":"2.0", "method":"notifications/initialized"})).await
        }).await.unwrap_or(Err(BrowseError::Interrupted));
        if let Err(error) = handshake {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        let (tx, mut rx) = mpsc::channel::<Request>(QUEUE);
        let (stop, mut stopped) = oneshot::channel();
        let (state_tx, state) = watch::channel(BrowseState::Ready);
        let task = tokio::spawn(async move {
            let mut id = 1u64;
            let terminal = loop {
                let request = tokio::select! {
                    _ = &mut stopped => break BrowseState::Closed,
                    _ = child.wait() => break BrowseState::Interrupted,
                    request = rx.recv() => match request { Some(request) => request, None => break BrowseState::Closed },
                };
                let Request {
                    method,
                    params,
                    timeout,
                    mut reply,
                } = request;
                if reply.is_closed() {
                    continue;
                } // Never dispatched; safe to discard.
                let handoff_tab = (method == "tools/call" && params["name"] == "handoff")
                    .then(|| params["arguments"]["tab"].as_u64())
                    .flatten();
                let mut result = tokio::select! {
                    _ = &mut stopped => { let _ = reply.send(Err(BrowseError::Interrupted)); break BrowseState::Closed; },
                    _ = reply.closed() => break BrowseState::Interrupted,
                    result = tokio::time::timeout(timeout, exchange(&mut input, &mut output, id, method, params)) => result.unwrap_or(Err(BrowseError::Interrupted)),
                };
                if let (Some(tab), Ok(value)) = (handoff_tab, &result)
                    && parse_handoff(value, tab).is_err()
                {
                    result = Err(BrowseError::ProtocolError);
                }
                id += 1;
                // A receiver can close after exchange wins select but before
                // delivery. Unknown observation must stop queued mutations too.
                if !deliver_completed(reply, result) {
                    break BrowseState::Interrupted;
                }
            };
            rx.close();
            let _ = child.kill().await;
            let _ = child.wait().await;
            while let Some(request) = rx.recv().await {
                let _ = request.reply.send(Err(BrowseError::Interrupted));
            }
            let _ = state_tx.send(terminal);
        });
        Ok(Self {
            owner,
            tx,
            stop: Some(stop),
            state,
            task: Some(task),
            timeout: config.request_timeout,
        })
    }
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn state(&self) -> BrowseState {
        *self.state.borrow()
    }
    pub fn watch_state(&self) -> watch::Receiver<BrowseState> {
        self.state.clone()
    }
    async fn request(
        &self,
        owner: Uuid,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, BrowseError> {
        if owner != self.owner {
            return Err(BrowseError::WrongOwner);
        }
        if self.state() != BrowseState::Ready {
            return Err(BrowseError::Interrupted);
        }
        if serde_json::to_vec(&params)
            .map_err(|_| BrowseError::InvalidRequest)?
            .len()
            > MAX_FRAME - 1024
        {
            return Err(BrowseError::InvalidRequest);
        }
        let (reply, response) = oneshot::channel();
        self.tx
            .try_send(Request {
                method,
                params,
                timeout,
                reply,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => BrowseError::Busy,
                _ => BrowseError::Interrupted,
            })?;
        response.await.map_err(|_| BrowseError::Interrupted)?
    }
    /// Raw MCP discovery result; the host must validate discovered schemas.
    pub async fn list_tools(&self, owner: Uuid) -> Result<Value, BrowseError> {
        self.request(owner, "tools/list", json!({}), self.timeout)
            .await
    }
    /// Tools run with upstream consent and ownership checks. No caller-supplied
    /// MCP metadata can override the process owner. Results may contain private
    /// page data and must remain scoped to the authenticated session.
    pub async fn call_tool(
        &self,
        owner: Uuid,
        name: &str,
        arguments: Value,
    ) -> Result<Value, BrowseError> {
        if name.is_empty() || name.len() > 128 || !arguments.is_object() {
            return Err(BrowseError::InvalidRequest);
        }
        self.request(
            owner,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            self.timeout,
        )
        .await
    }
    /// Await the pinned upstream `handoff` tool, not an approval surrogate.
    /// Host should run this behind its existing durable client-tool wait. It
    /// must map errors to interrupted/unknown, never successful continuation.
    pub async fn handoff(
        &self,
        owner: Uuid,
        tab: u64,
        message: &str,
        timeout: Duration,
    ) -> Result<HandoffOutcome, BrowseError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(3600)
            || message.is_empty()
            || message.len() > 4096
        {
            return Err(BrowseError::InvalidRequest);
        }
        let result = self.request(owner, "tools/call", json!({"name":"handoff","arguments":{"tab":tab,"message":message,"timeoutSeconds":timeout.as_secs().max(1)}}), timeout + Duration::from_secs(5)).await?;
        parse_handoff(&result, tab)
    }
    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for BrowseSupervisor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
async fn write(input: &mut ChildStdin, value: &Value) -> Result<(), BrowseError> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| BrowseError::ProtocolError)?;
    if bytes.len() > MAX_FRAME {
        return Err(BrowseError::ProtocolError);
    }
    bytes.push(b'\n');
    input
        .write_all(&bytes)
        .await
        .map_err(|_| BrowseError::Interrupted)?;
    input.flush().await.map_err(|_| BrowseError::Interrupted)
}
async fn exchange(
    input: &mut ChildStdin,
    output: &mut BufReader<ChildStdout>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, BrowseError> {
    write(
        input,
        &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    )
    .await?;
    // Bound frames and notification count; timeout also bounds total runtime.
    for _ in 0..128 {
        let mut line = Vec::new();
        loop {
            let available = output
                .fill_buf()
                .await
                .map_err(|_| BrowseError::Interrupted)?;
            if available.is_empty() {
                return Err(BrowseError::Interrupted);
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |p| p + 1);
            if line.len() + count > MAX_FRAME {
                return Err(BrowseError::ProtocolError);
            }
            let done = available[count - 1] == b'\n';
            line.extend_from_slice(&available[..count]);
            output.consume(count);
            if done {
                break;
            }
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| BrowseError::ProtocolError)?;
        if value["jsonrpc"] != "2.0" {
            return Err(BrowseError::ProtocolError);
        }
        if value["id"].as_u64() == Some(id) {
            return response_result(&value);
        }
        // This client advertises no server-initiated request capabilities.
        if value.get("id").is_some() {
            return Err(BrowseError::ProtocolError);
        }
    }
    Err(BrowseError::ProtocolError)
}
#[cfg(test)]
mod tests;

// True means the actor may dispatch another queued request. Delivery failure is
// terminal even when the subprocess completed successfully: the caller cancelled
// or disappeared, so it cannot safely observe the mutation outcome.
fn deliver_completed(
    reply: oneshot::Sender<Result<Value, BrowseError>>,
    result: Result<Value, BrowseError>,
) -> bool {
    let fatal = matches!(
        result,
        Err(BrowseError::Interrupted | BrowseError::ProtocolError)
    );
    reply.send(result).is_ok() && !fatal
}

fn parse_handoff(result: &Value, tab: u64) -> Result<HandoffOutcome, BrowseError> {
    let is_error = match result.get("isError") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => return Err(BrowseError::ProtocolError),
    };
    for block in result["content"]
        .as_array()
        .ok_or(BrowseError::ProtocolError)?
    {
        if block["type"] != "text" {
            continue;
        }
        let Some(text) = block["text"].as_str() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if value["tab"].as_u64() != Some(tab) {
            continue;
        }
        return match value["outcome"].as_str() {
            Some("continued") if !is_error => Ok(HandoffOutcome::Continued),
            Some("cancelled") if is_error => Ok(HandoffOutcome::Cancelled),
            Some("timeout") if is_error => Ok(HandoffOutcome::TimedOut),
            Some("closed") if is_error => Ok(HandoffOutcome::TabClosed),
            _ => Err(BrowseError::ProtocolError),
        };
    }
    Err(BrowseError::ProtocolError)
}

// A malformed JSON-RPC envelope is not an ordinary remote tool error: treating
// it as recoverable could dispatch a mutation after an ambiguous response.
fn response_result(value: &Value) -> Result<Value, BrowseError> {
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(Value::Object(error)))
            if error.get("code").and_then(Value::as_i64).is_some()
                && error.get("message").and_then(Value::as_str).is_some() =>
        {
            Err(BrowseError::RemoteError)
        }
        _ => Err(BrowseError::ProtocolError),
    }
}
