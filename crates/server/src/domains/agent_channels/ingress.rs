use std::sync::Arc;

use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::domains::agent_channels::record::{
    A2aChannelConfig, AgUiChannelConfig, AgentChannel, ChannelAuthConfig, ChannelStatus,
    ChannelType, FcpChannelConfig, PublicChatChannelConfig,
};
use crate::domains::agent_channels::record::{ScheduleChannelConfig, WebhookChannelConfig};
use crate::records::AgentChannelId;
use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId, VirtualUserId};
use uuid::Uuid;

use crate::storage::{EncryptionService, IngressChannelRow, StorageBackend};

impl IngressContext {
    pub(crate) fn agent_is_active(&self) -> bool {
        self.agent_status == "active"
    }
}

#[derive(Debug, Clone)]
pub struct IngressContext {
    pub public_id: AgentChannelId,
    pub internal_id: Uuid,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub agent_internal_id: Uuid,
    pub virtual_user_id: Option<VirtualUserId>,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    agent_status: String,
    exposures_suspended: bool,
}

#[cfg(test)]
impl IngressContext {
    pub(crate) fn for_test(name: &str, description: Option<&str>) -> Self {
        Self {
            public_id: AgentChannelId::from_seed(1),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: name.to_string(),
            description: description.map(str::to_string),
            harness_id: HarnessId::from_seed(2),
            agent_id: Some(AgentId::from_seed(4)),
            agent_internal_id: Uuid::nil(),
            virtual_user_id: None,
            owner_principal_id: PrincipalId::from_seed(3),
            resolved_owner_user_id: None,
            agent_status: "active".to_string(),
            exposures_suspended: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngressChannel {
    pub public_id: AgentChannelId,
    pub internal_id: Uuid,
    pub channel_type: ChannelType,
    pub channel_config: serde_json::Value,
    pub auth: Option<Box<ChannelAuthConfig>>,
    pub enabled: bool,
    pub status: ChannelStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl IngressChannel {
    /// Render the channel for the API.
    pub fn into_channel(self) -> AgentChannel {
        AgentChannel {
            public_id: self.public_id,
            internal_id: self.internal_id,
            agent_id: None,
            channel_type: self.channel_type,
            channel_config: self.channel_config,
            enabled: self.enabled,
            status: self.status,
            auth: self.auth,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
    pub fn slack_config(&self) -> Option<SlackChannelConfig> {
        self.config(ChannelType::Slack)
    }

    pub fn ag_ui_config(&self) -> Option<AgUiChannelConfig> {
        self.config(ChannelType::AgUi)
    }

    pub fn schedule_config(&self) -> Option<ScheduleChannelConfig> {
        self.config(ChannelType::Schedule)
    }

    pub fn webhook_config(&self) -> Option<WebhookChannelConfig> {
        self.config(ChannelType::Webhook)
    }

    pub fn fcp_config(&self) -> Option<FcpChannelConfig> {
        self.config(ChannelType::Fcp)
    }

    pub fn a2a_config(&self) -> Option<A2aChannelConfig> {
        self.config(ChannelType::A2a)
    }

    pub fn poppy_config(&self) -> Option<super::record::poppy::PoppyChannelConfig> {
        self.config(ChannelType::Poppy)
    }

    pub fn public_chat_config(&self) -> Option<PublicChatChannelConfig> {
        self.config(ChannelType::PublicChat)
    }

    fn config<T: serde::de::DeserializeOwned>(&self, expected: ChannelType) -> Option<T> {
        if self.channel_type != expected {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }
}

pub async fn resolve_channel(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_id: &str,
) -> anyhow::Result<Option<(IngressContext, IngressChannel)>> {
    db.get_ingress_channel_by_public_id(channel_id)
        .await?
        .map(|row| row_to_ingress(encryption, row))
        .transpose()
}

pub(crate) fn row_to_ingress(
    encryption: Option<&Arc<EncryptionService>>,
    row: IngressChannelRow,
) -> anyhow::Result<(IngressContext, IngressChannel)> {
    let channel_public_id = row
        .channel_public_id
        .parse()
        .unwrap_or_else(|_| AgentChannelId::from_uuid(row.channel_id));
    let mut channel_config = decrypt_json(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
        "channel configuration",
    );
    let legacy_auth = channel_config
        .as_object_mut()
        .and_then(|object| object.remove("auth"));
    let auth = if row.auth.is_some() || row.auth_encrypted.is_some() {
        Some(channel_auth_fail_closed(decrypt_json(
            encryption,
            row.auth_encrypted.as_deref(),
            &row.auth.clone().unwrap_or(serde_json::Value::Null),
            "channel authentication",
        )))
    } else {
        legacy_auth.map(channel_auth_fail_closed)
    };
    let context = IngressContext {
        public_id: channel_public_id,
        internal_id: row.channel_id,
        org_id: row.org_id,
        name: row.agent_name,
        description: row.agent_description,
        harness_id: HarnessId::from_uuid(row.harness_id),
        agent_id: Some(row.agent_public_id.parse()?),
        agent_internal_id: row.agent_id,
        virtual_user_id: row.virtual_user_id.map(VirtualUserId::from_uuid),
        owner_principal_id: PrincipalId::from_uuid(row.owner_principal_id),
        resolved_owner_user_id: row.resolved_owner_user_id,
        agent_status: row.agent_status,
        exposures_suspended: row.exposures_suspended,
    };
    let channel = IngressChannel {
        public_id: channel_public_id,
        internal_id: row.channel_id,
        channel_type: ChannelType::from_str_opt(&row.channel_type).unwrap_or(ChannelType::Slack),
        channel_config,
        auth,
        enabled: row.enabled,
        status: ChannelStatus::from(row.channel_status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    Ok((context, channel))
}

fn decrypt_json(
    encryption: Option<&Arc<EncryptionService>>,
    encrypted: Option<&[u8]>,
    plaintext: &serde_json::Value,
    field: &str,
) -> serde_json::Value {
    let Some(encrypted) = encrypted else {
        return plaintext.clone();
    };
    let Some(encryption) = encryption else {
        tracing::error!(field, "Encrypted ingress field cannot be decrypted");
        return serde_json::Value::Null;
    };
    encryption
        .decrypt_to_string(encrypted)
        .and_then(|json| serde_json::from_str(&json).map_err(Into::into))
        .unwrap_or_else(|error| {
            tracing::error!(%error, field, "Failed to decrypt ingress field");
            serde_json::Value::Null
        })
}

fn channel_auth_fail_closed(value: serde_json::Value) -> Box<ChannelAuthConfig> {
    Box::new(serde_json::from_value(value).unwrap_or_else(|error| {
        tracing::error!(%error, "Failed to parse channel authentication");
        serde_json::from_value(serde_json::json!({"mode": "http_basic"}))
            .expect("fail-closed channel authentication is valid")
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotLive {
    ChannelNotLive,
    AgentNotActive,
    ExposuresSuspended,
}

impl NotLive {
    pub fn as_str(self) -> &'static str {
        match self {
            NotLive::ChannelNotLive => "channel not live",
            NotLive::AgentNotActive => "agent not active",
            NotLive::ExposuresSuspended => "agent exposures suspended",
        }
    }
}

pub fn channel_liveness(context: &IngressContext, channel: &IngressChannel) -> Result<(), NotLive> {
    if !channel.status.is_live() {
        return Err(NotLive::ChannelNotLive);
    }
    if context.agent_status != "active" {
        return Err(NotLive::AgentNotActive);
    }
    if context.exposures_suspended {
        return Err(NotLive::ExposuresSuspended);
    }
    Ok(())
}
