//! Session-scoped store and event adapters.
use super::*;

// --- Session-scoped trait impls ---

#[async_trait]
impl<A: WorkerAdapters> crate::core::MessageRetriever for SessionAdapter<A> {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<RuntimeMessage>> {
        self.adapters
            .get_message(session_id.uuid(), message_id.uuid())
            .await
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<RuntimeMessage>> {
        self.adapters.load_messages(session_id.uuid()).await
    }

    async fn load_filtered(&self, query: crate::core::MessageQuery) -> Result<Vec<RuntimeMessage>> {
        Ok(self.adapters.load_message_history(query).await?.messages)
    }

    async fn load_filtered_history(
        &self,
        query: crate::core::MessageQuery,
    ) -> Result<crate::core::MessageHistory> {
        self.adapters.load_message_history(query).await
    }
}

#[async_trait]
impl<A: WorkerAdapters> crate::core::event_emitter::EventEmitter for SessionAdapter<A> {
    fn turn_causal_context(
        &self,
        context: &crate::core::events::EventContext,
    ) -> crate::core::causal::CausalContext {
        self.causal_trace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|(turn, _)| Some(*turn) == context.turn_id)
            .map(|(_, metadata)| {
                crate::core::causal::CausalContext::from_turn_metadata(Some(metadata))
            })
            .unwrap_or_default()
    }
    fn restore_turn_context(
        &self,
        context: &crate::core::events::EventContext,
        metadata: Option<&serde_json::Value>,
    ) {
        if let (Some(turn), Some(value)) = (
            context.turn_id,
            metadata.and_then(|m| m.get(crate::core::causal::TURN_TRACEPARENT_KEY)),
        ) && !crate::core::causal::CausalContext::from_turn_metadata(metadata).is_empty()
        {
            *self
                .causal_trace
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((
                turn,
                serde_json::json!({crate::core::causal::TURN_TRACEPARENT_KEY:value}),
            ));
        }
    }

    async fn emit(&self, mut request: EventRequest) -> Result<Event> {
        if let Some(extra) = &self.event_metadata {
            let mut metadata = request
                .metadata
                .take()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            for (key, value) in extra {
                metadata.entry(key.clone()).or_insert_with(|| value.clone());
            }
            request.metadata = Some(serde_json::Value::Object(metadata));
        }
        if let Some(reads) = &self.reads {
            reads.note_event(&request.event_type);
        }
        if let Some((turn, metadata)) = self
            .causal_trace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            && Some(*turn) == request.context.turn_id
        {
            let object = request
                .metadata
                .get_or_insert_with(|| serde_json::json!({}));
            if let Some(object) = object.as_object_mut() {
                object.insert(
                    crate::core::causal::TURN_TRACEPARENT_KEY.into(),
                    metadata[crate::core::causal::TURN_TRACEPARENT_KEY].clone(),
                );
            }
        }
        let mut pending = Vec::new();
        if let Some(queue) = &self.write_behind {
            // A queued store that gave up stops every later emit, so nothing
            // lands past its gap (see `crate::write_behind`).
            if let Some(error) = queue.failure() {
                return Err(error);
            }
            // The real trace anchor is returned by the authority on turn start;
            // retain it before tools can create children or session tasks.
            if WriteBehind::queues(&request) {
                let event = provisional_event(&request);
                let adapters = self.adapters.clone();
                let fence = self.fence.clone();
                queue.enqueue_event(request, move |batch| {
                    let adapters = adapters.clone();
                    let fence = fence.clone();
                    Box::pin(async move {
                        let stored = Self::store_batch(&adapters, fence.as_ref(), batch).await;
                        match (stored, &fence) {
                            // The renewer gave the lease up while this store
                            // failed: another holder may own the log by now.
                            (Err(error), Some(fence))
                                if fence.is_lost()
                                    && !crate::actor_lease_store::is_actor_lease_lost(&error) =>
                            {
                                Err(crate::actor_lease_store::actor_lease_lost(
                                    fence.session_id,
                                    format!("lease lost while storing queued events: {error}"),
                                ))
                            }
                            (stored, _) => stored,
                        }
                    })
                })?;
                return Ok(event);
            }
            pending = queue.take_pending();
            queue.flush().await;
            if let Some(error) = queue.failure() {
                return Err(error);
            }
        }
        let event = if let Some(fence) = &self.fence {
            let provisional = provisional_event(&request);
            pending.push(request);
            let stored = self.adapters.emit_events_fenced(pending, fence).await;
            fence.observe(&stored);
            stored?.unwrap_or(provisional)
        } else if !pending.is_empty() {
            self.adapters.emit_events_then(pending, request).await?
        } else {
            self.adapters.emit_event(request).await?
        };
        if event.event_type == crate::core::events::TURN_STARTED {
            self.restore_turn_context(&event.context, event.metadata.as_ref());
        }
        Ok(event)
    }
}

#[async_trait]
impl<A: WorkerAdapters> crate::core::session_files::SessionFileSystem for SessionAdapter<A> {
    fn session_scoped_path(&self, path: &str) -> bool {
        everruns_contracts::knowledge::is_mount(path)
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let org_id = self.require_org("read_file")?;
        self.adapters
            .read_file(org_id, session_id.uuid(), path)
            .await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let org_id = self.require_org("write_file")?;
        self.adapters
            .write_file(org_id, session_id.uuid(), path, content, encoding)
            .await
    }

    async fn create_file_if_absent(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
    ) -> Result<bool> {
        self.adapters
            .create_memory_artifact(
                self.require_org("create_memory_artifact")?,
                session.uuid(),
                path,
                content,
            )
            .await
    }
    async fn dreaming_scope(
        &self,
        session_id: SessionId,
        mount: &str,
    ) -> Result<Option<crate::core::dreaming::DreamMemoryScope>> {
        self.adapters
            .dreaming_scope(
                self.require_org("dreaming_scope")?,
                session_id.uuid(),
                mount,
            )
            .await
    }
    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let org_id = self.require_org("write_file_if_content_matches")?;
        self.adapters
            .write_file_if_content_matches(
                org_id,
                session_id.uuid(),
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let org_id = self.require_org("delete_file")?;
        self.adapters
            .delete_file(org_id, session_id.uuid(), path, recursive)
            .await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let org_id = self.require_org("list_directory")?;
        self.adapters
            .list_directory(org_id, session_id.uuid(), path)
            .await
    }

    async fn list_tree(&self, session_id: SessionId, path: &str) -> Result<Option<Vec<FileInfo>>> {
        let org_id = self.require_org("list_tree")?;
        self.adapters
            .list_tree(org_id, session_id.uuid(), path)
            .await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let org_id = self.require_org("stat_file")?;
        self.adapters
            .stat_file(org_id, session_id.uuid(), path)
            .await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let org_id = self.require_org("grep_files")?;
        self.adapters
            .grep_files(org_id, session_id.uuid(), pattern, path_pattern)
            .await
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let org_id = self.require_org("grep_files_with_options")?;
        self.adapters
            .grep_files_with_options(org_id, session_id.uuid(), pattern, options)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let org_id = self.require_org("create_directory")?;
        self.adapters
            .create_directory(org_id, session_id.uuid(), path)
            .await
    }

    fn is_mount_resolver(&self) -> bool {
        false
    }
}
