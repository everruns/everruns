//! Session-owned SQLite databases. The parent keeps only its child registry.
use super::*;

mod source;

use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskUpdate, TaskMessage,
};
use std::path::PathBuf;

/// A directory is routing information, never a shared SQLite transaction domain.
/// Cached entries have no directory reference, so the cache cannot retain itself.
pub(super) struct SessionDirectory {
    root: PathBuf,
    sources: Vec<PathBuf>,
    owner: SessionId,
    ownership: Arc<dyn Fn(SessionId) -> Result<Box<dyn Send + Sync>> + Send + Sync>,
    databases: Mutex<HashMap<SessionId, SqliteHosting>>,
}

impl SqliteHosting {
    /// Open one database per session under `directory/<session>/engine.sqlite`.
    /// Children, ancestor reads and task reports route to the owning session's
    /// database. The root's supervisor follows its durable child registry only.
    /// Keep the directory and the host's secret encryption keys together.
    ///
    /// Requires the `local` feature. This mode hosts code-registered builds;
    /// shared agent definitions require a separate host control plane.
    ///
    /// ```no_run
    /// use everruns::{SessionId, hosting::SqliteHosting};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let _hosting = SqliteHosting::open_session("sessions", SessionId::new())?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open_session(directory: impl AsRef<Path>, session: SessionId) -> Result<Self> {
        Self::open_session_from(directory, session, None::<&Path>, |_| Ok(Box::new(())))
    }

    /// As [`Self::open_session`], importing each session from a former shared
    /// database or session directory when its own database does not exist yet. Import includes the
    /// canonical journal, control state, receipts, tasks, schedules and sealed
    /// secrets. It never reconstructs authority from a JSONL projection.
    ///
    /// The caller must exclusively own each source session before opening it,
    /// and must not run that session on an older host after transferring it.
    /// `ownership` acquires and retains the host's exclusive source-session
    /// leases for the complete source family before copying any database. This
    /// includes ancestors and siblings when a worker is opened first. The leases
    /// cover every source producer, including cross-session reports and inputs.
    /// Other families may keep using the source. The source is left untouched
    /// for backup; a live execution epoch refuses transfer.
    pub fn open_session_from(
        directory: impl AsRef<Path>,
        session: SessionId,
        shared_source: Option<impl AsRef<Path>>,
        ownership: impl Fn(SessionId) -> Result<Box<dyn Send + Sync>> + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::open_session_from_sources(directory, session, shared_source, ownership)
    }

    /// Open a session directory, transferring state from earlier directories
    /// or shared database files in priority order. Existing destinations win.
    /// Directory sources preserve complete session databases, including WAL
    /// state and child registrations. The ownership contract is the same as
    /// [`Self::open_session_from`], covering the complete source family.
    pub fn open_session_from_sources(
        directory: impl AsRef<Path>,
        session: SessionId,
        sources: impl IntoIterator<Item = impl AsRef<Path>>,
        ownership: impl Fn(SessionId) -> Result<Box<dyn Send + Sync>> + Send + Sync + 'static,
    ) -> Result<Self> {
        let directory = Arc::new(SessionDirectory {
            root: directory.as_ref().to_path_buf(),
            sources: sources
                .into_iter()
                .map(|source| source.as_ref().to_path_buf())
                .collect(),
            owner: session,
            ownership: Arc::new(ownership),
            databases: Mutex::new(HashMap::new()),
        });
        let mut hosting = directory.open(session)?;
        hosting.directory = Some(directory);
        // Refuse startup if a retained child cannot be transferred safely,
        // rather than returning a host whose recovery supervisor would fail.
        hosting.registered_sessions()?;
        Ok(hosting)
    }

    /// The database and its WAL belong to exactly this session.
    pub fn session_db_path(directory: impl AsRef<Path>, session: SessionId) -> PathBuf {
        directory
            .as_ref()
            .join(session.to_string())
            .join("engine.sqlite")
    }

    pub(super) fn for_session(&self, session: SessionId) -> Result<Self> {
        match &self.directory {
            None => Ok(self.clone()),
            Some(directory) => {
                let mut hosting = directory.open(session)?;
                hosting.directory = Some(directory.clone());
                hosting.secrets = self.secrets.clone();
                hosting.changes = self.changes.clone();
                Ok(hosting)
            }
        }
    }

    pub(super) fn register_child(
        &self,
        child: SessionId,
        command: &super::spi::session_commands::SessionCommand,
        require_parent: bool,
    ) -> Result<()> {
        if self.directory.is_none() {
            return Ok(());
        }
        let super::spi::session_commands::SessionCommand::Create { binding } = command else {
            return Ok(());
        };
        let Some(parent) = binding
            .pointer("/metadata/parent_session_id")
            .and_then(Value::as_str)
        else {
            return Ok(());
        };
        let parent: SessionId = parent.parse().map_err(error)?;
        if parent == child || self.is_ancestor(parent, child)? {
            return Err(error("session parent link contains a cycle"));
        }
        // Record the recovery obligation before creating the child's authority.
        // A crash can leave an empty registration, never an undiscoverable child.
        self.transaction(parent, |tx, state| {
            // Legacy cutover imports children before their parent. Retain the
            // registration in the parent's database before that binding exists.
            if require_parent {
                state.binding(parent)?;
            }
            tx.execute(
                "INSERT OR IGNORE INTO framework_session_children VALUES (?)",
                [child.to_string()],
            )
            .map_err(error)?;
            Ok(())
        })
    }

    pub(super) fn registered_sessions(&self) -> Result<Vec<Self>> {
        let Some(directory) = &self.directory else {
            return Ok(vec![self.clone()]);
        };
        let mut pending = vec![directory.owner];
        let mut seen = HashSet::new();
        let mut sessions = Vec::new();
        while let Some(session) = pending.pop() {
            if !seen.insert(session) {
                continue;
            }
            if seen.len() > 65_536 {
                return Err(error("session child registry exceeds recovery limit"));
            }
            let hosting = self.for_session(session)?;
            let children = {
                let db = hosting.db.lock().map_err(error)?;
                let mut query = db
                    .prepare("SELECT session FROM framework_session_children")
                    .map_err(error)?;
                query
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(error)?
                    .map(|row| row.map_err(error)?.parse().map_err(error))
                    .collect::<Result<Vec<SessionId>>>()?
            };
            pending.extend(children);
            sessions.push(hosting);
        }
        Ok(sessions)
    }

    pub(super) async fn supervise_directory(
        &self,
        engine: crate::Engine,
        shutdown: crate::CancellationToken,
    ) -> Result<()> {
        let mut started = HashSet::new();
        let mut running = tokio::task::JoinSet::new();
        loop {
            for hosting in self.registered_sessions()? {
                // A local connection is unique per cached database, including
                // databases opened by this process after supervision started.
                let identity = Arc::as_ptr(&hosting.db) as usize;
                if started.insert(identity) {
                    let engine = engine.clone();
                    let shutdown = shutdown.clone();
                    running.spawn(async move { hosting.supervise_local(engine, shutdown).await });
                }
            }
            tokio::select! {
                () = shutdown.cancelled() => {
                    while let Some(result) = running.join_next().await { result.map_err(error)??; }
                    return Ok(());
                }
                result = running.join_next(), if !running.is_empty() => {
                    result.ok_or_else(|| error("session supervisor disappeared"))?.map_err(error)??;
                }
                () = everruns_contracts::rt::sleep(Duration::from_millis(100)) => {}
            }
        }
    }
}

impl SessionDirectory {
    fn open(&self, session: SessionId) -> Result<SqliteHosting> {
        let mut databases = self.databases.lock().map_err(error)?;
        if let Some(hosting) = databases.get(&session) {
            return Ok(hosting.clone());
        }
        if !SqliteHosting::session_db_path(&self.root, session).exists()
            && self.sources.iter().any(|source| source.exists())
        {
            let family = self.source_family(session)?;
            for member in &family.sessions {
                if let std::collections::hash_map::Entry::Vacant(entry) = databases.entry(*member) {
                    entry.insert(self.open_database(*member, true)?);
                }
            }
            // Drop source leases only after the complete family is published.
            drop(family);
            return databases
                .get(&session)
                .cloned()
                .ok_or_else(|| error("transferred session is missing"));
        }
        let hosting = self.open_database(session, false)?;
        databases.insert(session, hosting.clone());
        Ok(hosting)
    }

    fn open_database(&self, session: SessionId, source_leased: bool) -> Result<SqliteHosting> {
        let path = SqliteHosting::session_db_path(&self.root, session);
        let parent = path.parent().expect("session database parent");
        private_directory(&self.root)?;
        private_directory(parent)?;
        let lock_path = parent.join("open.lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let lock = options.open(&lock_path).map_err(error)?;
        if !lock.metadata().map_err(error)?.is_file() {
            return Err(error("invalid session database lock"));
        }
        lock.lock().map_err(error)?;
        if !path.exists() {
            let _ownership = if !source_leased && self.sources.iter().any(|source| source.exists())
            {
                Some((self.ownership)(session)?)
            } else {
                None
            };
            if let Some(source) = self.source_for(session)? {
                source.transfer(&path, session)?;
            }
        }
        // THREAT[TM-FS-014]: precreate privately before SQLite opens the file
        // or creates its WAL; the owner-only directory also contains transfers.
        crate::local::SqliteDb::open(&path).map_err(error)?;
        let hosting = SqliteHosting::open(&path)?;
        hosting
            .db
            .lock()
            .map_err(error)?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS framework_session_children(session TEXT PRIMARY KEY);",
            )
            .map_err(error)?;
        Ok(hosting)
    }
}

fn private_directory(path: &Path) -> Result<()> {
    if path.exists() {
        if !std::fs::symlink_metadata(path).map_err(error)?.is_dir() {
            return Err(error("session database directory must not be a symlink"));
        }
    } else {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path).map_err(error)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = std::fs::symlink_metadata(path).map_err(error)?;
        if metadata.uid() != unsafe { libc::getuid() } {
            return Err(error(
                "session database directory must be owned by the current user",
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(error)?;
        }
    }
    Ok(())
}

pub(super) struct RoutedTasks(pub(super) SqliteHosting, pub(super) bool);
impl RoutedTasks {
    fn registry(&self, session: SessionId) -> Result<Arc<dyn SessionTaskRegistry>> {
        let services = self.0.for_session(session)?.services;
        if self.1 {
            Ok(services.recovery.clone())
        } else {
            Ok(services.registry.clone())
        }
    }
}

#[async_trait]
impl SessionTaskRegistry for RoutedTasks {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        self.registry(input.session_id)?.create(input).await
    }
    async fn update(
        &self,
        session: SessionId,
        task: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        self.registry(session)?.update(session, task, update).await
    }
    async fn get(&self, session: SessionId, task: &str) -> Result<Option<SessionTask>> {
        self.registry(session)?.get(session, task).await
    }
    async fn list(
        &self,
        session: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        self.registry(session)?.list(session, filter).await
    }
    async fn request_cancel(&self, session: SessionId, task: &str) -> Result<Option<SessionTask>> {
        self.registry(session)?.request_cancel(session, task).await
    }
    async fn record_message(
        &self,
        session: SessionId,
        task: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        self.registry(session)?
            .record_message(session, task, message)
            .await
    }
    async fn list_messages(
        &self,
        session: SessionId,
        task: &str,
        limit: Option<u32>,
        after: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        self.registry(session)?
            .list_messages(session, task, limit, after)
            .await
    }
}

// A reader is infallible to construct; resolve its maintenance store at the
// fallible operation boundary so an I/O failure is reported, never panicked.
pub(super) struct SessionDreaming(pub(super) SqliteHosting, pub(super) SessionId);
#[async_trait]
impl everruns_core::dreaming::DreamingStore for SessionDreaming {
    async fn get(
        &self,
        org: i64,
        key: &str,
    ) -> Result<Option<everruns_core::dreaming::DreamStateRow>> {
        self.0.for_session(self.1)?.get(org, key).await
    }
    async fn compare_exchange(
        &self,
        org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session: Option<SessionId>,
        due: Option<i64>,
    ) -> Result<bool> {
        if self.0.directory.is_some() && session.is_some_and(|session| session != self.1) {
            return Err(error("cross-session maintenance access"));
        }
        self.0
            .for_session(self.1)?
            .compare_exchange(org, key, expected, value, session, due)
            .await
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        self.0.for_session(self.1)?.due(now, limit).await
    }
}

fn migrate_shared(source: &Path, destination: &Path, session: SessionId) -> Result<()> {
    use rusqlite::{Connection as RawConnection, OpenFlags};
    let mut source =
        RawConnection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(error)?;
    source.busy_timeout(Duration::from_secs(5)).map_err(error)?;
    let snapshot = source.transaction().map_err(error)?;
    let control: Option<String> = snapshot
        .query_row(
            "SELECT control FROM framework_authority WHERE session=?",
            [session.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?;
    let Some(control) = control else {
        return Ok(());
    };
    let state: Control = decode(control)?;
    state.binding(session)?;
    if state.holder.is_some() && state.expires > now() {
        return Err(error(
            "cannot transfer a session with a live execution epoch",
        ));
    }
    // Memory-wide maintenance cannot be divided into session authorities.
    let shared_maintenance: bool = snapshot
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM dreaming_state WHERE session_id IS NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(error)?;
    if shared_maintenance {
        return Err(error(
            "shared maintenance state requires an explicit authority migration",
        ));
    }
    let temporary = destination.with_file_name(format!("transfer-{}.sqlite", uuid::Uuid::new_v4()));
    let transfer = (|| {
        crate::local::SqliteDb::open(&temporary).map_err(error)?;
        let target = SqliteHosting::open(&temporary)?;
        let mut db = target.db.lock().map_err(error)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS framework_session_children(session TEXT PRIMARY KEY)",
        )
        .map_err(error)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        for (table, filter) in [
            ("framework_authority", "session=?1"),
            ("framework_events", "session=?1"),
            ("framework_objects", "session=?1"),
            ("framework_commands", "session=?1"),
            ("local_tasks", "session_id=?1"),
            (
                "local_task_messages",
                "task_id IN (SELECT id FROM local_tasks WHERE session_id=?1)",
            ),
            ("local_task_recovery_wakes", "session_id=?1"),
            ("local_schedules", "session_id=?1"),
            ("dreaming_state", "session_id=?1"),
        ] {
            copy_session_rows(&snapshot, &tx, table, filter, session)?;
        }
        let mut children = snapshot.prepare(
            "SELECT session FROM framework_authority WHERE json_extract(control,'$.binding.metadata.parent_session_id')=?"
        ).map_err(error)?;
        for child in children
            .query_map([session.to_string()], |row| row.get::<_, String>(0))
            .map_err(error)?
        {
            let child: SessionId = child.map_err(error)?.parse().map_err(error)?;
            tx.execute(
                "INSERT OR IGNORE INTO framework_session_children VALUES (?)",
                [child.to_string()],
            )
            .map_err(error)?;
        }
        // Definitions exist in newer authorities only. They contain no session
        // conversation; retain the source definitions needed by this binding.
        for table in ["framework_agent_definitions", "framework_agent_current"] {
            let exists: bool = snapshot
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
                    [table],
                    |row| row.get(0),
                )
                .map_err(error)?;
            if exists {
                copy_session_rows(&snapshot, &tx, table, "?1 IS NOT NULL", session)?;
            }
        }
        tx.commit().map_err(error)?;
        // Collapse WAL before publishing the standalone file. No connection
        // to the temporary database survives its rename.
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(error)?;
        drop(db);
        drop(target);
        std::fs::File::open(&temporary)
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
        std::fs::rename(&temporary, destination).map_err(error)?;
        #[cfg(unix)]
        std::fs::File::open(destination.parent().expect("session database parent"))
            .map_err(error)?
            .sync_all()
            .map_err(error)?;
        Ok(())
    })();
    if transfer.is_err() {
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(temporary.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(temporary.with_extension("sqlite-shm"));
    }
    transfer
}

fn copy_session_rows(
    source: &rusqlite::Connection,
    target: &Transaction<'_>,
    table: &str,
    filter: &str,
    session: SessionId,
) -> Result<()> {
    let exists: bool = source
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            [table],
            |row| row.get(0),
        )
        .map_err(error)?;
    if !exists {
        return Ok(());
    }
    let mut select = source
        .prepare(&format!("SELECT * FROM {table} WHERE {filter}"))
        .map_err(error)?;
    let columns = (0..select.column_count())
        .map(|index| {
            select
                .column_name(index)
                .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        })
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    let mut insert = target
        .prepare(&format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            columns.join(","),
            vec!["?"; columns.len()].join(",")
        ))
        .map_err(error)?;
    let mut rows = select.query([session.to_string()]).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        let values = (0..columns.len())
            .map(|column| row.get::<_, rusqlite::types::Value>(column))
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(error)?;
        insert
            .execute(rusqlite::params_from_iter(values))
            .map_err(error)?;
    }
    Ok(())
}

impl SqliteHosting {
    /// Session-owned schedule storage for host UI reads and cancellation.
    /// Execution uses the same store with its fenced journal attached.
    pub fn schedule_store(
        &self,
        session: SessionId,
        org: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionScheduleStore> {
        Arc::new(SessionSchedules {
            hosting: self.clone(),
            session,
            org,
        })
    }
}
struct SessionSchedules {
    hosting: SqliteHosting,
    session: SessionId,
    org: i64,
}
impl SessionSchedules {
    fn store(&self, session: SessionId) -> Result<crate::local::LocalScheduleStore> {
        Ok(self
            .hosting
            .for_session(session)?
            .services
            .schedules(self.org, None))
    }
}
#[async_trait]
impl everruns_core::session_services::SessionScheduleStore for SessionSchedules {
    async fn create_schedule(
        &self,
        session: SessionId,
        description: String,
        cron: Option<String>,
        at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> Result<everruns_contracts::runtime::session_schedule::SessionSchedule> {
        self.store(session)?
            .create_schedule(session, description, cron, at, timezone)
            .await
    }
    async fn create_schedule_enforcing_limits(
        &self,
        session: SessionId,
        description: String,
        cron: Option<String>,
        at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
        bounds: everruns_contracts::runtime::session_schedule::ScheduleBounds,
    ) -> std::result::Result<
        everruns_contracts::runtime::session_schedule::SessionSchedule,
        everruns_contracts::runtime::session_schedule::ScheduleLimitError,
    > {
        self.store(session)
            .map_err(everruns_contracts::runtime::session_schedule::ScheduleLimitError::Store)?
            .create_schedule_enforcing_limits(session, description, cron, at, timezone, bounds)
            .await
    }
    async fn cancel_schedule(
        &self,
        session: SessionId,
        schedule: everruns_contracts::typed_id::ScheduleId,
    ) -> Result<everruns_contracts::runtime::session_schedule::SessionSchedule> {
        self.store(session)?
            .cancel_schedule(session, schedule)
            .await
    }
    async fn list_schedules(
        &self,
        session: SessionId,
    ) -> Result<Vec<everruns_contracts::runtime::session_schedule::SessionSchedule>> {
        self.store(session)?.list_schedules(session).await
    }
    async fn count_active_schedules(&self, session: SessionId) -> Result<u32> {
        self.store(session)?.count_active_schedules(session).await
    }
    async fn count_active_org_schedules(&self) -> Result<u32> {
        self.store(self.session)?.count_active_org_schedules().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_locked_session_does_not_block_another_session() {
        let dir = tempfile::tempdir().unwrap();
        let a = SessionId::new();
        let b = SessionId::new();
        let hosting = SqliteHosting::open_session(dir.path(), a).unwrap();
        let build = AgentBuild::new("assistant", "isolated-v1");
        let engine = crate::Engine::builder()
            .hosting(hosting.clone())
            .register(
                build.clone(),
                crate::Agent::builder()
                    .instructions("Answer briefly.")
                    .model(crate::Model::simulated("Hello!"))
                    .build()
                    .unwrap(),
            )
            .build();
        engine.create_registered(a, &build).await.unwrap();
        engine.create_registered(b, &build).await.unwrap();
        let blocker = crate::sqlite::open(SqliteHosting::session_db_path(dir.path(), a)).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(
            SqliteHosting::visit_session_events(
                SqliteHosting::session_db_path(dir.path(), a),
                a,
                |_| Ok(())
            )
            .unwrap()
        );
        let task: CreateSessionTask = serde_json::from_value(json!({
            "session_id": b, "kind": "test", "display_name": "session B task"
        }))
        .unwrap();
        hosting
            .reader(b)
            .session_task_registry
            .unwrap()
            .create(task)
            .await
            .unwrap();
        hosting
            .schedule_store(b, 1)
            .create_schedule(
                b,
                "session B schedule".into(),
                None,
                Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                "UTC".into(),
            )
            .await
            .unwrap();

        engine.resume(b).await.unwrap().run("hello").await.unwrap();
        blocker.execute_batch("ROLLBACK").unwrap();
        for (id, expected) in [(a, 0), (b, 1)] {
            let db = crate::sqlite::open(SqliteHosting::session_db_path(dir.path(), id)).unwrap();
            let owners: i64 = db
                .query_row(
                    "SELECT count(*) FROM framework_authority WHERE session <> ?",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(owners, 0);
            let inputs: i64 = db
                .query_row(
                    "SELECT count(*) FROM framework_events WHERE type='input.message'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(inputs, expected);
            for table in ["local_tasks", "local_schedules"] {
                let count: i64 = db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, expected, "{table}");
            }
        }
    }
    fn agent() -> crate::Agent {
        crate::Agent::builder()
            .instructions("Answer briefly.")
            .model(crate::Model::simulated("Hello!"))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn migration_preserves_canonical_state_and_only_the_owning_sessions_rows() {
        use everruns_core::session_task::{CreateSessionTask, NewTaskMessage, SessionTaskState};
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let a = SessionId::new();
        let b = SessionId::new();
        let build = AgentBuild::new("assistant", "migration-v1");
        let source = SqliteHosting::open(&shared)
            .unwrap()
            .with_secret_keys("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        for id in [a, b] {
            engine.create_registered(id, &build).await.unwrap();
        }
        engine
            .resume(a)
            .await
            .unwrap()
            .run("keep this history")
            .await
            .unwrap();
        let pending = everruns_core::RuntimeMessage::user("pending input");
        let pending_id = pending.id;
        source.admit(a, pending, false).await.unwrap();
        let tasks = source.task_registry();
        let task = tasks
            .create(CreateSessionTask {
                session_id: a,
                id: None,
                kind: "assignment".into(),
                display_name: "retained task".into(),
                spec: json!({"private": "keep this spec"}),
                state: SessionTaskState::Queued,
                links: Default::default(),
                wake_policy: Default::default(),
            })
            .await
            .unwrap();
        tasks
            .record_message(
                a,
                &task.id,
                NewTaskMessage::inbound_text("keep this channel"),
            )
            .await
            .unwrap();
        source
            .transaction(a, |tx, _| {
                put(
                    tx,
                    a,
                    "secret",
                    "sealed",
                    &source.secret_keys()?.seal(a, "sealed", "retained secret")?,
                )?;

                Ok(())
            })
            .unwrap();
        let schedule = source
            .schedule_store(a, 1)
            .create_schedule(
                a,
                "retained schedule".into(),
                None,
                Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                "UTC".into(),
            )
            .await
            .unwrap();
        let source_events = engine
            .resume(a)
            .await
            .unwrap()
            .events_after(0)
            .await
            .unwrap();
        drop(engine);
        let target =
            SqliteHosting::open_session_from(&root, a, Some(&shared), |_| Ok(Box::new(())))
                .unwrap();
        let target = target
            .with_secret_keys("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .unwrap();
        assert_eq!(
            target
                .reader(a)
                .storage_store
                .get_secret(a, "sealed")
                .await
                .unwrap()
                .as_deref(),
            Some("retained secret")
        );
        let schedules = target.schedule_store(a, 1).list_schedules(a).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].id, schedule.id);
        assert!(target.status(a).await.unwrap().runnable);
        assert!(target.receipt(a, pending_id).await.unwrap().is_some());
        let target_events =
            crate::coordination::platform_messages(target.reader(a).event_log, a, None)
                .await
                .unwrap();
        assert!(target_events.iter().any(|message| {
            serde_json::to_string(&message.content)
                .unwrap()
                .contains("keep this history")
        }));
        for table in [
            "framework_authority",
            "framework_events",
            "framework_objects",
            "framework_commands",
            "local_tasks",
            "local_task_messages",
            "local_task_recovery_wakes",
            "local_schedules",
            "dreaming_state",
        ] {
            let query = format!("SELECT count(*) FROM {table}");
            let target_owner = target.for_session(a).unwrap();
            let target_count: i64 = target_owner
                .db
                .lock()
                .unwrap()
                .query_row(&query, [], |r| r.get(0))
                .unwrap();
            let source_count: i64 = source
                .db
                .lock()
                .unwrap()
                .query_row(&query, [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                target_count,
                source_count - i64::from(table == "framework_authority"),
                "{table}"
            );
        }
        assert_eq!(
            target
                .task_registry()
                .get(a, &task.id)
                .await
                .unwrap()
                .unwrap()
                .spec,
            task.spec
        );
        assert_eq!(
            target
                .task_registry()
                .list_messages(a, &task.id, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut export = Vec::new();
        target.export_jsonl(a, &mut export).unwrap();
        assert_eq!(
            export
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            source_events.len()
        );
        drop(target);
        let reopened = SqliteHosting::open_session_from(&root, a, Some(&shared), |_| {
            panic!("completed migration must not take the source lease again")
        })
        .unwrap();
        assert!(reopened.receipt(a, pending_id).await.unwrap().is_some());
        assert!(!SqliteHosting::session_db_path(&root, b).exists());
    }

    #[tokio::test]
    async fn migration_refuses_live_epochs_and_keeps_the_source_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let session = SessionId::new();
        let build = AgentBuild::new("assistant", "live-v1");
        let source = SqliteHosting::open(&shared).unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(session, &build).await.unwrap();
        source
            .transaction(session, |_, state| {
                state.holder = Some("live".into());
                state.expires = now() + 60_000;
                Ok(())
            })
            .unwrap();
        let error =
            SqliteHosting::open_session_from(&root, session, Some(&shared), |_| Ok(Box::new(())))
                .err()
                .expect("live session must refuse migration");
        assert!(
            error.to_string().contains("live execution epoch"),
            "{error}"
        );
        assert!(!SqliteHosting::session_db_path(&root, session).exists());
        assert!(source.status(session).await.unwrap().executing);
    }

    #[tokio::test]
    async fn child_transfer_takes_its_own_source_lease_and_retries_independently() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let parent = SessionId::new();
        let child = SessionId::new();
        let build = AgentBuild::new("assistant", "child-transfer-v1");
        let source = SqliteHosting::open(&shared).unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(parent, &build).await.unwrap();
        engine
            .create_registered_with_metadata(child, &build, json!({"parent_session_id": parent}))
            .await
            .unwrap();
        let failed = SqliteHosting::open_session_from(&root, parent, Some(&shared), move |id| {
            if id == child {
                Err(error("source child lease unavailable"))
            } else {
                Ok(Box::new(()))
            }
        });
        assert!(
            failed
                .err()
                .unwrap()
                .to_string()
                .contains("source child lease unavailable")
        );
        assert!(!SqliteHosting::session_db_path(&root, parent).exists());
        assert!(!SqliteHosting::session_db_path(&root, child).exists());
        let reopened = SqliteHosting::open_session_from(&root, parent, Some(&shared), move |id| {
            assert!([parent, child].contains(&id));
            Ok(Box::new(()))
        })
        .unwrap();
        assert_eq!(
            reopened.status(child).await.unwrap().binding.metadata["parent_session_id"],
            json!(parent)
        );
        assert_eq!(source.status(child).await.unwrap().binding.behavior, build);
    }

    #[tokio::test]
    async fn all_family_source_leases_precede_the_first_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let parent = SessionId::new();
        let child = SessionId::new();
        let build = AgentBuild::new("assistant", "family-transfer-v1");
        let source = SqliteHosting::open(&shared).unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(parent, &build).await.unwrap();
        engine
            .create_registered_with_metadata(child, &build, json!({"parent_session_id": parent}))
            .await
            .unwrap();
        let reporting_child = source.clone();
        let target = SqliteHosting::open_session_from(&root, parent, Some(&shared), move |id| {
            if id == child {
                // A source child can report to its parent until we acquire
                // that child's lease, even if the parent host has stopped.
                reporting_child.transaction(parent, |tx, _| {
                    put(
                        tx,
                        parent,
                        "framework",
                        "handoff/last_child_report",
                        &json!("reported before child stopped"),
                    )
                })?;
            }
            Ok(Box::new(()))
        })
        .unwrap();
        assert_eq!(
            target
                .record(parent, "handoff/last_child_report")
                .await
                .unwrap(),
            Some(json!("reported before child stopped"))
        );
    }

    #[tokio::test]
    async fn opening_a_source_child_first_quiesces_ancestors_and_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let parent = SessionId::new();
        let child = SessionId::new();
        let sibling = SessionId::new();
        let build = AgentBuild::new("assistant", "child-first-v1");
        let source = SqliteHosting::open(&shared).unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(parent, &build).await.unwrap();
        for id in [child, sibling] {
            engine
                .create_registered_with_metadata(id, &build, json!({"parent_session_id": parent}))
                .await
                .unwrap();
        }
        let acquired = Arc::new(Mutex::new(HashSet::new()));
        let observed = acquired.clone();
        let target = SqliteHosting::open_session_from(&root, child, Some(&shared), move |id| {
            observed.lock().unwrap().insert(id);
            Ok(Box::new(()))
        })
        .unwrap();
        assert_eq!(
            *acquired.lock().unwrap(),
            HashSet::from([parent, child, sibling])
        );
        for id in [parent, child, sibling] {
            assert!(SqliteHosting::session_db_path(&root, id).exists());
            assert_eq!(target.status(id).await.unwrap().binding.behavior, build);
        }
    }

    #[tokio::test]
    async fn a_live_source_child_refuses_before_publishing_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.sqlite");
        let root = dir.path().join("sessions");
        let parent = SessionId::new();
        let child = SessionId::new();
        let build = AgentBuild::new("assistant", "live-child-v1");
        let source = SqliteHosting::open(&shared).unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(parent, &build).await.unwrap();
        engine
            .create_registered_with_metadata(child, &build, json!({"parent_session_id": parent}))
            .await
            .unwrap();
        source
            .transaction(child, |_, state| {
                state.holder = Some("live child".into());
                state.expires = now() + 60_000;
                Ok(())
            })
            .unwrap();
        assert!(
            SqliteHosting::open_session_from(&root, parent, Some(&shared), |_| Ok(Box::new(())))
                .is_err()
        );
        assert!(!SqliteHosting::session_db_path(&root, parent).exists());
        assert!(!SqliteHosting::session_db_path(&root, child).exists());
    }

    #[test]
    fn routing_rejects_symlinked_session_directories() {
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let session = SessionId::new();
            std::os::unix::fs::symlink(outside.path(), dir.path().join(session.to_string()))
                .unwrap();
            assert!(SqliteHosting::open_session(dir.path(), session).is_err());
            assert!(!outside.path().join("engine.sqlite").exists());
        }
    }
    #[tokio::test]
    async fn directory_transfer_keeps_wal_state_children_and_late_reports() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join(".engine");
        let new = root.path().join("sessions");
        let parent = SessionId::new();
        let child = SessionId::new();
        let unrelated = SessionId::new();
        let build = AgentBuild::new("assistant", "directory-transfer");
        let key = "test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let source = SqliteHosting::open_session(&old, parent)
            .unwrap()
            .with_secret_keys(key, &[])
            .unwrap();
        let engine = crate::Engine::builder()
            .hosting(source.clone())
            .register(build.clone(), agent())
            .build();
        engine.create_registered(parent, &build).await.unwrap();
        engine.create_registered(unrelated, &build).await.unwrap();
        engine
            .create_registered_with_metadata(child, &build, json!({"parent_session_id":parent}))
            .await
            .unwrap();
        engine
            .resume(child)
            .await
            .unwrap()
            .run("child history")
            .await
            .unwrap();
        let input = everruns_core::RuntimeMessage::user("retain this pending input");
        let input_id = input.id;
        source.admit(parent, input, false).await.unwrap();
        source
            .transaction(parent, |tx, _| {
                put(
                    tx,
                    parent,
                    "secret",
                    "token",
                    &source.secret_keys()?.seal(parent, "token", "keep me")?,
                )
            })
            .unwrap();
        let denied = SqliteHosting::open_session_from(&new, parent, Some(&old), move |id| {
            if id == child {
                Err(error("child still owned"))
            } else {
                Ok(Box::new(()))
            }
        });
        assert!(
            denied
                .err()
                .unwrap()
                .to_string()
                .contains("child still owned")
        );
        assert!(!SqliteHosting::session_db_path(&new, parent).exists());
        let reporter = source.clone();
        let target = SqliteHosting::open_session_from(&new, child, Some(&old), move |id| {
            assert!([parent, child].contains(&id));
            if id == child {
                reporter.transaction(parent, |tx, _| {
                    put(tx, parent, "test", "late", &json!("last child report"))
                })?;
            }
            Ok(Box::new(()))
        })
        .unwrap()
        .with_secret_keys(key, &[])
        .unwrap();
        assert!(target.receipt(parent, input_id).await.unwrap().is_some());
        assert_eq!(
            target
                .reader(parent)
                .storage_store
                .get_secret(parent, "token")
                .await
                .unwrap()
                .as_deref(),
            Some("keep me")
        );
        assert_eq!(
            target
                .transaction(parent, |tx, _| object::<Value>(tx, parent, "test", "late"))
                .unwrap(),
            Some(json!("last child report"))
        );
        assert_eq!(
            target.status(child).await.unwrap().binding.metadata["parent_session_id"],
            json!(parent)
        );
        let db = rusqlite::Connection::open(SqliteHosting::session_db_path(&new, parent)).unwrap();
        let registered: String = db
            .query_row(
                "SELECT session FROM framework_session_children",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(registered, child.to_string());
        assert!(!SqliteHosting::session_db_path(&new, unrelated).exists());
        assert!(SqliteHosting::session_db_path(&old, parent).exists());
        let child_db =
            rusqlite::Connection::open(SqliteHosting::session_db_path(&new, child)).unwrap();
        let inputs: i64 = child_db
            .query_row(
                "SELECT count(*) FROM framework_events WHERE type='input.message'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(inputs, 1);
        let reopened = SqliteHosting::open_session_from(&new, child, Some(&old), |_| {
            panic!("completed transfer must use its destination")
        })
        .unwrap();
        assert!(reopened.receipt(parent, input_id).await.unwrap().is_some());
    }
}
