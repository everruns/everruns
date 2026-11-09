//! Minimal LSP client: Content-Length framing, request/response correlation,
//! document sync, and diagnostics capture over any async byte transport.
//!
//! The client is transport-generic (`AsyncRead`/`AsyncWrite`) so tests drive it
//! with an in-process fake server over `tokio::io::duplex`; production wraps a
//! language server started by a [`crate::CodeHost`] (see `manager`).

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Notify, oneshot};

/// Upper bound on a single framed message; a language server should never
/// legitimately send more, and the cap keeps a corrupt header from OOMing us.
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

type BoxWriter = Pin<Box<dyn AsyncWrite + Send>>;

/// Diagnostics last pushed by the server for one document, plus a generation
/// counter so callers can wait for a publish that arrives *after* their edit.
#[derive(Clone, Debug, Default)]
pub(crate) struct PublishedDiagnostics {
    pub diagnostics: Vec<Value>,
    pub generation: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum SyncOutcome {
    Opened,
    Changed,
    Unchanged,
}

struct OpenDoc {
    version: i64,
    text: String,
}

struct SharedState {
    pending: std::sync::Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    diagnostics: std::sync::Mutex<HashMap<String, PublishedDiagnostics>>,
    diagnostics_generation: AtomicI64,
    diagnostics_changed: Notify,
    exited: AtomicBool,
}

impl SharedState {
    fn fail_all_pending(&self, reason: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(reason.to_string()));
        }
    }
}

pub(crate) struct LspClient {
    writer: Arc<Mutex<BoxWriter>>,
    state: Arc<SharedState>,
    next_id: AtomicI64,
    request_timeout: Duration,
    open_docs: std::sync::Mutex<HashMap<String, OpenDoc>>,
    reader_task: tokio::task::JoinHandle<()>,
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}

impl LspClient {
    /// Start a client over raw transport halves and spawn the reader loop.
    /// The handshake (`initialize`) is a separate call so tests can script it.
    pub(crate) fn start(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
        request_timeout: Duration,
    ) -> Self {
        let writer: Arc<Mutex<BoxWriter>> = Arc::new(Mutex::new(Box::pin(writer)));
        let state = Arc::new(SharedState {
            pending: std::sync::Mutex::new(HashMap::new()),
            diagnostics: std::sync::Mutex::new(HashMap::new()),
            diagnostics_generation: AtomicI64::new(0),
            diagnostics_changed: Notify::new(),
            exited: AtomicBool::new(false),
        });
        let reader_task = tokio::spawn(reader_loop(
            BufReader::new(reader),
            writer.clone(),
            state.clone(),
        ));
        Self {
            writer,
            state,
            next_id: AtomicI64::new(1),
            request_timeout,
            open_docs: std::sync::Mutex::new(HashMap::new()),
            reader_task,
        }
    }

    pub(crate) fn is_exited(&self) -> bool {
        self.state.exited.load(Ordering::SeqCst)
    }

    /// Send a request and await its response, bounded by the client timeout.
    pub(crate) async fn request(&self, method: &str, params: Value) -> Result<Value> {
        if self.is_exited() {
            bail!("language server exited");
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.state
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(err) = self.send(&message).await {
            self.state
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            return Err(err);
        }
        match tokio::time::timeout(self.request_timeout, rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(err))) => Err(anyhow!("`{method}` failed: {err}")),
            Ok(Err(_)) => Err(anyhow!("`{method}` failed: language server exited")),
            Err(_) => {
                self.state
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                Err(anyhow!(
                    "`{method}` timed out after {}s (the server may still be indexing; retry shortly)",
                    self.request_timeout.as_secs()
                ))
            }
        }
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) -> Result<()> {
        if self.is_exited() {
            bail!("language server exited");
        }
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn send(&self, message: &Value) -> Result<()> {
        let mut writer = self.writer.lock().await;
        write_message(&mut *writer, message).await
    }

    /// LSP `initialize` handshake; returns the server's advertised capabilities.
    ///
    /// `process_id` is this process when the server runs beside it, and `None`
    /// when the server runs somewhere else: servers watch that process and
    /// exit when they cannot find it, which they never could from a sandbox.
    pub(crate) async fn initialize(
        &self,
        root_uri: &str,
        root_name: &str,
        process_id: Option<u32>,
    ) -> Result<Value> {
        let response = self
            .request(
                "initialize",
                json!({
                    "processId": process_id,
                    "clientInfo": { "name": "everruns-coding", "version": env!("CARGO_PKG_VERSION") },
                    "rootUri": root_uri,
                    "workspaceFolders": [{ "uri": root_uri, "name": root_name }],
                    "capabilities": client_capabilities(),
                }),
            )
            .await?;
        self.notify("initialized", json!({})).await?;
        // Some servers (pyright) defer all analysis until the client pushes a
        // configuration; without this, every later request stalls until
        // timeout. Servers that pull config via `workspace/configuration`
        // (answered with nulls by the reader loop) ignore the empty push.
        self.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": {} }),
        )
        .await?;
        Ok(response.get("capabilities").cloned().unwrap_or(Value::Null))
    }

    /// Make the server's view of a document match `text`: `didOpen` on first
    /// touch, full-text `didChange` when the content moved underneath it.
    pub(crate) async fn sync_document(
        &self,
        uri: &str,
        language_id: &str,
        text: &str,
    ) -> Result<SyncOutcome> {
        enum Action {
            Open,
            Change(i64),
        }
        let action = {
            let mut docs = self
                .open_docs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match docs.get_mut(uri) {
                None => {
                    docs.insert(
                        uri.to_string(),
                        OpenDoc {
                            version: 1,
                            text: text.to_string(),
                        },
                    );
                    Action::Open
                }
                Some(doc) if doc.text != text => {
                    doc.version += 1;
                    doc.text = text.to_string();
                    Action::Change(doc.version)
                }
                Some(_) => return Ok(SyncOutcome::Unchanged),
            }
        };
        match action {
            Action::Open => {
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": language_id,
                            "version": 1,
                            "text": text,
                        }
                    }),
                )
                .await?;
                Ok(SyncOutcome::Opened)
            }
            Action::Change(version) => {
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [{ "text": text }],
                    }),
                )
                .await?;
                Ok(SyncOutcome::Changed)
            }
        }
    }

    /// Snapshot of the latest published diagnostics for `uri`, if any.
    pub(crate) fn published_diagnostics(&self, uri: &str) -> Option<PublishedDiagnostics> {
        self.state
            .diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(uri)
            .cloned()
    }

    /// Wait until the server publishes diagnostics for `uri` with a generation
    /// beyond `after_generation`, or until `timeout` elapses. Returns the
    /// freshest snapshot either way; `bool` reports whether a new publish came.
    pub(crate) async fn wait_for_diagnostics(
        &self,
        uri: &str,
        after_generation: u64,
        timeout: Duration,
    ) -> (Option<PublishedDiagnostics>, bool) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Arm the notification *before* checking state so a publish that
            // lands between the check and the wait cannot be missed.
            let notified = self.state.diagnostics_changed.notified();
            if let Some(entry) = self.published_diagnostics(uri)
                && entry.generation > after_generation
            {
                return (Some(entry), true);
            }
            if self.is_exited() {
                return (self.published_diagnostics(uri), false);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return (self.published_diagnostics(uri), false);
            }
        }
    }
}

fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
        "textDocument": {
            "synchronization": { "dynamicRegistration": false },
            "publishDiagnostics": { "relatedInformation": true, "versionSupport": true },
            "diagnostic": { "dynamicRegistration": false },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "declaration": { "linkSupport": true },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": { "valueSet": [
                        "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                        "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"
                    ] }
                },
                "resolveSupport": { "properties": ["edit"] }
            },
            "rename": {}
        },
        "workspace": {
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": ["create", "rename", "delete"]
            },
            "symbol": {},
            "configuration": true,
            "workspaceFolders": true
        },
        "window": { "workDoneProgress": true }
    })
}

async fn reader_loop(
    mut reader: impl AsyncBufReadExt + Unpin,
    writer: Arc<Mutex<BoxWriter>>,
    state: Arc<SharedState>,
) {
    loop {
        match read_message(&mut reader).await {
            Ok(Some(message)) => handle_message(message, &writer, &state).await,
            Ok(None) => break,
            Err(err) => {
                tracing::debug!(target: "everruns::lsp", "lsp read error: {err:#}");
                break;
            }
        }
    }
    state.exited.store(true, Ordering::SeqCst);
    state.fail_all_pending("language server exited");
    // Wake diagnostics waiters so they observe the exit instead of timing out.
    state.diagnostics_changed.notify_waiters();
}

async fn handle_message(message: Value, writer: &Arc<Mutex<BoxWriter>>, state: &Arc<SharedState>) {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").cloned().filter(|id| !id.is_null());
    match (method, id) {
        // Server-initiated request: answer with a benign default so servers
        // that block on the reply (config, capability registration, progress
        // tokens) keep going.
        (Some(method), Some(id)) => {
            let response = server_request_response(method, message.get("params"), id);
            let mut writer = writer.lock().await;
            if let Err(err) = write_message(&mut *writer, &response).await {
                tracing::debug!(target: "everruns::lsp", "lsp reply failed: {err:#}");
            }
        }
        // Notification.
        (Some("textDocument/publishDiagnostics"), None) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                return;
            };
            let diagnostics = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let generation = state
                .diagnostics_generation
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1) as u64;
            state
                .diagnostics
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(
                    uri.to_string(),
                    PublishedDiagnostics {
                        diagnostics,
                        generation,
                    },
                );
            state.diagnostics_changed.notify_waiters();
        }
        (Some(_), None) => {} // window/logMessage, $/progress, …
        // Response to one of our requests.
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else { return };
            let Some(tx) = state
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id)
            else {
                return;
            };
            let result = match message.get("error") {
                Some(error) if !error.is_null() => Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown server error")
                    .to_string()),
                _ => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(result);
        }
        (None, None) => {}
    }
}

fn server_request_response(method: &str, params: Option<&Value>, id: Value) -> Value {
    let result = match method {
        // One `null` per requested item — "no client-side configuration".
        "workspace/configuration" => {
            let count = params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; count])
        }
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest" => Value::Null,
        "workspace/applyEdit" => json!({ "applied": false }),
        _ => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not supported by this client: {method}") },
            });
        }
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Read one `Content-Length`-framed JSON-RPC message; `None` on clean EOF.
pub(crate) async fn read_message<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .context("read LSP header")?;
        if read == 0 {
            if saw_header {
                bail!("unexpected EOF inside LSP message headers");
            }
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if saw_header {
                // End of headers; Content-Length must have been among them.
                break;
            }
            // Tolerate stray blank lines between messages.
            continue;
        }
        saw_header = true;
        let Some((name, value)) = line.split_once(':') else {
            bail!("malformed LSP header line: {line:?}");
        };
        if name.eq_ignore_ascii_case("content-length") {
            let length: usize = value
                .trim()
                .parse()
                .with_context(|| format!("invalid Content-Length: {value:?}"))?;
            if length > MAX_MESSAGE_BYTES {
                bail!("LSP message of {length} bytes exceeds the {MAX_MESSAGE_BYTES} byte cap");
            }
            content_length = Some(length);
        }
        // Content-Type and unknown headers are ignored.
    }
    let length = content_length.context("missing Content-Length header")?;
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .await
        .context("read LSP message body")?;
    Ok(Some(
        serde_json::from_slice(&body).context("parse LSP message body")?,
    ))
}

pub(crate) async fn write_message<W: AsyncWrite + Unpin + ?Sized>(
    writer: &mut W,
    message: &Value,
) -> Result<()> {
    let body = serde_json::to_vec(message).context("serialize LSP message")?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer
        .write_all(header.as_bytes())
        .await
        .context("write LSP header")?;
    writer.write_all(&body).await.context("write LSP body")?;
    writer.flush().await.context("flush LSP message")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn framing_roundtrip() {
        let (client_side, server_side) = duplex(4096);
        let (mut read_half, _keep) = tokio::io::split(client_side);
        let (_keep2, mut write_half) = tokio::io::split(server_side);

        let message = json!({ "jsonrpc": "2.0", "method": "ping", "params": { "x": "héllo" } });
        write_message(&mut write_half, &message)
            .await
            .expect("write");
        let mut reader = BufReader::new(&mut read_half);
        let parsed = read_message(&mut reader)
            .await
            .expect("read")
            .expect("some");
        assert_eq!(parsed, message);
    }

    #[tokio::test]
    async fn read_message_returns_none_on_eof() {
        let (client_side, server_side) = duplex(64);
        drop(server_side);
        let mut reader = BufReader::new(client_side);
        assert!(read_message(&mut reader).await.expect("read").is_none());
    }

    #[tokio::test]
    async fn read_message_rejects_missing_content_length() {
        let (client_side, mut server_side) = duplex(256);
        server_side
            .write_all(b"Content-Type: application/json\r\n\r\n")
            .await
            .expect("write");
        drop(server_side);
        let mut reader = BufReader::new(client_side);
        let err = read_message(&mut reader).await.expect_err("should fail");
        assert!(err.to_string().contains("Content-Length"), "{err}");
    }

    /// Scripted fake server: answers `initialize`, replies to a `demo/echo`
    /// request, asks for `workspace/configuration`, publishes diagnostics.
    async fn fake_server(transport: tokio::io::DuplexStream) {
        let (read_half, mut write_half) = tokio::io::split(transport);
        let mut reader = BufReader::new(read_half);
        while let Ok(Some(message)) = read_message(&mut reader).await {
            let method = message.get("method").and_then(Value::as_str).unwrap_or("");
            let id = message.get("id").cloned();
            match method {
                "initialize" => {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "capabilities": { "positionEncoding": "utf-8" } },
                    });
                    write_message(&mut write_half, &response)
                        .await
                        .expect("write");
                }
                "initialized" => {
                    // Exercise the server-request auto-reply path.
                    let request = json!({
                        "jsonrpc": "2.0",
                        "id": 999,
                        "method": "workspace/configuration",
                        "params": { "items": [{}, {}] },
                    });
                    write_message(&mut write_half, &request)
                        .await
                        .expect("write");
                }
                "textDocument/didOpen" => {
                    let uri = message["params"]["textDocument"]["uri"].clone();
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/publishDiagnostics",
                        "params": {
                            "uri": uri,
                            "diagnostics": [{
                                "range": {
                                    "start": { "line": 0, "character": 0 },
                                    "end": { "line": 0, "character": 1 }
                                },
                                "severity": 1,
                                "message": "fake error"
                            }]
                        },
                    });
                    write_message(&mut write_half, &notification)
                        .await
                        .expect("write");
                }
                "demo/echo" => {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": message.get("params").cloned().unwrap_or(Value::Null),
                    });
                    write_message(&mut write_half, &response)
                        .await
                        .expect("write");
                }
                _ => {
                    // Assert the client answered our workspace/configuration
                    // request with two nulls (routed here as a "response").
                    if message.get("id").and_then(Value::as_i64) == Some(999) {
                        assert_eq!(message["result"], json!([null, null]));
                    }
                }
            }
        }
    }

    fn start_pair() -> LspClient {
        let (client_transport, server_transport) = duplex(64 * 1024);
        tokio::spawn(fake_server(server_transport));
        let (read_half, write_half) = tokio::io::split(client_transport);
        LspClient::start(read_half, write_half, Duration::from_secs(5))
    }

    #[tokio::test]
    async fn initialize_and_request_roundtrip() {
        let client = start_pair();
        let capabilities = client
            .initialize("file:///tmp/x", "x", None)
            .await
            .expect("init");
        assert_eq!(capabilities["positionEncoding"], "utf-8");

        let echoed = client
            .request("demo/echo", json!({ "hello": "world" }))
            .await
            .expect("echo");
        assert_eq!(echoed, json!({ "hello": "world" }));
    }

    #[tokio::test]
    async fn did_open_captures_published_diagnostics() {
        let client = start_pair();
        client
            .initialize("file:///tmp/x", "x", None)
            .await
            .expect("init");

        let uri = "file:///tmp/x/main.rs";
        let outcome = client
            .sync_document(uri, "rust", "fn main() {}")
            .await
            .expect("sync");
        assert_eq!(outcome, SyncOutcome::Opened);

        let (entry, fresh) = client
            .wait_for_diagnostics(uri, 0, Duration::from_secs(5))
            .await;
        assert!(fresh);
        let entry = entry.expect("diagnostics entry");
        assert_eq!(entry.diagnostics.len(), 1);
        assert_eq!(entry.diagnostics[0]["message"], "fake error");

        // Same content again: no version bump, no didChange.
        let outcome = client
            .sync_document(uri, "rust", "fn main() {}")
            .await
            .expect("sync");
        assert_eq!(outcome, SyncOutcome::Unchanged);
        let outcome = client
            .sync_document(uri, "rust", "fn main() { }")
            .await
            .expect("sync");
        assert_eq!(outcome, SyncOutcome::Changed);
    }

    #[tokio::test]
    async fn pending_requests_fail_when_server_exits() {
        let (client_transport, server_transport) = duplex(64 * 1024);
        let (read_half, write_half) = tokio::io::split(client_transport);
        let client = LspClient::start(read_half, write_half, Duration::from_secs(30));
        let request = tokio::spawn(async move { client.request("demo/never", json!({})).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(server_transport);
        let err = request.await.expect("join").expect_err("should fail");
        assert!(err.to_string().contains("exited"), "{err}");
    }
}
