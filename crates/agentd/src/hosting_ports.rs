//! Engine's session record, session storage, compaction checkpoint and
//! event log ports over the shard. Split from `hosting.rs` for size.

use super::*;

#[async_trait]
impl everruns_core::execution_loading::SessionStore for Ports {
    async fn get_session(&self, session: SessionId) -> Result<Option<ExecutionSession>> {
        // Another session reads as missing, not as a failed turn: a child's
        // turn-tool lookups (`finish` for a spawned agent) read its parent's
        // record, which these ports never hold.
        if self.scope(session).is_err() {
            return Ok(None);
        }
        let (record, configuration, parent) = self
            .inner
            .read(session, |s| {
                (
                    s.authority.execution.clone(),
                    s.authority.configuration(),
                    s.platform
                        .origin
                        .as_ref()
                        .map(|origin| origin.parent_session_id.clone()),
                )
            })
            .await?;
        let Some(record) = record else {
            return Ok(None);
        };
        let mut record: ExecutionSession = serde_json::from_value(record).map_err(store_err)?;
        let configuration: everruns::hosting::SessionConfiguration =
            serde_json::from_value(configuration).map_err(store_err)?;
        configuration.apply(&mut record);
        // Decision: a child's parent is the one its create named
        // (`platform.origin`), since the Engine's own record of a session the
        // gateway created knows none. Parent tools (`finish`, `ask_parent`,
        // ...) find a thread's assignment through it.
        if record.parent_session_id.is_none() {
            record.parent_session_id = parent.and_then(|id| id.parse().ok());
        }
        Ok(Some(record))
    }
}

#[async_trait]
impl RuntimeSessionStore for Ports {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.scope(session.id)?;
        self.write(WriteOp::Execution {
            session: serde_json::to_value(session).map_err(store_err)?,
        })
        .await
        .map(|_| ())
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
        let record = self.write(WriteOp::Title { title }).await?;
        serde_json::from_value(record).map_err(store_err)
    }
}

/// Session secrets are kept sealed with the org's data key (design.md,
/// "Session secrets"): an executor seals what the agent stores and opens
/// what it reads, the control plane seals what a person stores, and shard
/// nodes, the log and its shipped copies only ever hold ciphertext.
///
/// A standalone shard (no control plane, so no `AGENTD_DP_KEY` and no org
/// data keys) keeps them as written, as before sealing: there is nothing
/// to seal with, and its sandboxes keep their state in a session secret.
impl Ports {
    async fn data_key(
        &self,
    ) -> Result<Option<(everruns_contracts::dataplane_seal::Key, String, String)>> {
        let material = self
            .inner
            .material
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AgentLoopError::store("session secrets open only on an executor"))?;
        if !material.seals() {
            return Ok(None);
        }
        let org = material
            .tenant(self.session)
            .ok_or_else(|| AgentLoopError::store("the session has no org"))?;
        let key = material.data_key(&org).await.map_err(store_err)?;
        Ok(key.map(|(key, key_id)| (key, key_id, org)))
    }

    async fn seal_secret(&self, name: &str, value: &str) -> Result<String> {
        let Some((key, key_id, org)) = self.data_key().await? else {
            return Ok(value.to_owned());
        };
        everruns_contracts::dataplane_seal::seal_session_secret(
            &key,
            &key_id,
            &org,
            &self.session.to_string(),
            name,
            value,
        )
        .map_err(store_err)
    }

    async fn open_secret(&self, name: &str, stored: &str) -> Result<String> {
        let Some((key, key_id, org)) = self.data_key().await? else {
            return Ok(stored.to_owned());
        };
        everruns_contracts::dataplane_seal::open_session_secret(
            &key,
            &key_id,
            &org,
            &self.session.to_string(),
            name,
            stored,
        )
        .map_err(store_err)
    }
}

fn key_info(key: &str) -> KeyInfo {
    let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    KeyInfo {
        key: key.to_owned(),
        created_at: epoch,
        updated_at: epoch,
    }
}

#[async_trait]
impl SessionStorageStore for Ports {
    async fn set_value(&self, session: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.write(WriteOp::SetValue {
            key: key.to_owned(),
            value: value.to_owned(),
        })
        .await
        .map(|_| ())
    }
    async fn get_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| s.authority.values.get(key).cloned())
            .await
    }
    async fn take_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::TakeValue {
                key: key.to_owned(),
            })
            .await?;
        Ok(reply.as_str().map(str::to_owned))
    }
    async fn delete_value(&self, session: SessionId, key: &str) -> Result<bool> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::DeleteValue {
                key: key.to_owned(),
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }
    async fn list_keys(&self, session: SessionId) -> Result<Vec<KeyInfo>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| {
                s.authority.values.keys().map(|k| key_info(k)).collect()
            })
            .await
    }
    async fn set_secret(&self, session: SessionId, name: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        let value = self.seal_secret(name, value).await?;
        self.write(WriteOp::SetSecret {
            key: name.to_owned(),
            value,
        })
        .await
        .map(|_| ())
    }
    async fn get_secret(&self, session: SessionId, name: &str) -> Result<Option<String>> {
        self.scope(session)?;
        let stored = self
            .inner
            .read(session, |s| s.authority.secrets.get(name).cloned())
            .await?;
        match stored {
            Some(stored) => self.open_secret(name, &stored).await.map(Some),
            None => Ok(None),
        }
    }
    async fn delete_secret(&self, session: SessionId, name: &str) -> Result<bool> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::DeleteSecret {
                key: name.to_owned(),
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }
    async fn list_secrets(&self, session: SessionId) -> Result<Vec<SecretInfo>> {
        self.scope(session)?;
        let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        self.inner
            .read(session, |s| {
                s.authority
                    .secrets
                    .keys()
                    .map(|name| SecretInfo {
                        name: name.clone(),
                        created_at: epoch,
                        updated_at: epoch,
                    })
                    .collect()
            })
            .await
    }
}

fn to_engine(session: SessionId, c: &Checkpoint) -> CompactionCheckpoint {
    CompactionCheckpoint {
        id: c.id,
        session_id: session,
        source_sequence: c.source_sequence,
        provider_type: c.provider_type.clone(),
        model: c.model.clone(),
        format_version: c.format_version,
        payload: c.payload.clone(),
    }
}

impl Ports {
    async fn latest(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        format: Option<u32>,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| {
                s.checkpoints
                    .iter()
                    .filter(|c| c.provider_type == provider_type && c.model == model)
                    .filter(|c| format.is_none_or(|f| c.format_version == f))
                    .max_by_key(|c| c.source_sequence)
                    .map(|c| to_engine(session, c))
            })
            .await
    }
}

#[async_trait]
impl CompactionCheckpointStore for Ports {
    async fn get_latest(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.latest(session, provider_type, model, None).await
    }

    async fn get_latest_format(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.latest(session, provider_type, model, Some(format))
            .await
    }

    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
        if checkpoint.session_id != self.session {
            return Ok(false);
        }
        let reply = self
            .write(WriteOp::Checkpoint {
                checkpoint: Checkpoint {
                    id: checkpoint.id,
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload: checkpoint.payload,
                },
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }

    async fn latest_source_sequence(&self, session: SessionId) -> Result<Option<i64>> {
        if session != self.session {
            return Ok(None);
        }
        self.inner
            .read(session, |s| {
                s.checkpoints.iter().map(|c| c.source_sequence).max()
            })
            .await
    }

    async fn get_proactive_attempt(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.scope(session)?;
        let key = format!("{provider_type}/{model}");
        self.inner
            .read(session, |s| {
                s.authority
                    .proactive
                    .get(&key)
                    .map(|a| ProactiveCompactionAttempt {
                        source_sequence: a.source_sequence,
                        estimated_input_tokens: a.estimated_input_tokens,
                        input_message_count: a.input_message_count,
                        source_fingerprint: a.source_fingerprint,
                    })
            })
            .await
    }

    async fn record_proactive_attempt(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.scope(session)?;
        self.write(WriteOp::Proactive {
            key: format!("{provider_type}/{model}"),
            attempt: ProactiveAttempt {
                source_sequence: attempt.source_sequence,
                estimated_input_tokens: attempt.estimated_input_tokens,
                input_message_count: attempt.input_message_count,
                source_fingerprint: attempt.source_fingerprint,
            },
        })
        .await
        .map(|_| ())
    }
}

#[async_trait]
impl EventReader for Ports {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        let backend = |detail: String| EventLogError::Backend { detail };
        self.scope(request.session_id())
            .map_err(|e| backend(e.to_string()))?;
        let after = request.cursor().map_or(0, |c| c.after_sequence());
        let limit = request.limit().get();
        let t = std::time::Instant::now();
        let (events, high, partial) = self
            .window(after, limit)
            .await
            .map_err(|e| backend(e.to_string()))?;
        crate::prof::add("read_page", t, 1);
        crate::prof::add(
            "read_page_events",
            std::time::Instant::now(),
            events.len() as u64,
        );
        read_page_at(&events, request, high, partial)
    }

    async fn last_sequence(
        &self,
        session: SessionId,
    ) -> std::result::Result<Option<i32>, EventLogError> {
        if session != self.session {
            return Ok(None);
        }
        let high = self
            .inner
            .read(session, |s| s.last_sequence().unwrap_or(0))
            .await
            .map_err(|e| EventLogError::Backend {
                detail: e.to_string(),
            })?;
        let pending = self.pending_after(high);
        Ok(Some(
            pending
                .last()
                .and_then(|e| e.sequence)
                .unwrap_or(high)
                .max(high),
        ))
    }
}

#[async_trait]
impl EventLog for Ports {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, None).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, Some(expected)).await
    }
    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}
