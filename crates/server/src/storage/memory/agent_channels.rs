// In-memory storage: agent channel CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use crate::errors::BadRequestError;
use crate::storage::{CreateAgentChannelRow, IngressChannelRow, UpdateAgentChannelRow};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn list_agent_channel_summaries(
        &self,
        org_id: i64,
        agent_ids: &[Uuid],
    ) -> Result<Vec<crate::storage::AgentChannelSummaryRow>> {
        let endpoints = self.ingress_channels.read();
        let mut rows: Vec<_> = endpoints
            .values()
            .filter(|endpoint| {
                endpoint.org_id == org_id
                    && agent_ids.contains(&endpoint.agent_id)
                    && endpoint.channel_type != "schedule"
            })
            .collect();
        rows.sort_by_key(|endpoint| (endpoint.created_at, endpoint.channel_id));
        Ok(rows
            .into_iter()
            .map(|endpoint| crate::storage::AgentChannelSummaryRow {
                agent_id: endpoint.agent_id,
                public_id: endpoint.channel_public_id.clone(),
                channel_type: endpoint.channel_type.clone(),
                enabled: endpoint.enabled,
                status: endpoint.channel_status.clone(),
            })
            .collect())
    }

    fn sync_ingress_channel(&self, channel: &AgentChannelRow) {
        let mut endpoints = self.ingress_channels.write();
        let Some(endpoint) = endpoints.get_mut(&channel.id) else {
            return;
        };
        endpoint.channel_type = channel.channel_type.clone();
        endpoint.channel_config = channel.channel_config.clone();
        endpoint.channel_config_encrypted = channel.channel_config_encrypted.clone();
        endpoint.auth = channel.auth.clone();
        endpoint.auth_encrypted = channel.auth_encrypted.clone();
        endpoint.enabled = channel.enabled;
        endpoint.channel_status = channel.status.clone();
        endpoint.updated_at = channel.updated_at;
    }
}

fn initial_status(enabled: bool) -> String {
    if enabled { "draft" } else { "disabled" }.to_string()
}

impl InMemoryDatabase {
    // ============================================
    // Agent channel CRUD
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

    pub async fn create_legacy_alias_channel(
        &self,
        app_id: Uuid,
        input: CreateLegacyAliasChannelRow,
    ) -> Result<AgentChannelRow> {
        self.require_app_agent(app_id)?;
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AgentChannelRow {
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
        let ingress = self.ingress_channel_row(row.clone());
        self.agent_channel_rows.write().insert(id, row.clone());
        if let Some(ingress) = ingress {
            self.ingress_channels.write().insert(id, ingress);
        }
        Ok(row)
    }

    pub async fn list_legacy_alias_channels(&self, app_id: Uuid) -> Result<Vec<AgentChannelRow>> {
        let channels = self.agent_channel_rows.read();
        let mut result: Vec<AgentChannelRow> = channels
            .values()
            .filter(|ch| ch.app_id == app_id)
            .cloned()
            .collect();
        result.sort_by_key(|channel| channel.created_at);
        Ok(result)
    }

    pub async fn get_channel_row_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AgentChannelRow>> {
        let channels = self.agent_channel_rows.read();
        Ok(channels
            .values()
            .find(|ch| ch.public_id == public_id)
            .cloned())
    }

    pub async fn get_ingress_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<IngressChannelRow>> {
        Ok(self
            .ingress_channels
            .read()
            .values()
            .find(|endpoint| endpoint.channel_public_id == public_id)
            .cloned())
    }

    pub async fn list_ingress_channels_by_legacy_alias(
        &self,
        legacy_alias_id: &str,
        channel_type: &str,
    ) -> Result<Vec<IngressChannelRow>> {
        let mut endpoints: Vec<_> = self
            .ingress_channels
            .read()
            .values()
            .filter(|endpoint| {
                endpoint.legacy_alias_id.as_deref() == Some(legacy_alias_id)
                    && endpoint.channel_type == channel_type
                    && endpoint.enabled
            })
            .cloned()
            .collect();
        endpoints.sort_by_key(|endpoint| (endpoint.created_at, endpoint.channel_id));
        Ok(endpoints)
    }

    pub async fn list_agent_channels(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<IngressChannelRow>> {
        let mut endpoints: Vec<_> = self
            .ingress_channels
            .read()
            .values()
            .filter(|endpoint| endpoint.org_id == org_id && endpoint.agent_id == agent_id)
            .cloned()
            .collect();
        endpoints.sort_by_key(|endpoint| (endpoint.created_at, endpoint.channel_id));
        Ok(endpoints)
    }

    pub async fn get_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<Option<IngressChannelRow>> {
        Ok(self
            .ingress_channels
            .read()
            .values()
            .find(|endpoint| {
                endpoint.org_id == org_id
                    && endpoint.agent_id == agent_id
                    && endpoint.channel_public_id == public_id
            })
            .cloned())
    }

    pub async fn create_agent_channel(
        &self,
        org_id: i64,
        input: CreateAgentChannelRow,
    ) -> Result<IngressChannelRow> {
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
        let channel_id = Uuid::now_v7();
        let row = IngressChannelRow {
            channel_id,
            channel_public_id: input.public_id,
            legacy_app_id: None,
            legacy_alias_id: None,
            org_id,
            agent_id: input.agent_id,
            agent_public_id: agent.public_id,
            agent_name: agent.display_name.unwrap_or(agent.name),
            agent_description: agent.description,
            harness_id: agent.harness_id.uuid(),
            agent_status: agent.status,
            exposures_suspended: agent.exposures_suspended,
            virtual_user_id: input.virtual_user_id,
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
            channel_status: input.status,
            created_at: now,
            updated_at: now,
        };
        self.ingress_channels
            .write()
            .insert(channel_id, row.clone());
        Ok(row)
    }

    pub async fn update_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        input: UpdateAgentChannelRow,
    ) -> Result<Option<IngressChannelRow>> {
        let mut endpoints = self.ingress_channels.write();
        let Some(endpoint) = endpoints.values_mut().find(|endpoint| {
            endpoint.org_id == org_id
                && endpoint.agent_id == agent_id
                && endpoint.channel_public_id == public_id
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
            endpoint.channel_status = status;
        }
        if let Some(policy) = input.agent_version_policy {
            endpoint.agent_version_policy = policy;
        }
        input.agent_version_id.apply(&mut endpoint.agent_version_id);
        endpoint.updated_at = Self::now();
        Ok(Some(endpoint.clone()))
    }

    pub async fn delete_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<bool> {
        let mut endpoints = self.ingress_channels.write();
        let Some(id) = endpoints
            .values()
            .find(|endpoint| {
                endpoint.org_id == org_id
                    && endpoint.agent_id == agent_id
                    && endpoint.channel_public_id == public_id
            })
            .map(|endpoint| endpoint.channel_id)
        else {
            return Ok(false);
        };
        if let Some(endpoint) = endpoints.get_mut(&id)
            && endpoint.legacy_app_id.is_some()
        {
            endpoint.enabled = false;
            endpoint.channel_status = "disabled".to_string();
            endpoint.updated_at = Self::now();
        } else {
            endpoints.remove(&id);
        }
        Ok(true)
    }

    fn ingress_channel_row(&self, channel: AgentChannelRow) -> Option<IngressChannelRow> {
        let app = self.apps.read().get(&channel.app_id)?.clone();
        let agent_id = app.agent_id?;
        let agent = self
            .agents
            .read()
            .values()
            .find(|agent| agent.id.uuid() == agent_id)?
            .clone();
        Some(IngressChannelRow {
            channel_id: channel.id,
            channel_public_id: channel.public_id,
            legacy_app_id: Some(app.id),
            legacy_alias_id: Some(app.public_id),
            org_id: app.org_id,
            agent_id,
            agent_public_id: agent.public_id,
            agent_name: agent.display_name.unwrap_or(agent.name),
            agent_description: agent.description,
            harness_id: agent.harness_id.uuid(),
            agent_status: agent.status,
            exposures_suspended: agent.exposures_suspended,
            virtual_user_id: app.virtual_user_id,
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
            channel_status: channel.status,
            created_at: channel.created_at,
            updated_at: channel.updated_at,
        })
    }

    pub async fn get_agent_channel_public_id(
        &self,
        org_id: i64,
        channel_id: Uuid,
    ) -> Result<Option<String>> {
        let channel = self.agent_channel_rows.read().get(&channel_id).cloned();
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

    pub async fn update_channel_config_by_id(
        &self,
        id: Uuid,
        config: serde_json::Value,
        encrypted: Option<Vec<u8>>,
    ) -> Result<bool> {
        let mut channels = self.agent_channel_rows.write();
        if let Some(channel) = channels.get_mut(&id) {
            channel.channel_config = config.clone();
            channel.channel_config_encrypted = encrypted.clone();
            channel.updated_at = Self::now();
        }
        drop(channels);
        let mut endpoints = self.ingress_channels.write();
        let Some(endpoint) = endpoints.get_mut(&id) else {
            return Ok(false);
        };
        endpoint.channel_config = config;
        endpoint.channel_config_encrypted = encrypted;
        endpoint.updated_at = Self::now();
        Ok(true)
    }

    pub async fn update_channel_by_id(
        &self,
        id: Uuid,
        input: UpdateChannelByIdRow,
    ) -> Result<Option<AgentChannelRow>> {
        let mut channels = self.agent_channel_rows.write();
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
        self.sync_ingress_channel(&updated);
        Ok(Some(updated))
    }

    pub async fn delete_channel_by_id(&self, id: Uuid) -> Result<bool> {
        let mut channels = self.agent_channel_rows.write();
        let removed = channels.remove(&id).is_some();
        drop(channels);
        self.ingress_channels.write().remove(&id);
        Ok(removed)
    }

    /// See the PostgreSQL backend.
    pub async fn agents_with_live_channels(
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
        let channels = self.agent_channel_rows.read();
        Ok(channels
            .values()
            .filter(|ch| ch.status == "live")
            .filter_map(|ch| app_agent.get(&ch.app_id).copied())
            .filter(|agent| wanted.contains(agent))
            .collect())
    }

    /// See the PostgreSQL backend: bridges App publish onto endpoint status.
    pub async fn set_app_channel_publish(&self, app_id: Uuid, published: bool) -> Result<u64> {
        let mut channels = self.agent_channel_rows.write();
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
            self.sync_ingress_channel(&channel);
        }
        Ok(changed)
    }
}
