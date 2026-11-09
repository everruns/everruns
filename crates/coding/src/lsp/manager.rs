//! Language-server lifecycle and the text/position plumbing LSP requires:
//! lazy start per project and language, file→language routing,
//! position-encoding conversion (LSP defaults to UTF-16 columns), file URIs,
//! and turning a `WorkspaceEdit` into file changes.
//!
//! Where the server runs and how files are read is the [`CodeHost`]'s
//! business. Two things follow from that:
//!
//! - A server is keyed by project root as well as language. A workspace that
//!   holds several repositories gets one server per repository, each rooted
//!   where its build files are.
//! - A remote server's bytes only move while someone pumps them, so every tool
//!   call runs inside [`LspManager::drive`].

use super::client::LspClient;
use crate::host::{
    CodeHost, FileOp, Pump, ServerCommand, normalize_lexically, protected_component,
};
use anyhow::{Context, Result, bail};
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::Duration;
#[cfg(test)]
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;

/// Pause between pump rounds that moved nothing; doubles up to the maximum.
const PUMP_IDLE_MIN: Duration = Duration::from_millis(100);
const PUMP_IDLE_MAX: Duration = Duration::from_millis(1_000);

pub(crate) const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 60_000;
pub(crate) const MAX_REQUEST_TIMEOUT_MS: u64 = 600_000;
pub(crate) const DEFAULT_DIAGNOSTICS_WAIT_MS: u64 = 15_000;
pub(crate) const MAX_DIAGNOSTICS_WAIT_MS: u64 = 120_000;

/// One configured language server: which command to run and which file
/// extensions route to it. `key` doubles as the config-override key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerSpec {
    pub key: String,
    pub command: String,
    pub args: Vec<String>,
    pub extensions: Vec<String>,
}

/// Built-in servers for the ecosystems yolop already understands via
/// tree-sitter. All are the de-facto standard binaries; each spawns only when
/// a file with a matching extension is queried and the binary is on PATH.
pub(crate) fn default_server_specs() -> Vec<ServerSpec> {
    let spec = |key: &str, command: &str, args: &[&str], extensions: &[&str]| ServerSpec {
        key: key.to_string(),
        command: command.to_string(),
        args: args.iter().map(ToString::to_string).collect(),
        extensions: extensions.iter().map(ToString::to_string).collect(),
    };
    vec![
        spec("rust", "rust-analyzer", &[], &["rs"]),
        spec(
            "typescript",
            "typescript-language-server",
            &["--stdio"],
            &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"],
        ),
        spec("python", "pyright-langserver", &["--stdio"], &["py", "pyi"]),
        spec("go", "gopls", &[], &["go"]),
        spec(
            "clangd",
            "clangd",
            &[],
            &["c", "h", "cc", "cpp", "cxx", "hpp", "hh"],
        ),
    ]
}

/// `languageId` for `textDocument/didOpen`, keyed by file extension.
pub(crate) fn language_id_for_extension(extension: &str) -> &'static str {
    match extension {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        _ => "plaintext",
    }
}

/// Column unit negotiated with the server (`positionEncoding`, LSP 3.17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionEncoding {
    Utf8,
    Utf16,
    Utf32,
}

impl PositionEncoding {
    pub(crate) fn from_capabilities(capabilities: &Value) -> Self {
        match capabilities.get("positionEncoding").and_then(Value::as_str) {
            Some("utf-8") => Self::Utf8,
            Some("utf-32") => Self::Utf32,
            // The LSP default when nothing is negotiated.
            _ => Self::Utf16,
        }
    }
}

/// A running language server plus everything needed to talk to it correctly.
pub(crate) struct LanguageServer {
    pub client: LspClient,
    pub capabilities: Value,
    pub encoding: PositionEncoding,
    /// When the server finished its `initialize` handshake. Cross-file
    /// queries retry on empty results while the server is younger than the
    /// warmup window (see `request_retrying_warmup`).
    pub started_at: std::time::Instant,
    // Held only so a local child process dies with this entry.
    _guard: Option<Box<dyn std::any::Any + Send + Sync>>,
}

#[cfg(test)]
pub(crate) type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
#[cfg(test)]
pub(crate) type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;
#[cfg(test)]
pub(crate) type TransportFactory =
    Box<dyn Fn(&ServerSpec) -> Result<(BoxRead, BoxWrite)> + Send + Sync + 'static>;

/// A document synced into its language server, ready for position-based
/// requests.
pub(crate) struct OpenedDocument {
    pub server: Arc<LanguageServer>,
    pub uri: String,
    pub text: String,
    pub outcome: super::client::SyncOutcome,
}

/// The language servers of one workspace.
pub(crate) struct LspManager {
    pub host: Arc<dyn CodeHost>,
    pub specs: Vec<ServerSpec>,
    pub request_timeout: Duration,
    pub diagnostics_wait: Duration,
    /// Names this workspace among all that share the process; part of every
    /// server's launch key.
    scope: String,
    /// Running servers by `<project root>|<server key>`.
    servers: Mutex<HashMap<String, Arc<LanguageServer>>>,
    /// Pumps of the servers above that need one, by the same key.
    pumps: std::sync::Mutex<Vec<(String, Arc<dyn Pump>)>>,
    #[cfg(test)]
    transport_factory: Option<TransportFactory>,
}

impl LspManager {
    pub(crate) fn new(
        host: Arc<dyn CodeHost>,
        scope: String,
        specs: Vec<ServerSpec>,
        request_timeout: Duration,
        diagnostics_wait: Duration,
    ) -> Self {
        Self {
            host,
            specs,
            request_timeout,
            diagnostics_wait,
            scope,
            servers: Mutex::new(HashMap::new()),
            pumps: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            transport_factory: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_transport_factory(mut self, factory: TransportFactory) -> Self {
        self.transport_factory = Some(factory);
        self
    }

    /// The server spec a file routes to, by extension.
    fn spec_for(&self, file: &str) -> Result<&ServerSpec> {
        let extension = extension_of(file);
        self.specs
            .iter()
            .find(|spec| spec.extensions.iter().any(|ext| ext == &extension))
            .with_context(|| {
                format!(
                    "no language server configured for `.{extension}` files; \
                     configured servers: {}",
                    self.spec_summary()
                )
            })
    }

    fn spec_summary(&self) -> String {
        let mut parts: Vec<String> = self
            .specs
            .iter()
            .map(|spec| format!("{} ({})", spec.key, spec.extensions.join(", ")))
            .collect();
        parts.sort();
        parts.join("; ")
    }

    /// Get or lazily start the server for `spec` in the project at `root`. A
    /// server that has exited is dropped and restarted on the next call.
    async fn language_server(
        &self,
        context: &ToolContext,
        spec: &ServerSpec,
        root: &str,
    ) -> Result<Arc<LanguageServer>> {
        let key = format!("{root}|{}", spec.key);
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get(&key) {
            if !server.client.is_exited() {
                return Ok(server.clone());
            }
            servers.remove(&key);
            self.drop_pump(&key);
        }
        let server = Arc::new(self.start_server(context, spec, root, &key).await?);
        servers.insert(key, server.clone());
        Ok(server)
    }

    async fn start_server(
        &self,
        context: &ToolContext,
        spec: &ServerSpec,
        root: &str,
        key: &str,
    ) -> Result<LanguageServer> {
        #[cfg(test)]
        if let Some(factory) = &self.transport_factory {
            let (reader, writer) = factory(spec)?;
            let client = LspClient::start(reader, writer, self.request_timeout);
            return finish_initialize(client, None, root, Some(std::process::id())).await;
        }

        let io = self
            .host
            .launch(
                context,
                &format!("{}|{key}", self.scope),
                &ServerCommand {
                    command: spec.command.clone(),
                    args: spec.args.clone(),
                },
                root,
            )
            .await
            .with_context(|| format!("start the `{}` language server", spec.key))?;
        // A pumped server runs somewhere else, so it must not watch our pid.
        let process_id = io.pump.is_none().then(std::process::id);
        if let Some(pump) = io.pump {
            // Registered before the handshake: `initialize` itself needs the
            // bytes to move.
            self.pumps
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((key.to_string(), pump));
        }
        let client = LspClient::start(io.reader, io.writer, self.request_timeout);
        let started = finish_initialize(client, io.guard, root, process_id).await;
        if started.is_err() {
            self.drop_pump(key);
        }
        started
    }

    fn drop_pump(&self, key: &str) {
        self.pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(pump_key, _)| pump_key != key);
    }

    /// One round over every pumped server. Returns whether any bytes moved.
    async fn pump_once(&self, context: &ToolContext) -> bool {
        let pumps = self
            .pumps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut moved = false;
        for (key, pump) in pumps {
            match pump.tick(context).await {
                Ok(any) => moved |= any,
                Err(err) => {
                    // The client sees end-of-input and marks the server
                    // exited; the next call starts a fresh one.
                    tracing::debug!(target: "everruns::lsp", "language server pump stopped: {err:#}");
                    self.drop_pump(&key);
                }
            }
        }
        moved
    }

    /// Run `work`, moving bytes for remote servers until it finishes.
    ///
    /// A pump round that is in flight when `work` finishes is allowed to
    /// complete: abandoning it half-way could lose bytes the client already
    /// handed over.
    pub(crate) async fn drive<T>(&self, context: &ToolContext, work: impl Future<Output = T>) -> T {
        tokio::pin!(work);
        let mut finished = None;
        let mut idle = PUMP_IDLE_MIN;
        loop {
            let round = self.pump_once(context);
            tokio::pin!(round);
            let moved = loop {
                tokio::select! {
                    output = &mut work, if finished.is_none() => finished = Some(output),
                    moved = &mut round => break moved,
                }
            };
            if let Some(output) = finished {
                return output;
            }
            if moved {
                idle = PUMP_IDLE_MIN;
                continue;
            }
            tokio::select! {
                output = &mut work => return output,
                () = tokio::time::sleep(idle) => {}
            }
            idle = (idle * 2).min(PUMP_IDLE_MAX);
        }
    }

    /// Sync `path` into its server; returns the server handle plus the
    /// document URI and text needed for position conversions.
    pub(crate) async fn open(&self, context: &ToolContext, path: &str) -> Result<OpenedDocument> {
        let file = self.host.resolve_file(context, path).await?;
        let spec = self.spec_for(&file)?.clone();
        // Read before starting anything: a path that is not a readable file
        // should not cost a server start.
        let text = self
            .host
            .read_texts(context, std::slice::from_ref(&file))
            .await?
            .pop()
            .flatten()
            .with_context(|| format!("`{path}` is not a readable text file in the workspace"))?;
        let root = self.host.project_root(context, &file).await?;
        let server = self.language_server(context, &spec, &root).await?;
        let uri = path_to_uri(Path::new(&file));
        let outcome = server
            .client
            .sync_document(&uri, language_id_for_extension(&extension_of(&file)), &text)
            .await?;
        Ok(OpenedDocument {
            server,
            uri,
            text,
            outcome,
        })
    }

    /// Any live server, for requests that are not tied to one document
    /// (workspace symbol search without a `hint_path`). Deterministic: picks
    /// the lowest server key.
    pub(crate) async fn any_running_server(&self) -> Option<Arc<LanguageServer>> {
        let servers = self.servers.lock().await;
        let mut keys: Vec<&String> = servers.keys().collect();
        keys.sort();
        keys.into_iter()
            .map(|key| servers[key].clone())
            .find(|server| !server.client.is_exited())
    }
}

fn extension_of(file: &str) -> String {
    Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

async fn finish_initialize(
    client: LspClient,
    guard: Option<Box<dyn std::any::Any + Send + Sync>>,
    root: &str,
    process_id: Option<u32>,
) -> Result<LanguageServer> {
    let root = Path::new(root);
    let root_uri = path_to_uri(root);
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .to_string();
    let capabilities = client.initialize(&root_uri, &root_name, process_id).await?;
    let encoding = PositionEncoding::from_capabilities(&capabilities);
    Ok(LanguageServer {
        client,
        capabilities,
        encoding,
        started_at: std::time::Instant::now(),
        _guard: guard,
    })
}

// ---------------------------------------------------------------------------
// File URIs

/// `file://` URI for an absolute path, percent-encoding per RFC 3986.
pub(crate) fn path_to_uri(path: &Path) -> String {
    const UNRESERVED_EXTRA: &[u8] = b"-._~/";
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || UNRESERVED_EXTRA.contains(byte) {
            uri.push(c);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

pub(crate) fn uri_to_path(uri: &str) -> Result<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .with_context(|| format!("not a file:// URI: {uri}"))?;
    // Strip an optional authority (`file://localhost/...`).
    let path_part = match rest.strip_prefix("localhost") {
        Some(stripped) => stripped,
        None => rest,
    };
    if !path_part.starts_with('/') {
        bail!("unsupported file URI (no absolute path): {uri}");
    }
    let mut bytes = Vec::with_capacity(path_part.len());
    let mut chars = path_part.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hi = chars.next().context("truncated percent escape")?;
            let lo = chars.next().context("truncated percent escape")?;
            let hex = [hi, lo];
            let hex = std::str::from_utf8(&hex).context("invalid percent escape")?;
            bytes.push(u8::from_str_radix(hex, 16).context("invalid percent escape")?);
        } else {
            bytes.push(byte);
        }
    }
    Ok(PathBuf::from(
        String::from_utf8(bytes).context("file URI is not valid UTF-8")?,
    ))
}

// ---------------------------------------------------------------------------
// Position conversion
//
// Tool-facing coordinates are 1-based (line, column) counted in *characters*,
// matching ast_grep/repo_map output. LSP is 0-based with columns in the
// negotiated encoding (UTF-16 by default).

pub(crate) fn line_text(text: &str, line0: usize) -> &str {
    text.split('\n')
        .nth(line0)
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .unwrap_or("")
}

fn encoded_len(c: char, encoding: PositionEncoding) -> usize {
    match encoding {
        PositionEncoding::Utf8 => c.len_utf8(),
        PositionEncoding::Utf16 => c.len_utf16(),
        PositionEncoding::Utf32 => 1,
    }
}

/// (1-based line, 1-based char column) → LSP position in `encoding`.
pub(crate) fn to_lsp_position(
    text: &str,
    line: usize,
    column: usize,
    encoding: PositionEncoding,
) -> Result<Value> {
    if line == 0 || column == 0 {
        bail!("`line` and `column` are 1-based and must be >= 1");
    }
    let line0 = line - 1;
    let line_str = line_text(text, line0);
    let mut character = 0usize;
    for (index, c) in line_str.chars().enumerate() {
        if index + 1 >= column {
            break;
        }
        character += encoded_len(c, encoding);
    }
    Ok(serde_json::json!({ "line": line0, "character": character }))
}

/// LSP position → (1-based line, 1-based char column).
pub(crate) fn from_lsp_position(
    text: &str,
    position: &Value,
    encoding: PositionEncoding,
) -> (usize, usize) {
    let line0 = position.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let target = position
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let line_str = line_text(text, line0);
    let mut consumed = 0usize;
    let mut column0 = 0usize;
    for c in line_str.chars() {
        if consumed >= target {
            break;
        }
        consumed += encoded_len(c, encoding);
        column0 += 1;
    }
    (line0 + 1, column0 + 1)
}

/// LSP position → byte offset into `text` (for applying edits).
pub(crate) fn position_to_byte_offset(
    text: &str,
    position: &Value,
    encoding: PositionEncoding,
) -> usize {
    let line0 = position.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let target = position
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut offset = 0usize;
    let mut current_line = 0usize;
    let mut chars = text.char_indices().peekable();
    // Advance to the start of the target line.
    while current_line < line0 {
        match chars.next() {
            Some((index, '\n')) => {
                current_line += 1;
                offset = index + 1;
            }
            Some(_) => {}
            None => return text.len(),
        }
    }
    let mut consumed = 0usize;
    for (index, c) in chars {
        if consumed >= target || c == '\n' {
            return index;
        }
        consumed += encoded_len(c, encoding);
    }
    text.len().max(offset)
}

// ---------------------------------------------------------------------------
// WorkspaceEdit application

#[derive(Debug, serde::Serialize)]
pub(crate) struct FileEditSummary {
    pub path: String,
    pub operation: String,
    pub edits: usize,
}

/// One entry of a `WorkspaceEdit`, with its targets already checked.
enum EditStep<'a> {
    Create {
        path: String,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        path: String,
        recursive: bool,
    },
    Text {
        path: String,
        edits: &'a [Value],
    },
}

/// Apply an LSP `WorkspaceEdit` to the workspace. Prefers `documentChanges`
/// over `changes` per the spec. Resource operations (create/rename/delete)
/// are supported; embedded commands are not.
///
/// The whole edit is worked out in memory first, against the files as they
/// are now, and only then handed to the host: a malformed edit changes
/// nothing, and a remote host receives one batch instead of a round trip per
/// file.
pub(crate) async fn apply_workspace_edit(
    manager: &LspManager,
    context: &ToolContext,
    edit: &Value,
    encoding: PositionEncoding,
    server: &LanguageServer,
) -> Result<Vec<FileEditSummary>> {
    let root = manager.host.root(context).await?;
    let steps = edit_steps(Path::new(&root), edit)?;

    // Current content of every file an edit reads: text-edit targets, and
    // rename sources (a later text edit may address the new name).
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for step in &steps {
        match step {
            EditStep::Text { path, .. } => {
                wanted.insert(path.clone());
            }
            EditStep::Rename { from, .. } => {
                wanted.insert(from.clone());
            }
            // An existing file kept by `ignoreIfExists` may be edited later.
            EditStep::Create { path, .. } => {
                wanted.insert(path.clone());
            }
            EditStep::Delete { .. } => {}
        }
    }
    let wanted: Vec<String> = wanted.into_iter().collect();
    let texts = manager.host.read_texts(context, &wanted).await?;
    let mut content: HashMap<String, String> = wanted
        .into_iter()
        .zip(texts)
        .filter_map(|(path, text)| text.map(|text| (path, text)))
        .collect();

    let relative = |path: &str| -> String {
        Path::new(path)
            .strip_prefix(&root)
            .unwrap_or(Path::new(path))
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut ops: Vec<FileOp> = Vec::new();
    let mut dirty: BTreeSet<String> = BTreeSet::new();
    let mut summaries = Vec::new();
    for step in steps {
        match step {
            EditStep::Create {
                path,
                overwrite,
                ignore_if_exists,
            } => {
                if overwrite || !content.contains_key(&path) {
                    content.insert(path.clone(), String::new());
                    dirty.remove(&path);
                }
                summaries.push(FileEditSummary {
                    path: relative(&path),
                    operation: "create".to_string(),
                    edits: 0,
                });
                ops.push(FileOp::Create {
                    path,
                    overwrite,
                    ignore_if_exists,
                });
            }
            EditStep::Rename { from, to } => {
                if let Some(text) = content.remove(&from) {
                    content.insert(to.clone(), text);
                }
                if dirty.remove(&from) {
                    dirty.insert(to.clone());
                }
                summaries.push(FileEditSummary {
                    path: format!("{} -> {}", relative(&from), relative(&to)),
                    operation: "rename".to_string(),
                    edits: 0,
                });
                ops.push(FileOp::Rename { from, to });
            }
            EditStep::Delete { path, recursive } => {
                content.remove(&path);
                dirty.remove(&path);
                summaries.push(FileEditSummary {
                    path: relative(&path),
                    operation: "delete".to_string(),
                    edits: 0,
                });
                ops.push(FileOp::Delete { path, recursive });
            }
            EditStep::Text { path, edits } => {
                let text = content.get(&path).with_context(|| {
                    format!(
                        "edit target {} is not a readable text file",
                        relative(&path)
                    )
                })?;
                let updated = apply_text_edits_to_string(text, edits, encoding)?;
                content.insert(path.clone(), updated);
                summaries.push(FileEditSummary {
                    path: relative(&path),
                    operation: "edit".to_string(),
                    edits: edits.len(),
                });
                dirty.insert(path);
            }
        }
    }
    // Resource operations run first, in the server's order; the final text of
    // every edited file is written afterwards under its final name.
    for path in &dirty {
        if let Some(text) = content.get(path) {
            ops.push(FileOp::Write {
                path: path.clone(),
                content: text.clone(),
            });
        }
    }
    manager.host.apply(context, &root, &ops).await?;

    // Keep the server's view current for documents it has open.
    for path in &dirty {
        if let Some(text) = content.get(path) {
            let _ = server
                .client
                .sync_document(
                    &path_to_uri(Path::new(path)),
                    language_id_for_extension(&extension_of(path)),
                    text,
                )
                .await;
        }
    }
    Ok(summaries)
}

fn edit_steps<'a>(root: &Path, edit: &'a Value) -> Result<Vec<EditStep<'a>>> {
    let mut steps = Vec::new();
    if let Some(document_changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            let flag = |pointer: &str| {
                change
                    .pointer(pointer)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            };
            let target = |key: &str| -> Result<String> {
                let uri = change
                    .get(key)
                    .and_then(Value::as_str)
                    .with_context(|| format!("workspace-edit operation without {key}"))?;
                edit_target(root, uri)
            };
            match change.get("kind").and_then(Value::as_str) {
                Some("create") => steps.push(EditStep::Create {
                    path: target("uri")?,
                    overwrite: flag("/options/overwrite"),
                    ignore_if_exists: flag("/options/ignoreIfExists"),
                }),
                Some("rename") => steps.push(EditStep::Rename {
                    from: target("oldUri")?,
                    to: target("newUri")?,
                }),
                Some("delete") => steps.push(EditStep::Delete {
                    path: target("uri")?,
                    recursive: flag("/options/recursive"),
                }),
                Some(other) => bail!("unsupported workspace-edit resource operation: {other}"),
                None => {
                    let uri = change
                        .pointer("/textDocument/uri")
                        .and_then(Value::as_str)
                        .context("documentChanges entry without textDocument.uri")?;
                    let edits = change
                        .get("edits")
                        .and_then(Value::as_array)
                        .context("documentChanges entry without edits")?;
                    steps.push(EditStep::Text {
                        path: edit_target(root, uri)?,
                        edits,
                    });
                }
            }
        }
        return Ok(steps);
    }
    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        // Deterministic order for stable output and tests.
        let mut uris: Vec<&String> = changes.keys().collect();
        uris.sort();
        for uri in uris {
            let edits = changes[uri]
                .as_array()
                .context("changes entry must be an edit array")?;
            steps.push(EditStep::Text {
                path: edit_target(root, uri)?,
                edits,
            });
        }
    }
    Ok(steps)
}

/// The absolute path a server-proposed change addresses, refused when it is
/// outside the workspace or inside a vendored or build directory. `..` is
/// folded lexically first, so a target that does not exist yet cannot smuggle
/// it past the prefix check. The host checks again when it applies the change
/// (a local host also resolves symlinks there).
fn edit_target(root: &Path, uri: &str) -> Result<String> {
    let path = normalize_lexically(&uri_to_path(uri)?)?;
    let Ok(relative) = path.strip_prefix(root) else {
        bail!(
            "server proposed an edit outside the workspace: {}",
            path.display()
        );
    };
    if let Some(name) = protected_component(relative) {
        bail!(
            "server proposed an edit inside the protected directory `{name}`: {}",
            path.display()
        );
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("edit target is not valid UTF-8"))
}

pub(crate) fn apply_text_edits_to_string(
    text: &str,
    edits: &[Value],
    encoding: PositionEncoding,
) -> Result<String> {
    let mut spans: Vec<(usize, usize, &str)> = Vec::with_capacity(edits.len());
    for edit in edits {
        let range = edit.get("range").context("text edit without range")?;
        let start = range.get("start").context("range without start")?;
        let end = range.get("end").context("range without end")?;
        let new_text = edit
            .get("newText")
            .and_then(Value::as_str)
            .context("text edit without newText")?;
        let start_offset = position_to_byte_offset(text, start, encoding);
        let end_offset = position_to_byte_offset(text, end, encoding);
        if end_offset < start_offset {
            bail!("text edit range is inverted");
        }
        spans.push((start_offset, end_offset, new_text));
    }
    // Apply back-to-front so earlier offsets stay valid.
    spans.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for window in spans.windows(2) {
        // After the descending sort, the *next* span ends before this starts.
        if window[1].1 > window[0].0 {
            bail!("text edits overlap; refusing to apply");
        }
    }
    let mut result = text.to_string();
    for (start, end, new_text) in spans {
        result.replace_range(start..end, new_text);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uri_roundtrip_with_spaces_and_unicode() {
        let path = Path::new("/tmp/my project/naïve file.rs");
        let uri = path_to_uri(path);
        assert!(uri.starts_with("file:///tmp/my%20project/"), "{uri}");
        assert_eq!(uri_to_path(&uri).expect("parse"), path);
    }

    #[test]
    fn uri_to_path_rejects_non_file() {
        assert!(uri_to_path("https://example.com/x").is_err());
    }

    #[test]
    fn utf16_position_conversion_handles_wide_chars() {
        // "a𐐀b" — 𐐀 is 2 UTF-16 units and 4 UTF-8 bytes.
        let text = "a𐐀b\nplain";
        let pos = to_lsp_position(text, 1, 3, PositionEncoding::Utf16).expect("pos");
        assert_eq!(pos, json!({ "line": 0, "character": 3 }));
        let pos8 = to_lsp_position(text, 1, 3, PositionEncoding::Utf8).expect("pos");
        assert_eq!(pos8, json!({ "line": 0, "character": 5 }));

        let (line, column) = from_lsp_position(
            text,
            &json!({ "line": 0, "character": 3 }),
            PositionEncoding::Utf16,
        );
        assert_eq!((line, column), (1, 3));
        let (line, column) = from_lsp_position(
            text,
            &json!({ "line": 1, "character": 2 }),
            PositionEncoding::Utf16,
        );
        assert_eq!((line, column), (2, 3));
    }

    #[test]
    fn byte_offset_clamps_past_line_end() {
        let text = "ab\ncd";
        let offset = position_to_byte_offset(
            text,
            &json!({ "line": 0, "character": 99 }),
            PositionEncoding::Utf16,
        );
        assert_eq!(offset, 2); // end of "ab", before the newline
        let offset = position_to_byte_offset(
            text,
            &json!({ "line": 5, "character": 0 }),
            PositionEncoding::Utf16,
        );
        assert_eq!(offset, text.len());
    }

    #[test]
    fn text_edits_apply_back_to_front() {
        let text = "let old = old + 1;";
        let edit = |start: u64, end: u64| {
            json!({
                "range": {
                    "start": { "line": 0, "character": start },
                    "end": { "line": 0, "character": end }
                },
                "newText": "renamed"
            })
        };
        let updated =
            apply_text_edits_to_string(text, &[edit(4, 7), edit(10, 13)], PositionEncoding::Utf16)
                .expect("apply");
        assert_eq!(updated, "let renamed = renamed + 1;");
    }

    #[test]
    fn overlapping_text_edits_are_rejected() {
        let text = "abcdef";
        let edit = |start: u64, end: u64| {
            json!({
                "range": {
                    "start": { "line": 0, "character": start },
                    "end": { "line": 0, "character": end }
                },
                "newText": "x"
            })
        };
        let err =
            apply_text_edits_to_string(text, &[edit(0, 3), edit(2, 5)], PositionEncoding::Utf16)
                .expect_err("overlap");
        assert!(err.to_string().contains("overlap"), "{err}");
    }

    #[test]
    fn edit_target_rejects_escapes() {
        let root = Path::new("/home/ws");
        assert!(edit_target(root, &path_to_uri(Path::new("/etc/passwd"))).is_err());
        let sneaky = path_to_uri(Path::new("/home/ws/sub/../../escape.txt"));
        assert!(edit_target(root, &sneaky).is_err());
        let inside = path_to_uri(Path::new("/home/ws/new/./file.rs"));
        assert_eq!(
            edit_target(root, &inside).expect("inside"),
            "/home/ws/new/file.rs"
        );
    }

    /// Workspace edits come from the language server, not a file tool, so
    /// vendored and build directories are refused here at any depth.
    #[test]
    fn edit_target_rejects_protected_directories() {
        let root = Path::new("/home/ws");
        for blocked in [
            ".git/config",
            "node_modules/pkg/index.js",
            "sub/target/out.rs",
        ] {
            let uri = path_to_uri(&root.join(blocked));
            let err = edit_target(root, &uri).expect_err(blocked);
            assert!(err.to_string().contains("protected directory"), "{err}");
        }
        // A file merely *named* like a protected directory is fine.
        assert!(edit_target(root, &path_to_uri(&root.join("targets.rs"))).is_ok());
    }

    #[test]
    fn edit_steps_keep_the_servers_order() {
        let root = Path::new("/home/ws");
        let edit = json!({
            "documentChanges": [
                { "kind": "rename", "oldUri": "file:///home/ws/a.rs", "newUri": "file:///home/ws/b.rs" },
                { "textDocument": { "uri": "file:///home/ws/b.rs" }, "edits": [] },
                { "kind": "delete", "uri": "file:///home/ws/c.rs" },
            ]
        });
        let steps = edit_steps(root, &edit).expect("steps");
        assert!(
            matches!(&steps[0], EditStep::Rename { from, to } if from == "/home/ws/a.rs" && to == "/home/ws/b.rs")
        );
        assert!(matches!(&steps[1], EditStep::Text { path, .. } if path == "/home/ws/b.rs"));
        assert!(
            matches!(&steps[2], EditStep::Delete { path, recursive: false } if path == "/home/ws/c.rs")
        );
    }

    #[test]
    fn default_specs_cover_expected_extensions() {
        let specs = default_server_specs();
        let find = |ext: &str| {
            specs
                .iter()
                .find(|spec| spec.extensions.iter().any(|e| e == ext))
                .map(|spec| spec.key.as_str())
        };
        assert_eq!(find("rs"), Some("rust"));
        assert_eq!(find("tsx"), Some("typescript"));
        assert_eq!(find("py"), Some("python"));
        assert_eq!(find("go"), Some("go"));
        assert_eq!(find("cpp"), Some("clangd"));
        assert_eq!(find("zig"), None);
    }
}
