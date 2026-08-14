// In-memory storage: Organization Settings (in-memory), Organizations, Organization Members

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_provider::typed_id::ModelId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Organization Settings (in-memory)
    // ============================================

    pub async fn get_organization_settings(
        &self,
        org_id: i64,
    ) -> Result<Option<OrganizationSettingsRow>> {
        Ok(self.org_settings.read().get(&org_id).cloned())
    }

    pub async fn upsert_organization_settings(
        &self,
        org_id: i64,
        default_model_id: Option<Uuid>,
    ) -> Result<OrganizationSettingsRow> {
        let now = Self::now();
        let mut settings = self.org_settings.write();
        let row = settings
            .entry(org_id)
            .or_insert_with(|| OrganizationSettingsRow {
                org_id,
                default_model_id: None,
                default_harness_id: None,
                base_harness_id: None,
                default_provider_per_service: sqlx::types::Json(ServiceProviderDefaults::new()),
                created_at: now,
                updated_at: now,
            });
        row.default_model_id = default_model_id.map(ModelId::from_uuid);
        row.updated_at = now;
        Ok(row.clone())
    }

    pub async fn patch_organization_settings(
        &self,
        org_id: i64,
        input: UpdateOrganizationSettings,
    ) -> Result<OrganizationSettingsRow> {
        let now = Self::now();
        let mut settings = self.org_settings.write();
        let row = settings
            .entry(org_id)
            .or_insert_with(|| OrganizationSettingsRow {
                org_id,
                default_model_id: None,
                default_harness_id: None,
                base_harness_id: None,
                default_provider_per_service: sqlx::types::Json(ServiceProviderDefaults::new()),
                created_at: now,
                updated_at: now,
            });

        input.default_model_id.apply(&mut row.default_model_id);
        input.default_harness_id.apply(&mut row.default_harness_id);
        input.base_harness_id.apply(&mut row.base_harness_id);
        // The per-service default map is replaced wholesale (mirrors the SQL path).
        match input.default_provider_per_service {
            everruns_durable::UpdateField::Unchanged => {}
            everruns_durable::UpdateField::Clear => {
                row.default_provider_per_service =
                    sqlx::types::Json(ServiceProviderDefaults::new());
            }
            everruns_durable::UpdateField::Set(map) => {
                row.default_provider_per_service = sqlx::types::Json(map);
            }
        }

        row.updated_at = now;
        Ok(row.clone())
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        let now = Self::now();
        let mut orgs = self.organizations.write();
        let org_id = orgs.keys().max().unwrap_or(&0) + 1;
        let row = OrganizationRow {
            org_id,
            public_id: input.public_id,
            name: input.name,
            created_at: now,
            updated_at: now,
            external_id: None,
            created_by: input.created_by,
            // User-created orgs start un-onboarded; the setup wizard marks them.
            onboarding_completed_at: None,
        };
        orgs.insert(org_id, row.clone());
        Ok(row)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        let now = Self::now();
        let mut orgs = self.organizations.write();
        if orgs.contains_key(&org_id) {
            return Ok(None);
        }
        let row = OrganizationRow {
            org_id,
            public_id: input.public_id,
            name: input.name,
            created_at: now,
            updated_at: now,
            external_id: None,
            created_by: input.created_by,
            // Seeded orgs (default org) are already-onboarded. See migration 090.
            onboarding_completed_at: Some(now),
        };
        orgs.insert(org_id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        Ok(self.organizations.read().get(&org_id).cloned())
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        Ok(self
            .organizations
            .read()
            .values()
            .find(|o| o.public_id == public_id)
            .cloned())
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        let orgs = self.organizations.read();
        let mut result: Vec<_> = orgs.values().cloned().collect();
        result.sort_by_key(|organization| std::cmp::Reverse(organization.created_at));
        Ok(result)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        let mut orgs = self.organizations.write();
        if let Some(org) = orgs.get_mut(&org_id) {
            if let Some(name) = input.name {
                org.name = name;
            }
            org.updated_at = Self::now();
            return Ok(Some(org.clone()));
        }
        Ok(None)
    }

    /// Idempotently mark an org's onboarding complete (sets the timestamp only
    /// when currently NULL), mirroring the Postgres path.
    pub async fn mark_org_onboarding_complete(&self, org_id: i64) -> Result<()> {
        let now = Self::now();
        let mut orgs = self.organizations.write();
        if let Some(org) = orgs.get_mut(&org_id)
            && org.onboarding_completed_at.is_none()
        {
            org.onboarding_completed_at = Some(now);
            org.updated_at = now;
        }
        Ok(())
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        Ok(self.organizations.write().remove(&org_id).is_some())
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
        let now = Self::now();
        let row = OrganizationMemberRow {
            org_id,
            user_id,
            role: role.to_string(),
            created_at: now,
        };
        self.organization_members
            .write()
            .insert((org_id, user_id), row.clone());
        Ok(row)
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        Ok(self
            .organization_members
            .write()
            .remove(&(org_id, user_id))
            .is_some())
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        let members = self.organization_members.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect();
        result.sort_by_key(|invitation| std::cmp::Reverse(invitation.created_at));
        Ok(result)
    }

    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.org_id == org_id)
            .filter_map(|m| {
                users
                    .get(&m.user_id)
                    .map(|u| OrganizationMemberWithUserRow {
                        user_id: u.id,
                        email: u.email.clone(),
                        name: u.name.clone(),
                        avatar_url: u.avatar_url.clone(),
                        role: m.role.clone(),
                        joined_at: m.created_at,
                    })
            })
            .collect();
        result.sort_by_key(|membership| membership.joined_at);
        Ok(result)
    }

    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();
        Ok(members.get(&(org_id, user_id)).and_then(|m| {
            users
                .get(&m.user_id)
                .map(|u| OrganizationMemberWithUserRow {
                    user_id: u.id,
                    email: u.email.clone(),
                    name: u.name.clone(),
                    avatar_url: u.avatar_url.clone(),
                    role: m.role.clone(),
                    joined_at: m.created_at,
                })
        }))
    }

    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        let mut members = self.organization_members.write();
        if let Some(member) = members.get_mut(&(org_id, user_id)) {
            member.role = role.to_string();
            return Ok(Some(member.clone()));
        }
        Ok(None)
    }

    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        let members = self.organization_members.read();
        let count = members
            .values()
            .filter(|m| m.org_id == org_id && m.role == "owner")
            .count();
        Ok(count as i64)
    }

    pub async fn count_user_created_organizations(&self, user_id: Uuid) -> Result<i64> {
        let orgs = self.organizations.read();
        let count = orgs
            .values()
            .filter(|o| o.created_by == Some(user_id))
            .count();
        Ok(count as i64)
    }

    pub async fn count_organization_members(&self, org_id: i64) -> Result<i64> {
        let members = self.organization_members.read();
        let count = members.values().filter(|m| m.org_id == org_id).count();
        Ok(count as i64)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        let members = self.organization_members.read();
        let orgs = self.organizations.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.user_id == user_id)
            .filter_map(|m| {
                orgs.get(&m.org_id).map(|o| OrganizationWithRoleRow {
                    org_id: o.org_id,
                    public_id: o.public_id.clone(),
                    name: o.name.clone(),
                    role: m.role.clone(),
                })
            })
            .collect();
        result.sort_by_key(|organization| organization.name.clone());
        Ok(result)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        Ok(self
            .organization_members
            .read()
            .contains_key(&(org_id, user_id)))
    }

    /// Get user by external identity provider ID
    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| u.external_id.as_deref() == Some(external_id))
            .cloned())
    }

    /// Get organization by external identity provider ID
    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        Ok(self
            .organizations
            .read()
            .values()
            .find(|o| o.external_id.as_deref() == Some(external_id))
            .cloned())
    }

    /// Upsert organization by external ID (for external auth provider sync)
    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        let mut orgs = self.organizations.write();

        // Check if org with this external_id already exists
        if let Some(existing) = orgs
            .values_mut()
            .find(|o| o.external_id.as_deref() == Some(external_id))
        {
            existing.name = name.to_string();
            existing.updated_at = Self::now();
            return Ok(existing.clone());
        }

        // Create new org
        let org_id = orgs.keys().max().unwrap_or(&0) + 1;
        let now = Self::now();
        let row = OrganizationRow {
            org_id,
            public_id: public_id.to_string(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            external_id: Some(external_id.to_string()),
            created_by: None,
            // Externally-synced orgs are already-onboarded. See migration 090.
            onboarding_completed_at: Some(now),
        };
        orgs.insert(org_id, row.clone());
        Ok(row)
    }

    /// Ensure user is a member of organization with the given role (upsert).
    /// Updates the role if the membership already exists.
    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        let key = (org_id, user_id);
        let mut members = self.organization_members.write();
        members
            .entry(key)
            .and_modify(|m| m.role = role.to_string())
            .or_insert_with(|| OrganizationMemberRow {
                org_id,
                user_id,
                role: role.to_string(),
                created_at: Self::now(),
            });
        Ok(())
    }

    /// Reconcile org memberships to match an authoritative list.
    /// Operates under a single write lock for atomicity.
    /// Duplicate user_ids in the authoritative list are deduplicated (last wins).
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

        let mut members = self.organization_members.write();
        let now = Self::now();

        // Build current state under the write lock
        let current_map: std::collections::HashMap<Uuid, String> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), row)| (*uid, row.role.clone()))
            .collect();

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;

        // Add or update from the deduped map
        for (user_id, role) in &auth_map {
            match current_map.get(user_id) {
                None => {
                    members.insert(
                        (org_id, *user_id),
                        OrganizationMemberRow {
                            org_id,
                            user_id: *user_id,
                            role: role.to_string(),
                            created_at: now,
                        },
                    );
                    added += 1;
                }
                Some(existing_role) if existing_role != role => {
                    if let Some(row) = members.get_mut(&(org_id, *user_id)) {
                        row.role = role.to_string();
                    }
                    updated += 1;
                }
                _ => {} // unchanged
            }
        }

        // Remove members not in authoritative list
        for user_id in current_map.keys() {
            if !auth_map.contains_key(user_id) {
                members.remove(&(org_id, *user_id));
                removed += 1;
            }
        }

        Ok((added, updated, removed))
    }

    // ============================================
    // Organization Task Webhooks
    // ============================================

    pub async fn list_org_task_webhooks(&self, org_id: i64) -> Result<Vec<OrgTaskWebhookRow>> {
        let mut rows: Vec<_> = self
            .org_task_webhooks
            .read()
            .iter()
            .filter(|w| w.org_id == org_id)
            .cloned()
            .collect();
        rows.sort_by_key(|w| w.created_at);
        Ok(rows)
    }

    pub async fn list_enabled_org_task_webhooks(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrgTaskWebhookRow>> {
        let mut rows: Vec<_> = self
            .org_task_webhooks
            .read()
            .iter()
            .filter(|w| w.org_id == org_id && w.enabled)
            .cloned()
            .collect();
        rows.sort_by_key(|w| w.created_at);
        Ok(rows)
    }

    pub async fn create_org_task_webhook(
        &self,
        input: CreateOrgTaskWebhook,
    ) -> Result<OrgTaskWebhookRow> {
        let now = Self::now();
        let mut webhooks = self.org_task_webhooks.write();
        let id = webhooks.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        let row = OrgTaskWebhookRow {
            id,
            public_id: input.public_id,
            org_id: input.org_id,
            url: input.url,
            secret: input.secret,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
        };
        webhooks.push(row.clone());
        Ok(row)
    }

    pub async fn get_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        Ok(self
            .org_task_webhooks
            .read()
            .iter()
            .find(|w| w.org_id == org_id && w.public_id == public_id)
            .cloned())
    }

    pub async fn update_org_task_webhook(
        &self,
        org_id: i64,
        public_id: &str,
        input: UpdateOrgTaskWebhook,
    ) -> Result<Option<OrgTaskWebhookRow>> {
        let now = Self::now();
        let mut webhooks = self.org_task_webhooks.write();
        if let Some(w) = webhooks
            .iter_mut()
            .find(|w| w.org_id == org_id && w.public_id == public_id)
        {
            if let Some(url) = input.url {
                w.url = url;
            }
            if let Some(secret) = input.secret {
                w.secret = secret;
            }
            if let Some(enabled) = input.enabled {
                w.enabled = enabled;
            }
            w.updated_at = now;
            return Ok(Some(w.clone()));
        }
        Ok(None)
    }

    pub async fn delete_org_task_webhook(&self, org_id: i64, public_id: &str) -> Result<bool> {
        let mut webhooks = self.org_task_webhooks.write();
        let before = webhooks.len();
        webhooks.retain(|w| !(w.org_id == org_id && w.public_id == public_id));
        Ok(webhooks.len() < before)
    }

    // ============================================
    // Organization Invitations (EVE-602)
    // ============================================

    pub async fn create_org_invitation(
        &self,
        input: CreateOrgInvitation,
    ) -> Result<OrgInvitationRow> {
        let now = Self::now();
        let mut invitations = self.org_invitations.write();
        let id = invitations.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let row = OrgInvitationRow {
            id,
            public_id: input.public_id,
            org_id: input.org_id,
            email: input.email,
            role: input.role,
            invited_by: input.invited_by,
            token_hash: input.token_hash,
            expires_at: input.expires_at,
            accepted_at: None,
            accepted_by: None,
            revoked_at: None,
            created_at: now,
            updated_at: now,
        };
        invitations.push(row.clone());
        Ok(row)
    }

    /// List invitations that are still outstanding (not accepted, not revoked),
    /// newest first. Expired-but-unresolved invites are included so callers can
    /// present them distinctly; callers derive status from the timestamps.
    pub async fn list_pending_org_invitations(&self, org_id: i64) -> Result<Vec<OrgInvitationRow>> {
        let mut rows: Vec<_> = self
            .org_invitations
            .read()
            .iter()
            .filter(|i| i.org_id == org_id && i.accepted_at.is_none() && i.revoked_at.is_none())
            .cloned()
            .collect();
        rows.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        Ok(rows)
    }

    pub async fn get_org_invitation_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        Ok(self
            .org_invitations
            .read()
            .iter()
            .find(|i| i.token_hash == token_hash)
            .cloned())
    }

    pub async fn get_org_invitation_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        Ok(self
            .org_invitations
            .read()
            .iter()
            .find(|i| i.org_id == org_id && i.public_id == public_id)
            .cloned())
    }

    /// The outstanding (not accepted, not revoked) invitation for an email in an
    /// org, if any. Mirrors the Postgres `idx_org_invitations_pending_email`
    /// partial unique index predicate — including expired-but-unrevoked rows.
    pub async fn get_outstanding_org_invitation_by_email(
        &self,
        org_id: i64,
        email: &str,
    ) -> Result<Option<OrgInvitationRow>> {
        Ok(self
            .org_invitations
            .read()
            .iter()
            .find(|i| {
                i.org_id == org_id
                    && i.email == email
                    && i.accepted_at.is_none()
                    && i.revoked_at.is_none()
            })
            .cloned())
    }

    /// Mark a pending invitation revoked. Returns false if it was missing or no
    /// longer pending.
    pub async fn revoke_org_invitation(&self, org_id: i64, public_id: &str) -> Result<bool> {
        let now = Self::now();
        let mut invitations = self.org_invitations.write();
        if let Some(inv) = invitations.iter_mut().find(|i| {
            i.org_id == org_id
                && i.public_id == public_id
                && i.accepted_at.is_none()
                && i.revoked_at.is_none()
        }) {
            inv.revoked_at = Some(now);
            inv.updated_at = now;
            return Ok(true);
        }
        Ok(false)
    }

    /// Atomically mark an invitation accepted. Returns the updated row only when
    /// it was still pending, so concurrent accepts cannot both win.
    pub async fn accept_org_invitation(
        &self,
        invitation_id: i64,
        accepted_by: Uuid,
    ) -> Result<Option<OrgInvitationRow>> {
        let now = Self::now();
        let mut invitations = self.org_invitations.write();
        if let Some(inv) = invitations
            .iter_mut()
            .find(|i| i.id == invitation_id && i.accepted_at.is_none() && i.revoked_at.is_none())
        {
            inv.accepted_at = Some(now);
            inv.accepted_by = Some(accepted_by);
            inv.updated_at = now;
            return Ok(Some(inv.clone()));
        }
        Ok(None)
    }
}
