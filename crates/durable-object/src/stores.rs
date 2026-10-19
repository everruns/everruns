use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns::hosting::spi::actor::{ActorJournal, ActorRelease, ActorRenewal, SessionActorStore};
use everruns::hosting::spi::error::{AgentLoopError, Result};
use everruns::hosting::spi::events::{Event, EventRequest};
use everruns::hosting::spi::ids::SessionId;
use everruns::hosting::spi::loading::SessionStore;
use everruns::hosting::spi::storage::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns::hosting::spi::{
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore, ExecutionSession,
    ProactiveCompactionAttempt,
};
use everruns::hosting::spi::{
    EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
    EventReader, HostBackends, RuntimeSessionStore, SessionMutator,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Authority;

fn log_error(error: AgentLoopError) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

impl Authority {
    async fn checkpoint(
        &self,
        session_id: SessionId,
        provider: &str,
        model: &str,
        format: Option<u32>,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.check_session(session_id)?;
        let value: Option<Checkpoint> = self
            .store(
                "checkpoint",
                "latest",
                &json!([provider, model]).to_string(),
                json!(format),
            )
            .await?;
        Ok(value.map(|c| CompactionCheckpoint {
            id: c.id,
            session_id: c.session_id,
            source_sequence: c.source_sequence,
            provider_type: c.provider_type,
            model: c.model,
            format_version: c.format_version,
            payload: c.payload,
        }))
    }
    /// Coherent session stores; agent/provider definitions are reconstructed from
    /// the retained build. Workspace durability is supplied separately by a host.
    pub fn backends(&self) -> HostBackends {
        let mut backends = HostBackends::in_memory();
        backends.dreaming_store = self.client.dreaming_store.clone();
        backends.event_log = Arc::new(self.clone());
        backends.session_store = Arc::new(self.clone());
        backends.compaction_checkpoint_store = Arc::new(self.clone());
        backends.storage_store = Arc::new(self.clone());
        backends
    }

    async fn append_checked(
        &self,
        event: EventRequest,
        conditional: bool,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.check_session(event.session_id).map_err(log_error)?;
        if event.is_ephemeral() || event.event_type != event.data.event_type() {
            return Err(EventLogError::InvalidAppend {
                detail: "only typed canonical events can be appended".to_owned(),
            });
        }
        #[derive(Deserialize)]
        struct Appended {
            event: Option<Event>,
            actual: Option<i32>,
            conflict: bool,
        }
        let result: Appended = self
            .call(
                "append",
                json!({"event": event, "conditional": conditional, "expected": expected}),
            )
            .await
            .map_err(log_error)?;
        if result.conflict {
            return Err(EventLogError::SequenceConflict {
                session_id: self.session_id,
                expected,
                actual: result.actual,
            });
        }
        result.event.ok_or_else(|| EventLogError::Corruption {
            detail: "append response omitted canonical event".to_owned(),
        })
    }

    async fn store<T: serde::de::DeserializeOwned>(
        &self,
        kind: &str,
        operation: &str,
        key: &str,
        value: Value,
    ) -> Result<T> {
        self.call(
            "store",
            json!({"kind": kind, "operation": operation, "key": key, "value": value}),
        )
        .await
    }
}

#[async_trait]
impl EventReader for Authority {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        self.check_session(request.session_id())
            .map_err(log_error)?;
        if request
            .cursor()
            .is_some_and(|cursor| cursor.session_id() != self.session_id)
        {
            return Err(EventLogError::CrossSessionCursor {
                detail: "authority cursor belongs to another session".to_owned(),
            });
        }
        let after = request.cursor().map_or(0, EventCursor::after_sequence);
        let through = request
            .cursor()
            .and_then(EventCursor::snapshot_high_watermark);
        let batch = self
            .events(after, through, request.limit().get())
            .await
            .map_err(log_error)?;
        let next = if batch.has_more {
            let last = batch
                .events
                .last()
                .and_then(|event| event.sequence)
                .ok_or_else(|| EventLogError::Corruption {
                    detail: "nonterminal event page is empty".to_owned(),
                })?;
            Some(EventCursor::continuation(
                self.session_id,
                last,
                batch.high_watermark,
            )?)
        } else {
            None
        };
        EventPage::new(batch.events, next, batch.high_watermark)
    }
}

#[async_trait]
impl EventLog for Authority {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        self.append_checked(request, false, None).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.append_checked(request, true, expected).await
    }
    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

#[async_trait]
impl SessionActorStore for Authority {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.check_session(session_id)?;
        self.call("renew", json!({"holder": holder, "fence": fence, "ttl_ms": ttl.as_millis(), "consumed_seq": consumed_seq})).await
    }
    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_seq: i64,
    ) -> Result<ActorRelease> {
        self.check_session(session_id)?;
        self.call(
            "release",
            json!({"holder": holder, "fence": fence, "consumed_seq": consumed_seq}),
        )
        .await
    }
    async fn read_actor_journal(
        &self,
        session_id: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.check_session(session_id)?;
        self.call("journal", json!({"after": after, "limit": limit}))
            .await
    }
}

#[async_trait]
impl SessionStore for Authority {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.check_session(session_id)?;
        #[derive(Deserialize)]
        struct ExecutionView {
            execution: Option<ExecutionSession>,
            configuration: everruns::hosting::SessionConfiguration,
        }
        let mut view: ExecutionView = self.call("execution", json!({})).await?;
        if let Some(record) = view.execution.as_mut() {
            view.configuration.apply(record);
        }
        Ok(view.execution)
    }
}

#[async_trait]
impl RuntimeSessionStore for Authority {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.check_session(session.id)?;
        self.store::<Value>("session", "set", "record", json!(session))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl SessionMutator for Authority {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.check_session(session_id)?;
        self.store("session", "title", "record", json!(title)).await
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
}

#[async_trait]
impl CompactionCheckpointStore for Authority {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.checkpoint(session_id, provider, model, None).await
    }
    async fn get_latest_format(
        &self,
        session_id: SessionId,
        provider: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.checkpoint(session_id, provider, model, Some(format))
            .await
    }
    async fn install(&self, c: CompactionCheckpoint) -> Result<bool> {
        self.check_session(c.session_id)?;
        let key = json!([c.provider_type, c.model, c.format_version]).to_string();
        self.store(
            "checkpoint",
            "install",
            &key,
            json!(Checkpoint {
                id: c.id,
                session_id: c.session_id,
                source_sequence: c.source_sequence,
                provider_type: c.provider_type,
                model: c.model,
                format_version: c.format_version,
                payload: c.payload
            }),
        )
        .await
    }
    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.check_session(session_id)?;
        Ok(self
            .proactive
            .lock()
            .await
            .get(&(provider.to_owned(), model.to_owned()))
            .copied())
    }
    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.check_session(session_id)?;
        let mut attempts = self.proactive.lock().await;
        if attempts.len() >= 1024 {
            attempts.clear();
        }
        attempts.insert((provider.to_owned(), model.to_owned()), attempt);
        Ok(())
    }
}

#[async_trait]
impl SessionStorageStore for Authority {
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        self.check_session(session_id)?;
        self.store::<Value>("value", "set", key, json!(value))
            .await?;
        Ok(())
    }
    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        self.check_session(session_id)?;
        self.store("value", "get", key, Value::Null).await
    }
    async fn take_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        self.check_session(session_id)?;
        self.store("value", "take", key, Value::Null).await
    }
    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        self.check_session(session_id)?;
        self.store("value", "delete", key, Value::Null).await
    }
    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        self.check_session(session_id)?;
        #[derive(Deserialize)]
        struct Key {
            key: String,
            created_at: chrono::DateTime<chrono::Utc>,
            updated_at: chrono::DateTime<chrono::Utc>,
        }
        let keys: Vec<Key> = self.store("value", "list", "", Value::Null).await?;
        Ok(keys
            .into_iter()
            .map(|key| KeyInfo {
                key: key.key,
                created_at: key.created_at,
                updated_at: key.updated_at,
            })
            .collect())
    }
    async fn set_secret(&self, _session_id: SessionId, _name: &str, _value: &str) -> Result<()> {
        Err(AgentLoopError::config(
            "object profile requires an external encrypted secret store",
        ))
    }
    async fn get_secret(&self, _session_id: SessionId, _name: &str) -> Result<Option<String>> {
        Err(AgentLoopError::config(
            "object profile requires an external encrypted secret store",
        ))
    }
    async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> Result<bool> {
        Err(AgentLoopError::config(
            "object profile requires an external encrypted secret store",
        ))
    }
    async fn list_secrets(&self, _session_id: SessionId) -> Result<Vec<SecretInfo>> {
        Err(AgentLoopError::config(
            "object profile requires an external encrypted secret store",
        ))
    }
}
