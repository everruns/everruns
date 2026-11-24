//! Versioned delivery projection state, excluding the credential-bearing context.
use super::*;
use crate::channel_runtime::{ChannelError, ChannelStore};

#[derive(Serialize, Deserialize)]
struct DeliveryState {
    boundary_seen: bool,
    delivered: bool,
    finished: bool,
    tools_running: usize,
    last_status: Option<String>,
    streams: BTreeMap<String, Stream>,
    done: HashSet<String>,
    progress: TaskProgress,
    unstreamable: HashSet<String>,
    notes: HashSet<String>,
    sent_explicitly: bool,
    failure_code: Option<String>,
    keys: HashMap<String, String>,
    #[serde(default)]
    pending_links: Vec<(String, String)>,
    last_sequence: Option<i64>,
    effects_done: HashSet<String>,
}

impl TurnDelivery {
    pub(in crate::channel_runtime) async fn enable_checkpoints(
        &mut self,
        store: Arc<dyn ChannelStore>,
        holder: String,
        store_session_id: &str,
    ) -> Result<(), ChannelError> {
        let checkpoint = store
            .delivery_checkpoint(store_session_id, &self.input_message_id)
            .await?
            .unwrap_or_default();
        if checkpoint.version != 1 {
            return Err(ChannelError::Session(
                "unsupported delivery checkpoint version".into(),
            ));
        }
        if !checkpoint.state.is_null() {
            let state: DeliveryState = serde_json::from_value(checkpoint.state.clone())
                .map_err(|_| ChannelError::Session("invalid delivery state".into()))?;
            if state.streams.values().any(|stream| {
                stream.sent > stream.text.len() || !stream.text.is_char_boundary(stream.sent)
            }) {
                return Err(ChannelError::Session(
                    "invalid delivery stream prefix".into(),
                ));
            }
            self.boundary_seen = state.boundary_seen;
            self.delivered = state.delivered;
            self.finished = state.finished;
            self.tools_running = state.tools_running;
            self.last_status = state.last_status;
            self.streams = state.streams;
            self.done = state.done;
            self.progress = state.progress;
            self.unstreamable = state.unstreamable;
            self.notes = state.notes;
            self.sent_explicitly = state.sent_explicitly;
            self.failure_code = state.failure_code;
            self.keys = state.keys;
            self.new_links = state.pending_links;
            self.last_sequence = state.last_sequence;
            self.effects_done = state.effects_done;
            if !checkpoint.retries.is_empty()
                || checkpoint
                    .in_flight
                    .as_ref()
                    .is_some_and(|attempt| attempt.idempotent)
            {
                self.finished = false;
            }
            if !self.streams.is_empty() && self.adapter.streaming().is_none() {
                return Err(ChannelError::Session(
                    "pending native streams require their original surface".into(),
                ));
            }
        }
        // Recreate idempotent link writes that may have been interrupted after the
        // output state committed but before the separate message-link transaction.
        self.new_links
            .extend(self.keys.iter().map(|(id, key)| (key.clone(), id.clone())));
        self.new_links.sort();
        self.new_links.dedup();
        // A receipt can commit before the consumer commits its new handle. Recover
        // that handle for cleanup, never to replay the uncertain stream contents.
        for attempt in checkpoint
            .in_flight
            .iter()
            .chain(checkpoint.uncertain.iter())
        {
            if let Some(message_id) = attempt
                .key
                .strip_prefix("reply:")
                .and_then(|key| key.strip_suffix(":post"))
                .filter(|id| *id != "notice")
            {
                for key in &attempt.confirmed {
                    self.keys
                        .entry(message_id.into())
                        .or_insert_with(|| key.clone());
                    self.new_links.push((key.clone(), message_id.into()));
                }
            }

            if let Some(message_id) = attempt
                .key
                .strip_prefix("reply:")
                .and_then(|key| key.strip_suffix(":start"))
                && let Some(handle) = attempt.confirmed.first()
            {
                self.streams
                    .entry(message_id.into())
                    .or_insert_with(|| Stream {
                        handle: handle.clone(),
                        text: String::new(),
                        sent: 0,
                        replacement: None,
                    });
                if let Some(reactions) = self.adapter.reactions() {
                    let key = reactions.stream_message_key(handle, &self.context);
                    self.keys
                        .entry(message_id.into())
                        .or_insert_with(|| key.clone());
                    self.new_links.push((key, message_id.into()));
                }
            }
            if attempt.key == "progress:post"
                && self.progress.handle().is_none()
                && let Some(handle) = attempt.confirmed.first()
            {
                self.progress.mark_posted(handle.clone());
            }
        }
        let journal = Arc::new(DeliveryJournal::new(
            store,
            store_session_id.to_string(),
            self.input_message_id.clone(),
            holder,
            checkpoint,
        ));
        self.context.receipts = Some(journal.clone());
        self.journal = Some(journal);
        Ok(())
    }

    pub(super) fn snapshot(&self) -> Value {
        // State contains output and scoped identifiers, never the credential-bearing context.
        serde_json::to_value(DeliveryState {
            boundary_seen: self.boundary_seen,
            delivered: self.delivered,
            finished: self.finished,
            tools_running: self.tools_running,
            last_status: self.last_status.clone(),
            streams: self.streams.clone(),
            done: self.done.clone(),
            progress: self.progress.clone(),
            unstreamable: self.unstreamable.clone(),
            notes: self.notes.clone(),
            sent_explicitly: self.sent_explicitly,
            failure_code: self.failure_code.clone(),
            keys: self.keys.clone(),
            pending_links: self.new_links.clone(),
            last_sequence: self.last_sequence,
            effects_done: self.effects_done.clone(),
        })
        .unwrap_or(Value::Null)
    }

    pub(super) async fn before_write(
        &self,
        key: impl Into<String>,
        idempotent: bool,
    ) -> Result<(), DeliveryResult> {
        match &self.journal {
            Some(journal) => journal.begin(self.snapshot(), key.into(), idempotent).await,
            None => Ok(()),
        }
    }

    pub(super) async fn after_write(&self, result: &DeliveryResult) {
        if let Some(journal) = &self.journal {
            journal.outcome(result).await;
        }
    }

    pub(super) async fn checkpoint(&self) {
        if let Some(journal) = &self.journal
            && (self.finished || journal.has_changes())
            && let Err(error) = journal.save(self.snapshot()).await
        {
            warn!(session_id = %self.session_id, %error, "channel: output checkpoint failed; writes stopped");
        }
    }

    pub(in crate::channel_runtime) async fn close_uncertain_streams(&mut self) {
        // The journal rejects non-idempotent appends while uncertain. Only known
        // handle closure/replacement and final progress replacement can reach I/O.
        for id in self.streams.keys().cloned().collect::<Vec<_>>() {
            self.close_stream(&id).await;
        }
        if self.progress.handle().is_some() {
            self.push_progress(true).await;
        }
        self.checkpoint().await;
    }

    pub(in crate::channel_runtime) async fn needs_recovery(&self) -> bool {
        match &self.journal {
            Some(journal) => journal.pending().await,
            None => false,
        }
    }

    pub(in crate::channel_runtime) async fn needs_reconciliation(&self) -> bool {
        match &self.journal {
            Some(journal) => journal.blocked().await,
            None => false,
        }
    }
}
