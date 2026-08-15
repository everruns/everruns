//! Neutral session-scoped storage, schedule, and resource contracts.

use crate::error::Result;
use crate::leased_resource::{LeasedResource, UpsertLeasedResource};
use crate::session_schedule::SessionSchedule;
use crate::typed_id::{ScheduleId, SessionId};
use async_trait::async_trait;

/// Info about a stored key (without its value)
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Info about a stored secret (without its value)
#[derive(Debug, Clone)]
pub struct SecretInfo {
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Trait for session key/value and secret storage operations
///
/// This trait abstracts storage operations for tools that need to persist
/// data within a session. Implementations can:
/// - Store data in a database (production)
/// - Use in-memory storage for testing
///
/// Storage for session-scoped key/value pairs and secrets.
///
/// Key/value storage is for general data that doesn't need encryption.
/// Secret storage is for sensitive data that is encrypted at rest.
#[async_trait]
pub trait SessionStorageStore: Send + Sync {
    // Key/Value operations (plain text)

    /// Set a key/value pair (creates or updates)
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()>;

    /// Get a value by key
    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>>;

    /// Delete a key/value pair
    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool>;

    /// List all keys in a session
    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>>;

    // Secret operations (encrypted)

    /// Set a secret (creates or updates, value is encrypted before storage)
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()>;

    /// Get a secret by name (value is decrypted before returning)
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>>;

    /// Delete a secret
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool>;

    /// List all secret names in a session (without values)
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>>;
}

// ============================================================================
// SessionScheduleStore - For session-scoped schedule operations
// ============================================================================

/// Trait for session schedule CRUD operations.
///
/// Used by scheduling tools to create, cancel, and list schedules.
#[async_trait]
pub trait SessionScheduleStore: Send + Sync {
    /// Create a new schedule for a session.
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule>;

    /// Create a new schedule after enforcing create-time limits in the same
    /// store operation. Backends with shared mutable state must override this
    /// to make the check-and-create sequence atomic.
    async fn create_schedule_enforcing_limits(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: String,
    ) -> std::result::Result<SessionSchedule, crate::session_schedule::ScheduleLimitError> {
        let per_session = self
            .count_active_schedules(session_id)
            .await
            .map_err(crate::session_schedule::ScheduleLimitError::Store)?;
        if per_session >= crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION {
            return Err(crate::session_schedule::ScheduleLimitError::Rejected(
                format!(
                    "Maximum {} active schedules per session. Cancel an existing schedule first.",
                    crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION
                ),
            ));
        }

        let max_per_org = crate::session_schedule::DEFAULT_MAX_SCHEDULES_PER_ORG;
        let per_org = self
            .count_active_org_schedules()
            .await
            .map_err(crate::session_schedule::ScheduleLimitError::Store)?;
        if i64::from(per_org) >= max_per_org {
            return Err(crate::session_schedule::ScheduleLimitError::Rejected(
                format!(
                    "Maximum {max_per_org} active schedules per org reached. Cancel an existing schedule first."
                ),
            ));
        }

        if let Some(cron) = cron_expression.as_deref() {
            crate::session_schedule::validate_cron_min_interval(cron)
                .map_err(crate::session_schedule::ScheduleLimitError::Rejected)?;
        }

        self.create_schedule(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
        )
        .await
        .map_err(crate::session_schedule::ScheduleLimitError::Store)
    }

    /// Cancel (disable) a schedule.
    async fn cancel_schedule(
        &self,
        session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule>;

    /// List schedules for a session.
    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>>;

    /// Count active (enabled) schedules for a session.
    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32>;

    /// Count active (enabled) schedules across the whole org this store is
    /// scoped to. Used to enforce a per-org cap independent of session count:
    /// `count_active_schedules` only bounds one session, so unlimited sessions
    /// would otherwise imply unlimited active schedules per org.
    async fn count_active_org_schedules(&self) -> Result<u32>;
}

// ============================================================================
// SessionResourceRegistry - Generic session-scoped resource registry
// ============================================================================

/// Generic registry of resources active alongside a session.
///
/// Capabilities register resources here (sandboxes, subagents, browser sessions).
/// Agents query it ("what's running?"), infrastructure scans it for cleanup.
/// See `knowledge/runtime-resources/session-resources.md`.
#[async_trait]
pub trait SessionResourceRegistry: Send + Sync {
    /// Register a resource (or update if resource_id already exists for this session).
    async fn register(
        &self,
        entry: crate::session_resource::RegisterSessionResource,
    ) -> Result<crate::session_resource::SessionResourceEntry>;

    /// Update the status of a registered resource.
    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: crate::session_resource::SessionResourceStatus,
    ) -> Result<Option<crate::session_resource::SessionResourceEntry>>;

    /// Get a specific resource by ID.
    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<crate::session_resource::SessionResourceEntry>>;

    /// List resources for a session, optionally filtered.
    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&crate::session_resource::SessionResourceFilter>,
    ) -> Result<Vec<crate::session_resource::SessionResourceEntry>>;

    /// Remove a resource from the registry.
    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool>;
}

// ============================================================================
// LeasedResourceStore - For lifecycle-managed external resources
// ============================================================================

/// Trait for session-scoped leased resource operations.
///
/// Tools use this store to register or refresh leases when they create or use
/// external provider resources. Cleanup workers operate through control-plane
/// storage APIs directly so they can claim work across organizations.
#[async_trait]
pub trait LeasedResourceStore: Send + Sync {
    /// Create or refresh a leased resource for a session.
    ///
    /// Implementations must treat this as an idempotent upsert keyed by the
    /// provider-specific resource identity so repeated tool usage extends the
    /// same lease instead of creating duplicate rows.
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource>;

    /// Mark a leased resource as explicitly released.
    ///
    /// This is the fast path for explicit user intent such as "close browser"
    /// or "delete sandbox". It should transition the resource to `released`
    /// without waiting for the durable cleanup worker to observe lease expiry.
    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>>;

    /// List leased resources currently associated with a session.
    ///
    /// Session surfaces use this for visibility. Released resources remain
    /// visible so operators can inspect cleanup outcomes and failure history.
    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>>;
}
