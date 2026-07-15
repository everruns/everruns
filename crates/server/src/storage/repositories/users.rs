// PostgreSQL repository: Users

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Users
    // ============================================

    pub async fn create_user(&self, input: CreateUserRow) -> Result<UserRow> {
        let roles_json = serde_json::to_value(&input.roles)?;
        // EVE-704: store the canonical (trim+lowercase) email so email identity
        // is case-insensitive, matched by the unique index on lower(email).
        let email = normalize_email(&input.email);

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, external_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            "#,
        )
        .bind(&email)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .bind(&input.auth_provider)
        .bind(&input.auth_provider_id)
        .bind(&input.external_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create user with a specific UUID (for seeding).
    /// Returns None if id already exists.
    pub async fn create_user_with_id(
        &self,
        id: Uuid,
        input: CreateUserRow,
    ) -> Result<Option<UserRow>> {
        let roles_json = serde_json::to_value(&input.roles)?;
        // EVE-704: canonicalize email on the seeding path too.
        let email = normalize_email(&input.email);

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, external_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO NOTHING
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            "#,
        )
        .bind(id)
        .bind(&email)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .bind(&input.auth_provider)
        .bind(&input.auth_provider_id)
        .bind(&input.external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        // EVE-704: look up by canonical email against the lower(email) index so
        // any casing of a mailbox resolves to its single account.
        let email = normalize_email(email);
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            FROM users
            WHERE lower(email) = $1
            "#,
        )
        .bind(&email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            FROM users
            WHERE auth_provider = $1 AND auth_provider_id = $2
            "#,
        )
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Attach an OAuth identity to an existing account so a subsequent
    /// `get_user_by_oauth(provider, provider_id)` resolves to it. Only touches
    /// the provider columns; `password_hash` is preserved so a linked account
    /// keeps password login and password reset (`is_local_password_user` keys
    /// off `password_hash`). Callers must confirm the provider verified the
    /// email before linking (see TM-AUTH-017 / `oauth_identity_rejection_reason`).
    pub async fn link_oauth_identity(
        &self,
        id: Uuid,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE users
            SET auth_provider = $2, auth_provider_id = $3, updated_at = NOW()
            WHERE id = $1
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            "#,
        )
        .bind(id)
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        let roles_json = input.roles.map(|r| serde_json::to_value(&r)).transpose()?;

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE users
            SET
                name = COALESCE($2, name),
                avatar_url = COALESCE($3, avatar_url),
                roles = COALESCE($4, roles),
                password_hash = COALESCE($5, password_hash),
                email_verified = COALESCE($6, email_verified),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all users with optional search query
    /// Search matches name or email (case-insensitive, partial match)
    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        let rows = match search {
            Some(query) if !query.trim().is_empty() => {
                let search_pattern = format!("%{}%", query.trim().to_lowercase());
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
                    FROM users
                    WHERE LOWER(name) LIKE $1 OR LOWER(email) LIKE $1
                    ORDER BY created_at DESC
                    "#,
                )
                .bind(&search_pattern)
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
                    FROM users
                    ORDER BY created_at DESC
                    "#,
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows)
    }

    /// Hard-delete a user and all associated data.
    /// FK constraints use ON DELETE CASCADE, so this removes all dependent rows.
    pub async fn delete_user_account(&self, user_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Export all user-owned data as a structured JSON value.
    pub async fn export_user_data(&self, user_id: Uuid) -> Result<Option<serde_json::Value>> {
        let user = self.get_user(user_id).await?;
        let Some(user) = user else {
            return Ok(None);
        };

        let personal_access_tokens = self.list_personal_access_tokens_for_user(user_id).await?;
        let orgs = self.list_user_organizations(user_id).await?;

        let export = serde_json::json!({
            "user": {
                "id": user.id.to_string(),
                "email": user.email,
                "name": user.name,
                "avatar_url": user.avatar_url,
                "email_verified": user.email_verified,
                "auth_provider": user.auth_provider,
                "created_at": user.created_at,
                "updated_at": user.updated_at,
            },
            "organizations": orgs.iter().map(|o| serde_json::json!({
                "org_id": o.org_id,
                "public_id": o.public_id,
                "name": o.name,
                "role": o.role,
            })).collect::<Vec<_>>(),
            "personal_access_tokens": personal_access_tokens.iter().map(|t| serde_json::json!({
                "id": t.id.to_string(),
                "name": t.name,
                "token_prefix": t.token_prefix,
                "scopes": t.scopes,
                "expires_at": t.expires_at,
                "last_used_at": t.last_used_at,
                "created_at": t.created_at,
            })).collect::<Vec<_>>(),
            "exported_at": chrono::Utc::now(),
        });

        Ok(Some(export))
    }

    /// List users within an organization (TM-TENANT-008: org-scoped user listing)
    /// Filters via organization_members join to enforce tenant isolation.
    pub async fn list_users_by_org(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<UserRow>> {
        let rows = match search {
            Some(query) if !query.trim().is_empty() => {
                let search_pattern = format!("%{}%", query.trim().to_lowercase());
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT u.id, u.email, u.name, u.avatar_url, u.roles, u.password_hash, u.email_verified, u.auth_provider, u.auth_provider_id, u.created_at, u.updated_at, u.external_id
                    FROM users u
                    JOIN organization_members om ON u.id = om.user_id
                    WHERE om.org_id = $1 AND (LOWER(u.name) LIKE $2 OR LOWER(u.email) LIKE $2)
                    ORDER BY u.created_at DESC
                    "#,
                )
                .bind(org_id)
                .bind(&search_pattern)
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT u.id, u.email, u.name, u.avatar_url, u.roles, u.password_hash, u.email_verified, u.auth_provider, u.auth_provider_id, u.created_at, u.updated_at, u.external_id
                    FROM users u
                    JOIN organization_members om ON u.id = om.user_id
                    WHERE om.org_id = $1
                    ORDER BY u.created_at DESC
                    "#,
                )
                .bind(org_id)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows)
    }
}
