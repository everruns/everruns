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

/// Hosted interface and its existing Agent execution policies.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkChatConfig {
    /// Owner-curated models available to consumers; empty keeps the Agent default.
    #[serde(default)]
    pub allowed_model_ids: Vec<everruns_contracts::typed_id::ModelId>,
    #[serde(default)]
    #[schema(example = "multi_chat")]
    pub interface_mode: ChatInterface,
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

impl Default for WorkChatConfig {
    fn default() -> Self {
        Self {
            allowed_model_ids: Vec::new(),
            interface_mode: ChatInterface::MultiChat,
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
