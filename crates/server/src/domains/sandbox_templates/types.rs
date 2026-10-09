// Sandbox Template domain types (read models shared with the HTTP layer).
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a session's commands run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxTarget {
    /// Shape of the target: `host`, `machine`, `vfs`, `container`, `managed`.
    #[schema(example = "managed")]
    pub kind: String,
    /// Concrete provider, when the kind has one (`bashkit`, `daytona`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "daytona")]
    pub provider: Option<String>,
    /// Registered connection used by a machine target.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "conn_01933b5a000070008000000000000001")]
    pub connection_id: Option<String>,
}

/// What commands may touch, and who enforces it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxContainment {
    /// `none`, `native`, or `isolated`.
    #[schema(example = "isolated")]
    pub level: String,
    /// Outbound network policy: `deny`, `allowlist`, or `allow`.
    #[schema(example = "allow")]
    pub network: String,
}

/// What the Sandbox target can actually do.
///
/// Read this before assuming a shell behaves like Linux. Bashkit reports
/// `native_processes: false`, which is why a build fails there; the answer is
/// available before the first turn rather than after a confusing tool error.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
pub struct SandboxCapabilities {
    /// Whether commands can spawn operating-system processes.
    pub native_processes: bool,
    /// Whether the runtime can install operating-system packages.
    pub packages: bool,
    /// Whether interactive pseudo-terminals are supported.
    pub pty: bool,
    /// Whether workloads can bind and expose network ports.
    pub ports: bool,
    /// Whether filesystem state has a portable checkpoint representation.
    pub portable_checkpoint: bool,
    /// Whether the target enforces the declared outbound network policy.
    pub network_enforced: bool,
}

/// The primary Sandbox a Session is running in.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionSandboxResponse {
    /// Durable logical primary Sandbox id. Absent for legacy capability-derived sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    /// Reusable Sandbox Template revision pinned into this Sandbox.
    #[serde(
        skip_serializing_if = "Option::is_none",
        alias = "environment_revision_id"
    )]
    pub sandbox_template_revision_id: Option<String>,
    /// `primary` for the implicit shell/files binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Agent template binding selected for this Session (`inline` for one-offs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Target, absent when the session has no compute at all and only reads and
    /// writes files. That is a real configuration, not a misconfiguration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SandboxTarget>,
    pub containment: SandboxContainment,
    /// `checkpointed`, `provider_snapshot`, or `none`. Declared per target, so a
    /// session on somebody else's machine is never reported as recoverable.
    pub durability: String,
    pub capabilities: SandboxCapabilities,
    /// How this view was produced. `capabilities` means it was derived from the
    /// Session's effective capability set rather than a stored Sandbox spec.
    pub resolved_from: String,
    /// Capability that supplied the compute, for operators tracing a surprise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_capability: Option<String>,
    /// Immutable resolved specification pinned when the Session was created.
    #[serde(skip_serializing_if = "Option::is_none", alias = "profile")]
    pub spec: Option<crate::records::ResolvedSandboxSpec>,
    /// Control-plane lifecycle intent and latest observed physical state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_state: Option<String>,
    /// Latest lifecycle state observed from the physical provider resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_state: Option<String>,
    /// Physical incarnation fence. Increments whenever compute is replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    /// Most recent checkpoint used to recover this logical Sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_checkpoint_id: Option<String>,
    /// Last recorded runtime activity used by lifecycle policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One target this deployment can offer, and what it can do.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxTargetDescriptor {
    /// Human-readable provider/target name.
    pub display_name: String,
    /// Lucide icon name used by management surfaces.
    pub icon: String,
    /// Provider-neutral target class.
    pub kind: String,
    /// Concrete provider adapter, when the target class requires one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Whether this deployment can actually run it right now.
    pub available: bool,
    /// Why it is unavailable. Present only when `available` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Capabilities the deployment can honestly provide for this target.
    pub capabilities: SandboxCapabilities,
    /// Containment levels this target supports, weakest first.
    pub containment_levels: Vec<String>,
    /// Recovery guarantee offered by this target.
    pub durability: String,
    /// Credential sources accepted by this target.
    pub credential_sources: Vec<String>,
}

/// Response body for the `list_sandbox_targets` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxTargetsResponse {
    /// Target descriptors known to this deployment.
    pub items: Vec<SandboxTargetDescriptor>,
}
