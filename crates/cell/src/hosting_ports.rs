//! Engine's session record, session storage and compaction checkpoint
//! ports over the cell. Split from `hosting.rs` for size.

use super::*;

#[async_trait]
impl everruns_core::execution_loading::SessionStore for Ports {
    async fn get_session(&self, session: SessionId) -> Result<Option<ExecutionSession>> {
        self.scope(session)?;
        let (record, configuration) = self
            .inner
            .read(session, |s| {
                (s.authority.execution.clone(), s.authority.configuration())
            })
            .await?;
        let Some(record) = record else {
            return Ok(None);
        };
        let mut record: ExecutionSession = serde_json::from_value(record).map_err(store_err)?;
        let configuration: everruns::hosting::SessionConfiguration =
            serde_json::from_value(configuration).map_err(store_err)?;
        configuration.apply(&mut record);
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
        self.write(WriteOp::SetSecret {
            key: name.to_owned(),
            value: value.to_owned(),
        })
        .await
        .map(|_| ())
    }
    async fn get_secret(&self, session: SessionId, name: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| s.authority.secrets.get(name).cloned())
            .await
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
