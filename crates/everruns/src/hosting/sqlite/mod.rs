//! Single-domain durable authority. All canonical/control mutations use an
//! IMMEDIATE SQLite transaction with FULL synchronization. JSONL is not a
//! second writer. A startup supervisor rediscovers owed work after process loss.
mod commands;
mod dreaming;
mod export;
mod import;
mod lineage;
mod ports;
mod secrets;
mod services;
use super::*;
use crate::sqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use everruns_contracts::{
    error::AgentLoopError,
    typed_id::{EventId, TurnId},
};
use everruns_core::{
    events::{Event, EventContext, EventData, EventRequest},
    host::session_actor::store::actor_lease_lost,
};
use rusqlite::Transaction;
use serde::de::DeserializeOwned;
use serde_json::json;
pub(crate) use services::task_is_running_elsewhere;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Mutex, RwLock},
};

type ProactiveAttempts =
    Arc<Mutex<HashMap<(SessionId, String, String), everruns_core::ProactiveCompactionAttempt>>>;

// Separate branches let SQLite seek both recovery obligations without scanning
// idle session history. UNION also deduplicates a runnable session with a due timer.
const DISCOVER_RUNNABLE: &str = "
    SELECT session FROM framework_authority
    WHERE json_extract(control,'$.runnable')=1 AND json_extract(control,'$.suspended')=0
        AND json_extract(control,'$.expires')<=?1
    UNION
    SELECT session FROM framework_authority
    WHERE json_extract(control,'$.wake_at')<=?1 AND json_extract(control,'$.suspended')=0
        AND json_extract(control,'$.expires')<=?1
    LIMIT 64";

/// Transactional embedded session authority. Multiple connections/processes
/// may share one local database. A host must run [`supervise`](Self::supervise)
/// on startup; the SQLite file and its WAL must survive executor loss.
#[derive(Clone)]
pub struct SqliteHosting {
    db: Arc<Mutex<Connection>>,
    proactive: ProactiveAttempts,
    /// In-process [`SessionHosting::changes`] hints. Other processes sharing
    /// the database are seen by the waiter's poll, not by these.
    changes: super::ChangeHints,
    /// Key encryption keys for session secrets; without them the authority
    /// refuses secret operations rather than storing plaintext.
    secrets: Option<secrets::SecretKeys>,
    /// Task registry, recovery and child-session services in this database.
    services: Arc<services::AuthorityServices>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Control {
    wake_at: Option<i64>,
    resume_generation: u64,
    resume_claimed: u64,
    #[serde(default)]
    created_at: String,
    binding: Option<SessionBinding>,
    head: i32,
    inbox: i64,
    consumed: i64,
    next_input: i64,
    holder: Option<String>,
    fence: u64,
    expires: u64,
    attempts: u32,
    cancel: Option<i64>,
    cancel_reported: bool,
    runnable: bool,
    suspended: bool,
    open: Option<(TurnId, MessageId)>,
    pending: Vec<crate::ToolCall>,
    finalizing: HashSet<TurnId>,
    target_cancels: HashMap<MessageId, i64>,
    /// Active conversation branch; `None` until the first selection.
    lineage: Option<lineage::Lineage>,
}

impl Control {
    fn binding(&self, session: SessionId) -> Result<SessionBinding> {
        self.binding
            .clone()
            .ok_or(AgentLoopError::SessionNotFound(session))
    }
    fn owns(&self, holder: &str, fence: u64) -> bool {
        self.holder.as_deref() == Some(holder) && self.fence == fence && self.expires > now()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}
fn encode<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(error)
}
fn decode<T: DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_str(&value).map_err(error)
}

impl SqliteHosting {
    /// Open an authority database, separate from the legacy local profile.
    /// Use a local filesystem with working POSIX locks; remote object mounts
    /// and restoring snapshots over this database are unsupported.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = crate::sqlite::open(path.as_ref()).map_err(error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS framework_authority(session TEXT PRIMARY KEY,control TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS framework_runnable ON framework_authority(json_extract(control,'$.expires')) WHERE json_extract(control,'$.runnable')=1 AND json_extract(control,'$.suspended')=0;
            CREATE INDEX IF NOT EXISTS framework_timers ON framework_authority(json_extract(control,'$.wake_at'),json_extract(control,'$.expires')) WHERE json_extract(control,'$.suspended')=0 AND json_extract(control,'$.wake_at') IS NOT NULL;
            CREATE TABLE IF NOT EXISTS framework_events(session TEXT NOT NULL,seq INTEGER NOT NULL,type TEXT NOT NULL,turn_id TEXT,identity TEXT,value TEXT NOT NULL,PRIMARY KEY(session,seq),UNIQUE(session,identity));
            CREATE INDEX IF NOT EXISTS framework_event_ids ON framework_events(session,json_extract(value,'$.id'));
            CREATE INDEX IF NOT EXISTS framework_event_turns ON framework_events(session,type,seq);
            CREATE TABLE IF NOT EXISTS framework_objects(session TEXT NOT NULL,kind TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,created TEXT NOT NULL,updated TEXT NOT NULL,PRIMARY KEY(session,kind,key));
            CREATE TABLE IF NOT EXISTS dreaming_state(org_id INTEGER NOT NULL,key TEXT NOT NULL,revision INTEGER NOT NULL,value TEXT NOT NULL,session_id TEXT,due_at INTEGER,PRIMARY KEY(org_id,key));
            CREATE INDEX IF NOT EXISTS dreaming_due ON dreaming_state(due_at) WHERE due_at IS NOT NULL;
            CREATE TABLE IF NOT EXISTS framework_commands(session TEXT NOT NULL,id TEXT NOT NULL,fingerprint TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(session,id));").map_err(error)?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            proactive: Arc::default(),
            changes: super::ChangeHints::default(),
            secrets: None,
            services: Arc::new(services::AuthorityServices::open(path.as_ref())?),
        })
    }

    /// Store session secrets encrypted with these key encryption keys, in the
    /// platform's `key_id:base64_key` format (32-byte AES-256 keys). The first
    /// key seals new values; `previous` keys still open older ones during
    /// rotation. Keep the keys outside the database and stable across
    /// restarts: a secret sealed with a lost key cannot be read.
    /// Stability: experimental.
    pub fn with_secret_keys(mut self, primary: &str, previous: &[&str]) -> Result<Self> {
        self.secrets = Some(secrets::SecretKeys::new(primary, previous)?);
        Ok(self)
    }

    fn secret_keys(&self) -> Result<&secrets::SecretKeys> {
        self.secrets.as_ref().ok_or_else(|| {
            AgentLoopError::config(
                "this SQLite authority has no secret keys; configure SqliteHosting::with_secret_keys",
            )
        })
    }

    fn transaction<T>(
        &self,
        session: SessionId,
        f: impl FnOnce(&Transaction<'_>, &mut Control) -> Result<T>,
    ) -> Result<T> {
        let mut db = self
            .db
            .lock()
            .map_err(|_| error("authority database lock poisoned"))?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let saved: Option<String> = tx
            .query_row(
                "SELECT control FROM framework_authority WHERE session=?",
                [session.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        let mut state: Control = saved.clone().map(decode).transpose()?.unwrap_or_default();
        let before = encode(&state)?;
        if state.wake_at.is_some_and(|at| at <= now() as i64) {
            state.wake_at = None;
            state.resume_generation += 1;
            state.runnable = true;
        }
        let result = f(&tx, &mut state)?;
        let encoded = encode(&state)?;
        if state.binding.is_some() && (saved.is_none() || before != encoded) {
            tx.execute("INSERT INTO framework_authority VALUES (?,?) ON CONFLICT(session) DO UPDATE SET control=excluded.control", params![session.to_string(),encoded]).map_err(error)?;
        }
        tx.commit().map_err(error)?;
        Ok(result)
    }

    fn ports(&self, session: SessionId) -> Ports {
        Ports {
            hosting: self.clone(),
            session,
            fence: Arc::new(RwLock::new(None)),
        }
    }

    /// Continuously rediscover runnable/expired sessions. Run once per host at
    /// startup, independently of client traffic. Duplicate supervisors are safe;
    /// claims and every write remain authority-fenced. Shutdown cancels local
    /// execution and leaves the committed recovery obligation intact.
    pub async fn supervise(
        &self,
        engine: crate::Engine,
        shutdown: crate::CancellationToken,
    ) -> Result<()> {
        // Executions lost with an earlier process are settled before new
        // work starts; their wakes become admissions the loop below runs.
        self.services
            .recovery
            .set_wake_observer(Arc::new(services::HostedWaker::new(
                &engine,
                self.clone(),
                true,
            )));
        if let Err(error) = self.services.recovery.recover_all().await {
            tracing::warn!(%error, "task recovery at startup failed");
        }
        self.services.recovery.start_maintenance();
        let schedules = self.start_schedules(&engine)?;
        let mut running = tokio::task::JoinSet::new();
        loop {
            let sessions: Vec<SessionId> = {
                let db = self
                    .db
                    .lock()
                    .map_err(|_| error("authority database lock poisoned"))?;
                let mut query = db.prepare(DISCOVER_RUNNABLE).map_err(error)?;
                query
                    .query_map([now() as i64], |row| row.get::<_, String>(0))
                    .map_err(error)?
                    .map(|row| row.map_err(error)?.parse().map_err(error))
                    .collect::<Result<_>>()?
            };
            for session in sessions {
                let engine = engine.clone();
                running.spawn(async move {
                    if let Err(error) = engine.activate(session).await {
                        tracing::warn!(%session,%error,"Native Engine activation failed");
                    }
                });
            }
            let due = everruns_core::dreaming::DreamingStore::due(
                self,
                everruns_contracts::rt::utc_now().timestamp(),
                32,
            )
            .await?;
            for (_, session) in due {
                let engine = engine.clone();
                running.spawn(async move {
                    let _ = engine.maintain_dreaming(session).await;
                });
            }
            while running.try_join_next().is_some() {}
            tokio::select! {
                () = shutdown.cancelled() => {
                    running.abort_all();
                    while running.join_next().await.is_some() {}
                    if let Err(error) = schedules.shutdown().await {
                        tracing::warn!(%error, "schedule runner shutdown failed");
                    }
                    return Ok(());
                }
                () = everruns_contracts::rt::sleep(Duration::from_millis(100)) => {}
            }
        }
    }
}

fn object<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    session: SessionId,
    kind: &str,
    key: &str,
) -> Result<Option<T>> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT value FROM framework_objects WHERE session=? AND kind=? AND key=?",
            params![session.to_string(), kind, key],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?;
    raw.map(decode).transpose()
}
fn put(
    tx: &Transaction<'_>,
    session: SessionId,
    kind: &str,
    key: &str,
    value: &impl Serialize,
) -> Result<()> {
    if key.len() > 512 {
        return Err(error("authority object key exceeds limit"));
    }
    let raw = encode(value)?;
    if raw.len() > 2 * 1024 * 1024 {
        return Err(error("authority object exceeds size limit"));
    }
    let at = everruns_contracts::rt::utc_now().to_rfc3339();
    tx.execute("INSERT INTO framework_objects VALUES (?,?,?,?,?,?) ON CONFLICT(session,kind,key) DO UPDATE SET value=excluded.value,updated=excluded.updated", params![session.to_string(),kind,key,raw,at,at]).map_err(error)?;
    Ok(())
}
fn retain_command(
    tx: &Transaction<'_>,
    session: SessionId,
    id: &str,
    fingerprint: &Value,
    value: &impl Serialize,
) -> Result<()> {
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM framework_commands WHERE session=?",
            [session.to_string()],
            |row| row.get(0),
        )
        .map_err(error)?;
    if count >= 100_000 {
        return Err(error("session command capacity reached"));
    }
    tx.execute(
        "INSERT INTO framework_commands VALUES (?,?,?,?)",
        params![
            session.to_string(),
            id,
            encode(fingerprint)?,
            encode(value)?
        ],
    )
    .map_err(error)?;
    Ok(())
}
fn append(
    tx: &Transaction<'_>,
    state: &mut Control,
    request: EventRequest,
    expected: Option<Option<i32>>,
) -> Result<Event> {
    let session = request.session_id;
    state.binding(session)?;
    if request.is_ephemeral() || request.event_type != request.data.event_type() {
        return Err(AgentLoopError::config(
            "only typed canonical events may enter the authority",
        ));
    }
    let actual = (state.head > 0).then_some(state.head);
    if let Some(expected) = expected
        && expected != actual
    {
        return Err(error("authority event sequence conflict"));
    }
    let turn = request.context.turn_id;
    let identity = event_identity(&request.data, turn)?;
    if let Some(identity) = &identity {
        let old: Option<String> = tx
            .query_row(
                "SELECT value FROM framework_events WHERE session=? AND identity=?",
                params![session.to_string(), identity],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(old) = old {
            let event: Event = decode(old)?;
            if event.event_type != request.event_type
                || serde_json::to_value(&event.data).map_err(error)?
                    != serde_json::to_value(&request.data).map_err(error)?
            {
                return Err(error("conflicting canonical operation outcome"));
            }
            return Ok(event);
        }
    }
    if matches!(request.data, EventData::TurnStarted(_)) && state.open.is_some() {
        return Err(error("session already has an open turn"));
    }
    state.head = state
        .head
        .checked_add(1)
        .ok_or_else(|| error("session event capacity reached"))?;
    let event = request.into_event(EventId::new(), state.head);
    let encoded = encode(&event)?;
    if encoded.len() > 2 * 1024 * 1024 {
        return Err(error("canonical event exceeds size limit"));
    }
    tx.execute(
        "INSERT INTO framework_events VALUES (?,?,?,?,?,?)",
        params![
            session.to_string(),
            state.head,
            event.event_type,
            turn.map(|id| id.to_string()),
            identity,
            encoded
        ],
    )
    .map_err(error)?;
    advance(state, &event);
    Ok(event)
}

fn is_terminal(data: &EventData) -> bool {
    matches!(
        data,
        EventData::TurnCompleted(_)
            | EventData::TurnCancelled(_)
            | EventData::TurnFailed(_)
            | EventData::TurnSealed(_)
    )
}

/// Operation identity deduplicating terminal and tool outcomes per turn.
fn event_identity(data: &EventData, turn: Option<TurnId>) -> Result<Option<String>> {
    Ok(if is_terminal(data) {
        Some(format!(
            "terminal:{}",
            turn.ok_or_else(|| error("terminal event lacks turn"))?
        ))
    } else {
        match data {
            EventData::ToolStarted(data) => Some(format!(
                "started:{}:{}",
                turn.ok_or_else(|| error("tool event lacks turn"))?,
                data.tool_call.id
            )),
            EventData::ToolCompleted(data) => Some(format!(
                "completed:{}:{}",
                turn.ok_or_else(|| error("tool event lacks turn"))?,
                data.tool_call_id
            )),
            _ => None,
        }
    })
}

/// Control transitions of one committed event. Live appends and legacy
/// imports share them, so an imported journal leaves the same open turn and
/// pending calls a native authority would hold at that position.
fn advance(state: &mut Control, event: &Event) {
    match &event.data {
        EventData::TurnStarted(data) => state.open = Some((data.turn_id, data.input_message_id)),
        EventData::ToolCallRequested(data) => state.pending = data.tool_calls.clone(),
        EventData::ToolCompleted(data) => state.pending.retain(|call| call.id != data.tool_call_id),
        _ => {}
    }
    if is_terminal(&event.data)
        && let Some(turn) = event.context.turn_id
    {
        state.open = None;
        state.pending.clear();
        state.finalizing.insert(turn);
    }
}

#[derive(Clone)]
struct Ports {
    hosting: SqliteHosting,
    session: SessionId,
    fence: Arc<RwLock<Option<ActorFence>>>,
}
impl Ports {
    fn scope(&self, session: SessionId) -> Result<()> {
        if session != self.session {
            return Err(AgentLoopError::config("cross-session authority access"));
        }
        Ok(())
    }
    fn check(&self, state: &Control) -> Result<()> {
        let fence = self
            .fence
            .read()
            .map_err(|_| error("epoch lock poisoned"))?;
        if fence
            .as_ref()
            .is_some_and(|fence| !fence.is_lost() && state.owns(&fence.holder, fence.fence))
        {
            return Ok(());
        }
        let rejected = actor_lease_lost(self.session, "SQLite authority epoch expired or absent");
        if let Some(fence) = &*fence {
            fence.observe::<()>(&Err(actor_lease_lost(self.session, "ownership lost")));
        }
        Err(rejected)
    }
}

#[async_trait]
impl SessionHosting for SqliteHosting {
    async fn command(
        &self,
        session: SessionId,
        command: super::spi::session_commands::SessionCommand,
    ) -> Result<Value> {
        let reply = self.execute_command(session, command);
        if reply.is_ok() {
            self.changes.notify(session);
        }
        reply
    }

    fn changes(&self, session: SessionId) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.changes.subscribe(session))
    }

    async fn query_snapshot(&self, session: SessionId) -> Result<SessionQuerySnapshot> {
        self.transaction(session, |tx, state| {
            Ok(SessionQuerySnapshot {
                configuration:object(tx,session,"framework","configuration")?.unwrap_or_default(),
                binding: state.binding(session)?,
                high_watermark: state.head,
                execution: object(tx, session, "session", "record")?,
                rejected_inputs:{
                    let mut statement=tx.prepare("SELECT key FROM framework_objects WHERE session=? AND kind='framework' AND key>='rejected/' AND key<'rejected0' ORDER BY key LIMIT 4097").map_err(error)?;
                    let keys=statement.query_map([session.to_string()],|row|row.get::<_,String>(0)).map_err(error)?.map(|key|key.map_err(error).and_then(|key|key[9..].parse().map_err(error))).collect::<Result<Vec<_>>>()?;
                    if keys.len()>4096{return Err(error("input rejection scan limit exceeded"));}keys
                },
            })
        })
    }
    async fn event_sequence(&self, session: SessionId, event_id: &str) -> Result<Option<i32>> {
        self.db.lock().map_err(|_|error("authority database lock poisoned"))?.query_row("SELECT seq FROM framework_events WHERE session=? AND json_extract(value,'$.id')=? LIMIT 1",params![session.to_string(),event_id],|row|row.get(0)).optional().map_err(error)
    }
    async fn status(&self, session: SessionId) -> Result<HostingStatus> {
        self.transaction(session, |tx, state| {
            Ok(HostingStatus {
                executing: state.holder.is_some() && state.expires > now(),
                now_ms: everruns_contracts::rt::utc_now().timestamp_millis(),
                wake_at: state.wake_at,
                created_at: state.created_at.clone(),
                binding: state.binding(session)?,
                runnable: state.runnable,
                suspended: state.suspended,
                suspension_reason: if state.suspended {
                    object::<Value>(tx, session, "framework", "suspension")?
                        .and_then(|value| value["reason"].as_str().map(str::to_owned))
                } else {
                    None
                },
                open_turn: state.open.map(|(turn, _)| turn),
                waiting: state.pending.clone(),
            })
        })
    }
    async fn receipt(
        &self,
        session: SessionId,
        message: MessageId,
    ) -> Result<Option<AdmissionReceipt>> {
        self.transaction(session, |tx, state| {
            state.binding(session)?;
            let raw: Option<String> = tx
                .query_row(
                    "SELECT value FROM framework_commands WHERE session=? AND id=?",
                    params![session.to_string(), message.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(error)?;
            raw.map(decode).transpose()
        })
    }
    fn reader(&self, session: SessionId) -> HostBackends {
        // Task reads (threads, task tools on query runtimes) see the authority's
        // registry; nothing here is canonical journal state.
        self.ports(session)
            .backends()
            .with_session_task_registry(self.services.registry.clone())
    }
    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        SessionEpoch::record(&self.ports(session), key).await
    }
    async fn claim(
        &self,
        session: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        self.transaction(session, |_, state| {
            let binding = state.binding(session)?;
            if !state.runnable || state.suspended || state.expires > now() {
                return Ok(None);
            }
            state.holder = Some(holder.to_owned());
            state.fence = state
                .fence
                .checked_add(1)
                .ok_or_else(|| error("epoch capacity reached"))?;
            state.expires = now().saturating_add(ttl.as_millis() as u64);
            state.attempts += 1;
            state.cancel_reported = state.cancel.is_some();
            state.resume_claimed = state.resume_generation;
            let actor = LeasedActor {
                session_id: session,
                org_id: 1,
                fence: state.fence,
                inbox_seq: state.inbox,
                cancel_inbox_seq: state.cancel,
                consumed_seq: state.consumed,
                next_input_seq: state.next_input,
                attempts: state.attempts,
            };
            let epoch = Ports {
                hosting: self.clone(),
                session,
                fence: Arc::new(RwLock::new(Some(ActorFence::new(
                    session,
                    holder,
                    state.fence,
                )))),
            };
            Ok(Some(SessionActivation {
                binding,
                actor,
                epoch: Arc::new(epoch),
            }))
        })
    }
    fn process_scheduler(&self) -> bool {
        true
    }
    async fn prepare_activation(
        &self,
        engine: &crate::Engine,
        session: SessionId,
        environment: &crate::Environment,
        backends: HostBackends,
    ) -> Result<HostBackends> {
        self.prepare(engine, session, environment, backends).await
    }
    async fn history_lineage(
        &self,
        session: SessionId,
    ) -> Result<super::spi::session_commands::history::HistoryLineage> {
        self.transaction(session, |tx, state| {
            state.binding(session)?;
            lineage::read(tx, session, state)
        })
    }
    async fn import_legacy(&self, session: SessionId, image: LegacySessionImage) -> Result<bool> {
        let replayed = self.import_image(session, image)?;
        self.changes.notify(session);
        Ok(replayed)
    }
}

#[cfg(test)]
mod tests;
