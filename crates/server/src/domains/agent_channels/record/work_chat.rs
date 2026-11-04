//! Hosted chat is a channel, not a new deployment or session aggregate.
//! The publisher chooses the interface; the effective agent must qualify.
use crate::domains::agent_channels::record::api::{AgentApiChannelConfig, ApiVisibility};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Conversation layout offered by a published Work Chat channel.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatInterface {
    SingleWithThreads,
    #[default]
    MultiChat,
}

/// Who may sign in. Further modes, such as a publisher-chosen identity
/// provider, are additional variants.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkChatMembership {
    /// Anyone the deployment identity provider accepts.
    #[default]
    Open,
    /// One person, plus the verified emails they invite.
    Invites,
    /// Current members of the organization that owns the channel.
    OrgUsers,
}

impl WorkChatMembership {
    fn is_open(&self) -> bool {
        *self == Self::Open
    }
}

pub const MAX_WORK_CHAT_INVITES: usize = 50;

/// Hosted interface and its existing Agent execution policies.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkChatConfig {
    /// Older configs stored a consumer model list under this name. Work Chat
    /// uses the Agent's model. The list is accepted so those configs still
    /// load, then dropped on the next save.
    #[serde(default, skip_serializing)]
    #[schema(ignore)]
    allowed_model_ids: Vec<everruns_contracts::typed_id::ModelId>,
    #[serde(default)]
    #[schema(example = "multi_chat")]
    pub interface_mode: ChatInterface,
    /// Who may sign in. Missing on older channels, which stay open.
    #[serde(default, skip_serializing_if = "WorkChatMembership::is_open")]
    #[schema(example = "open")]
    pub membership: WorkChatMembership,
    /// The one person an invites chat is for, as a verified email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_email: Option<String>,
    /// Extra verified emails allowed on an invites chat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invites: Vec<String>,
    /// Existing execution auth, approval and spending policies.
    #[serde(default = "default_execution")]
    pub execution: AgentApiChannelConfig,
}

/// This contract is shared by publication and runtime bootstrap. A worker-role
/// coordination capability has no thread tools and must never qualify.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct ChatFeatures {
    /// Effective coordinator tools support child work threads.
    #[schema(example = true)]
    pub threads: bool,
    /// Effective Agent exposes a session filesystem.
    #[schema(example = true)]
    pub files: bool,
}

impl ChatFeatures {
    pub fn validate(&self, mode: ChatInterface) -> Result<(), &'static str> {
        if mode == ChatInterface::SingleWithThreads && !self.threads {
            return Err(
                "Single chat with threads requires coordinator thread tools on the effective Agent",
            );
        }
        Ok(())
    }
}

/// Public DNS label. Ten characters, so it cannot be a channel id
/// (`appchan_` plus 32 hex digits). The alphabet skips 0/o and 1/l.
pub const HOST_LABEL_ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
pub const HOST_LABEL_LEN: usize = 10;

pub fn host_label_ok(label: &str) -> bool {
    label.len() == HOST_LABEL_LEN
        && label
            .bytes()
            .all(|byte| HOST_LABEL_ALPHABET.contains(&byte))
}

pub fn new_host_label() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..HOST_LABEL_LEN)
        .map(|_| HOST_LABEL_ALPHABET[rng.random_range(0..HOST_LABEL_ALPHABET.len())] as char)
        .collect()
}

fn default_execution() -> AgentApiChannelConfig {
    AgentApiChannelConfig {
        visibility: ApiVisibility::Full,
        ..Default::default()
    }
}

impl WorkChatConfig {
    /// Forget a consumer model list and normalize membership.
    /// The Agent's configured model is used. Invite addresses are stored only
    /// for that mode.
    pub fn prepare(&mut self) {
        self.allowed_model_ids.clear();
        if self.membership != WorkChatMembership::Invites {
            self.member_email = None;
            self.invites.clear();
            return;
        }
        self.member_email = self
            .member_email
            .as_deref()
            .map(normalize_member_email)
            .filter(|email| !email.is_empty());
        let owner = self.member_email.clone();
        let mut seen = std::collections::HashSet::new();
        self.invites = std::mem::take(&mut self.invites)
            .into_iter()
            .filter_map(|email| {
                let email = normalize_member_email(&email);
                if email.is_empty() || Some(&email) == owner.as_ref() || !seen.insert(email.clone())
                {
                    None
                } else {
                    Some(email)
                }
            })
            .collect();
    }

    pub fn validate_membership(&self) -> Result<(), &'static str> {
        if self.membership != WorkChatMembership::Invites {
            return Ok(());
        }
        if self
            .member_email
            .as_deref()
            .is_none_or(|email| !plausible_email(email))
        {
            return Err("Enter the email of the person this chat is for");
        }
        if self.invites.len() > MAX_WORK_CHAT_INVITES {
            return Err("A Work Chat can invite at most 50 people");
        }
        if self.invites.iter().any(|email| !plausible_email(email)) {
            return Err("Invite emails must look like name@example.com");
        }
        Ok(())
    }
}

pub(crate) fn normalize_member_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

pub(crate) fn plausible_email(email: &str) -> bool {
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    parts.next().is_none()
        && !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !email.chars().any(|c| c.is_whitespace())
}

impl Default for WorkChatConfig {
    fn default() -> Self {
        Self {
            allowed_model_ids: Vec::new(),
            interface_mode: ChatInterface::MultiChat,
            membership: WorkChatMembership::Open,
            member_email: None,
            invites: Vec::new(),
            execution: default_execution(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multi_chat_does_not_require_coordination_but_single_chat_does() {
        let features = ChatFeatures::default();
        assert!(features.validate(ChatInterface::MultiChat).is_ok());
        assert!(features.validate(ChatInterface::SingleWithThreads).is_err());
        assert!(
            ChatFeatures {
                threads: true,
                files: false
            }
            .validate(ChatInterface::SingleWithThreads)
            .is_ok()
        );
    }

    #[test]
    fn a_saved_consumer_model_list_is_dropped() {
        let id = everruns_contracts::typed_id::ModelId::new();
        let mut config: WorkChatConfig = serde_json::from_value(serde_json::json!({
            "allowed_model_ids": [id],
            "interface_mode": "multi_chat"
        }))
        .unwrap();
        assert_eq!(config.allowed_model_ids, vec![id]);
        let stored = serde_json::to_value(&config).unwrap();
        assert!(stored.get("allowed_model_ids").is_none());
        config.prepare();
        assert!(config.allowed_model_ids.is_empty());
    }

    #[test]
    fn invites_keep_one_person_and_drop_when_the_chat_is_open() {
        let mut config: WorkChatConfig = serde_json::from_value(serde_json::json!({
            "interface_mode": "multi_chat",
            "membership": "invites",
            "member_email": "Owner@Example.com",
            "invites": ["A@Example.com", "a@example.com", "Owner@example.com", " "]
        }))
        .unwrap();
        config.prepare();
        assert_eq!(config.member_email.as_deref(), Some("owner@example.com"));
        assert_eq!(config.invites, vec!["a@example.com".to_string()]);
        assert!(config.validate_membership().is_ok());
        config.membership = WorkChatMembership::Open;
        config.prepare();
        assert!(config.member_email.is_none());
        assert!(config.invites.is_empty());
        let stored = serde_json::to_value(&config).unwrap();
        assert!(stored.get("membership").is_none());
        assert!(stored.get("invites").is_none());
    }

    #[test]
    fn an_older_config_stays_open() {
        let config: WorkChatConfig =
            serde_json::from_value(serde_json::json!({"interface_mode": "multi_chat"})).unwrap();
        assert_eq!(config.membership, WorkChatMembership::Open);
        assert!(
            serde_json::from_value::<WorkChatConfig>(
                serde_json::json!({"membership": "external_oidc"})
            )
            .is_err()
        );
    }

    #[test]
    fn a_host_label_is_not_a_channel_id() {
        let id = "01933b5a000070008000000000000001";
        assert!(!host_label_ok(id));
        assert!(!host_label_ok(&format!("appchan_{id}")));
        assert!(!host_label_ok("login"));
        let label = new_host_label();
        assert!(host_label_ok(&label), "{label}");
        assert!(!label.contains(id));
        assert!(label.chars().all(|c| !"01lo".contains(c)));
    }
}
