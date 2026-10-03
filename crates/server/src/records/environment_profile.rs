//! Authored and resolved execution-environment profiles.
//!
//! Agent versions carry [`EnvironmentSet`] as desired configuration. A session
//! resolves one profile exactly once and persists a [`ResolvedEnvironmentProfile`]
//! snapshot on its logical environment, so later agent edits cannot move a
//! running session to different compute.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use utoipa::ToSchema;

/// Named execution environments offered by an Agent version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentSet {
    /// Profile inherited when session creation does not choose one explicitly.
    pub default: String,
    /// Human-authored profiles addressable by name at session creation.
    #[serde(default)]
    pub profiles: BTreeMap<String, EnvironmentProfile>,
}

/// Desired environment configuration authored by a human or application.
///
/// Containment and durability may be omitted when the target has exactly one
/// honest answer. Resolution fills those fields before the profile is pinned
/// to a Session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentProfile {
    pub target: EnvironmentTargetProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containment: Option<EnvironmentContainmentProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<EnvironmentDurability>,
    #[serde(default)]
    pub lifecycle: EnvironmentLifecycle,
    #[serde(default)]
    pub bootstrap: EnvironmentBootstrap,
}

/// Session-pinned profile. Every security- and recovery-relevant default has
/// been made explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResolvedEnvironmentProfile {
    pub target: EnvironmentTargetProfile,
    pub containment: EnvironmentContainmentProfile,
    pub durability: EnvironmentDurability,
    pub lifecycle: EnvironmentLifecycle,
    pub bootstrap: EnvironmentBootstrap,
}

/// Where commands execute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTargetProfile {
    pub kind: EnvironmentTargetKind,
    /// Concrete adapter for target kinds with more than one implementation.
    #[serde(default, alias = "vendor", skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Credential/transport binding for a registered machine target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    /// Provider-owned, non-secret configuration. Credentials are references,
    /// never values in this object.
    #[serde(default = "empty_object", skip_serializing_if = "is_empty_object")]
    #[schema(value_type = Object)]
    pub options: serde_json::Value,
}

impl EnvironmentTargetProfile {
    pub fn vfs(provider: impl Into<String>) -> Self {
        Self {
            kind: EnvironmentTargetKind::Vfs,
            provider: Some(provider.into()),
            connection_id: None,
            options: empty_object(),
        }
    }

    pub fn managed(provider: impl Into<String>) -> Self {
        Self {
            kind: EnvironmentTargetKind::Managed,
            provider: Some(provider.into()),
            connection_id: None,
            options: empty_object(),
        }
    }

    pub fn host() -> Self {
        Self {
            kind: EnvironmentTargetKind::Host,
            provider: None,
            connection_id: None,
            options: empty_object(),
        }
    }
}

/// Provider-neutral target class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentTargetKind {
    Host,
    Machine,
    Vfs,
    Container,
    Managed,
}

impl EnvironmentTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Machine => "machine",
            Self::Vfs => "vfs",
            Self::Container => "container",
            Self::Managed => "managed",
        }
    }
}

/// What commands may touch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentContainmentProfile {
    pub level: EnvironmentContainmentLevel,
    #[serde(default)]
    pub network: EnvironmentNetworkPolicy,
    #[serde(default)]
    pub filesystem: EnvironmentFilesystemPolicy,
    #[serde(default)]
    pub escalation: EnvironmentEscalation,
}

impl EnvironmentContainmentProfile {
    pub fn isolated() -> Self {
        Self {
            level: EnvironmentContainmentLevel::Isolated,
            network: EnvironmentNetworkPolicy::Deny,
            filesystem: EnvironmentFilesystemPolicy::default(),
            escalation: EnvironmentEscalation::Never,
        }
    }

    pub fn uncontained() -> Self {
        Self {
            level: EnvironmentContainmentLevel::None,
            network: EnvironmentNetworkPolicy::Allow,
            filesystem: EnvironmentFilesystemPolicy::default(),
            escalation: EnvironmentEscalation::Never,
        }
    }
}

/// Filesystem paths the target permits command execution to mutate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentFilesystemPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentContainmentLevel {
    None,
    Native,
    Isolated,
}

/// Outbound network policy the target must actually enforce.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EnvironmentNetworkPolicy {
    #[default]
    Deny,
    Allowlist {
        allowed_hosts: Vec<String>,
    },
    Allow,
}

/// Who may widen containment after a session starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentEscalation {
    #[default]
    Never,
    Approval,
    Auto,
}

/// What survives physical compute loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentDurability {
    Checkpointed,
    ProviderSnapshot,
    None,
}

/// Control-plane lifecycle intent. Providers do not own these timers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentLifecycle {
    #[serde(default = "default_idle_after_seconds")]
    pub idle_after_seconds: u64,
    #[serde(default)]
    pub idle_action: EnvironmentIdleAction,
}

impl Default for EnvironmentLifecycle {
    fn default() -> Self {
        Self {
            idle_after_seconds: default_idle_after_seconds(),
            idle_action: EnvironmentIdleAction::CheckpointAndStop,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentIdleAction {
    #[default]
    CheckpointAndStop,
    Stop,
    KeepRunning,
}

/// Reproducible initialization pinned with the profile snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentBootstrap {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

/// Session-level selection: use an Agent profile, or provide an inline one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EnvironmentSelection {
    Named { r#use: String },
    Inline(EnvironmentProfile),
}

fn default_idle_after_seconds() -> u64 {
    180
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

fn is_empty_object(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn named_selection_is_distinct_from_inline_profile() {
        let named: EnvironmentSelection = serde_json::from_value(json!({"use": "build"})).unwrap();
        assert_eq!(
            named,
            EnvironmentSelection::Named {
                r#use: "build".into()
            }
        );

        let inline: EnvironmentSelection = serde_json::from_value(json!({
            "target": {"kind": "vfs", "provider": "bashkit"}
        }))
        .unwrap();
        assert!(matches!(inline, EnvironmentSelection::Inline(_)));
    }

    #[test]
    fn target_accepts_vendor_as_a_legacy_authoring_alias() {
        let target: EnvironmentTargetProfile = serde_json::from_value(json!({
            "kind": "managed",
            "vendor": "daytona"
        }))
        .unwrap();
        assert_eq!(target.provider.as_deref(), Some("daytona"));
        assert_eq!(serde_json::to_value(target).unwrap()["provider"], "daytona");
    }

    #[test]
    fn omitted_target_options_resolve_to_an_empty_object() {
        let target: EnvironmentTargetProfile = serde_json::from_value(json!({
            "kind": "vfs",
            "provider": "bashkit"
        }))
        .unwrap();
        assert_eq!(target.options, json!({}));
    }
}
