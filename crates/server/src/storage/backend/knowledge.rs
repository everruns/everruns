//! Memory search, knowledge entries, notifications, turns, and providers.

use super::*;

impl StorageBackend {
    pub async fn grep_memory_files(
        &self,
        memory_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        dispatch!(
            self,
            grep_memory_files,
            memory_id,
            pattern,
            path_pattern,
            max_file_bytes
        )
    }

    pub async fn memory_file_exists(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, memory_file_exists, memory_id, path)
    }

    pub async fn memory_directory_has_children(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, memory_directory_has_children, memory_id, path)
    }

    // ============================================
    // Knowledge Bases
    // ============================================

    pub async fn create_knowledge_base(
        &self,
        org_id: i64,
        input: CreateKnowledgeBaseRow,
    ) -> Result<KnowledgeBaseRow> {
        dispatch!(self, create_knowledge_base, org_id, input)
    }

    pub async fn get_knowledge_base(
        &self,
        org_id: i64,
        kb_id: KnowledgeBaseId,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(
            self,
            get_knowledge_base_by_public_id,
            org_id,
            &kb_id.to_string()
        )
    }

    pub async fn get_knowledge_base_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(self, get_knowledge_base_by_id, org_id, id)
    }

    pub async fn list_knowledge_bases(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeBaseRow>> {
        dispatch!(self, list_knowledge_bases, org_id, search, include_archived)
    }

    pub async fn update_knowledge_base(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeBase,
    ) -> Result<Option<KnowledgeBaseRow>> {
        dispatch!(self, update_knowledge_base, org_id, id, input)
    }

    pub async fn archive_knowledge_base(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_knowledge_base, org_id, id)
    }

    pub async fn create_knowledge_entry(
        &self,
        kb_id: Uuid,
        input: CreateKnowledgeEntryRow,
    ) -> Result<KnowledgeEntryRow> {
        dispatch!(self, create_knowledge_entry, kb_id, input)
    }

    pub async fn get_knowledge_entry(
        &self,
        kb_id: Uuid,
        entry_id: KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryRow>> {
        dispatch!(
            self,
            get_knowledge_entry_by_public_id,
            kb_id,
            &entry_id.to_string()
        )
    }

    pub async fn list_knowledge_entries(
        &self,
        kb_id: Uuid,
        search: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        dispatch!(self, list_knowledge_entries, kb_id, search, kind)
    }

    pub async fn search_knowledge_entries(
        &self,
        kb_ids: &[Uuid],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        dispatch!(
            self,
            search_knowledge_entries,
            kb_ids,
            query,
            kind,
            tags,
            limit
        )
    }

    pub async fn update_knowledge_entry(
        &self,
        kb_id: Uuid,
        id: Uuid,
        input: UpdateKnowledgeEntry,
    ) -> Result<Option<KnowledgeEntryRow>> {
        dispatch!(self, update_knowledge_entry, kb_id, id, input)
    }

    pub async fn delete_knowledge_entry(&self, kb_id: Uuid, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_knowledge_entry, kb_id, id)
    }

    // ============================================
    // Knowledge Indexes (see knowledge/runtime-resources/knowledge-indexes.md)
    // ============================================

    pub async fn create_knowledge_index(
        &self,
        org_id: i64,
        input: CreateKnowledgeIndexRow,
    ) -> Result<KnowledgeIndexRow> {
        dispatch!(self, create_knowledge_index, org_id, input)
    }

    pub async fn get_knowledge_index(
        &self,
        org_id: i64,
        index_id: KnowledgeIndexId,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(
            self,
            get_knowledge_index_by_public_id,
            org_id,
            &index_id.to_string()
        )
    }

    pub async fn get_knowledge_index_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, get_knowledge_index_by_public_id, org_id, public_id)
    }

    pub async fn get_knowledge_index_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, get_knowledge_index_by_id, org_id, id)
    }

    pub async fn list_knowledge_indexes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeIndexRow>> {
        dispatch!(
            self,
            list_knowledge_indexes,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_knowledge_index(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeIndex,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, update_knowledge_index, org_id, id, input)
    }

    pub async fn archive_knowledge_index(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_knowledge_index, org_id, id)
    }

    pub async fn list_knowledge_index_documents(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexDocumentRow>> {
        dispatch!(self, list_knowledge_index_documents, index_id)
    }

    pub async fn count_knowledge_index_documents(
        &self,
        index_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, usize>> {
        dispatch!(self, count_knowledge_index_documents, index_ids)
    }

    pub async fn list_knowledge_index_chunks(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexChunkRow>> {
        dispatch!(self, list_knowledge_index_chunks, index_id)
    }

    pub async fn get_knowledge_index_chunks_with_documents(
        &self,
        index_id: Uuid,
        chunk_public_ids: &[String],
    ) -> Result<Vec<KnowledgeIndexChunkWithDocument>> {
        dispatch!(
            self,
            get_knowledge_index_chunks_with_documents,
            index_id,
            chunk_public_ids
        )
    }

    pub async fn enqueue_knowledge_index_sync(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, enqueue_knowledge_index_sync, org_id, id)
    }

    pub async fn claim_next_knowledge_index_sync(&self) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, claim_next_knowledge_index_sync)
    }

    pub async fn complete_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        documents: Vec<CreateKnowledgeIndexDocumentWithChunks>,
        vector_dim: Option<i32>,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(
            self,
            complete_knowledge_index_sync,
            index_id,
            claimed_at,
            documents,
            vector_dim
        )
    }

    pub async fn fail_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        dispatch!(self, fail_knowledge_index_sync, index_id, claimed_at, error)
    }

    // ============================================
    // Pinned Sessions
    // ============================================

    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        dispatch!(self, pin_session, user_id, session_id, org_id)
    }

    pub async fn unpin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<bool> {
        dispatch!(self, unpin_session, user_id, session_id, org_id)
    }

    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, list_pinned_session_ids, user_id, org_id)
    }

    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        dispatch!(self, create_notification_turn_request, input)
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        dispatch!(self, get_notification_turn_request, input_message_id)
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        dispatch!(self, create_notification, input)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        dispatch!(self, get_notification, org_id, user_id, id)
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        dispatch!(self, list_notifications, org_id, user_id, limit)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        dispatch!(
            self,
            list_notifications_updated_since,
            org_id,
            user_id,
            updated_since,
            limit
        )
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        dispatch!(self, count_unviewed_notifications, org_id, user_id)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        dispatch!(
            self,
            count_unviewed_notifications_by_kind,
            org_id,
            user_id,
            kind
        )
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        dispatch!(self, mark_notification_viewed, org_id, user_id, id)
    }

    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        #[cfg(test)]
        self.fail_if_forced("create_event")?;
        dispatch!(self, create_event, input)
    }

    pub async fn create_waiting_turn_resolution_event(
        &self,
        input: CreateEventRow,
        resolution_id: Uuid,
        event_index: i32,
    ) -> Result<(EventRow, bool)> {
        #[cfg(test)]
        self.fail_if_forced("create_event")?;
        dispatch!(
            self,
            create_waiting_turn_resolution_event,
            input,
            resolution_id,
            event_index
        )
    }

    /// Check if an input.message event with a given slack_ts already exists in a session.
    /// Used for Slack event dedup across server instances.
    pub async fn has_event_with_slack_ts(
        &self,
        session_id: SessionId,
        slack_ts: &str,
    ) -> Result<bool> {
        dispatch!(self, has_event_with_slack_ts, session_id, slack_ts)
    }

    /// Atomically consume a still-current Slack approval card.
    pub async fn claim_slack_approval_card(
        &self,
        session_id: SessionId,
        card_id: &str,
        turn_id: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            claim_slack_approval_card,
            session_id,
            card_id,
            turn_id
        )
    }

    pub async fn find_input_message_event(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<EventRow>> {
        dispatch!(self, find_input_message_event, session_id, message_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        dispatch!(
            self,
            list_events,
            session_id,
            since_sequence,
            since_id,
            filter_types,
            exclude_types,
            before_sequence,
            limit
        )
    }

    /// Count events for a session using SELECT COUNT(*) — no row materialization.
    pub async fn count_events(
        &self,
        session_id: SessionId,
        exclude_types: &[String],
    ) -> Result<i64> {
        dispatch!(self, count_events, session_id, exclude_types)
    }

    /// Advanced event listing for debugging.
    /// Supports time range, context-id (turn/exec/trace), tags, tool name,
    /// full-text search, around-id windowing, and direction.
    pub async fn list_events_advanced(
        &self,
        params: &crate::storage::models::ListEventsParams,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_events_advanced, params)
    }

    /// One-shot debug summary: per-type counts + first/last timestamps.
    pub async fn events_summary(
        &self,
        session_id: SessionId,
    ) -> Result<crate::storage::models::EventsSummary> {
        dispatch!(self, events_summary, session_id)
    }

    /// Find the nearest turn.started sequence at or before the given sequence.
    pub async fn find_turn_boundary(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> Result<Option<i32>> {
        dispatch!(self, find_turn_boundary, session_id, before_sequence)
    }

    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events, session_id)
    }

    /// List message events with an optional limit on count.
    /// Returns most recent N messages in sequence order when limit is provided.
    pub async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events_limited, session_id, limit)
    }

    /// Count message events for a session using COUNT(*) — no row materialization.
    pub async fn count_message_events(&self, session_id: SessionId) -> Result<i64> {
        dispatch!(self, count_message_events, session_id)
    }

    /// List message events with filters applied
    ///
    /// This method applies the filters from the MessageQuery to efficiently
    /// retrieve messages. DB-mappable filters are pushed to the database,
    /// while custom filters are applied in-memory.
    ///
    /// Note: Injections are NOT applied here - they should be applied at the
    /// MessageRetriever layer after converting events to messages.
    pub async fn list_message_events_filtered(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<EventRow>> {
        dispatch!(self, list_message_events_filtered, query)
    }

    pub async fn get_compaction_checkpoint(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        format_version: i32,
    ) -> Result<Option<CompactionCheckpointRow>> {
        dispatch!(
            self,
            get_compaction_checkpoint,
            session_id,
            provider_type,
            model,
            format_version
        )
    }

    pub async fn install_compaction_checkpoint(
        &self,
        input: InstallCompactionCheckpointRow,
    ) -> Result<bool> {
        dispatch!(self, install_compaction_checkpoint, input)
    }

    pub async fn copy_compaction_checkpoints(
        &self,
        source_session_id: SessionId,
        target_session_id: SessionId,
        through_sequence: i32,
    ) -> Result<u64> {
        dispatch!(
            self,
            copy_compaction_checkpoints,
            source_session_id,
            target_session_id,
            through_sequence
        )
    }

    /// Get preview text for multiple sessions (first user message)
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_session_previews, session_ids)
    }

    /// Get output preview text for multiple sessions (last agent message)
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_session_output_previews, session_ids)
    }

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_provider(
        &self,
        org_id: i64,
        input: CreateProviderRow,
    ) -> Result<ProviderRow> {
        dispatch!(self, create_provider, org_id, input)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    pub async fn create_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateProviderRow,
    ) -> Result<Option<ProviderRow>> {
        dispatch!(self, create_provider_with_id, org_id, id, input)
    }

    pub async fn get_provider(&self, org_id: i64, id: Uuid) -> Result<Option<ProviderRow>> {
        dispatch!(self, get_provider, org_id, id)
    }

    pub async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        dispatch!(self, list_providers, org_id)
    }

    pub async fn update_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateProvider,
    ) -> Result<Option<ProviderRow>> {
        dispatch!(self, update_provider, org_id, id, input)
    }

    pub async fn delete_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_provider, org_id, id)
    }

    /// Mark (or unmark) a provider as host-managed (EVE-810).
    pub async fn set_provider_managed(&self, org_id: i64, id: Uuid, managed: bool) -> Result<bool> {
        dispatch!(self, set_provider_managed, org_id, id, managed)
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        org_id: i64,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            update_provider_last_synced,
            org_id,
            id,
            last_synced_at
        )
    }

    /// Get a provider with its decrypted API key
    /// Note: This is not async, so cannot use dispatch! macro
    pub fn get_provider_with_api_key(
        &self,
        provider: &ProviderRow,
        encryption: &super::super::EncryptionService,
    ) -> Result<ProviderWithApiKey> {
        match self {
            Self::Postgres(db) => db.get_provider_with_api_key(provider, encryption),
            Self::InMemory(db) => db.get_provider_with_api_key(provider, encryption),
        }
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn get_default_model(&self, org_id: i64) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_default_model, org_id)
    }

    pub async fn get_organization_settings(
        &self,
        org_id: i64,
    ) -> Result<Option<OrganizationSettingsRow>> {
        dispatch!(self, get_organization_settings, org_id)
    }

    pub async fn upsert_organization_settings(
        &self,
        org_id: i64,
        default_model_id: Option<uuid::Uuid>,
    ) -> Result<OrganizationSettingsRow> {
        dispatch!(self, upsert_organization_settings, org_id, default_model_id)
    }

    pub async fn patch_organization_settings(
        &self,
        org_id: i64,
        input: UpdateOrganizationSettings,
    ) -> Result<OrganizationSettingsRow> {
        dispatch!(self, patch_organization_settings, org_id, input)
    }

    pub async fn list_org_feature_flags(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, bool>> {
        dispatch!(self, list_org_feature_flags, org_id)
    }

    pub async fn replace_org_feature_flags(
        &self,
        org_id: i64,
        flags: &std::collections::HashMap<String, bool>,
    ) -> Result<()> {
        dispatch!(self, replace_org_feature_flags, org_id, flags)
    }

    pub async fn create_model(&self, org_id: i64, input: CreateModelRow) -> Result<ModelRow> {
        dispatch!(self, create_model, org_id, input)
    }
}
