// In-memory storage: App Channel CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use crate::errors::BadRequestError;
use crate::storage::{CreateAgentEndpointRow, IngressEndpointRow, UpdateAgentEndpointRow};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    fn sync_ingress_endpoint(&self, channel: &AppChannelRow) {
        let mut endpoints = self.ingress_endpoints.write();
        let Some(endpoint) = endpoints.get_mut(&channel.id) else {
            return;
        };
        endpoint.channel_type = channel.channel_type.clone();
        endpoint.channel_config = channel.channel_config.clone();
        endpoint.channel_config_encrypted = channel.channel_config_encrypted.clone();
        endpoint.auth = channel.auth.clone();
        endpoint.auth_encrypted = channel.auth_encrypted.clone();
        endpoint.enabled = channel.enabled;
        endpoint.endpoint_status = channel.status.clone();
        endpoint.updated_at = channel.updated_at;
    }
}

fn initial_status(enabled: bool) -> String {
    if enabled { "draft" } else { "disabled" }.to_string()
}

impl InMemoryDatabase {
    // ============================================
    // App Channel CRUD
    // ============================================

    // Mirrors the PostgreSQL backend, where an endpoint row carries a NOT NULL
    // `agent_id` derived from its App (EVE-1003). Without this the in-memory
    // backend would accept a channel the real one rejects.
    fn require_app_agent(&self, app_id: Uuid) -> Result<()> {
        let has_agent = self
            .apps
            .read()
            .get(&app_id)
            .is_some_and(|app| app.agent_id.is_some());
        if has_agent {
            Ok(())
        } else {
            Err(BadRequestError::new(format!(
                "App {app_id} was not found or has no agent; an endpoint must be owned by an agent"
            ))
            .into())
        }
    }

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        self.require_app_agent(app_id)?;
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AppChannelRow {
            id,
            app_id,
            public_id: input.public_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            auth: input.auth,
            auth_encrypted: input.auth_encrypted,
            durable_schedule_id: input.durable_schedule_id,
            enabled: input.enabled,
            status: initial_status(input.enabled),
            created_at: now,
            updated_at: now,
        };
        let ingress = self.ingress_endpoint_row(row.clone());
        self.app_channels.write().insert(id, row.clone());
        if let Some(ingress) = ingress {
            self.ingress_endpoints.write().insert(id, ingress);
        }
        Ok(row)
    }

    pub async fn create_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        app_id: Uuid,
        input: CreateAppChannelRow,
        max_enabled_schedule_channels: i64,
    ) -> Result<AppChannelRow> {
        self.require_app_agent(app_id)?;
        let apps = self.apps.read();
        let org_app_ids: std::collections::HashSet<Uuid> = apps
            .values()
            .filter(|a| a.org_id == org_id)
            .map(|a| a.id)
            .collect();
        drop(apps);

        let mut channels = self.app_channels.write();
        let count = channels
            .values()
            .filter(|ch| {
                org_app_ids.contains(&ch.app_id) && ch.channel_type == "schedule" && ch.enabled
            })
            .count() as i64;
        if count >= max_enabled_schedule_channels {
            return Err(BadRequestError::new(format!(
                "Organization may have at most {max_enabled_schedule_channels} enabled schedule channel(s); currently has {count}"
            ))
            .into());
        }

        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AppChannelRow {
            id,
            app_id,
            public_id: input.public_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            auth: input.auth,
            auth_encrypted: input.auth_encrypted,
            durable_schedule_id: input.durable_schedule_id,
            enabled: input.enabled,
            status: initial_status(input.enabled),
            created_at: now,
            updated_at: now,
        };
        let ingress = self.ingress_endpoint_row(row.clone());
        channels.insert(id, row.clone());
        drop(channels);
        if let Some(ingress) = ingress {
            self.ingress_endpoints.write().insert(id, ingress);
        }
        Ok(row)
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        let channels = self.app_channels.read();
        let mut result: Vec<AppChannelRow> = channels
            .values()
            .filter(|ch| ch.app_id == app_id)
            .cloned()
            .collect();
        result.sort_by_key(|channel| channel.created_at);
        Ok(result)
    }

    pub async fn app_has_channels(&self, app_id: Uuid) -> Result<bool> {
        let channels = self.app_channels.read();
        Ok(channels.values().any(|ch| ch.app_id == app_id))
    }

    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        let channels = self.app_channels.read();
        Ok(channels
            .values()
            .find(|ch| ch.public_id == public_id)
            .cloned())
    }

    pub async fn get_ingress_endpoint_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        Ok(self
            .ingress_endpoints
            .read()
            .values()
            .find(|endpoint| endpoint.endpoint_public_id == public_id)
            .cloned())
    }

    pub async fn list_ingress_endpoints_by_legacy_alias(
        &self,
        legacy_app_public_id: &str,
        channel_type: &str,
    ) -> Result<Vec<IngressEndpointRow>> {
        let mut endpoints: Vec<_> = self
            .ingress_endpoints
            .read()
            .values()
            .filter(|endpoint| {
                endpoint.legacy_app_public_id.as_deref() == Some(legacy_app_public_id)
                    && endpoint.channel_type == channel_type
                    && endpoint.enabled
            })
            .cloned()
            .collect();
        endpoints.sort_by_key(|endpoint| (endpoint.created_at, endpoint.endpoint_id));
        Ok(endpoints)
    }

    pub async fn list_agent_endpoints(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<IngressEndpointRow>> {
        let mut endpoints: Vec<_> = self
            .ingress_endpoints
            .read()
            .values()
            .filter(|endpoint| endpoint.org_id == org_id && endpoint.agent_id == agent_id)
            .cloned()
            .collect();
        endpoints.sort_by_key(|endpoint| (endpoint.created_at, endpoint.endpoint_id));
        Ok(endpoints)
    }

    pub async fn get_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        Ok(self
            .ingress_endpoints
            .read()
            .values()
            .find(|endpoint| {
                endpoint.org_id == org_id
                    && endpoint.agent_id == agent_id
                    && endpoint.endpoint_public_id == public_id
            })
            .cloned())
    }

    pub async fn create_agent_endpoint(
        &self,
        org_id: i64,
        input: CreateAgentEndpointRow,
    ) -> Result<IngressEndpointRow> {
        let agent = self
            .agents
            .read()
            .values()
            .find(|agent| {
                agent.org_id == org_id
                    && agent.id.uuid() == input.agent_id
                    && agent.status == "active"
            })
            .cloned()
            .ok_or_else(|| BadRequestError::new("Agent was not found or is not active"))?;
        let now = Self::now();
        let endpoint_id = Uuid::now_v7();
        let row = IngressEndpointRow {
            endpoint_id,
            endpoint_public_id: input.public_id,
            legacy_app_id: None,
            legacy_app_public_id: None,
            org_id,
            agent_id: input.agent_id,
            agent_public_id: agent.public_id,
            agent_name: agent.display_name.unwrap_or(agent.name),
            agent_description: agent.description,
            harness_id: agent.harness_id.uuid(),
            agent_status: agent.status,
            exposures_suspended: agent.exposures_suspended,
            agent_identity_id: input.agent_identity_id,
            agent_version_policy: input.agent_version_policy,
            agent_version_id: input.agent_version_id,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            auth: input.auth,
            auth_encrypted: input.auth_encrypted,
            enabled: input.enabled,
            endpoint_status: input.status,
            created_at: now,
            updated_at: now,
        };
        self.ingress_endpoints
            .write()
            .insert(endpoint_id, row.clone());
        Ok(row)
    }

    pub async fn update_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        input: UpdateAgentEndpointRow,
    ) -> Result<Option<IngressEndpointRow>> {
        let mut endpoints = self.ingress_endpoints.write();
        let Some(endpoint) = endpoints.values_mut().find(|endpoint| {
            endpoint.org_id == org_id
                && endpoint.agent_id == agent_id
                && endpoint.endpoint_public_id == public_id
        }) else {
            return Ok(None);
        };
        if let Some(channel_type) = input.channel_type {
            endpoint.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            endpoint.channel_config = channel_config;
        }
        input
            .channel_config_encrypted
            .apply(&mut endpoint.channel_config_encrypted);
        input.auth.apply(&mut endpoint.auth);
        input.auth_encrypted.apply(&mut endpoint.auth_encrypted);
        if let Some(enabled) = input.enabled {
            endpoint.enabled = enabled;
        }
        if let Some(status) = input.status {
            endpoint.endpoint_status = status;
        }
        endpoint.updated_at = Self::now();
        Ok(Some(endpoint.clone()))
    }

    pub async fn delete_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<bool> {
        let mut endpoints = self.ingress_endpoints.write();
        let Some(id) = endpoints
            .values()
            .find(|endpoint| {
                endpoint.org_id == org_id
                    && endpoint.agent_id == agent_id
                    && endpoint.endpoint_public_id == public_id
            })
            .map(|endpoint| endpoint.endpoint_id)
        else {
            return Ok(false);
        };
        if let Some(endpoint) = endpoints.get_mut(&id)
            && endpoint.legacy_app_id.is_some()
        {
            endpoint.enabled = false;
            endpoint.endpoint_status = "disabled".to_string();
            endpoint.updated_at = Self::now();
        } else {
            endpoints.remove(&id);
        }
        Ok(true)
    }

    fn ingress_endpoint_row(&self, channel: AppChannelRow) -> Option<IngressEndpointRow> {
        let app = self.apps.read().get(&channel.app_id)?.clone();
        let agent_id = app.agent_id?;
        let agent = self
            .agents
            .read()
            .values()
            .find(|agent| agent.id.uuid() == agent_id)?
            .clone();
        Some(IngressEndpointRow {
            endpoint_id: channel.id,
            endpoint_public_id: channel.public_id,
            legacy_app_id: Some(app.id),
            legacy_app_public_id: Some(app.public_id),
            org_id: app.org_id,
            agent_id,
            agent_public_id: agent.public_id,
            agent_name: agent.display_name.unwrap_or(agent.name),
            agent_description: agent.description,
            harness_id: agent.harness_id.uuid(),
            agent_status: agent.status,
            exposures_suspended: agent.exposures_suspended,
            agent_identity_id: app.agent_identity_id,
            agent_version_policy: app.agent_version_policy,
            agent_version_id: app.agent_version_id,
            owner_principal_id: app.owner_principal_id.uuid(),
            resolved_owner_user_id: app.resolved_owner_user_id,
            channel_type: channel.channel_type,
            channel_config: channel.channel_config,
            channel_config_encrypted: channel.channel_config_encrypted,
            auth: channel.auth,
            auth_encrypted: channel.auth_encrypted,
            enabled: channel.enabled,
            endpoint_status: channel.status,
            created_at: channel.created_at,
            updated_at: channel.updated_at,
        })
    }

    pub async fn get_agent_endpoint_public_id(
        &self,
        org_id: i64,
        endpoint_id: Uuid,
    ) -> Result<Option<String>> {
        let channel = self.app_channels.read().get(&endpoint_id).cloned();
        let Some(channel) = channel else {
            return Ok(None);
        };
        let agent_id = self
            .apps
            .read()
            .get(&channel.app_id)
            .and_then(|app| app.agent_id);
        let belongs_to_org = agent_id.is_some_and(|agent_id| {
            self.agents
                .read()
                .values()
                .any(|agent| agent.id.uuid() == agent_id && agent.org_id == org_id)
        });
        Ok(belongs_to_org.then_some(channel.public_id))
    }

    pub async fn update_app_channel(
        &self,
        id: Uuid,
        input: UpdateAppChannel,
    ) -> Result<Option<AppChannelRow>> {
        let mut channels = self.app_channels.write();
        let Some(ch) = channels.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(channel_type) = input.channel_type {
            ch.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            ch.channel_config = channel_config;
        }
        input
            .channel_config_encrypted
            .apply(&mut ch.channel_config_encrypted);
        input.auth.apply(&mut ch.auth);
        input.auth_encrypted.apply(&mut ch.auth_encrypted);
        input.durable_schedule_id.apply(&mut ch.durable_schedule_id);
        if let Some(enabled) = input.enabled {
            ch.enabled = enabled;
            if !enabled {
                ch.status = "disabled".to_string();
            }
        }
        if let Some(status) = input.status.clone() {
            ch.status = status;
        }
        ch.updated_at = Self::now();
        let updated = ch.clone();
        drop(channels);
        self.sync_ingress_endpoint(&updated);
        Ok(Some(updated))
    }

    pub async fn update_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateAppChannel,
        max_enabled_schedule_channels: i64,
    ) -> Result<Option<AppChannelRow>> {
        let apps = self.apps.read();
        let org_app_ids: std::collections::HashSet<Uuid> = apps
            .values()
            .filter(|a| a.org_id == org_id)
            .map(|a| a.id)
            .collect();
        drop(apps);

        let mut channels = self.app_channels.write();
        let count = channels
            .values()
            .filter(|ch| {
                org_app_ids.contains(&ch.app_id) && ch.channel_type == "schedule" && ch.enabled
            })
            .count() as i64;
        if count >= max_enabled_schedule_channels {
            return Err(BadRequestError::new(format!(
                "Organization may have at most {max_enabled_schedule_channels} enabled schedule channel(s); currently has {count}"
            ))
            .into());
        }

        let Some(ch) = channels.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(channel_type) = input.channel_type {
            ch.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            ch.channel_config = channel_config;
        }
        input
            .channel_config_encrypted
            .apply(&mut ch.channel_config_encrypted);
        input.auth.apply(&mut ch.auth);
        input.auth_encrypted.apply(&mut ch.auth_encrypted);
        input.durable_schedule_id.apply(&mut ch.durable_schedule_id);
        if let Some(enabled) = input.enabled {
            ch.enabled = enabled;
            // Same derivation as the PostgreSQL backend: disabling always
            // lowers the endpoint, and re-enabling returns it to whatever the
            // owning App's publish state implies — so re-enabling a channel on
            // a published App makes it live again rather than stranding it in
            // draft.
            ch.status = derive_status(enabled, app_status.as_deref());
        }
        if let Some(status) = input.status.clone() {
            ch.status = status;
        }
        ch.updated_at = Self::now();
        let updated = ch.clone();
        drop(channels);
        self.sync_ingress_endpoint(&updated);
        Ok(Some(updated))
    }

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        let mut channels = self.app_channels.write();
        let removed = channels.remove(&id).is_some();
        drop(channels);
        self.ingress_endpoints.write().remove(&id);
        Ok(removed)
    }

    /// See the PostgreSQL backend.
    pub async fn agents_with_live_endpoints(
        &self,
        agent_ids: &[Uuid],
    ) -> Result<std::collections::HashSet<Uuid>> {
        let wanted: std::collections::HashSet<Uuid> = agent_ids.iter().copied().collect();
        let apps = self.apps.read();
        let app_agent: std::collections::HashMap<Uuid, Uuid> = apps
            .values()
            .filter_map(|a| a.agent_id.map(|agent| (a.id, agent)))
            .collect();
        drop(apps);
        let channels = self.app_channels.read();
        Ok(channels
            .values()
            .filter(|ch| ch.status == "live")
            .filter_map(|ch| app_agent.get(&ch.app_id).copied())
            .filter(|agent| wanted.contains(agent))
            .collect())
    }

    /// See the PostgreSQL backend: bridges App publish onto endpoint status.
    pub async fn set_app_endpoint_publish(&self, app_id: Uuid, published: bool) -> Result<u64> {
        let mut channels = self.app_channels.write();
        let mut changed = 0;
        for ch in channels.values_mut().filter(|ch| ch.app_id == app_id) {
            if published {
                if ch.enabled && ch.status != "live" {
                    ch.status = "live".to_string();
                    changed += 1;
                }
            } else if ch.status == "live" {
                ch.status = "draft".to_string();
                changed += 1;
            }
        }
        let updated: Vec<_> = channels
            .values()
            .filter(|channel| channel.app_id == app_id)
            .cloned()
            .collect();
        drop(channels);
        for channel in updated {
            self.sync_ingress_endpoint(&channel);
        }
        Ok(changed)
    }

    pub async fn count_enabled_schedule_channels_for_org(&self, org_id: i64) -> Result<i64> {
        let apps = self.apps.read();
        let channels = self.app_channels.read();
        let org_app_ids: std::collections::HashSet<Uuid> = apps
            .values()
            .filter(|a| a.org_id == org_id)
            .map(|a| a.id)
            .collect();
        let count = channels
            .values()
            .filter(|ch| {
                org_app_ids.contains(&ch.app_id) && ch.channel_type == "schedule" && ch.enabled
            })
            .count();
        Ok(count as i64)
    }
}
