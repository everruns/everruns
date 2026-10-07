//! Authored Sandbox Templates and resolved Session Sandbox specifications.
//!
//! Agent versions carry [`SandboxPolicy`] as desired configuration. A session
//! resolves one template exactly once and persists a [`ResolvedSandboxSpec`]
//! snapshot on its logical Sandbox, so later Agent edits cannot move a running
//! Session to different compute.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{SandboxTemplateId, SandboxTemplateRevisionId};
use serde::{Deserialize, Serialize};

use utoipa::ToSchema;

/// Agent policy for selecting the Session's primary Sandbox Template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxPolicy {
    /// Who may choose the primary Sandbox for a Session. Older rows omit this
    /// field; one template then means `fixed`, while several mean `selectable`.
    #[serde(default, alias = "policy", skip_serializing_if = "Option::is_none")]
    pub mode: Option<SandboxPolicyMode>,
    /// Template binding inherited when Session creation does not choose one.
    #[schema(example = "primary")]
    pub default: String,
    /// Template snapshots addressable by binding name at Session creation.
    #[serde(default, alias = "profiles")]
    pub templates: BTreeMap<String, SandboxTemplateSpec>,
}

impl SandboxPolicy {
    /// Effective policy, including the deterministic legacy migration rule.
    pub fn effective_mode(&self) -> SandboxPolicyMode {
        self.mode.unwrap_or(if self.templates.len() <= 1 {
            SandboxPolicyMode::Fixed
        } else {
            SandboxPolicyMode::Selectable
        })
    }
}

/// Agent policy for choosing the Session's one immutable primary Sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPolicyMode {
    /// Always use the default Sandbox Template; Session overrides are rejected.
    Fixed,
    /// A Session may select only one of the declared template bindings.
    Selectable,
    /// A Session may select a binding or submit a constrained one-off specification.
    Configurable,
}

/// Desired Sandbox configuration authored by a human or application.
///
/// Containment and durability may be omitted when the target has exactly one
/// honest answer. Resolution fills those fields before the specification is pinned
/// to a Session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxTemplateSpec {
    /// Immutable Sandbox Template revision this specification was copied from.
    /// The specification remains complete so Agent snapshots are portable and
    /// later template revisions cannot change an existing snapshot.
    #[serde(
        default,
        alias = "source_revision_id",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub template_revision_id: Option<SandboxTemplateRevisionId>,
    /// Provider-neutral target plus its concrete adapter binding.
    pub target: SandboxTargetSpec,
    /// Requested isolation policy; resolution supplies an honest target default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containment: Option<SandboxContainmentSpec>,
    /// Requested recovery guarantee; resolution supplies a target default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<SandboxDurability>,
    /// Idle lifecycle policy controlled by Everruns.
    #[serde(default)]
    pub lifecycle: SandboxLifecycle,
    /// Reproducible commands run when physical compute is initialized.
    #[serde(default)]
    pub bootstrap: SandboxBootstrap,
}

/// Session-pinned specification. Every security- and recovery-relevant default has
/// been made explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResolvedSandboxSpec {
    /// Reusable revision from which this complete snapshot was copied.
    #[serde(
        default,
        alias = "source_revision_id",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub template_revision_id: Option<SandboxTemplateRevisionId>,
    /// Exact target pinned for the Session.
    pub target: SandboxTargetSpec,
    /// Fully resolved containment contract.
    pub containment: SandboxContainmentSpec,
    /// Fully resolved recovery guarantee.
    pub durability: SandboxDurability,
    /// Pinned lifecycle policy.
    pub lifecycle: SandboxLifecycle,
    /// Pinned initialization commands.
    pub bootstrap: SandboxBootstrap,
}

/// Where commands execute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxTargetSpec {
    /// Provider-neutral target class.
    pub kind: SandboxTargetKind,
    /// Concrete adapter for target kinds with more than one implementation.
    #[serde(default, alias = "vendor", skip_serializing_if = "Option::is_none")]
    #[schema(example = "daytona")]
    pub provider: Option<String>,
    /// Credential/transport binding for a registered machine target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "conn_01933b5a000070008000000000000001")]
    pub connection_id: Option<String>,
    /// Provider-owned, non-secret configuration. Credentials are references,
    /// never values in this object.
    #[serde(default = "empty_object", skip_serializing_if = "is_empty_object")]
    #[schema(value_type = Object)]
    pub options: serde_json::Value,
}

impl SandboxTargetSpec {
    pub fn vfs(provider: impl Into<String>) -> Self {
        Self {
            kind: SandboxTargetKind::Vfs,
            provider: Some(provider.into()),
            connection_id: None,
            options: empty_object(),
        }
    }

    pub fn managed(provider: impl Into<String>) -> Self {
        Self {
            kind: SandboxTargetKind::Managed,
            provider: Some(provider.into()),
            connection_id: None,
            options: empty_object(),
        }
    }

    pub fn host() -> Self {
        Self {
            kind: SandboxTargetKind::Host,
            provider: None,
            connection_id: None,
            options: empty_object(),
        }
    }
}

/// Provider-neutral target class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxTargetKind {
    Host,
    Machine,
    Vfs,
    Container,
    Managed,
}

impl SandboxTargetKind {
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
pub struct SandboxContainmentSpec {
    /// Isolation mechanism the provider must supply.
    pub level: SandboxContainmentLevel,
    /// Outbound network access the provider must enforce.
    #[serde(default)]
    pub network: SandboxNetworkPolicy,
    /// Filesystem mutation boundary for command execution.
    #[serde(default)]
    pub filesystem: SandboxFilesystemPolicy,
    /// Whether the runtime may widen containment after Session creation.
    #[serde(default)]
    pub escalation: SandboxEscalation,
}

impl SandboxContainmentSpec {
    pub fn isolated() -> Self {
        Self {
            level: SandboxContainmentLevel::Isolated,
            network: SandboxNetworkPolicy::Deny,
            filesystem: SandboxFilesystemPolicy::default(),
            escalation: SandboxEscalation::Never,
        }
    }

    pub fn uncontained() -> Self {
        Self {
            level: SandboxContainmentLevel::None,
            network: SandboxNetworkPolicy::Allow,
            filesystem: SandboxFilesystemPolicy::default(),
            escalation: SandboxEscalation::Never,
        }
    }
}

/// Filesystem paths the target permits command execution to mutate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SandboxFilesystemPolicy {
    /// Absolute roots the runtime may mutate; empty means provider default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxContainmentLevel {
    None,
    Native,
    Isolated,
}

/// Outbound network policy the target must actually enforce.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SandboxNetworkPolicy {
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
pub enum SandboxEscalation {
    #[default]
    Never,
    Approval,
    Auto,
}

/// What survives physical compute loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxDurability {
    Checkpointed,
    ProviderSnapshot,
    None,
}

/// Control-plane lifecycle intent. Providers do not own these timers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SandboxLifecycle {
    /// Inactivity interval before applying `idle_action`.
    #[serde(default = "default_idle_after_seconds")]
    #[schema(example = 300)]
    pub idle_after_seconds: u64,
    /// Action Everruns requests after the idle interval.
    #[serde(default)]
    pub idle_action: SandboxIdleAction,
}

impl Default for SandboxLifecycle {
    fn default() -> Self {
        Self {
            idle_after_seconds: default_idle_after_seconds(),
            idle_action: SandboxIdleAction::CheckpointAndStop,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxIdleAction {
    #[default]
    CheckpointAndStop,
    Stop,
    KeepRunning,
}

/// Reproducible initialization pinned with the specification snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SandboxBootstrap {
    /// Ordered commands replayed when creating or recovering physical compute.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

/// Session-level selection: use an Agent template binding or provide an inline specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SandboxSelection {
    Named { r#use: String },
    Inline(SandboxTemplateSpec),
}

/// Organization-scoped reusable Sandbox Template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxTemplate {
    /// Stable public Sandbox Template identifier.
    #[serde(rename = "id")]
    #[schema(value_type = String)]
    pub public_id: SandboxTemplateId,
    /// Addressable name used in configuration.
    #[schema(example = "coding-daytona")]
    pub name: String,
    /// Human-readable name shown in management surfaces.
    #[schema(example = "Coding - Daytona")]
    pub display_name: String,
    /// Optional explanation of the Sandbox Template's intended workload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Recoverable coding workspace managed by Daytona")]
    pub description: Option<String>,
    /// Whether the definition is owned and sealed by the platform.
    pub is_managed: bool,
    /// Lifecycle state such as `active` or `archived`.
    #[schema(example = "active")]
    pub status: String,
    /// Latest immutable revision used for new references.
    pub current_revision: SandboxTemplateRevision,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the most recent definition or revision change.
    pub updated_at: DateTime<Utc>,
}

/// Immutable revision of a Sandbox Template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SandboxTemplateRevision {
    /// Stable public revision identifier pinned into Agent and Session snapshots.
    #[serde(rename = "id")]
    #[schema(value_type = String)]
    pub public_id: SandboxTemplateRevisionId,
    /// Parent reusable Sandbox Template identifier.
    #[serde(alias = "environment_id")]
    #[schema(value_type = String)]
    pub sandbox_template_id: SandboxTemplateId,
    /// Monotonically increasing revision number within the Sandbox Template.
    #[schema(example = 3)]
    pub revision: i32,
    /// Complete immutable authored specification.
    #[serde(alias = "profile")]
    pub spec: SandboxTemplateSpec,
    /// Revision creation timestamp.
    pub created_at: DateTime<Utc>,
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
    fn named_selection_is_distinct_from_inline_spec() {
        let named: SandboxSelection = serde_json::from_value(json!({"use": "build"})).unwrap();
        assert_eq!(
            named,
            SandboxSelection::Named {
                r#use: "build".into()
            }
        );

        let inline: SandboxSelection = serde_json::from_value(json!({
            "target": {"kind": "vfs", "provider": "bashkit"}
        }))
        .unwrap();
        assert!(matches!(inline, SandboxSelection::Inline(_)));
    }

    #[test]
    fn omitted_mode_gets_a_deterministic_policy() {
        let spec: SandboxTemplateSpec = serde_json::from_value(json!({
            "target": {"kind": "vfs", "provider": "bashkit"}
        }))
        .unwrap();
        let fixed = SandboxPolicy {
            mode: None,
            default: "default".into(),
            templates: BTreeMap::from([("default".into(), spec.clone())]),
        };
        assert_eq!(fixed.effective_mode(), SandboxPolicyMode::Fixed);

        let selectable = SandboxPolicy {
            mode: None,
            default: "one".into(),
            templates: BTreeMap::from([("one".into(), spec.clone()), ("two".into(), spec)]),
        };
        assert_eq!(selectable.effective_mode(), SandboxPolicyMode::Selectable);
    }

    #[test]
    fn legacy_policy_keys_are_input_only_aliases() {
        let policy: SandboxPolicy = serde_json::from_value(json!({
            "policy": "selectable",
            "default": "scratch",
            "profiles": {
                "scratch": {"target": {"kind": "vfs", "provider": "bashkit"}}
            }
        }))
        .unwrap();

        let serialized = serde_json::to_value(policy).unwrap();
        assert_eq!(serialized["mode"], "selectable");
        assert!(serialized.get("templates").is_some());
        assert!(serialized.get("policy").is_none());
        assert!(serialized.get("profiles").is_none());
    }

    #[test]
    fn target_accepts_vendor_as_a_legacy_authoring_alias() {
        let target: SandboxTargetSpec = serde_json::from_value(json!({
            "kind": "managed",
            "vendor": "daytona"
        }))
        .unwrap();
        assert_eq!(target.provider.as_deref(), Some("daytona"));
        assert_eq!(serde_json::to_value(target).unwrap()["provider"], "daytona");
    }

    #[test]
    fn omitted_target_options_resolve_to_an_empty_object() {
        let target: SandboxTargetSpec = serde_json::from_value(json!({
            "kind": "vfs",
            "provider": "bashkit"
        }))
        .unwrap();
        assert_eq!(target.options, json!({}));
    }
}
