// In-memory storage: Organization Settings (in-memory), Organizations, Organization Members

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::ModelId;
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
                created_at: now,
                updated_at: now,
            });

        input.default_model_id.apply(&mut row.default_model_id);
        input.default_harness_id.apply(&mut row.default_harness_id);
        input.base_harness_id.apply(&mut row.base_harness_id);

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
        result.sort_by(|a, b| a.name.cmp(&b.name));
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
}
