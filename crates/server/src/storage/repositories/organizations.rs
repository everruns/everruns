// PostgreSQL repository: Organization Settings, Organizations, Organization Members

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Organization Settings
    // ============================================

    /// Get organization settings
    pub async fn get_organization_settings(
        &self,
        org_id: i64,
    ) -> Result<Option<OrganizationSettingsRow>> {
        let row = sqlx::query_as::<_, OrganizationSettingsRow>(
            "SELECT org_id, default_model_id, default_harness_id, base_harness_id, created_at, updated_at FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Upsert organization settings (create or update)
    pub async fn upsert_organization_settings(
        &self,
        org_id: i64,
        default_model_id: Option<uuid::Uuid>,
    ) -> Result<OrganizationSettingsRow> {
        let row = sqlx::query_as::<_, OrganizationSettingsRow>(
            r#"
            INSERT INTO organization_settings (org_id, default_model_id)
            VALUES ($1, $2)
            ON CONFLICT (org_id) DO UPDATE SET
                default_model_id = EXCLUDED.default_model_id,
                updated_at = NOW()
            RETURNING org_id, default_model_id, default_harness_id, base_harness_id, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(default_model_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn patch_organization_settings(
        &self,
        org_id: i64,
        input: UpdateOrganizationSettings,
    ) -> Result<OrganizationSettingsRow> {
        let row = sqlx::query_as::<_, OrganizationSettingsRow>(
            r#"
            INSERT INTO organization_settings (org_id, default_model_id, default_harness_id, base_harness_id)
            VALUES ($1, $2, $4, $6)
            ON CONFLICT (org_id) DO UPDATE SET
                default_model_id = CASE
                    WHEN $3 THEN $2
                    ELSE organization_settings.default_model_id
                END,
                default_harness_id = CASE
                    WHEN $5 THEN $4
                    ELSE organization_settings.default_harness_id
                END,
                base_harness_id = CASE
                    WHEN $7 THEN $6
                    ELSE organization_settings.base_harness_id
                END,
                updated_at = NOW()
            RETURNING org_id, default_model_id, default_harness_id, base_harness_id, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.default_model_id.clone().into_value().map(|id| id.uuid()))
        .bind(input.default_model_id.is_changed())
        .bind(input.default_harness_id.clone().into_value().map(|id| id.uuid()))
        .bind(input.default_harness_id.is_changed())
        .bind(input.base_harness_id.clone().into_value().map(|id| id.uuid()))
        .bind(input.base_harness_id.is_changed())
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get a provider with its decrypted API key from database.
    ///
    /// Note: Environment variable fallback (DEFAULT_*_API_KEY) is handled
    /// at the service layer (LlmResolverService), not here.
    pub fn get_provider_with_api_key(
        &self,
        provider: &LlmProviderRow,
        encryption: &super::super::EncryptionService,
    ) -> Result<LlmProviderWithApiKey> {
        let api_key = if let Some(ref encrypted) = provider.api_key_encrypted {
            Some(encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        // Convert settings from sqlx JsonValue to serde_json::Value
        let settings: serde_json::Value =
            serde_json::from_str(&provider.settings.to_string()).unwrap_or_default();

        Ok(LlmProviderWithApiKey {
            id: provider.id,
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key,
            settings,
        })
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (public_id, name, created_by)
            VALUES ($1, $2, $3)
            RETURNING org_id, public_id, name, created_at, updated_at, external_id, created_by
            "#,
        )
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.created_by)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (org_id, public_id, name, created_by)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (org_id) DO NOTHING
            RETURNING org_id, public_id, name, created_at, updated_at, external_id, created_by
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.created_by)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
            FROM organizations
            WHERE org_id = $1
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
            FROM organizations
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        let rows = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
            FROM organizations
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            UPDATE organizations
            SET
                name = COALESCE($2, name),
                updated_at = NOW()
            WHERE org_id = $1
            RETURNING org_id, public_id, name, created_at, updated_at, external_id
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM organizations WHERE org_id = $1")
            .bind(org_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
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
        let row = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            INSERT INTO organization_members (org_id, user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role
            RETURNING org_id, user_id, role, created_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM organization_members WHERE org_id = $1 AND user_id = $2")
                .bind(org_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        let rows = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            SELECT org_id, user_id, role, created_at
            FROM organization_members
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List members with user info (for API responses)
    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        let rows = sqlx::query_as::<_, OrganizationMemberWithUserRow>(
            r#"
            SELECT u.id as user_id, u.email, u.name, u.avatar_url, om.role, om.created_at as joined_at
            FROM organization_members om
            JOIN users u ON u.id = om.user_id
            WHERE om.org_id = $1
            ORDER BY om.created_at
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get a specific organization member with user info
    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        let row = sqlx::query_as::<_, OrganizationMemberWithUserRow>(
            r#"
            SELECT u.id as user_id, u.email, u.name, u.avatar_url, om.role, om.created_at as joined_at
            FROM organization_members om
            JOIN users u ON u.id = om.user_id
            WHERE om.org_id = $1 AND om.user_id = $2
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update a member's role
    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        let row = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            UPDATE organization_members SET role = $3
            WHERE org_id = $1 AND user_id = $2
            RETURNING org_id, user_id, role, created_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Count owners in an organization (for preventing last owner removal)
    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM organization_members WHERE org_id = $1 AND role = 'owner'",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Count organizations created by a user (for resource limits).
    pub async fn count_user_created_organizations(&self, user_id: Uuid) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM organizations WHERE created_by = $1")
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    /// Count members in an organization (for resource limits).
    pub async fn count_organization_members(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM organization_members WHERE org_id = $1")
                .bind(org_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        let rows = sqlx::query_as::<_, OrganizationWithRoleRow>(
            r#"
            SELECT o.org_id, o.public_id, o.name, om.role
            FROM organizations o
            JOIN organization_members om ON o.org_id = om.org_id
            WHERE om.user_id = $1
            ORDER BY o.name
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1 as exists_flag
            FROM organization_members
            WHERE org_id = $1 AND user_id = $2
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }

    /// Get user by external identity provider ID
    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            FROM users
            WHERE external_id = $1
            "#,
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get organization by external identity provider ID
    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
            FROM organizations
            WHERE external_id = $1
            "#,
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Upsert organization by external ID (for external auth provider sync)
    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (public_id, name, external_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (external_id) DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
            RETURNING org_id, public_id, name, created_at, updated_at, external_id
            "#,
        )
        .bind(public_id)
        .bind(name)
        .bind(external_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Ensure user is a member of organization with the given role (upsert).
    /// Updates the role if the membership already exists.
    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, $3) \
             ON CONFLICT (org_id, user_id) DO UPDATE \
             SET role = EXCLUDED.role \
             WHERE organization_members.role IS DISTINCT FROM EXCLUDED.role",
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Reconcile org memberships to match an authoritative list from an external
    /// identity provider. Adds missing members, updates changed roles, and
    /// removes members not present in the authoritative list.
    ///
    /// Runs inside a transaction for atomicity. Duplicate user_ids in the
    /// authoritative list are deduplicated (last wins).
    ///
    /// Returns `(added, updated, removed)` counts.
    pub async fn reconcile_memberships(
        &self,
        org_id: i64,
        authoritative: &[(Uuid, String)],
    ) -> Result<(usize, usize, usize)> {
        // Deduplicate authoritative input (last occurrence wins)
        let auth_map: std::collections::HashMap<Uuid, &str> = authoritative
            .iter()
            .map(|(uid, role)| (*uid, role.as_str()))
            .collect();

        let mut tx = self.pool.begin().await?;

        // Read current memberships inside the transaction (lock rows)
        let current: Vec<OrganizationMemberRow> = sqlx::query_as(
            "SELECT org_id, user_id, role, created_at \
             FROM organization_members WHERE org_id = $1 FOR UPDATE",
        )
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await?;

        let current_map: std::collections::HashMap<Uuid, String> =
            current.into_iter().map(|m| (m.user_id, m.role)).collect();

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;

        // Add or update from the deduped map
        for (user_id, role) in &auth_map {
            match current_map.get(user_id) {
                None => {
                    sqlx::query(
                        "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, $3)",
                    )
                    .bind(org_id)
                    .bind(user_id)
                    .bind(role)
                    .execute(&mut *tx)
                    .await?;
                    added += 1;
                }
                Some(existing_role) if existing_role != role => {
                    sqlx::query(
                        "UPDATE organization_members SET role = $1 \
                         WHERE org_id = $2 AND user_id = $3",
                    )
                    .bind(role)
                    .bind(org_id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
                    updated += 1;
                }
                _ => {} // unchanged
            }
        }

        // Remove members not in authoritative list
        for user_id in current_map.keys() {
            if !auth_map.contains_key(user_id) {
                sqlx::query("DELETE FROM organization_members WHERE org_id = $1 AND user_id = $2")
                    .bind(org_id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
                removed += 1;
            }
        }

        tx.commit().await?;

        Ok((added, updated, removed))
    }
}
