// Leased resource domain types
//
// Decision: Leased resources are the generic control-plane primitive for
// lifecycle-managed external resources (Daytona sandboxes, persistent browsers,
// and similar provider-owned state).
// Decision: A lease is represented by an explicit `lease_expires_at` timestamp,
// not only `last_touched_at + policy`, so schedulers can claim due cleanups via
// a simple indexed predicate.
// Decision: Resources are session-associated but may outlive the session row
// during cleanup, so `session_id` is optional in the domain model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::typed_id::{LeasedResourceId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Session/UI feature string for leased resource visibility.
pub const LEASED_RESOURCES_FEATURE: &str = "leased_resources";

/// Runtime status for a leased resource.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LeasedResourceStatus {
    /// Resource is alive and eligible for lease extension via tool activity.
    Active,
    /// Cleanup worker currently owns this resource and is attempting cleanup.
    Cleaning,
    /// Cleanup completed or the resource was explicitly released.
    Released,
    /// A cleanup attempt failed. The lease expiry is moved forward for retry.
    CleanupFailed,
}

impl std::fmt::Display for LeasedResourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Cleaning => write!(f, "cleaning"),
            Self::Released => write!(f, "released"),
            Self::CleanupFailed => write!(f, "cleanup_failed"),
        }
    }
}

/// A lifecycle-managed external resource owned by a session-capable workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LeasedResource {
    /// Unique identifier (format: resource_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "resource_01933b5a00007000800000000000001"))]
    pub id: LeasedResourceId,
    /// Session that currently owns the resource, if still attached.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "session_01933b5a00007000800000000000001"))]
    pub session_id: Option<SessionId>,
    /// External provider responsible for the resource (e.g. "daytona").
    #[cfg_attr(feature = "openapi", schema(example = "daytona"))]
    pub provider: String,
    /// Provider-specific resource type (e.g. "sandbox", "browser_session").
    #[cfg_attr(feature = "openapi", schema(example = "sandbox"))]
    pub resource_type: String,
    /// Stable provider identifier for cleanup calls.
    #[cfg_attr(feature = "openapi", schema(example = "sbx_a3f1c9d2e8"))]
    pub external_id: String,
    /// Optional user-facing label.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Customer 42 — Q3 brief"))]
    pub display_name: Option<String>,
    /// Current lifecycle status.
    pub status: LeasedResourceStatus,
    /// User connection owner used for provider cleanup, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "uuid", example = "550e8400-e29b-41d4-a716-446655440000"))]
    pub owner_user_id: Option<Uuid>,
    /// Lease duration used when refreshing the lease.
    #[cfg_attr(feature = "openapi", schema(example = 900))]
    pub lease_duration_seconds: u32,
    /// Last successful touch from tool activity.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:14:00Z"))]
    pub last_touched_at: DateTime<Utc>,
    /// Absolute deadline after which cleanup becomes due.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:29:00Z"))]
    pub lease_expires_at: DateTime<Utc>,
    /// Cleanup attempt start time when the resource is currently claimed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:30:00Z"))]
    pub cleanup_started_at: Option<DateTime<Utc>>,
    /// Cleanup completion time for released resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:30:04Z"))]
    pub cleanup_completed_at: Option<DateTime<Utc>>,
    /// Number of cleanup attempts so far.
    #[cfg_attr(feature = "openapi", schema(example = 0))]
    pub cleanup_attempts: u32,
    /// Last cleanup error message, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "daytona.api.timeout: provider call exceeded 10s")
    )]
    pub last_cleanup_error: Option<String>,
    /// Provider-specific non-secret metadata for UI/debugging.
    /// THREAT[TM-API-015]: This field is returned by the session resources API
    /// and rendered in the UI, so providers must never persist bearer tokens or
    /// other secrets here.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = json!({"region": "us-west-2", "snapshot_id": "snap_42"})))]
    pub metadata: serde_json::Value,
    /// Timestamp when this leased resource was created (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:00:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this leased resource was last updated (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:14:00Z"))]
    pub updated_at: DateTime<Utc>,
}

/// Input used by tools to create or refresh a lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLeasedResource {
    pub session_id: SessionId,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub owner_user_id: Option<Uuid>,
    pub lease_duration_seconds: u32,
    #[serde(default)]
    pub metadata: serde_json::Value,
}
