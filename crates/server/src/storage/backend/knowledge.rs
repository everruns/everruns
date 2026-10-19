//! Memory search, knowledge entries, notifications, turns, and providers.

use super::*;

impl StorageBackend {
    // ============================================
    // Knowledge Bases
    // ============================================

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

    // ============================================
    // Knowledge Indexes (see knowledge/runtime-resources/knowledge-indexes.md)
    // ============================================

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

    // ============================================
    // Pinned Sessions
    // ============================================

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
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        #[cfg(test)]
        self.fail_if_forced("create_event")?;
        dispatch!(self, create_event, input)
    }

    pub async fn create_event_if_last(
        &self,
        input: CreateEventRow,
        expected_last_sequence: Option<i32>,
    ) -> Result<EventRow> {
        #[cfg(test)]
        self.fail_if_forced("create_event")?;
        dispatch!(self, create_event_if_last, input, expected_last_sequence)
    }

    pub async fn create_events(&self, inputs: Vec<CreateEventRow>) -> Result<Vec<EventRow>> {
        #[cfg(test)]
        self.fail_if_forced("create_event")?;
        dispatch!(self, create_events, inputs)
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

    /// Get a provider with its decrypted API key
    /// Note: This is not async, so cannot use dispatch! macro
    pub fn get_provider_with_api_key(
        &self,
        provider: &ProviderRow,
        encryption: &super::super::EncryptionService,
    ) -> Result<ProviderWithApiKey> {
        {
            let db = self.database();
            db.get_provider_with_api_key(provider, encryption)
        }
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn create_model(&self, org_id: i64, input: CreateModelRow) -> Result<ModelRow> {
        let mut input = input;
        if let Some(provider) = self.get_provider(org_id, input.provider_id.uuid()).await?
            && input
                .provider_metadata
                .as_ref()
                .and_then(|m| m.get(crate::domains::models::catalog::BINDING))
                .is_none()
        {
            input.provider_metadata = Some(crate::domains::models::catalog::assign(
                &provider.provider_type,
                &input.model_id,
                &input.capabilities,
                None,
                None,
                input.provider_metadata,
            )?);
        }
        if let Some(service) = input
            .provider_metadata
            .as_ref()
            .map(|metadata| crate::domains::models::catalog::service(Some(metadata)))
        {
            let tag = service.to_string();
            if !input.capabilities.contains(&tag) {
                input.capabilities.push(tag);
            }
        }
        dispatch!(self, create_model, org_id, input)
    }
}
