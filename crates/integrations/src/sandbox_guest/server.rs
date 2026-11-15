//! The guest side: an axum server that runs inside the sandbox.
//!
//! Decisions:
//! - Commands run with `sh -c` in their own process group, so a timeout kills
//!   everything the command started, not only the shell.
//! - Output past [`MAX_EXEC_OUTPUT_BYTES`] is drained and dropped rather than
//!   buffered, so a command printing gigabytes cannot exhaust the guest.
//! - The guest runs with the sandbox user's own rights. It is not a security
//!   boundary inside the sandbox; the sandbox (VM or container) is. It only
//!   keeps callers without the per-sandbox token out.
//! - Archive import uses `tar`'s `unpack`, which refuses entries escaping the
//!   target directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use everruns_contracts::sandbox_guest::{
    DEFAULT_EXEC_TIMEOUT_MS, GUEST_PROTOCOL_VERSION, GUEST_TOKEN_HEADER, GuestArchiveImport,
    GuestCapability, GuestError, GuestExecRequest, GuestExecResponse, GuestFileKind, GuestFileList,
    GuestFileStat, GuestHealth, GuestWriteResponse, MAX_EXEC_OUTPUT_BYTES, MAX_EXEC_TIMEOUT_MS,
    MAX_FILE_BYTES, paths,
};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// Settings for one guest process.
#[derive(Debug, Clone)]
pub struct GuestConfig {
    /// The per-sandbox token every request must carry.
    pub token: String,
    /// Default directory for commands and relative paths.
    pub workspace: PathBuf,
    /// `KEY=VALUE` file merged into every command's environment.
    pub exec_env_file: Option<PathBuf>,
    /// Report [`GuestCapability::Desktop`].
    pub desktop: bool,
}

struct GuestState {
    config: GuestConfig,
}

type Shared = Arc<GuestState>;

/// The guest's router: every protocol path, behind the token check.
pub fn router(config: GuestConfig) -> Router {
    let state = Arc::new(GuestState { config });
    let protected = Router::new()
        .route(paths::EXEC, post(exec))
        .route(paths::FILES, get(read_file).put(write_file).delete(delete))
        .route(paths::STAT, get(stat))
        .route(paths::LIST, get(list))
        .route(paths::ARCHIVE, get(export_archive).put(import_archive))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));
    Router::new()
        .route(paths::HEALTH, get(health))
        .merge(protected)
        // Archives carry a whole workspace; plain files stay at the file cap.
        .layer(DefaultBodyLimit::max(MAX_FILE_BYTES))
        .with_state(state)
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        axum::Json(GuestError {
            code: code.to_string(),
            message: message.into(),
        }),
    )
        .into_response()
}

fn authorized(state: &GuestState, headers: &HeaderMap) -> bool {
    let Some(presented) = headers
        .get(GUEST_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    constant_time_eq(presented.as_bytes(), state.config.token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_token(State(state): State<Shared>, request: Request, next: Next) -> Response {
    if authorized(&state, request.headers()) {
        next.run(request).await
    } else {
        error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or wrong guest token",
        )
    }
}

async fn health(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let mut answer = GuestHealth {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol: GUEST_PROTOCOL_VERSION,
        capabilities: Vec::new(),
        workspace: None,
    };
    if authorized(&state, &headers) {
        answer.capabilities = vec![
            GuestCapability::Exec,
            GuestCapability::Files,
            GuestCapability::Archive,
        ];
        if state.config.desktop {
            answer.capabilities.push(GuestCapability::Desktop);
        }
        answer.workspace = Some(state.config.workspace.display().to_string());
    }
    axum::Json(answer).into_response()
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

fn resolve(state: &GuestState, path: &str) -> Result<PathBuf, Box<Response>> {
    if path.is_empty() || path.contains('\0') {
        return Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "path must be a non-empty string",
        )));
    }
    let path = Path::new(path);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.config.workspace.join(path)
    })
}

fn io_error(path: &Path, err: std::io::Error) -> Response {
    let status = match err.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let code = match status {
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::FORBIDDEN => "forbidden",
        _ => "internal",
    };
    error(status, code, format!("{}: {err}", path.display()))
}

/// Entries of a `KEY=VALUE` file; blank lines and `#` comments skipped.
fn read_env_file(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.to_string()))
        .collect()
}

/// Read up to `cap` bytes, then keep draining so the child never blocks on
/// a full pipe. Returns the kept bytes and whether anything was dropped.
async fn read_capped(mut reader: impl AsyncRead + Unpin, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                if room >= n {
                    kept.extend_from_slice(&chunk[..n]);
                } else {
                    kept.extend_from_slice(&chunk[..room]);
                    truncated = true;
                }
            }
        }
    }
    (kept, truncated)
}

fn kill_group(pid: u32) {
    // SAFETY: kill(2) with a negative pid signals the process group the
    // command was started in (`process_group(0)`); it touches no memory.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

async fn exec(
    State(state): State<Shared>,
    axum::Json(request): axum::Json<GuestExecRequest>,
) -> Response {
    let started = Instant::now();
    let cwd = match request.cwd.as_deref() {
        Some(cwd) => match resolve(&state, cwd) {
            Ok(cwd) => cwd,
            Err(response) => return *response,
        },
        None => state.config.workspace.clone(),
    };
    let timeout = Duration::from_millis(
        request
            .timeout_ms
            .unwrap_or(DEFAULT_EXEC_TIMEOUT_MS)
            .clamp(1, MAX_EXEC_TIMEOUT_MS),
    );
    let mut command = match request.argv.split_first() {
        Some((program, args)) => {
            let mut command = tokio::process::Command::new(program);
            command.args(args);
            command
        }
        None if request.command.is_empty() => {
            return error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "exec needs a command or an argv",
            );
        }
        None => {
            let mut command = tokio::process::Command::new("sh");
            command.arg("-c").arg(&request.command);
            command
        }
    };
    command
        .current_dir(&cwd)
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    if let Some(file) = &state.config.exec_env_file {
        command.envs(read_env_file(file));
    }
    command.envs(&request.env);
    // The guest's own secrets never reach commands.
    command
        .env_remove(everruns_contracts::sandbox_guest::GUEST_TOKEN_ENV)
        .env_remove(everruns_contracts::sandbox_guest::GUEST_TOKEN_FILE_ENV);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("cannot start the command in {}: {err}", cwd.display()),
            );
        }
    };
    let pid = child.id();
    if let (Some(input), Some(mut stdin)) = (request.stdin, child.stdin.take()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(input.as_bytes()).await;
        });
    }
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the command's output is not piped",
        );
    };
    let out = tokio::spawn(read_capped(stdout, MAX_EXEC_OUTPUT_BYTES));
    let err = tokio::spawn(read_capped(stderr, MAX_EXEC_OUTPUT_BYTES));

    let (exit_code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => (exit_code(status), false),
        Ok(Err(err)) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                err.to_string(),
            );
        }
        Err(_) => {
            if let Some(pid) = pid {
                kill_group(pid);
            }
            let _ = child.wait().await;
            (124, true)
        }
    };
    // Background children that kept the pipes open must not hold the answer.
    if let Some(pid) = pid {
        kill_group(pid);
    }
    let (stdout, out_cut) = out.await.unwrap_or_default();
    let (stderr, err_cut) = err.await.unwrap_or_default();
    axum::Json(GuestExecResponse {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated: out_cut || err_cut,
        timed_out,
        duration_ms: started.elapsed().as_millis() as u64,
    })
    .into_response()
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(-1)
}

async fn read_file(State(state): State<Shared>, Query(query): Query<PathQuery>) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_dir() => {
            return error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("{} is a directory", path.display()),
            );
        }
        Ok(meta) if meta.len() > MAX_FILE_BYTES as u64 => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                format!(
                    "{} is {} bytes, more than the {MAX_FILE_BYTES} a read returns",
                    path.display(),
                    meta.len()
                ),
            );
        }
        Ok(_) => {}
        Err(err) => return io_error(&path, err),
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(err) => io_error(&path, err),
    }
}

async fn write_file(
    State(state): State<Shared>,
    Query(query): Query<PathQuery>,
    body: Bytes,
) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    if let Some(parent) = path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        return io_error(parent, err);
    }
    match tokio::fs::write(&path, &body).await {
        Ok(()) => axum::Json(GuestWriteResponse {
            path: path.display().to_string(),
            bytes_written: body.len() as u64,
        })
        .into_response(),
        Err(err) => io_error(&path, err),
    }
}

async fn delete(State(state): State<Shared>, Query(query): Query<PathQuery>) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    let result = match tokio::fs::symlink_metadata(&path).await {
        Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(&path).await,
        Ok(_) => tokio::fs::remove_file(&path).await,
        // Deleting what is not there succeeds: the caller wanted it gone.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    };
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => io_error(&path, err),
    }
}

fn file_stat(path: &Path, meta: &std::fs::Metadata) -> GuestFileStat {
    use std::os::unix::fs::PermissionsExt;
    let kind = if meta.file_type().is_symlink() {
        GuestFileKind::Symlink
    } else if meta.is_dir() {
        GuestFileKind::Directory
    } else if meta.is_file() {
        GuestFileKind::File
    } else {
        GuestFileKind::Other
    };
    GuestFileStat {
        path: path.display().to_string(),
        kind,
        size: meta.len(),
        mode: meta.permissions().mode() & 0o7777,
        modified: meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs() as i64),
    }
}

async fn stat(State(state): State<Shared>, Query(query): Query<PathQuery>) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    match tokio::fs::symlink_metadata(&path).await {
        Ok(meta) => axum::Json(file_stat(&path, &meta)).into_response(),
        Err(err) => io_error(&path, err),
    }
}

/// Most entries one listing returns.
const MAX_LIST_ENTRIES: usize = 10_000;

async fn list(State(state): State<Shared>, Query(query): Query<PathQuery>) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    let mut dir = match tokio::fs::read_dir(&path).await {
        Ok(dir) => dir,
        Err(err) => return io_error(&path, err),
    };
    let mut entries = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
        let entry_path = entry.path();
        if let Ok(meta) = tokio::fs::symlink_metadata(&entry_path).await {
            entries.push(file_stat(&entry_path, &meta));
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    axum::Json(GuestFileList {
        path: path.display().to_string(),
        entries,
    })
    .into_response()
}

async fn export_archive(State(state): State<Shared>, Query(query): Query<PathQuery>) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    let source = path.clone();
    let built = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let encoder =
            flate2::write::GzEncoder::new(CappedWriter::default(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        builder.follow_symlinks(false);
        builder.append_dir_all(".", &source)?;
        let encoder = builder.into_inner()?;
        Ok(encoder.finish()?.bytes)
    })
    .await;
    match built {
        Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "application/gzip")], bytes).into_response(),
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::OutOfMemory => error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!(
                "{} packs to more than {MAX_FILE_BYTES} bytes",
                path.display()
            ),
        ),
        Ok(Err(err)) => io_error(&path, err),
        Err(err) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            err.to_string(),
        ),
    }
}

/// A buffer that refuses to grow past [`MAX_FILE_BYTES`].
#[derive(Default)]
struct CappedWriter {
    bytes: Vec<u8>,
}

impl std::io::Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() + buf.len() > MAX_FILE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "archive too large",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn import_archive(
    State(state): State<Shared>,
    Query(query): Query<PathQuery>,
    body: Bytes,
) -> Response {
    let path = match resolve(&state, &query.path) {
        Ok(path) => path,
        Err(response) => return *response,
    };
    let target = path.clone();
    let unpacked = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        std::fs::create_dir_all(&target)?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(body.as_ref()));
        archive.set_preserve_permissions(true);
        let mut count = 0;
        for entry in archive.entries()? {
            let mut entry = entry?;
            // `unpack_in` refuses paths that would land outside `target`.
            if entry.unpack_in(&target)? {
                count += 1;
            }
        }
        Ok(count)
    })
    .await;
    match unpacked {
        Ok(Ok(entries)) => axum::Json(GuestArchiveImport {
            path: path.display().to_string(),
            entries,
        })
        .into_response(),
        Ok(Err(err)) => error(
            StatusCode::BAD_REQUEST,
            "bad_request",
            format!("cannot unpack into {}: {err}", path.display()),
        ),
        Err(err) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            err.to_string(),
        ),
    }
}

/// Settings from the environment, as the guest binary reads them.
pub fn config_from_env() -> Result<GuestConfig, String> {
    use everruns_contracts::sandbox_guest::{
        GUEST_EXEC_ENV_FILE_ENV, GUEST_TOKEN_ENV, GUEST_TOKEN_FILE_ENV,
    };
    let token = match std::env::var(GUEST_TOKEN_FILE_ENV) {
        Ok(file) => std::fs::read_to_string(&file)
            .map_err(|err| format!("cannot read {GUEST_TOKEN_FILE_ENV} ({file}): {err}"))?
            .trim()
            .to_string(),
        Err(_) => std::env::var(GUEST_TOKEN_ENV).unwrap_or_default(),
    };
    // A short token would make the bearer guessable.
    if token.len() < 32 {
        return Err(format!(
            "set {GUEST_TOKEN_FILE_ENV} or {GUEST_TOKEN_ENV} to a token of at least 32 characters"
        ));
    }
    let workspace = std::env::var("EVERRUNS_GUEST_WORKSPACE")
        .map(PathBuf::from)
        .or_else(|_| std::env::current_dir())
        .map_err(|err| format!("no workspace directory: {err}"))?;
    Ok(GuestConfig {
        token,
        workspace,
        exec_env_file: std::env::var(GUEST_EXEC_ENV_FILE_ENV)
            .ok()
            .map(PathBuf::from),
        desktop: std::env::var("EVERRUNS_GUEST_DESKTOP").is_ok_and(|value| value == "1")
            || std::env::var_os("DISPLAY").is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn env_files_skip_comments_and_keep_equals_in_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        std::fs::write(&file, "# proxy\nALL_PROXY=socks5h://u:p=q@h:1\n\nBAD\n").unwrap();
        let env = read_env_file(&file);
        assert_eq!(env.len(), 1);
        assert_eq!(env["ALL_PROXY"], "socks5h://u:p=q@h:1");
    }
}
