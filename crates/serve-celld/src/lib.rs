//! # serve-celld (experimental)
//!
//! Run a [serve](https://docs.rs/everruns-serve) app durably on
//! [celld](https://github.com/denoland/celld), self-hosted distributed
//! Durable Objects, part of the [Everruns](https://everruns.com) ecosystem.
//!
//! > **Experimental.** Like serve, this is a proof of concept with no
//! > compatibility promise.
//!
//! On celld a Durable Object (a *cell*) has one owner node at a time and a
//! replicated SQLite database that survives a move to another node. A
//! container the cell supervises does not: a move, a node restart or a reset
//! destroys it. So the agent runs in the container and the cell keeps what
//! must survive:
//!
//! - the latest **snapshot** of serve's state (the session catalog, the
//!   engine's canonical event log and the workspace), taken whenever the
//!   app is idle;
//! - a **journal** of the requests that arrived since, replayed into a
//!   fresh container after the snapshot is restored.
//!
//! This crate is the container half. It serves serve's own `/v1` wire API
//! and three routes only the cell calls:
//!
//! - `GET /celld/state`: `{"boot_id", "booted", "busy"}`. A new `boot_id`
//!   tells the cell the container is fresh and must be restored. Never boots
//!   the app.
//! - `GET /celld/snapshot`: a tar of the data directory, with every SQLite
//!   database copied through the online backup API so the copy is
//!   consistent. `409` while a turn runs.
//! - `PUT /celld/restore`: unpack a snapshot into the data directory. `409`
//!   once the app has booted, so a restore can never race live state.
//!
//! serve boots on the first other request, after the cell had its chance to
//! restore. The [JavaScript cell example](https://github.com/everruns/everruns/tree/main/examples/serve/celld/worker)
//! decides when to snapshot, what to journal and when to replay.
//!
//! ```no_run
//! use serve::prelude::*;
//!
//! #[agent]
//! fn assistant() -> Agent {
//!     Agent::builder()
//!         .model("anthropic/claude-sonnet-5")
//!         .instructions("Be brief.")
//!         .build()
//! }
//!
//! #[tokio::main]
//! async fn main() -> serve::Result {
//!     // No command (what the container runs) or `celld`: the celld contract
//!     // on :8080. `dev`, `start`, `eval`, `manifest`, `deploy`: serve's own.
//!     serve_celld::start(App::builder().discover().build()).await
//! }
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns::sqlite as rusqlite;
use std::ffi::OsString;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use clap::Parser;
use serde_json::{Value, json};
use serve::{App, Mode, Server};
use tokio::sync::{Mutex, OnceCell};
use tower::ServiceExt;

/// The port the container listens on, and the cell's `defaultPort`.
pub const PORT: u16 = 8080;

/// The largest snapshot `PUT /celld/restore` accepts.
pub const MAX_SNAPSHOT_BYTES: usize = 512 * 1024 * 1024;

/// The `celld` command line.
#[derive(Parser, Debug)]
#[command(about = "Serve this app as a celld container (experimental).")]
struct Cli {
    /// Port to listen on.
    #[arg(long, env = "PORT", default_value_t = PORT)]
    port: u16,
    /// Run in serve's `dev` mode (simulator fallback, no secret check)
    /// instead of `start`, to try the contract locally.
    #[arg(long)]
    dev: bool,
}

/// How [`router`] boots the app.
#[derive(Clone, Debug)]
pub struct Options {
    /// serve's mode: [`Mode::Start`] in production.
    pub mode: Mode,
    /// Where serve keeps its state inside the container. Ephemeral by
    /// design: the cell holds the durable copy. Defaults to
    /// [`default_data_dir`].
    pub data_dir: PathBuf,
}

impl Options {
    /// Production defaults: `start` mode, state in [`default_data_dir`].
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            data_dir: default_data_dir(),
        }
    }
}

/// `SERVE_DATA_DIR` when set, else `serve-celld` in the temp directory.
pub fn default_data_dir() -> PathBuf {
    std::env::var_os("SERVE_DATA_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("serve-celld"))
}

/// Run the app: with no command, or with `celld`, serve the celld container
/// contract; any other command is serve's (see [`serve::start`]).
pub async fn start(app: App) -> serve::Result {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    match args.get(1).and_then(|arg| arg.to_str()) {
        None => {}
        Some("celld") => {
            args.remove(1);
        }
        Some(_) => return serve::start(app).await,
    }
    let cli = Cli::parse_from(args);
    let options = Options::new(if cli.dev { Mode::Dev } else { Mode::Start });
    let name = app.name().to_string();
    let target = Target::new(app, options.clone())?;
    let boot_id = target.boot_id.clone();
    let router = target.router();

    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "serve-celld · {name} · {:?} · boot {boot_id} · state in {} · listening on {addr}",
        options.mode,
        options.data_dir.display()
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// The celld container contract around `app`: `/celld/*` and serve's `/v1`
/// wire API.
///
/// The app is checked at once (secrets in `start`, every agent resolved), but
/// the server that keeps state boots on the first request outside
/// `/celld/*`, so the cell can restore a snapshot first.
pub fn router(app: App, options: Options) -> serve::Result<Router> {
    Ok(Target::new(app, options)?.router())
}

/// The state of one container process.
#[derive(Clone)]
struct Target {
    app: App,
    options: Arc<Options>,
    /// New on every process start: how the cell tells a fresh container.
    boot_id: String,
    booted: Arc<OnceCell<Booted>>,
    /// Serializes booting against restoring and snapshotting.
    gate: Arc<Mutex<()>>,
}

struct Booted {
    server: Server,
    wire: Router,
}

impl Target {
    fn new(app: App, options: Options) -> serve::Result<Self> {
        // A throwaway in-memory boot: fails fast on a bad app.
        drop(boot(app.clone(), &options, false)?);
        Ok(Self {
            app,
            options: Arc::new(options),
            boot_id: uuid::Uuid::now_v7().to_string(),
            booted: Arc::new(OnceCell::new()),
            gate: Arc::new(Mutex::new(())),
        })
    }

    fn router(self) -> Router {
        Router::new()
            .route("/celld/state", get(state))
            .route("/celld/snapshot", get(snapshot))
            .route(
                "/celld/restore",
                put(restore).layer(DefaultBodyLimit::max(MAX_SNAPSHOT_BYTES)),
            )
            .fallback(forward)
            .with_state(self)
    }

    /// The stateful server, booted on first use.
    async fn booted(&self) -> Result<&Booted, Response> {
        if let Some(booted) = self.booted.get() {
            return Ok(booted);
        }
        let _gate = self.gate.lock().await;
        self.booted
            .get_or_try_init(|| async {
                std::fs::create_dir_all(&self.options.data_dir)?;
                let server = boot(self.app.clone(), &self.options, true)?;
                server.spawn_schedules();
                let wire = server.router();
                Ok::<_, serve::Error>(Booted { server, wire })
            })
            .await
            .map_err(|err| {
                eprintln!("serve-celld: boot failed: {err:#}");
                problem(StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}"))
            })
    }

    fn busy(&self) -> bool {
        self.booted.get().is_some_and(|booted| booted.server.busy())
    }
}

/// Boot serve. `persistent: false` boots in memory.
fn boot(app: App, options: &Options, persistent: bool) -> serve::Result<Server> {
    let workspace = options.data_dir.join("workspace");
    let mut builder =
        Server::builder(app, options.mode).microvm(move |agent| microvm(agent, &workspace));
    if persistent {
        builder = builder.data_dir(options.data_dir.clone());
    }
    builder.build()
}

/// `[sandbox] kind = "microvm"` on celld: a real shell and file tools over a
/// workspace inside the data directory, so snapshots carry it.
///
/// Decision: no kernel containment inside the container. The container is
/// the boundary, with celld's fence around it; for code you did not write,
/// give the container class a runtime with its own kernel (`runsc`,
/// `kata`).
fn microvm(agent: everruns::AgentBuilder, workspace: &Path) -> everruns::AgentBuilder {
    agent
        .workspace(workspace)
        .workspace_policy(everruns::WorkspacePolicy::read_write())
        .capability(everruns::HostShell::new().containment(everruns::ContainmentMode::FullAccess))
}

/// `GET /celld/state`. Never boots the app.
async fn state(State(target): State<Target>) -> Json<Value> {
    Json(json!({
        "boot_id": target.boot_id,
        "booted": target.booted.get().is_some(),
        "busy": target.busy(),
    }))
}

/// `GET /celld/snapshot`: the data directory as a tar.
async fn snapshot(State(target): State<Target>) -> Response {
    let _gate = target.gate.lock().await;
    if target.busy() {
        return problem(
            StatusCode::CONFLICT,
            "a turn is running; snapshot when idle".into(),
        );
    }
    let dir = target.options.data_dir.clone();
    match tokio::task::spawn_blocking(move || snapshot_dir(&dir)).await {
        Ok(Ok(bytes)) => (
            [(header::CONTENT_TYPE, "application/x-tar")],
            Body::from(bytes),
        )
            .into_response(),
        Ok(Err(err)) => {
            eprintln!("serve-celld: snapshot failed: {err:#}");
            problem(StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}"))
        }
        Err(err) => problem(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// `PUT /celld/restore`: replace the data directory with a snapshot.
async fn restore(State(target): State<Target>, body: Bytes) -> Response {
    let _gate = target.gate.lock().await;
    if target.booted.get().is_some() {
        return problem(
            StatusCode::CONFLICT,
            "the app has booted; restore only into a fresh container".into(),
        );
    }
    let dir = target.options.data_dir.clone();
    match tokio::task::spawn_blocking(move || restore_dir(&dir, &body)).await {
        Ok(Ok(files)) => Json(json!({ "restored_files": files })).into_response(),
        Ok(Err(err)) => problem(StatusCode::BAD_REQUEST, format!("{err:#}")),
        Err(err) => problem(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// Everything else: serve's `/v1` wire API and `/health`.
async fn forward(State(target): State<Target>, request: Request<Body>) -> Response {
    match target.booted().await {
        Ok(booted) => match booted.wire.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
        Err(response) => response,
    }
}

/// Tar `dir`. SQLite databases go through the online backup API, which reads
/// a consistent copy while serve keeps them open; their `-wal`, `-shm` and
/// `-journal` siblings and lock files are left out, since the copy already
/// holds every committed page. Other regular files are copied as they are;
/// symbolic links and special files are skipped.
pub fn snapshot_dir(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    builder.mode(tar::HeaderMode::Deterministic);
    if dir.is_dir() {
        let scratch = tempfile_dir()?;
        append_dir(&mut builder, dir, Path::new(""), &scratch)?;
        let _ = std::fs::remove_dir_all(&scratch);
    }
    Ok(builder.into_inner()?)
}

fn append_dir(
    builder: &mut tar::Builder<Vec<u8>>,
    root: &Path,
    rel: &Path,
    scratch: &Path,
) -> anyhow::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(root.join(rel))?
        .collect::<Result<_, _>>()
        .with_context(|| format!("read {}", root.join(rel).display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let path = rel.join(&name);
        let kind = entry.file_type()?;
        let text = name.to_string_lossy();
        if text.ends_with(".lock") || is_sqlite_sidecar(&text) {
            continue;
        }
        if kind.is_dir() {
            builder.append_dir(&path, entry.path())?;
            append_dir(builder, root, &path, scratch)?;
        } else if kind.is_file() {
            if is_sqlite(&entry.path())? {
                let copy = scratch.join(uuid::Uuid::now_v7().to_string());
                rusqlite::backup(entry.path(), &copy)
                    .with_context(|| format!("back up {}", entry.path().display()))?;
                builder.append_path_with_name(&copy, &path)?;
                let _ = std::fs::remove_file(&copy);
            } else {
                builder.append_path_with_name(entry.path(), &path)?;
            }
        }
    }
    Ok(())
}

fn is_sqlite_sidecar(name: &str) -> bool {
    ["-wal", "-shm", "-journal"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn is_sqlite(path: &Path) -> anyhow::Result<bool> {
    let mut magic = [0u8; 16];
    let mut file = std::fs::File::open(path)?;
    let read = file.read(&mut magic)?;
    Ok(read == 16 && &magic == b"SQLite format 3\0")
}

fn tempfile_dir() -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("serve-celld-snap-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Replace `dir` with the snapshot in `bytes`. Accepts only regular files
/// and directories with relative paths that stay inside `dir`; anything else
/// fails the whole restore before `dir` is touched. Returns the file count.
pub fn restore_dir(dir: &Path, bytes: &[u8]) -> anyhow::Result<usize> {
    // Validate first, so a bad snapshot leaves the directory as it was.
    let mut archive = tar::Archive::new(bytes);
    let mut files = 0;
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.into_owned();
        let safe = path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)));
        if !safe {
            bail!(
                "snapshot entry `{}` leaves the data directory",
                path.display()
            );
        }
        match entry.header().entry_type() {
            tar::EntryType::Regular => files += 1,
            tar::EntryType::Directory => {}
            other => bail!(
                "snapshot entry `{}` is a {other:?}, not a file or directory",
                path.display()
            ),
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("clear {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir)?;
    let mut archive = tar::Archive::new(bytes);
    archive.set_preserve_permissions(false);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.unpack_in(dir)? {
            return Err(anyhow!("snapshot entry escaped {}", dir.display()));
        }
    }
    Ok(files)
}

/// RFC 9457 problem details, as serve's own routes answer.
fn problem(status: StatusCode, detail: String) -> Response {
    let body = json!({
        "title": status.canonical_reason().unwrap_or("Error"),
        "status": status.as_u16(),
        "detail": detail,
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    walk(root, &entry.path(), out);
                } else {
                    let rel = entry.path().strip_prefix(root).unwrap().to_path_buf();
                    out.push(rel.to_string_lossy().into_owned());
                }
            }
        }
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn snapshot_round_trips_an_open_wal_database_and_files() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("workspace/notes")).unwrap();
        std::fs::write(src.path().join("workspace/notes/a.md"), "hello").unwrap();
        let db = src.path().join("serve.db");
        let conn = rusqlite::open(&db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('kept');",
        )
        .unwrap();
        // Still open, rows only in the WAL: the backup must see them anyway.
        assert!(src.path().join("serve.db-wal").exists());
        std::fs::write(src.path().join("serve.db.lock"), "").unwrap();

        let bytes = snapshot_dir(src.path()).unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(dst.path().join("stale.txt"), "gone").unwrap();
        let count = restore_dir(dst.path(), &bytes).unwrap();

        assert_eq!(count, 2);
        assert_eq!(files(dst.path()), ["serve.db", "workspace/notes/a.md"]);
        assert_eq!(
            std::fs::read_to_string(dst.path().join("workspace/notes/a.md")).unwrap(),
            "hello"
        );
        let copy = rusqlite::open(dst.path().join("serve.db")).unwrap();
        let value: String = copy.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(value, "kept");
        drop(conn);
    }

    #[test]
    fn snapshot_of_a_missing_directory_is_empty_and_restores_to_empty() {
        let missing = tempfile::tempdir().unwrap().path().join("nope");
        let bytes = snapshot_dir(&missing).unwrap();
        let dst = tempfile::tempdir().unwrap();
        assert_eq!(restore_dir(&dst.path().join("data"), &bytes).unwrap(), 0);
        assert!(dst.path().join("data").is_dir());
    }

    #[test]
    fn restore_refuses_escaping_paths_and_links_without_touching_the_directory() {
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(dst.path().join("keep.txt"), "kept").unwrap();

        let mut escape = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        // `append_data` refuses `..`, so write the raw name.
        header.as_gnu_mut().unwrap().name[..9].copy_from_slice(b"../x.txt\0");
        header.set_cksum();
        escape.append(&header, &b"x"[..]).unwrap();
        let err = restore_dir(dst.path(), &escape.into_inner().unwrap()).unwrap_err();
        assert!(
            err.to_string().contains("leaves the data directory"),
            "{err}"
        );

        let mut link = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        link.append_link(&mut header, "evil", "/etc/passwd")
            .unwrap();
        let err = restore_dir(dst.path(), &link.into_inner().unwrap()).unwrap_err();
        assert!(err.to_string().contains("Symlink"), "{err}");

        assert!(restore_dir(dst.path(), b"definitely not a tar archive at all").is_err());
        assert_eq!(
            std::fs::read_to_string(dst.path().join("keep.txt")).unwrap(),
            "kept"
        );
    }
}
