//! AG-UI threads: which session answers each `threadId`.

// Decision: the store maps a thread key to a session id and nothing else. The
// session's conversation lives in the engine's backend, so durability across
// restarts needs both a durable store and a backend that keeps sessions
// (`LocalConfig`). A store entry whose session the backend no longer has (an
// in-memory engine after a restart) is replaced by a new session, seeded from
// the client's copy of the conversation.
//
// Decision: the threads hold every session they resolve. The engine keeps
// only a weak handle to a live session, and between runs nobody else holds
// one: the host answered the last request and dropped its stream. A turn
// parked across that gap, on a frontend tool call or an interrupt, lives in
// the session, so dropping it lost the turn and the next run answered
// nothing. A handle is small next to the conversation the backend already
// keeps for every thread.
//
// Decision: resolution is serialized by one lock per `AgUiThreads`, so two
// first runs of a thread cannot create two sessions. Resolving is a store
// read plus, at most, an attach; the run itself happens outside the lock.

#[cfg(feature = "local")]
use everruns_durable::sqlite as rusqlite;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::{AgUiError, AgUiOptions, AgUiStream, RunAgentInput, invalid, lock};
use crate::{Agent, Engine, ResumeError, Session, SessionId};

/// Longest scope [`AgUiThreads::run_in`] accepts, in bytes.
const MAX_SCOPE_BYTES: usize = 256;

/// Why a [`ThreadStore`] could not read or record a thread.
///
/// ```
/// use everruns::ag_ui::ThreadStoreError;
///
/// let error = ThreadStoreError::new("database is locked");
/// assert_eq!(error.to_string(), "AG-UI thread store: database is locked");
/// ```
#[derive(Debug, Clone)]
pub struct ThreadStoreError(String);

impl ThreadStoreError {
    /// An error carrying `message`.
    ///
    /// ```
    /// let error = everruns::ag_ui::ThreadStoreError::new("offline");
    /// # let _ = error;
    /// ```
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

impl std::fmt::Display for ThreadStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AG-UI thread store: {}", self.0)
    }
}

impl std::error::Error for ThreadStoreError {}

/// Why [`AgUiThreads`] could not resolve a thread to a session.
///
/// ```
/// use everruns::ag_ui::{AgUiError, ThreadError, ThreadStoreError};
///
/// let error = AgUiError::Thread(ThreadError::Store(ThreadStoreError::new("offline")));
/// assert!(error.to_string().contains("offline"));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum ThreadError {
    /// The store failed.
    Store(ThreadStoreError),
    /// The thread's session exists but could not be reopened.
    Resume(ResumeError),
}

impl std::fmt::Display for ThreadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Resume(error) => write!(f, "cannot reopen the thread's session: {error}"),
        }
    }
}

impl std::error::Error for ThreadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Resume(error) => Some(error),
        }
    }
}

impl From<ThreadStoreError> for AgUiError {
    fn from(error: ThreadStoreError) -> Self {
        Self::Thread(ThreadError::Store(error))
    }
}

/// Where [`AgUiThreads`] records which session answers each thread.
///
/// Keys are opaque strings built by [`AgUiThreads`] from the host's scope and
/// the client's `threadId`. [`InMemoryThreadStore`] forgets them with the
/// process; with the `local` feature, [`SqliteThreadStore`] keeps them in a
/// file. Implement it over the host's own database to keep threads there.
///
/// ```
/// use std::collections::HashMap;
/// use std::sync::Mutex;
///
/// use everruns::SessionId;
/// use everruns::ag_ui::{ThreadStore, ThreadStoreError};
///
/// #[derive(Default)]
/// struct MyStore(Mutex<HashMap<String, SessionId>>);
///
/// #[async_trait::async_trait]
/// impl ThreadStore for MyStore {
///     async fn get(&self, key: &str) -> Result<Option<SessionId>, ThreadStoreError> {
///         Ok(self.0.lock().unwrap().get(key).copied())
///     }
///     async fn bind(&self, key: &str, session_id: SessionId) -> Result<(), ThreadStoreError> {
///         self.0.lock().unwrap().insert(key.to_string(), session_id);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait ThreadStore: Send + Sync + 'static {
    /// The session bound to `key`, if any.
    async fn get(&self, key: &str) -> Result<Option<SessionId>, ThreadStoreError>;

    /// Bind `key` to `session_id`, replacing any earlier binding.
    async fn bind(&self, key: &str, session_id: SessionId) -> Result<(), ThreadStoreError>;
}

/// A shared store: hosts that keep one store beside several
/// [`AgUiThreads`] pass an `Arc` of it.
#[async_trait]
impl<T: ThreadStore + ?Sized> ThreadStore for Arc<T> {
    async fn get(&self, key: &str) -> Result<Option<SessionId>, ThreadStoreError> {
        (**self).get(key).await
    }

    async fn bind(&self, key: &str, session_id: SessionId) -> Result<(), ThreadStoreError> {
        (**self).bind(key, session_id).await
    }
}

/// A [`ThreadStore`] in process memory: threads are forgotten on exit.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::SessionId;
/// use everruns::ag_ui::{InMemoryThreadStore, ThreadStore};
///
/// let store = InMemoryThreadStore::new();
/// let session_id = SessionId::new();
/// store.bind("thread-1", session_id).await?;
/// assert_eq!(store.get("thread-1").await?, Some(session_id));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct InMemoryThreadStore {
    threads: Mutex<HashMap<String, SessionId>>,
}

impl InMemoryThreadStore {
    /// An empty store.
    ///
    /// ```
    /// let store = everruns::ag_ui::InMemoryThreadStore::new();
    /// # let _ = store;
    /// ```
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ThreadStore for InMemoryThreadStore {
    async fn get(&self, key: &str) -> Result<Option<SessionId>, ThreadStoreError> {
        Ok(lock(&self.threads).get(key).copied())
    }

    async fn bind(&self, key: &str, session_id: SessionId) -> Result<(), ThreadStoreError> {
        lock(&self.threads).insert(key.to_string(), session_id);
        Ok(())
    }
}

/// A [`ThreadStore`] in a SQLite file, so threads survive a restart.
///
/// Pair it with an agent on the [`LocalConfig`](crate::LocalConfig) backend,
/// which keeps the sessions themselves. Requires the `local` feature.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::{LocalConfig, SessionId};
/// use everruns::ag_ui::{SqliteThreadStore, ThreadStore};
///
/// let dir = tempfile::tempdir()?;
/// let config = LocalConfig::new(dir.path());
/// let session_id = SessionId::new();
/// SqliteThreadStore::local(&config)?.bind("thread-1", session_id).await?;
/// // A later process reads the same file.
/// let reopened = SqliteThreadStore::local(&config)?;
/// assert_eq!(reopened.get("thread-1").await?, Some(session_id));
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "local")]
#[derive(Clone)]
pub struct SqliteThreadStore {
    db: crate::local::SqliteDb,
}

#[cfg(feature = "local")]
impl std::fmt::Debug for SqliteThreadStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteThreadStore").finish_non_exhaustive()
    }
}

#[cfg(feature = "local")]
impl SqliteThreadStore {
    /// File name [`local`](Self::local) uses under the config's data
    /// directory.
    pub const FILE_NAME: &'static str = "ag-ui-threads.db";

    /// Open (creating if needed) the store at `path`. The file is private to
    /// the current user, like the other local databases.
    ///
    /// ```
    /// let dir = tempfile::tempdir()?;
    /// let store = everruns::ag_ui::SqliteThreadStore::open(dir.path().join("threads.db"))?;
    /// # let _ = store;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// When the file cannot be opened or its table created.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, ThreadStoreError> {
        let db = crate::local::SqliteDb::open(path).map_err(ThreadStoreError::new)?;
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS ag_ui_threads (
                     thread_key TEXT PRIMARY KEY NOT NULL,
                     session_id TEXT NOT NULL
                 )",
            )
        })
        .map_err(ThreadStoreError::new)?;
        Ok(Self { db })
    }

    /// Open the store in `config`'s data directory, as
    /// [`FILE_NAME`](Self::FILE_NAME), creating the directory if needed.
    ///
    /// ```
    /// let dir = tempfile::tempdir()?;
    /// let config = everruns::LocalConfig::new(dir.path().join("state"));
    /// let store = everruns::ag_ui::SqliteThreadStore::local(&config)?;
    /// # let _ = store;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// When the directory or file cannot be created or opened.
    pub fn local(config: &crate::LocalConfig) -> Result<Self, ThreadStoreError> {
        std::fs::create_dir_all(config.data_dir()).map_err(ThreadStoreError::new)?;
        Self::open(config.data_dir().join(Self::FILE_NAME))
    }
}

#[cfg(feature = "local")]
#[async_trait]
impl ThreadStore for SqliteThreadStore {
    async fn get(&self, key: &str) -> Result<Option<SessionId>, ThreadStoreError> {
        use rusqlite::OptionalExtension;

        let stored: Option<String> = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT session_id FROM ag_ui_threads WHERE thread_key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()
            })
            .map_err(ThreadStoreError::new)?;
        stored
            .map(|id| id.parse().map_err(ThreadStoreError::new))
            .transpose()
    }

    async fn bind(&self, key: &str, session_id: SessionId) -> Result<(), ThreadStoreError> {
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO ag_ui_threads (thread_key, session_id) VALUES (?1, ?2)",
                    [key, session_id.to_string().as_str()],
                )
            })
            .map_err(ThreadStoreError::new)?;
        Ok(())
    }
}

/// The session a thread resolved to.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::ag_ui::AgUiThreads;
/// use everruns::{Agent, Engine, Model};
///
/// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
/// let threads = AgUiThreads::new(Engine::new(), agent);
/// let first = threads.session("thread-1").await?;
/// assert!(first.created());
/// let again = threads.session("thread-1").await?;
/// assert!(!again.created());
/// assert_eq!(first.session().session_id(), again.session().session_id());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ThreadSession {
    session: Session,
    created: bool,
}

impl std::fmt::Debug for ThreadSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadSession")
            .field("session_id", &self.session.session_id())
            .field("created", &self.created)
            .finish()
    }
}

impl ThreadSession {
    /// The thread's session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Whether this resolution created the session: the thread was new, or
    /// its session was gone from the backend.
    pub fn created(&self) -> bool {
        self.created
    }

    /// The session, by value.
    pub fn into_session(self) -> Session {
        self.session
    }
}

/// Maps AG-UI `threadId`s to sessions, so one host serves many threads.
///
/// The first run of a thread creates a session from the agent and records it
/// in the [`ThreadStore`]; later runs reopen it, through
/// [`Engine::attach`] after a restart. A thread whose session is gone (an
/// in-memory engine after a restart, or a store shared with another host)
/// gets a new session, and the run's earlier messages are seeded into it as
/// prior history ([`AgUiOptions::seed_history`]), so a client that holds the
/// conversation does not lose it.
///
/// Thread ids are client input: anyone who knows one continues that
/// conversation. Scope them to the caller with [`run_in`](Self::run_in)
/// when one host serves several users.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::ag_ui::{AgUiOptions, AgUiThreads, Event, Message, RunAgentInput};
/// use everruns::{Agent, Engine, Model};
/// use futures::StreamExt;
///
/// let agent = Agent::builder()
///     .instructions("Be brief.")
///     .model(Model::simulated("Hello."))
///     .build()?;
/// let threads = AgUiThreads::new(Engine::new(), agent);
/// let input = RunAgentInput {
///     thread_id: "thread-1".into(),
///     run_id: "run-1".into(),
///     messages: vec![Message::user("m1", "Hi")],
///     ..RunAgentInput::default()
/// };
/// let events: Vec<Event> = threads
///     .run(input, AgUiOptions::new())
///     .await?
///     .collect()
///     .await;
/// assert!(matches!(events.last(), Some(Event::RunFinished(_))));
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AgUiThreads {
    inner: Arc<ThreadsInner>,
}

struct ThreadsInner {
    engine: Engine,
    agent: Agent,
    store: Arc<dyn ThreadStore>,
    resolving: tokio::sync::Mutex<()>,
    /// Every session resolved so far, so a parked turn outlives its run.
    live: Mutex<HashMap<SessionId, Session>>,
}

impl std::fmt::Debug for AgUiThreads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgUiThreads").finish_non_exhaustive()
    }
}

impl AgUiThreads {
    /// Threads of `agent`'s sessions on `engine`, recorded in an
    /// [`InMemoryThreadStore`].
    ///
    /// ```
    /// use everruns::ag_ui::AgUiThreads;
    /// use everruns::{Agent, Engine, Model};
    ///
    /// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
    /// let threads = AgUiThreads::new(Engine::new(), agent);
    /// # let _ = threads;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(engine: Engine, agent: Agent) -> Self {
        Self::with_store(engine, agent, InMemoryThreadStore::new())
    }

    /// Threads recorded in `store` instead.
    ///
    /// ```
    /// use everruns::ag_ui::{AgUiThreads, InMemoryThreadStore};
    /// use everruns::{Agent, Engine, Model};
    ///
    /// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
    /// let threads = AgUiThreads::with_store(Engine::new(), agent, InMemoryThreadStore::new());
    /// # let _ = threads;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_store(engine: Engine, agent: Agent, store: impl ThreadStore) -> Self {
        Self {
            inner: Arc::new(ThreadsInner {
                engine,
                agent,
                store: Arc::new(store),
                resolving: tokio::sync::Mutex::new(()),
                live: Mutex::default(),
            }),
        }
    }

    /// The session behind `thread_id`, created on first sight.
    ///
    /// # Errors
    ///
    /// [`AgUiError::InvalidInput`] when the id is not 1 to 128 characters of
    /// `[A-Za-z0-9-_.]`; [`AgUiError::Thread`] when the store fails or the
    /// session exists but cannot be reopened.
    pub async fn session(&self, thread_id: &str) -> Result<ThreadSession, AgUiError> {
        self.resolve(&thread_key(None, thread_id)?).await
    }

    /// The session behind `thread_id` within `scope`, created on first
    /// sight. The same id in two scopes names two threads.
    ///
    /// # Errors
    ///
    /// As [`session`](Self::session), and [`AgUiError::InvalidInput`] for an
    /// empty scope or one over 256 bytes.
    pub async fn session_in(
        &self,
        scope: &str,
        thread_id: &str,
    ) -> Result<ThreadSession, AgUiError> {
        self.resolve(&thread_key(Some(scope), thread_id)?).await
    }

    /// Answer one AG-UI request on the session behind its `threadId`.
    ///
    /// A thread's first run seeds the input's earlier messages into the new
    /// session ([`AgUiOptions::seed_history`]); otherwise this is
    /// [`Session::ag_ui_with`] on the thread's session.
    ///
    /// # Errors
    ///
    /// As [`session`](Self::session) and [`Session::ag_ui_with`].
    pub async fn run(
        &self,
        input: RunAgentInput,
        options: AgUiOptions,
    ) -> Result<AgUiStream, AgUiError> {
        let key = thread_key(None, &input.thread_id)?;
        self.run_key(&key, input, options).await
    }

    /// [`run`](Self::run) with the thread scoped to `scope`, such as the
    /// authenticated caller's id, so one caller cannot continue another's
    /// thread by guessing its id.
    ///
    /// # Errors
    ///
    /// As [`session_in`](Self::session_in) and [`Session::ag_ui_with`].
    pub async fn run_in(
        &self,
        scope: &str,
        input: RunAgentInput,
        options: AgUiOptions,
    ) -> Result<AgUiStream, AgUiError> {
        let key = thread_key(Some(scope), &input.thread_id)?;
        self.run_key(&key, input, options).await
    }

    async fn run_key(
        &self,
        key: &str,
        input: RunAgentInput,
        options: AgUiOptions,
    ) -> Result<AgUiStream, AgUiError> {
        let thread = self.resolve(key).await?;
        let options = if thread.created {
            options.seed_history(true)
        } else {
            options
        };
        thread.session.ag_ui_with(input, options).await
    }

    async fn resolve(&self, key: &str) -> Result<ThreadSession, AgUiError> {
        let inner = &self.inner;
        let _resolving = inner.resolving.lock().await;
        if let Some(session_id) = inner.store.get(key).await? {
            if let Some(session) = lock(&inner.live).get(&session_id).cloned() {
                return Ok(ThreadSession {
                    session,
                    created: false,
                });
            }
            match self.reopen(session_id).await {
                Ok(session) => {
                    lock(&inner.live).insert(session_id, session.clone());
                    return Ok(ThreadSession {
                        session,
                        created: false,
                    });
                }
                Err(ResumeError::SessionNotFound { .. }) => {
                    tracing::info!(
                        session_id = %session_id,
                        "AG-UI thread's session is gone; starting a new one"
                    );
                }
                Err(error) => return Err(AgUiError::Thread(ThreadError::Resume(error))),
            }
        }
        let session = inner.engine.create(inner.agent.clone());
        inner.store.bind(key, session.session_id()).await?;
        lock(&inner.live).insert(session.session_id(), session.clone());
        Ok(ThreadSession {
            session,
            created: true,
        })
    }

    /// The engine's session, attaching it from the backend after a restart.
    async fn reopen(&self, session_id: SessionId) -> Result<Session, ResumeError> {
        let engine = &self.inner.engine;
        match engine.resume(session_id).await {
            Err(ResumeError::SessionNotFound { .. }) => {
                engine.attach(session_id, self.inner.agent.clone()).await?;
                engine.resume(session_id).await
            }
            other => other,
        }
    }
}

/// The store key of `thread_id` in `scope`. Thread ids cannot contain `/`,
/// so the key is unambiguous whatever the scope holds.
// THREAT[TM-TENANT-017]: thread ids are client-chosen; they are bounded, and a
// host that serves several callers scopes them so one caller's id never
// reaches another caller's session.
fn thread_key(scope: Option<&str>, thread_id: &str) -> Result<String, AgUiError> {
    let valid = !thread_id.is_empty()
        && thread_id.len() <= 128
        && thread_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !valid {
        return Err(invalid(
            "threadId must be 1 to 128 characters of [A-Za-z0-9-_.]",
        ));
    }
    match scope {
        None => Ok(thread_id.to_string()),
        Some(scope) if scope.is_empty() || scope.len() > MAX_SCOPE_BYTES => Err(invalid(format!(
            "a thread scope must be 1 to {MAX_SCOPE_BYTES} bytes"
        ))),
        Some(scope) => Ok(format!("{scope}/{thread_id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_keys_are_bounded_and_scoped() {
        assert_eq!(thread_key(None, "t-1.a_b").unwrap(), "t-1.a_b");
        assert_eq!(thread_key(Some("alice"), "t").unwrap(), "alice/t");
        assert!(thread_key(None, "").is_err());
        assert!(thread_key(None, "a/b").is_err());
        assert!(thread_key(None, &"x".repeat(129)).is_err());
        assert!(thread_key(Some(""), "t").is_err());
        assert!(thread_key(Some(&"s".repeat(257)), "t").is_err());
        // A `/` in the scope cannot collide with another scope's thread.
        assert_ne!(
            thread_key(Some("a/b"), "c").unwrap(),
            thread_key(Some("a"), "b").unwrap()
        );
    }
}
