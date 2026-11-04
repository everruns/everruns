//! Epoch-scoped runtime persistence over the same admission transaction domain.
use super::*;
use everruns_core::{
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore, ExecutionSession,
    ProactiveCompactionAttempt,
    execution_loading::SessionStore,
    host::{
        EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
        EventReader, RuntimeSessionStore, SessionMutator,
        session_actor::store::{ActorJournal, ActorRelease, ActorRenewal},
    },
    session_services::{KeyInfo, SecretInfo, SessionStorageStore},
};

use crate::hosting::spi::session_commands::history::{self, SequenceRange};

fn log_error(error: impl std::fmt::Display) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

#[async_trait]
impl SessionEpoch for Ports {
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        self.hosting.transaction(self.session, |_, state| {
            self.check(state)?;
            state.wake_at = wake_at;
            Ok(())
        })
    }
    fn backends(&self) -> HostBackends {
        let mut backends = HostBackends::in_memory();
        backends.dreaming_store = Arc::new(self.hosting.clone());
        backends.event_log = Arc::new(self.clone());
        backends.session_store = Arc::new(self.clone());
        backends.storage_store = Arc::new(self.clone());
        backends.compaction_checkpoint_store = Arc::new(self.clone());
        backends
    }
    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        Arc::new(self.clone())
    }
    fn observe_fence(&self, fence: &ActorFence) {
        let mut current = self.fence.write().unwrap_or_else(|p| p.into_inner());
        if current
            .as_ref()
            .is_some_and(|old| old.holder == fence.holder && old.fence == fence.fence)
        {
            *current = Some(fence.clone());
        }
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        self.hosting.transaction(self.session, |tx, state| {
            state.binding(self.session)?;
            object(tx, self.session, "framework", key)
        })
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.hosting.transaction(self.session,|tx,state| {
        state.binding(self.session)?;
        let mut query=tx.prepare("SELECT key,value FROM framework_objects WHERE session=? AND kind='framework' AND substr(key,1,?)=? ORDER BY key LIMIT 4097").map_err(error)?;
        let records=query.query_map(params![self.session.to_string(),prefix.chars().count() as i64,prefix],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(error)?
            .map(|row| { let (key,value)=row.map_err(error)?; Ok((key,decode(value)?)) }).collect::<Result<Vec<_>>>()?;
        if records.len()>4096 { return Err(error("lifecycle receipt scan limit exceeded")); } Ok(records)
    })
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        let written = self.hosting.transaction(self.session, |tx, state| {
            self.check(state)?;
            if absent && object::<Value>(tx, self.session, "framework", key)?.is_some() {
                return Ok(false);
            }
            if let Some(input) = key
                .strip_prefix("assignment/")
                .and_then(|id| id.parse::<MessageId>().ok())
                && let Some(position) = state.target_cancels.get(&input).copied()
            {
                let assignment: super::super::TurnAssignment =
                    serde_json::from_value(value.clone()).map_err(error)?;
                if state.open.is_none()
                    || state
                        .open
                        .is_some_and(|(turn, _)| turn == assignment.turn_id)
                {
                    state.cancel = Some(position);
                    state.cancel_reported = false;
                    state.runnable = true;
                }
            }
            if let Some(turn) = key
                .strip_prefix("outcome/")
                .and_then(|id| id.parse::<TurnId>().ok())
            {
                state.finalizing.remove(&turn);
            }
            put(tx, self.session, "framework", key, &value)?;
            Ok(true)
        })?;
        if written {
            self.hosting.changes.notify(self.session);
        }
        Ok(written)
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.hosting.transaction(self.session, |tx, state| {
            self.check(state)?;
            state.suspended = true;
            put(
                tx,
                self.session,
                "framework",
                "suspension",
                &json!({"reason":reason}),
            )
        })?;
        self.hosting.changes.notify(self.session);
        Ok(())
    }
}

#[async_trait]
impl SessionActorStore for Ports {
    async fn renew_actor_lease(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.scope(session)?;
        self.hosting.transaction(session, |_, state| {
            state.binding(session)?;
            if !state.owns(holder, fence) {
                return Ok(None);
            }
            if consumed < 0 || consumed > i64::from(state.head) {
                return Err(error("invalid consumed cursor"));
            }
            state.consumed = state.consumed.max(consumed);
            state.expires = now().saturating_add(ttl.as_millis() as u64);
            state.cancel_reported = state.cancel.is_some();
            Ok(Some(ActorRenewal {
                inbox_seq: state.inbox,
                cancel_inbox_seq: state.cancel,
            }))
        })
    }
    async fn release_actor(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<ActorRelease> {
        self.scope(session)?;
        self.hosting.transaction(session, |_, state| {
            state.binding(session)?;
            if !state.owns(holder, fence) {
                return Ok(ActorRelease::LeaseLost);
            }
            if consumed < 0 || consumed > i64::from(state.head) {
                return Err(error("invalid consumed cursor"));
            }
            state.consumed = state.consumed.max(consumed);
            state.holder = None;
            state.expires = 0;
            state.attempts = 0;
            state.runnable = !state.finalizing.is_empty()
                || state.resume_generation > state.resume_claimed
                || state.inbox > state.consumed
                || (state.cancel.is_some() && !state.cancel_reported);
            if state.cancel_reported {
                state.cancel = None;
            }
            state.cancel_reported = false;
            Ok(ActorRelease::Released {
                still_runnable: state.runnable,
            })
        })
    }
    async fn read_actor_journal(
        &self,
        session: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.scope(session)?;
        self.hosting.transaction(session,|tx,state| {
            state.binding(session)?;
            // Recovery reads the active branch: an abandoned turn is not the latest.
            let (ranges,base)=super::lineage::filter(state)?;
            let after=match after {Some(after)=>after,None=>tx.query_row(&format!("SELECT coalesce(max(seq)-1,0) FROM framework_events WHERE session=?1 AND type=?2 AND {}",super::lineage::ACTIVE),params![session.to_string(),"turn.started",ranges,base],|row|row.get(0)).map_err(error)?};
            let (events,more)=page(tx,session,state,after,state.head,if limit==0 {256}else{limit.min(1024) as usize})?;
            Ok(ActorJournal {events,has_more:more,head_sequence:(state.head>0).then_some(state.head)})
        })
    }
}

/// One page of the active branch; abandoned events keep their sequences but
/// are not context. Raw export reads the table directly.
fn page(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
    after: i32,
    through: i32,
    limit: usize,
) -> Result<(Vec<Event>, bool)> {
    let (ranges, base) = super::lineage::filter(state)?;
    let mut query = tx
        .prepare(&format!(
            "SELECT value FROM framework_events WHERE session=?1 AND seq>?2 AND {} AND seq<=?5 ORDER BY seq LIMIT ?6",
            super::lineage::ACTIVE
        ))
        .map_err(error)?;
    let rows = query
        .query_map(
            params![
                session.to_string(),
                after,
                ranges,
                base,
                through,
                (limit + 1) as i64
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(error)?;
    let mut events = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    for row in rows {
        let raw = row.map_err(error)?;
        if events.len() == limit || (bytes + raw.len() > 2 * 1024 * 1024 && !events.is_empty()) {
            more = true;
            break;
        }
        bytes += raw.len();
        events.push(decode(raw)?);
    }
    Ok((events, more))
}

#[async_trait]
impl EventReader for Ports {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        self.scope(request.session_id()).map_err(log_error)?;
        if request
            .cursor()
            .is_some_and(|cursor| cursor.session_id() != self.session)
        {
            return Err(EventLogError::CrossSessionCursor {
                detail: "authority cursor belongs to another session".into(),
            });
        }
        let after = request.cursor().map_or(0, EventCursor::after_sequence);
        let (events, more, through) = self
            .hosting
            .transaction(self.session, |tx, state| {
                state.binding(self.session)?;
                let through = request
                    .cursor()
                    .and_then(EventCursor::snapshot_high_watermark)
                    .unwrap_or(state.head);
                if through > state.head || after > through {
                    return Err(error("invalid authority event cursor"));
                }
                let (events, more) = page(
                    tx,
                    self.session,
                    state,
                    after,
                    through,
                    request.limit().get(),
                )?;
                Ok((events, more, through))
            })
            .map_err(log_error)?;
        let next = if more {
            let last = events
                .last()
                .and_then(|event| event.sequence)
                .ok_or_else(|| log_error("nonterminal authority page is empty"))?;
            Some(EventCursor::continuation(self.session, last, through)?)
        } else {
            None
        };
        EventPage::new(events, next, through)
    }
}
#[async_trait]
impl EventLog for Ports {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        self.scope(request.session_id).map_err(log_error)?;
        self.hosting
            .transaction(self.session, |tx, state| {
                if !request.is_dreaming_metadata() {
                    self.check(state)?;
                } else {
                    state.binding(self.session)?;
                }
                append(tx, state, request, None)
            })
            .map_err(log_error)
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.scope(request.session_id).map_err(log_error)?;
        // Preserve the typed conflict required by compaction's retry contract.
        self.hosting
            .transaction(self.session, |tx, state| {
                self.check(state)?;
                let actual = (state.head > 0).then_some(state.head);
                if actual != expected {
                    return Ok(Err(EventLogError::SequenceConflict {
                        session_id: self.session,
                        expected,
                        actual,
                    }));
                }
                let event = append(tx, state, request, Some(expected))?;
                Ok(Ok(event))
            })
            .map_err(log_error)?
    }
    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

#[async_trait]
impl SessionStore for Ports {
    async fn get_session(&self, session: SessionId) -> Result<Option<ExecutionSession>> {
        // A child session reads its ancestors' catalog records (subagent
        // depth, delegation results); nothing else crosses sessions.
        if session != self.session && !self.hosting.is_ancestor(self.session, session)? {
            self.scope(session)?;
        }
        self.hosting.transaction(session, |tx, state| {
            state.binding(session)?;
            let mut record: Option<ExecutionSession> = object(tx, session, "session", "record")?;
            let configuration: SessionConfiguration =
                object(tx, session, "framework", "configuration")?.unwrap_or_default();
            if let Some(record) = record.as_mut() {
                configuration.apply(record);
            }
            Ok(record)
        })
    }
}
#[async_trait]
impl RuntimeSessionStore for Ports {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.scope(session.id)?;
        self.hosting.transaction(self.session, |tx, state| {
            self.check(state)?;
            put(tx, self.session, "session", "record", &session)
        })
    }
}
#[async_trait]
impl SessionMutator for Ports {
    async fn update_session_title(
        &self,
        session: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.scope(session)?;
        self.hosting.transaction(session, |tx, state| {
            self.check(state)?;
            let mut record: ExecutionSession = object(tx, session, "session", "record")?
                .ok_or(AgentLoopError::SessionNotFound(session))?;
            record.title = Some(title);
            put(tx, session, "session", "record", &record)?;
            Ok(record)
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    id: uuid::Uuid,
    session_id: SessionId,
    source_sequence: i64,
    provider_type: String,
    model: String,
    format_version: u32,
    payload: CompactionCheckpointPayload,
    /// Active lineage prefix this checkpoint summarized (EVE-1305). Absent on
    /// checkpoints stored before lineage: they cover the linear journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lineage: Option<Vec<SequenceRange>>,
}
impl From<Checkpoint> for CompactionCheckpoint {
    fn from(c: Checkpoint) -> Self {
        Self {
            id: c.id,
            session_id: c.session_id,
            source_sequence: c.source_sequence,
            provider_type: c.provider_type,
            model: c.model,
            format_version: c.format_version,
            payload: c.payload,
        }
    }
}
/// Checkpoint slot key. Ranges enter as a digest to bound the key length.
pub(super) fn checkpoint_key(
    provider: &str,
    model: &str,
    format: u32,
    lineage: &[SequenceRange],
) -> String {
    json!([provider, model, format, history::lineage_digest(lineage)]).to_string()
}
/// Ancestor checkpoints retained per branch, so a rewind to an earlier point
/// still finds compatible compaction state without unbounded growth.
const CHECKPOINTS_PER_BRANCH: usize = 8;

/// Delete checkpoints of `c`'s provider/model/format that are no longer among
/// the newest eligible checkpoints of any branch once `c` is installed.
/// Returns whether `c` should be installed: not when an equal or newer
/// checkpoint already covers the active branch.
fn prune_checkpoints(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
    c: &CompactionCheckpoint,
    lineage: &[SequenceRange],
    key: &str,
) -> Result<bool> {
    let branches: Vec<Vec<SequenceRange>> = super::lineage::read(tx, session, state)?
        .branches
        .into_iter()
        .map(|branch| branch.ranges)
        .collect();
    let first = super::lineage::first_sequence(tx, session)?;
    let mut query = tx
        .prepare("SELECT key,value FROM framework_objects WHERE session=? AND kind='checkpoint' AND json_extract(value,'$.provider_type')=? AND json_extract(value,'$.model')=? AND json_extract(value,'$.format_version')=? LIMIT 4096")
        .map_err(error)?;
    let mut candidates: Vec<(Option<String>, i64, Option<Vec<SequenceRange>>)> = query
        .query_map(
            params![
                session.to_string(),
                c.provider_type,
                c.model,
                c.format_version
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(error)?
        .map(|row| {
            let (key, value) = row.map_err(error)?;
            let old = decode::<Checkpoint>(value)?;
            let prefix =
                super::lineage::summarized(old.lineage.as_deref(), old.source_sequence, first);
            Ok((Some(key), old.source_sequence, prefix))
        })
        .collect::<Result<_>>()?;
    // Existing rows win ties, so an equal checkpoint is not reinstalled.
    candidates.push((None, c.source_sequence, Some(lineage.to_vec())));
    let mut keep = vec![false; candidates.len()];
    for branch in &branches {
        let mut eligible: Vec<(usize, i64)> = candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, source, prefix))| {
                prefix
                    .as_deref()
                    .is_some_and(|prefix| super::lineage::covers(branch, *source, prefix))
            })
            .map(|(index, (_, source, _))| (index, *source))
            .collect();
        // Newest first; on a tie the existing row stays.
        eligible.sort_by_key(|(index, source)| (std::cmp::Reverse(*source), *index));
        for (index, _) in eligible.into_iter().take(CHECKPOINTS_PER_BRANCH) {
            keep[index] = true;
        }
    }
    // The new checkpoint is redundant when an equal or newer one already
    // covers the active branch at its prefix.
    let install = keep.last().copied().unwrap_or(false)
        && !candidates[..candidates.len() - 1]
            .iter()
            .any(|(_, source, prefix)| {
                *source >= c.source_sequence
                    && prefix
                        .as_deref()
                        .is_some_and(|prefix| super::lineage::covers(&branches[0], *source, prefix))
            });
    for ((old, ..), keep) in candidates.iter().zip(&keep) {
        if let Some(old) = old
            && (!keep || (install && old == key))
        {
            tx.execute(
                "DELETE FROM framework_objects WHERE session=? AND kind='checkpoint' AND key=?",
                params![session.to_string(), old],
            )
            .map_err(error)?;
        }
    }
    Ok(install)
}
impl Ports {
    fn checkpoint(
        &self,
        session: SessionId,
        provider: &str,
        model: &str,
        format: Option<u32>,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.scope(session)?;
        self.hosting.transaction(session,|tx,state| {
        state.binding(session)?;
        // Newest checkpoint whose summarized prefix is the active branch's own
        // prefix at its source: ancestors stay eligible, abandoned suffixes never.
        let active=super::lineage::active_ranges(tx,session,state)?;
        let first=super::lineage::first_sequence(tx,session)?;
        let mut query=tx.prepare("SELECT value FROM framework_objects WHERE session=? AND kind='checkpoint' AND json_extract(value,'$.provider_type')=? AND json_extract(value,'$.model')=? AND (? IS NULL OR json_extract(value,'$.format_version')=?) ORDER BY json_extract(value,'$.source_sequence') DESC LIMIT 4096").map_err(error)?;
        let rows=query.query_map(params![session.to_string(),provider,model,format,format],|row|row.get::<_,String>(0)).map_err(error)?;
        for row in rows {
            let checkpoint=decode::<Checkpoint>(row.map_err(error)?)?;
            if super::lineage::summarized(checkpoint.lineage.as_deref(),checkpoint.source_sequence,first)
                .is_some_and(|prefix|super::lineage::covers(&active,checkpoint.source_sequence,&prefix))
            {
                return Ok(Some(checkpoint.into()));
            }
        }
        Ok(None)
    })
    }
}
#[async_trait]
impl CompactionCheckpointStore for Ports {
    async fn get_latest(
        &self,
        session: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.checkpoint(session, provider, model, None)
    }
    async fn get_latest_format(
        &self,
        session: SessionId,
        provider: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.checkpoint(session, provider, model, Some(format))
    }
    async fn install(&self, c: CompactionCheckpoint) -> Result<bool> {
        self.scope(c.session_id)?;
        self.hosting.transaction(self.session, |tx, state| {
            self.check(state)?;
            if c.source_sequence > i64::from(state.head) {
                return Err(error("checkpoint exceeds canonical journal"));
            }
            // Slots are per lineage prefix, so a branch never overwrites another
            // branch's compaction state. Pruning keeps only checkpoints that are
            // the newest eligible one for some retained branch.
            let active = super::lineage::active_ranges(tx, self.session, state)?;
            let lineage = i32::try_from(c.source_sequence)
                .ok()
                .and_then(|source| history::clip(&active, source))
                .ok_or_else(|| error("checkpoint source is not on the active branch"))?;
            let key = checkpoint_key(&c.provider_type, &c.model, c.format_version, &lineage);
            if !prune_checkpoints(tx, self.session, state, &c, &lineage, &key)? {
                return Ok(false);
            }
            let stored = Checkpoint {
                id: c.id,
                session_id: c.session_id,
                source_sequence: c.source_sequence,
                provider_type: c.provider_type,
                model: c.model,
                format_version: c.format_version,
                payload: c.payload,
                lineage: Some(lineage),
            };
            put(tx, self.session, "checkpoint", &key, &stored)?;
            Ok(true)
        })
    }
    async fn get_proactive_attempt(
        &self,
        session: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.scope(session)?;
        Ok(self
            .hosting
            .proactive
            .lock()
            .map_err(error)?
            .get(&(session, provider.into(), model.into()))
            .copied())
    }
    async fn record_proactive_attempt(
        &self,
        session: SessionId,
        provider: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.scope(session)?;
        self.hosting.transaction(session, |_, state| {
            self.check(state)?;
            let mut cache = self.hosting.proactive.lock().map_err(error)?;
            if cache.len() >= 1024 {
                cache.clear();
            }
            cache.insert((session, provider.into(), model.into()), attempt);
            Ok(())
        })
    }
}

#[async_trait]
impl SessionStorageStore for Ports {
    async fn get_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.hosting.transaction(session, |tx, state| {
            state.binding(session)?;
            object(tx, session, "value", key)
        })
    }
    async fn set_value(&self, session: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.hosting.transaction(session, |tx, state| {
            self.check(state)?;
            put(tx, session, "value", key, &value)
        })
    }
    async fn take_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.hosting.transaction(session, |tx, state| {
            self.check(state)?;
            let value = object(tx, session, "value", key)?;
            tx.execute(
                "DELETE FROM framework_objects WHERE session=? AND kind='value' AND key=?",
                params![session.to_string(), key],
            )
            .map_err(error)?;
            Ok(value)
        })
    }
    async fn delete_value(&self, session: SessionId, key: &str) -> Result<bool> {
        Ok(self.take_value(session, key).await?.is_some())
    }
    async fn list_keys(&self, session: SessionId) -> Result<Vec<KeyInfo>> {
        self.scope(session)?;
        self.hosting.transaction(session,|tx,state| {state.binding(session)?;
        let mut query=tx.prepare("SELECT key,created,updated FROM framework_objects WHERE session=? AND kind='value' ORDER BY key LIMIT 1025").map_err(error)?;
        let keys=query.query_map([session.to_string()],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).map_err(error)?
            .map(|row| {let(key,created,updated)=row.map_err(error)?;Ok(KeyInfo{key,created_at:created.parse().map_err(error)?,updated_at:updated.parse().map_err(error)?})}).collect::<Result<Vec<_>>>()?;
        if keys.len()>1024 {return Err(error("session key capacity exceeded"));}Ok(keys)
    })
    }
    async fn set_secret(&self, session: SessionId, name: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        let sealed = self.hosting.secret_keys()?.seal(session, name, value)?;
        self.hosting.transaction(session, |tx, state| {
            self.check(state)?;
            put(tx, session, "secret", name, &sealed)
        })
    }
    async fn get_secret(&self, session: SessionId, name: &str) -> Result<Option<String>> {
        self.scope(session)?;
        let keys = self.hosting.secret_keys()?;
        let sealed: Option<super::secrets::SealedSecret> =
            self.hosting.transaction(session, |tx, state| {
                state.binding(session)?;
                object(tx, session, "secret", name)
            })?;
        sealed
            .map(|sealed| keys.open(session, name, &sealed))
            .transpose()
    }
    async fn delete_secret(&self, session: SessionId, name: &str) -> Result<bool> {
        self.scope(session)?;
        self.hosting.secret_keys()?;
        self.hosting.transaction(session, |tx, state| {
            self.check(state)?;
            let deleted = tx
                .execute(
                    "DELETE FROM framework_objects WHERE session=? AND kind='secret' AND key=?",
                    params![session.to_string(), name],
                )
                .map_err(error)?;
            Ok(deleted > 0)
        })
    }
    async fn list_secrets(&self, session: SessionId) -> Result<Vec<SecretInfo>> {
        self.scope(session)?;
        self.hosting.secret_keys()?;
        list_secret_infos(&self.hosting, session)
    }
}

/// Secret names and timestamps, never values.
pub(super) fn list_secret_infos(
    hosting: &SqliteHosting,
    session: SessionId,
) -> Result<Vec<SecretInfo>> {
    hosting.transaction(session, |tx, state| {
        state.binding(session)?;
        let mut query = tx
            .prepare("SELECT key,created,updated FROM framework_objects WHERE session=? AND kind='secret' ORDER BY key LIMIT 1025")
            .map_err(error)?;
        let secrets = query
            .query_map([session.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(error)?
            .map(|row| {
                let (name, created, updated) = row.map_err(error)?;
                Ok(SecretInfo {
                    name,
                    created_at: created.parse().map_err(error)?,
                    updated_at: updated.parse().map_err(error)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if secrets.len() > 1024 {
            return Err(error("session secret capacity exceeded"));
        }
        Ok(secrets)
    })
}
