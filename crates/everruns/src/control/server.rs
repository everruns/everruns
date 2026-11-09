//! The local control socket a host serves.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;

use super::registry::{ControlError, ControlRegistry};
use super::wire::{ControlCommand, ControlRequest, ControlResponse, ControlSent, Projector};
use super::{SOCKET_FILE, TOKEN_FILE};
use crate::{EventStreamError, Session};

/// Largest request line accepted, including the message text.
pub const MAX_REQUEST_BYTES: u64 = 256 * 1024;
/// How long a connection may take to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one response line may take to reach a client that stopped reading.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Default lifetime of an issued token before it is rotated.
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
/// Default bound on concurrently served connections.
const DEFAULT_MAX_CONNECTIONS: usize = 32;

/// Where an attached process finds a running host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachInfo {
    /// The private control directory (mode 0700).
    pub dir: PathBuf,
    /// The Unix socket inside it (mode 0600).
    pub socket: PathBuf,
    /// The current bearer token (mode 0600). Re-read it for every connection:
    /// it changes on rotation and disappears on revocation.
    pub token_file: PathBuf,
}

impl AttachInfo {
    /// The socket and token file paths inside `dir`.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            socket: dir.join(SOCKET_FILE),
            token_file: dir.join(TOKEN_FILE),
            dir,
        }
    }
}

/// Configures a [`ControlServer`].
#[derive(Clone, Debug)]
pub struct ControlServerBuilder {
    dir: PathBuf,
    token_ttl: Option<Duration>,
    max_connections: usize,
}

impl ControlServerBuilder {
    /// Rotate the token this often. Defaults to [`DEFAULT_TOKEN_TTL`]; `None`
    /// keeps one token until [`ControlServer::rotate_token`] or
    /// [`ControlServer::revoke`].
    pub fn token_ttl(mut self, ttl: Option<Duration>) -> Self {
        self.token_ttl = ttl.filter(|ttl| !ttl.is_zero());
        self
    }

    /// Bound concurrently served connections (attach streams included).
    /// Connections beyond it get `busy` and are closed.
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = max.max(1);
        self
    }

    /// Create the private directory, bind the socket, issue the first token
    /// and start serving `registry`.
    ///
    /// Must be called inside a Tokio runtime. Fails when `dir` exists but is
    /// not a directory owned by this user with no group or other permissions,
    /// or when another server is already listening there.
    pub fn start(self, registry: ControlRegistry) -> io::Result<ControlServer> {
        let uid = effective_uid();
        prepare_private_dir(&self.dir, uid)?;
        let info = AttachInfo::in_dir(&self.dir);
        remove_stale_socket(&info.socket)?;
        let listener = UnixListener::bind(&info.socket)?;
        // The directory already keeps other users out; the socket mode is
        // defense in depth against a later chmod of the directory.
        fs::set_permissions(&info.socket, fs::Permissions::from_mode(0o600))?;
        let (closing, _) = watch::channel(Closing::Open);
        let shared = Arc::new(Shared {
            info,
            uid,
            registry,
            token: Mutex::new(None),
            closing,
            permits: Arc::new(Semaphore::new(self.max_connections)),
        });
        shared.rotate(true)?;
        let mut tasks = vec![tokio::spawn(accept_loop(shared.clone(), listener))];
        if let Some(ttl) = self.token_ttl {
            let rotating = shared.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(ttl).await;
                    // A revoked server stays revoked; only `rotate_token` re-opens it.
                    if rotating.rotate(false).is_err() {
                        // A token we cannot write must not stay valid.
                        rotating.revoke();
                    }
                }
            }));
        }
        Ok(ControlServer { shared, tasks })
    }
}

/// A local control plane for a running host.
///
/// Serves a [`ControlRegistry`] on a Unix socket inside a private directory,
/// so a second process run by the same user can list sessions, attach to
/// their output, send input and cancel turns. Who may attach:
///
/// - only the same OS user: the directory is mode 0700, the socket 0600, and
///   every connection's peer credentials must match this process's effective
///   uid;
/// - only with the current bearer token, read from the 0600 token file.
///
/// The token rotates every [`token_ttl`](ControlServerBuilder::token_ttl)
/// and on [`rotate_token`](Self::rotate_token); the old one stops opening
/// connections at once. [`revoke`](Self::revoke) deletes the token and ends
/// every open attach stream and waiting send. Dropping the server stops
/// serving and removes the socket and token file. Sessions are unaffected by
/// any of this: disconnecting, revoking or shutting down never cancels a turn.
pub struct ControlServer {
    shared: Arc<Shared>,
    tasks: Vec<JoinHandle<()>>,
}

impl ControlServer {
    /// Configure a server rooted at `dir`. Use a directory only this host
    /// uses; [`default_dir`](super::default_dir) picks one.
    pub fn builder(dir: impl Into<PathBuf>) -> ControlServerBuilder {
        ControlServerBuilder {
            dir: dir.into(),
            token_ttl: Some(DEFAULT_TOKEN_TTL),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Start with the default token lifetime and connection bound.
    pub fn start(dir: impl Into<PathBuf>, registry: ControlRegistry) -> io::Result<Self> {
        Self::builder(dir).start(registry)
    }

    /// Where attached processes find this server.
    pub fn attach_info(&self) -> &AttachInfo {
        &self.shared.info
    }

    /// Issue a new token and invalidate the old one for new connections.
    /// Also re-enables a revoked server.
    pub fn rotate_token(&self) -> io::Result<()> {
        self.shared.rotate(true)?;
        let _ = self.shared.closing.send_if_modified(|state| {
            let revoked = *state == Closing::Revoked;
            if revoked {
                *state = Closing::Open;
            }
            revoked
        });
        Ok(())
    }

    /// Invalidate the token, delete the token file and end every open attach
    /// stream and waiting send. New connections are refused until
    /// [`rotate_token`](Self::rotate_token).
    pub fn revoke(&self) {
        self.shared.revoke();
    }

    /// Stop serving now. Equivalent to dropping the server.
    pub fn shutdown(self) {}
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.shared.closing.send_replace(Closing::Shutdown);
        *self.shared.lock_token() = None;
        let _ = fs::remove_file(&self.shared.info.socket);
        let _ = fs::remove_file(&self.shared.info.token_file);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Closing {
    Open,
    Revoked,
    Shutdown,
}

impl Closing {
    fn reason(self) -> &'static str {
        match self {
            Closing::Open => "open",
            Closing::Revoked => "revoked",
            Closing::Shutdown => "shutdown",
        }
    }
}

struct Shared {
    info: AttachInfo,
    uid: u32,
    registry: ControlRegistry,
    token: Mutex<Option<String>>,
    closing: watch::Sender<Closing>,
    permits: Arc<Semaphore>,
}

impl Shared {
    fn lock_token(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Issue a new token. With `reopen` false, a revoked server is left
    /// revoked. The file and the in-memory token change under one lock so a
    /// scheduled rotation cannot race a revocation back open.
    fn rotate(&self, reopen: bool) -> io::Result<()> {
        let mut current = self.lock_token();
        if current.is_none() && !reopen {
            return Ok(());
        }
        let token = new_token();
        write_private_file(&self.info.token_file, token.as_bytes())?;
        *current = Some(token);
        Ok(())
    }

    fn revoke(&self) {
        {
            let mut current = self.lock_token();
            *current = None;
            let _ = fs::remove_file(&self.info.token_file);
        }
        self.closing.send_replace(Closing::Revoked);
    }

    fn authorized(&self, given: &str) -> bool {
        self.lock_token()
            .as_deref()
            .is_some_and(|expected| constant_time_eq(expected.as_bytes(), given.as_bytes()))
    }
}

async fn accept_loop(shared: Arc<Shared>, listener: UnixListener) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!(%error, "control socket accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        // THREAT[TM-DOS-056]: bound concurrent control connections; attach
        // streams hold a permit for as long as they stay open.
        let Ok(permit) = shared.permits.clone().try_acquire_owned() else {
            tokio::spawn(async move {
                let mut stream = stream;
                let _ =
                    write_line(&mut stream, &error("busy", "too many control connections")).await;
            });
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            serve_connection(&shared, stream).await;
            drop(permit);
        });
    }
}

async fn serve_connection(shared: &Shared, stream: UnixStream) {
    // THREAT[TM-AUTH-040]: only the same OS user may use the control socket,
    // checked from kernel peer credentials before any byte is read.
    let same_user = stream
        .peer_cred()
        .map(|cred| cred.uid() == shared.uid)
        .unwrap_or(false);
    let (read, mut write) = stream.into_split();
    if !same_user {
        let _ = write_line(
            &mut write,
            &error("unauthorized", "peer is a different user"),
        )
        .await;
        return;
    }
    let mut reader = BufReader::new(read.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    let read = tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line)).await;
    if !matches!(read, Ok(Ok(n)) if n > 0 && line.ends_with('\n')) {
        let _ = write_line(
            &mut write,
            &error("bad_request", "expected one request line"),
        )
        .await;
        return;
    }
    let request: ControlRequest = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(parse) => {
            let _ = write_line(&mut write, &error("bad_request", &parse.to_string())).await;
            return;
        }
    };
    // THREAT[TM-AUTH-040]: the bearer token gates every connection; compared
    // in constant time, absent after revocation.
    if !shared.authorized(&request.token) {
        let _ = write_line(
            &mut write,
            &error("unauthorized", "invalid or expired token"),
        )
        .await;
        return;
    }
    let mut closing = shared.closing.subscribe();
    if *closing.borrow_and_update() != Closing::Open {
        let _ = write_line(
            &mut write,
            &error("unauthorized", "control access is closed"),
        )
        .await;
        return;
    }
    let _ = dispatch(shared, request.command, reader, &mut write, closing).await;
}

async fn dispatch<R, W>(
    shared: &Shared,
    command: ControlCommand,
    mut reader: R,
    write: &mut W,
    mut closing: watch::Receiver<Closing>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let registry = &shared.registry;
    match command {
        ControlCommand::List => {
            let sessions = registry.list();
            write_line(write, &ControlResponse::Sessions { sessions }).await
        }
        ControlCommand::Send {
            session,
            text,
            wait,
        } => {
            let sent = match registry.send(&session, text).await {
                Ok(sent) => sent,
                Err(failure) => return write_line(write, &control_error(&failure)).await,
            };
            let receipt = ControlSent::new(
                sent.message_id.clone(),
                sent.turn_id.clone(),
                sent.disposition,
            );
            write_line(write, &ControlResponse::Sent(receipt)).await?;
            if !wait {
                return Ok(());
            }
            // Disconnecting from a waiting send never cancels the turn.
            tokio::select! {
                result = sent.wait() => match result {
                    Ok(turn) => write_line(write, &ControlResponse::Turn(turn.into())).await,
                    Err(failure) => write_line(write, &error("failed", &failure.to_string())).await,
                },
                reason = closed(&mut closing) => write_line(write, &ControlResponse::Closed {
                    reason: reason.to_owned(),
                }).await,
            }
        }
        ControlCommand::Cancel { session } => match registry.cancel(&session).await {
            Ok(turn) => write_line(write, &ControlResponse::Cancelled { turn }).await,
            Err(failure) => write_line(write, &control_error(&failure)).await,
        },
        ControlCommand::Attach { session, after } => {
            let session = match registry.session(&session) {
                Ok(session) => session,
                Err(failure) => return write_line(write, &control_error(&failure)).await,
            };
            attach(session, after, &mut reader, write, closing).await
        }
    }
}

async fn attach<R, W>(
    session: Session,
    after: Option<i32>,
    reader: &mut R,
    write: &mut W,
    mut closing: watch::Receiver<Closing>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut events = match after {
        None => session.events(),
        Some(after) => match session.events_from(after).await {
            Ok(events) => events,
            Err(failure) => return write_line(write, &error("failed", &failure.to_string())).await,
        },
    };
    write_line(
        write,
        &ControlResponse::Attached {
            session: session.id(),
        },
    )
    .await?;
    let mut projector = Projector::default();
    let mut scratch = [0u8; 64];
    loop {
        tokio::select! {
            reason = closed(&mut closing) => {
                return write_line(write, &ControlResponse::Closed { reason: reason.to_owned() }).await;
            }
            // The client sends nothing after its request; EOF means it left.
            read = reader.read(&mut scratch) => {
                if matches!(read, Ok(0) | Err(_)) {
                    return Ok(());
                }
            }
            next = events.recv() => match next {
                Ok(Some(event)) => {
                    if let Some(event) = projector.project(&event) {
                        write_line(write, &ControlResponse::Event(event)).await?;
                    }
                }
                Ok(None) => {
                    return write_line(write, &ControlResponse::Closed {
                        reason: "session_closed".to_owned(),
                    }).await;
                }
                Err(EventStreamError::Lagged { missed }) => {
                    let event = super::wire::ControlEvent {
                        sequence: None,
                        event_type: "control.lagged".to_owned(),
                        kind: super::wire::ControlEventKind::Lagged { missed },
                    };
                    write_line(write, &ControlResponse::Event(event)).await?;
                }
                Err(failure) => {
                    return write_line(write, &error("failed", &format!("{failure:?}"))).await;
                }
            }
        }
    }
}

/// Resolve once the server is revoked or shut down.
async fn closed(closing: &mut watch::Receiver<Closing>) -> &'static str {
    loop {
        let state = *closing.borrow_and_update();
        if state != Closing::Open {
            return state.reason();
        }
        if closing.changed().await.is_err() {
            return Closing::Shutdown.reason();
        }
    }
}

fn error(code: &str, message: &str) -> ControlResponse {
    ControlResponse::Error {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn control_error(failure: &ControlError) -> ControlResponse {
    let code = match failure {
        ControlError::NotFound(_) => "not_found",
        ControlError::Ambiguous(_) => "ambiguous",
        ControlError::NoActiveTurn => "no_active_turn",
        ControlError::SessionClosed => "session_closed",
        _ => "failed",
    };
    error(code, &failure.to_string())
}

pub(super) async fn write_line<W: AsyncWrite + Unpin>(
    write: &mut W,
    response: &impl serde::Serialize,
) -> io::Result<()> {
    let mut line = serde_json::to_vec(response).map_err(io::Error::other)?;
    line.push(b'\n');
    tokio::time::timeout(WRITE_TIMEOUT, async {
        write.write_all(&line).await?;
        write.flush().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "control client stopped reading"))?
}

pub(super) fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Create `dir` as 0700, or accept it only when it already is a directory
/// owned by `uid` with no group or other permissions.
fn prepare_private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(_) => check_private_dir(dir, uid),
        Err(missing) if missing.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            fs::DirBuilder::new().mode(0o700).create(dir)?;
            check_private_dir(dir, uid)
        }
        Err(other) => Err(other),
    }
}

/// THREAT[TM-AUTH-040]: refuse a control directory another user could have
/// planted or can read; both ends check it before trusting what is inside.
pub(super) fn check_private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(insecure(dir, "is not a directory"));
    }
    if meta.uid() != uid {
        return Err(insecure(dir, "is owned by another user"));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(insecure(
            dir,
            "is accessible to group or others; use mode 0700",
        ));
    }
    Ok(())
}

pub(super) fn check_private_file(path: &Path, uid: u32) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(insecure(
            path,
            "must be a regular file owned by this user, mode 0600",
        ));
    }
    Ok(())
}

fn insecure(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("control path {} {why}", path.display()),
    )
}

fn remove_stale_socket(socket: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(socket) {
        Ok(meta) => meta,
        Err(missing) if missing.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(other) => return Err(other),
    };
    if !meta.file_type().is_socket() {
        return Err(insecure(socket, "exists and is not a socket"));
    }
    if std::os::unix::net::UnixStream::connect(socket).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "a control server is already listening on {}",
                socket.display()
            ),
        ));
    }
    fs::remove_file(socket)
}

/// Write via a temporary file and rename, so a reader never sees a partial
/// token and the file is never briefly readable by others.
fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

fn new_token() -> String {
    use rand::Rng as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}
