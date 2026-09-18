//! Plugin installs, images, organizations, webhooks, session keys, and user connections.

use super::*;

impl StorageBackend {
    pub async fn delete_plugin_install(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_plugin_install, org_id, id)
    }

    // ============================================
    // Skill Files
    // ============================================

    pub async fn create_skill_file(&self, input: CreateSkillFileRow) -> Result<SkillFileRow> {
        dispatch!(self, create_skill_file, input)
    }

    pub async fn list_skill_files(&self, skill_id: Uuid) -> Result<Vec<SkillFileRow>> {
        dispatch!(self, list_skill_files, skill_id)
    }

    pub async fn delete_skill_files(&self, skill_id: Uuid) -> Result<u64> {
        dispatch!(self, delete_skill_files, skill_id)
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        turn_id: Option<Uuid>,
        event_id: Option<Uuid>,
        model: String,
        provider: Option<String>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: Option<f64>,
        estimated_cost_usd: Option<f64>,
        duration_ms: Option<i32>,
        finish_reason: Option<String>,
        provider_response_id: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        dispatch!(
            self,
            create_llm_generation,
            org_id,
            session_id,
            turn_id,
            event_id,
            model,
            provider,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            duration_ms,
            finish_reason,
            provider_response_id,
            created_at
        )
    }

    pub async fn list_unreconciled_llm_generations(
        &self,
        provider: &str,
        limit: i64,
    ) -> Result<Vec<crate::storage::models::UnreconciledGeneration>> {
        dispatch!(self, list_unreconciled_llm_generations, provider, limit)
    }

    pub async fn reconcile_llm_generation(
        &self,
        id: Uuid,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        actual_cost_usd: Option<f64>,
        reconciled_provider: Option<&str>,
        reconciled_model: Option<&str>,
    ) -> Result<()> {
        dispatch!(
            self,
            reconcile_llm_generation,
            id,
            input_tokens,
            output_tokens,
            actual_cost_usd,
            reconciled_provider,
            reconciled_model
        )
    }

    pub async fn mark_llm_generation_reconciliation_failed(
        &self,
        id: Uuid,
        retry_after_seconds: i32,
    ) -> Result<()> {
        dispatch!(
            self,
            mark_llm_generation_reconciliation_failed,
            id,
            retry_after_seconds
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_session_usage,
            session_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            cost_usd
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        dispatch!(
            self,
            increment_agent_usage,
            agent_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd,
            estimated_cost_usd,
            cost_usd
        )
    }

    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, org_id: i64, input: CreateImageRow) -> Result<ImageRow> {
        dispatch!(self, create_image, org_id, input)
    }

    pub async fn get_image(&self, org_id: i64, id: Uuid) -> Result<Option<ImageRow>> {
        dispatch!(self, get_image, org_id, id)
    }

    pub async fn get_image_info(&self, org_id: i64, id: Uuid) -> Result<Option<ImageInfoRow>> {
        dispatch!(self, get_image_info, org_id, id)
    }

    pub async fn delete_image(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_image, org_id, id)
    }

    pub async fn list_images(
        &self,
        org_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ImageInfoRow>> {
        dispatch!(self, list_images, org_id, limit, offset)
    }

    pub async fn create_file(&self, org_id: i64, input: CreateFileRow) -> Result<FileRow> {
        dispatch!(self, create_file, org_id, input)
    }

    pub async fn get_file(&self, org_id: i64, id: Uuid) -> Result<Option<FileRow>> {
        dispatch!(self, get_file, org_id, id)
    }

    pub async fn get_file_info(&self, org_id: i64, id: Uuid) -> Result<Option<FileInfoRow>> {
        dispatch!(self, get_file_info, org_id, id)
    }

    pub async fn delete_file(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_file, org_id, id)
    }

    pub async fn list_files(&self, org_id: i64, limit: i64) -> Result<Vec<FileInfoRow>> {
        dispatch!(self, list_files, org_id, limit)
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        dispatch!(self, create_organization, input)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, create_organization_with_id, org_id, input)
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization, org_id)
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization_by_public_id, public_id)
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        dispatch!(self, list_organizations)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, update_organization, org_id, input)
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        dispatch!(self, delete_organization, org_id)
    }

    /// Idempotently mark an org's onboarding complete (no-op if already set).
    pub async fn mark_org_onboarding_complete(&self, org_id: i64) -> Result<()> {
        dispatch!(self, mark_org_onboarding_complete, org_id)
    }

    // ============================================
    // Organization Members
    // ============================================

    pub async fn add_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<OrganizationMemberRow> {
        dispatch!(self, add_organization_member, org_id, user_id, role)
    }
    pub async fn add_organization_member_with_capacity(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
        max_members: i64,
    ) -> Result<AddOrganizationMemberOutcome> {
        dispatch!(
            self,
            add_organization_member_with_capacity,
            org_id,
            user_id,
            role,
            max_members
        )
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        dispatch!(self, remove_organization_member, org_id, user_id)
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        dispatch!(self, list_organization_members, org_id)
    }

    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        dispatch!(self, list_organization_members_with_users, org_id)
    }

    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        dispatch!(self, get_organization_member, org_id, user_id)
    }

    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        dispatch!(self, update_organization_member_role, org_id, user_id, role)
    }

    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_organization_owners, org_id)
    }

    pub async fn count_user_created_organizations(&self, user_id: Uuid) -> Result<i64> {
        dispatch!(self, count_user_created_organizations, user_id)
    }

    pub async fn count_organization_members(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_organization_members, org_id)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        dispatch!(self, list_user_organizations, user_id)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        dispatch!(self, is_organization_member, org_id, user_id)
    }

    // ============================================
    // External Identity (for SaaS auth providers)
    // ============================================

    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        dispatch!(self, get_user_by_external_id, external_id)
    }

    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        dispatch!(self, get_organization_by_external_id, external_id)
    }

    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        dispatch!(
            self,
            upsert_org_by_external_id,
            external_id,
            public_id,
            name
        )
    }

    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        dispatch!(self, ensure_membership, user_id, org_id, role)
    }

    /// Reconcile org memberships to match an authoritative list from an external
    /// identity provider. Returns `(added, updated, removed)` counts.
    pub async fn reconcile_memberships(
        &self,
        org_id: i64,
        authoritative: &[(Uuid, String)],
    ) -> Result<(usize, usize, usize)> {
        dispatch!(self, reconcile_memberships, org_id, authoritative)
    }

    // ============================================
    // Organization Task Webhooks
    // ============================================

    pub async fn list_org_task_webhooks(&self, org_id: i64) -> Result<Vec<OrgTaskWebhookRow>> {
        dispatch!(self, list_org_task_webhooks, org_id)
    }

    pub async fn list_enabled_org_task_webhooks(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrgTaskWebhookRow>> {
        dispatch!(self, list_enabled_org_task_webhooks, org_id)
    }

    pub async fn create_org_task_webhook(
        &self,
        input: CreateOrgTaskWebhook,
    ) -> Result<OrgTaskWebhookRow> {
        dispatch!(self, create_org_task_webhook, input)
    }

    pub async fn get_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        dispatch!(self, get_org_task_webhook, org_id, public_id)
    }

    pub async fn update_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
        input: UpdateOrgTaskWebhook,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        dispatch!(self, update_org_task_webhook, org_id, public_id, input)
    }

    pub async fn delete_org_task_webhook(&self, org_id: i64, public_id: &str) -> Result<bool> {
        dispatch!(self, delete_org_task_webhook, org_id, public_id)
    }

    // ============================================
    // Organization Invitations (EVE-602)
    // ============================================

    pub async fn create_org_invitation(
        &self,
        input: CreateOrgInvitation,
    ) -> Result<OrgInvitationRow> {
        dispatch!(self, create_org_invitation, input)
    }

    pub async fn list_pending_org_invitations(&self, org_id: i64) -> Result<Vec<OrgInvitationRow>> {
        dispatch!(self, list_pending_org_invitations, org_id)
    }

    pub async fn get_org_invitation_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, get_org_invitation_by_token_hash, token_hash)
    }

    pub async fn get_org_invitation_by_public_id_and_email(
        &self,
        public_id: &str,
        email: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(
            self,
            get_org_invitation_by_public_id_and_email,
            public_id,
            email
        )
    }

    pub async fn list_outstanding_org_invitations_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<OutstandingOrgInvitationRow>> {
        dispatch!(self, list_outstanding_org_invitations_by_email, email)
    }

    pub async fn get_outstanding_org_invitation_by_email(
        &self,
        org_id: i64,
        email: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, get_outstanding_org_invitation_by_email, org_id, email)
    }

    pub async fn revoke_org_invitation(&self, org_id: i64, public_id: &str) -> Result<bool> {
        dispatch!(self, revoke_org_invitation, org_id, public_id)
    }

    pub async fn accept_org_invitation(
        &self,
        invitation_id: i64,
        accepted_by: Uuid,
    ) -> Result<Option<OrgInvitationRow>> {
        dispatch!(self, accept_org_invitation, invitation_id, accepted_by)
    }

    pub async fn accept_org_invitation_with_membership(
        &self,
        invitation_id: i64,
        accepted_by: Uuid,
        max_members: i64,
    ) -> Result<AcceptOrgInvitationOutcome> {
        dispatch!(
            self,
            accept_org_invitation_with_membership,
            invitation_id,
            accepted_by,
            max_members
        )
    }

    // ============================================
    // Session Storage (Key-Value & Secrets)
    // ============================================

    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        dispatch!(self, list_session_keys, session_id)
    }

    pub async fn upsert_session_key_value(
        &self,
        input: UpsertSessionKeyValue,
    ) -> Result<SessionKeyValueRow> {
        dispatch!(self, upsert_session_key_value, input)
    }

    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        dispatch!(self, get_session_key_value, session_id, key)
    }

    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        dispatch!(self, list_session_secrets, session_id)
    }

    pub async fn get_session_secret(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionSecretRow>> {
        dispatch!(self, get_session_secret, session_id, name)
    }

    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        dispatch!(self, upsert_session_secret, input)
    }

    pub async fn delete_session_secret(&self, session_id: Uuid, name: &str) -> Result<bool> {
        dispatch!(self, delete_session_secret, session_id, name)
    }

    pub async fn get_mcp_oauth_session_credentials(
        &self,
        session_id: SessionId,
        server_id: Uuid,
    ) -> Result<Option<McpOAuthSessionCredentialsRow>> {
        dispatch!(
            self,
            get_mcp_oauth_session_credentials,
            session_id,
            server_id
        )
    }

    pub async fn upsert_mcp_oauth_session_credentials(
        &self,
        input: UpsertMcpOAuthSessionCredentials,
    ) -> Result<()> {
        dispatch!(self, upsert_mcp_oauth_session_credentials, input)
    }

    // ============================================
    // User Connections
    // ============================================

    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        dispatch!(self, upsert_user_connection, input)
    }

    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        dispatch!(self, get_user_connection, user_id, provider)
    }

    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        dispatch!(self, list_user_connections, user_id)
    }

    pub async fn update_user_connection_oauth_tokens(
        &self,
        input: UpdateUserConnectionOAuthTokens,
    ) -> Result<Option<UserConnectionRow>> {
        dispatch!(self, update_user_connection_oauth_tokens, input)
    }

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        dispatch!(self, delete_user_connection, user_id, provider)
    }

    pub async fn list_user_preferences(
        &self,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<UserPreferenceRow>> {
        dispatch!(self, list_user_preferences, user_id, limit)
    }

    pub async fn get_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
    ) -> Result<Option<UserPreferenceRow>> {
        dispatch!(self, get_user_preference, user_id, key)
    }

    pub async fn set_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
        value: &str,
        max_preferences: usize,
    ) -> Result<UserPreferenceRow> {
        dispatch!(
            self,
            set_user_preference,
            user_id,
            key,
            value,
            max_preferences
        )
    }

    pub async fn delete_user_preference(&self, user_id: Uuid, key: &str) -> Result<bool> {
        dispatch!(self, delete_user_preference, user_id, key)
    }

    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        dispatch!(self, get_connection_token_for_session, session_id, provider)
    }

    pub async fn get_connection_metadata_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        dispatch!(
            self,
            get_connection_metadata_for_session,
            session_id,
            provider
        )
    }

    pub async fn get_connection_user_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        dispatch!(self, get_connection_user_for_session, session_id, provider)
    }

    pub async fn session_has_human_initiator(&self, session_id: SessionId) -> Result<bool> {
        dispatch!(self, session_has_human_initiator, session_id)
    }

    pub async fn get_agent_identity_connection_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        dispatch!(
            self,
            get_agent_identity_connection_for_session,
            session_id,
            provider
        )
    }
    pub async fn get_agent_identity_connection_row_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        dispatch!(
            self,
            get_agent_identity_connection_row_for_session,
            session_id,
            provider
        )
    }

    pub async fn get_owner_user_connection_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        dispatch!(
            self,
            get_owner_user_connection_for_session,
            session_id,
            provider
        )
    }

    pub async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        dispatch!(self, get_connection_token_for_user, user_id, provider)
    }

    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        dispatch!(self, get_installation_id_for_session, session_id, provider)
    }

    pub async fn get_installation_id_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<i64>> {
        dispatch!(self, get_installation_id_for_user, user_id, provider)
    }
}
