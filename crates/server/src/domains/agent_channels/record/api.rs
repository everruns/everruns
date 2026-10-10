// Agent Execution API records: the `api` channel's config and the agent keys
// that call it.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - An agent key is org-owned, granted to one api channel, and reaches only the
//   execution routes under that channel's base URL. `created_by` is audit, not
//   ownership, so a key survives its creator leaving.
// - Only the SHA-256 hash and a display prefix are stored; the secret is shown
//   once. Rotation keeps the key id, so sessions tagged with it survive, and the
//   previous secret stays valid for an overlap window.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::SessionBinding;
use super::exposure::PublicToolVisibility;

/// Bindings an api channel offers: one session per caller conversation
/// (`per_user`, keyed on the caller) or one per created session
/// (`session_per_invocation`). A session shared by every caller is not offered:
/// callers must never see each other's conversations.
pub(crate) const API_BINDINGS: [SessionBinding; 2] =
    [SessionBinding::Requester, SessionBinding::Ephemeral];

/// Prefix of every agent key.
pub const AGENT_KEY_PREFIX: &str = "evr_ak_";

/// Generate a new agent key secret: the prefix and 32 random bytes as hex.
pub fn generate_agent_key() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
    format!("{AGENT_KEY_PREFIX}{}", hex::encode(bytes))
}

/// SHA-256 hex digest of an agent key, the only form stored.
pub fn hash_agent_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Non-secret display prefix, e.g. `evr_ak_1a2b3c4d...`.
pub fn agent_key_display_prefix(key: &str) -> String {
    let end = (AGENT_KEY_PREFIX.len() + 8).min(key.len());
    format!("{}...", &key[..end])
}

/// What an api channel's event stream and message reads show.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiVisibility {
    /// Assistant text only.
    Messages,
    /// Assistant text plus tool start and finish, shown with the channel's
    /// public tool activity text instead of tool names or arguments.
    #[default]
    Activity,
    /// The raw canonical events: tool names, arguments, results, reasoning and
    /// usage. For callers who own both ends.
    Full,
}

/// How much an api channel says about a failure.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorDetail {
    /// The four public error codes only.
    #[default]
    Public,
    /// Problem details with internal codes, for key-authenticated developers.
    Detailed,
}

/// Who answers a tool call the agent's `tool_approval` gate held back.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiToolApprovals {
    /// Someone with access to the agent in Everruns. The caller sees that a
    /// call waits, not what it is.
    #[default]
    Operator,
    /// The key holder, through `POST …/tool-approvals`. It then sees the tool
    /// and its arguments, which it needs to decide.
    Caller,
}

/// Typed `api` channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentApiChannelConfig {
    /// `per_user` (default): each caller has its own sessions.
    /// `session_per_invocation`: same, named per created session.
    #[serde(default = "default_api_binding")]
    pub session_binding: SessionBinding,
    /// What the event stream shows. Default `activity`.
    #[serde(default)]
    pub visibility: ApiVisibility,
    /// How much a failure says. Default `public`.
    #[serde(default)]
    pub errors: ApiErrorDetail,
    /// Tool activity text in `activity` visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_activity_text: Option<String>,
    /// Who answers held-back tool calls. Default `operator`: a held-back
    /// action is not the caller's to allow unless the owner says so
    /// (THREAT[TM-AGENTKEY-006]).
    #[serde(default)]
    pub tool_approvals: ApiToolApprovals,
    /// Optional per-IP rate limit, requests per minute. `None` or `0` turns
    /// the per-channel limit off (the global API limit still applies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
}

fn default_api_binding() -> SessionBinding {
    SessionBinding::Requester
}

impl AgentApiChannelConfig {
    /// The text a tool start or finish shows in `activity` visibility.
    pub fn activity_text(&self) -> Option<&str> {
        super::exposure::public_tool_activity_text(
            PublicToolVisibility::Generic,
            self.tool_activity_text.as_deref().unwrap_or_default(),
        )
    }
}

/// What a key may do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentKeyPermission {
    /// The session routes.
    Sessions,
}

impl AgentKeyPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sessions" => Some(Self::Sessions),
            _ => None,
        }
    }
}

/// An agent key as management sees it. Never carries the secret.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentKey {
    /// Key id (`agentkey_…`). Stable across rotations.
    #[schema(example = "agentkey_01933b5a000070008000000000000001")]
    pub id: String,
    /// The api channel the key calls.
    #[schema(example = "appchan_01933b5a000070008000000000000001")]
    pub channel_id: String,
    /// Display name.
    #[schema(example = "Support backend")]
    pub name: String,
    /// Non-secret display prefix of the current secret.
    #[schema(example = "evr_ak_1a2b3c4d...")]
    pub prefix: String,
    /// What the key may do.
    pub permissions: Vec<AgentKeyPermission>,
    /// When the key stops working, if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Until when the secret replaced by the last rotation still works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_valid_until: Option<DateTime<Utc>>,
    /// Last successful use (updated at most once a minute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the key was revoked; a revoked key never works again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A key with its secret, returned once by create and rotate.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentKeyWithSecret {
    #[serde(flatten)]
    pub key: AgentKey,
    /// The secret. Shown only in this response; store it now.
    #[schema(example = "evr_ak_1a2b3c4d5e6f…")]
    pub secret: String,
}

/// Public id of an agent key row.
pub fn agent_key_public_id(id: uuid::Uuid) -> String {
    format!("agentkey_{}", id.simple())
}

/// Row id of a public agent key id.
pub fn parse_agent_key_id(id: &str) -> Option<uuid::Uuid> {
    let hex = id.strip_prefix("agentkey_")?;
    (hex.len() == 32)
        .then(|| uuid::Uuid::parse_str(hex).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_prefixed_and_hashed() {
        let key = generate_agent_key();
        assert!(key.starts_with(AGENT_KEY_PREFIX));
        assert_eq!(key.len(), AGENT_KEY_PREFIX.len() + 64);
        assert_ne!(key, generate_agent_key());
        assert_eq!(hash_agent_key(&key).len(), 64);
        assert_eq!(
            agent_key_display_prefix(&key).len(),
            AGENT_KEY_PREFIX.len() + 11
        );
    }

    #[test]
    fn key_ids_round_trip() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(parse_agent_key_id(&agent_key_public_id(id)), Some(id));
        assert_eq!(parse_agent_key_id("agentkey_nope"), None);
        assert_eq!(parse_agent_key_id(&id.to_string()), None);
    }

    #[test]
    fn config_defaults_are_the_safe_ones() {
        let config: AgentApiChannelConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(config.session_binding, SessionBinding::Requester);
        assert_eq!(config.visibility, ApiVisibility::Activity);
        assert_eq!(config.errors, ApiErrorDetail::Public);
        assert_eq!(config.activity_text(), Some("Working..."));
        assert!(
            serde_json::from_value::<AgentApiChannelConfig>(
                serde_json::json!({"api_key_hash": "x"})
            )
            .is_err()
        );
    }
}
