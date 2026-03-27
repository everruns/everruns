// In-memory storage: Users

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Users
    // ============================================

    pub async fn create_user(&self, input: CreateUserRow) -> Result<UserRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = UserRow {
            id,
            email: input.email,
            name: input.name,
            avatar_url: input.avatar_url,
            roles: serde_json::to_value(&input.roles)?,
            password_hash: input.password_hash,
            email_verified: input.email_verified,
            auth_provider: input.auth_provider,
            auth_provider_id: input.auth_provider_id,
            external_id: input.external_id,
            created_at: now,
            updated_at: now,
        };
        self.users.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create user with a specific UUID (for seeding).
    /// Returns None if id already exists.
    pub async fn create_user_with_id(
        &self,
        id: Uuid,
        input: CreateUserRow,
    ) -> Result<Option<UserRow>> {
        let now = Self::now();
        let mut users = self.users.write();
        if users.contains_key(&id) {
            return Ok(None);
        }
        let row = UserRow {
            id,
            email: input.email,
            name: input.name,
            avatar_url: input.avatar_url,
            roles: serde_json::to_value(&input.roles)?,
            password_hash: input.password_hash,
            email_verified: input.email_verified,
            auth_provider: input.auth_provider,
            auth_provider_id: input.auth_provider_id,
            external_id: input.external_id,
            created_at: now,
            updated_at: now,
        };
        users.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| u.email == email)
            .cloned())
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        Ok(self.users.read().get(&id).cloned())
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| {
                u.auth_provider.as_deref() == Some(provider)
                    && u.auth_provider_id.as_deref() == Some(provider_id)
            })
            .cloned())
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        let mut users = self.users.write();
        if let Some(user) = users.get_mut(&id) {
            if let Some(name) = input.name {
                user.name = name;
            }
            if let Some(avatar_url) = input.avatar_url {
                user.avatar_url = Some(avatar_url);
            }
            if let Some(roles) = input.roles {
                user.roles = serde_json::to_value(&roles)?;
            }
            if let Some(password_hash) = input.password_hash {
                user.password_hash = Some(password_hash);
            }
            if let Some(email_verified) = input.email_verified {
                user.email_verified = email_verified;
            }
            user.updated_at = Self::now();
            return Ok(Some(user.clone()));
        }
        Ok(None)
    }

    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        let users = self.users.read();
        let mut result: Vec<_> = match search {
            Some(query) if !query.trim().is_empty() => {
                let pattern = query.trim().to_lowercase();
                users
                    .values()
                    .filter(|u| {
                        u.name.to_lowercase().contains(&pattern)
                            || u.email.to_lowercase().contains(&pattern)
                    })
                    .cloned()
                    .collect()
            }
            _ => users.values().cloned().collect(),
        };
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    /// Hard-delete a user and all associated data.
    pub async fn delete_user_account(&self, user_id: Uuid) -> Result<bool> {
        let removed = self.users.write().remove(&user_id).is_some();
        if removed {
            // Cascade: remove API keys
            self.api_keys.write().retain(|_, k| k.user_id != user_id);
            // Cascade: remove refresh tokens
            self.refresh_tokens
                .write()
                .retain(|_, t| t.user_id != user_id);
            // Cascade: remove CLI auth sessions
            self.cli_auth_sessions
                .write()
                .retain(|_, s| s.user_id != Some(user_id));
            // Cascade: remove organization memberships
            self.organization_members
                .write()
                .retain(|&(_, uid), _| uid != user_id);
            // Cascade: remove user connections
            self.user_connections
                .write()
                .retain(|_, c| c.user_id != user_id);
            // Cascade: remove pinned sessions
            self.pinned_sessions
                .write()
                .retain(|&(uid, _), _| uid != user_id);
            // Cascade: remove notifications
            self.notifications
                .write()
                .retain(|_, n| n.user_id != user_id);
            // Cascade: remove OAuth auth codes
            self.oauth_authorization_codes
                .write()
                .retain(|_, c| c.user_id != user_id);
            // Cascade: remove OAuth refresh tokens
            self.oauth_refresh_tokens
                .write()
                .retain(|_, t| t.user_id != user_id);
        }
        Ok(removed)
    }

    /// Export all user-owned data as a structured JSON value.
    pub async fn export_user_data(&self, user_id: Uuid) -> Result<Option<serde_json::Value>> {
        let user = self.get_user(user_id).await?;
        let Some(user) = user else {
            return Ok(None);
        };

        let api_keys = self.list_api_keys_for_user(user_id).await?;
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
            "api_keys": api_keys.iter().map(|k| serde_json::json!({
                "id": k.id.to_string(),
                "name": k.name,
                "key_prefix": k.key_prefix,
                "scopes": k.scopes,
                "expires_at": k.expires_at,
                "last_used_at": k.last_used_at,
                "created_at": k.created_at,
            })).collect::<Vec<_>>(),
            "exported_at": chrono::Utc::now(),
        });

        Ok(Some(export))
    }

    /// List users within an organization (TM-TENANT-008: org-scoped user listing)
    pub async fn list_users_by_org(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<UserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();

        // Get user IDs that belong to this org
        let org_user_ids: std::collections::HashSet<_> = members
            .keys()
            .filter(|(oid, _)| *oid == org_id)
            .map(|(_, uid)| *uid)
            .collect();

        let mut result: Vec<_> = users
            .values()
            .filter(|u| org_user_ids.contains(&u.id))
            .filter(|u| match search {
                Some(query) if !query.trim().is_empty() => {
                    let pattern = query.trim().to_lowercase();
                    u.name.to_lowercase().contains(&pattern)
                        || u.email.to_lowercase().contains(&pattern)
                }
                _ => true,
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }
}
