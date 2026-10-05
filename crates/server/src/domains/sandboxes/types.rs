// Response shapes of the org-wide Sandbox fleet.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::storage::backend::sandbox_fleet::{
    SandboxFleetRow, SandboxInstanceRow, SandboxTransitionRow, fleet_state,
};
use everruns_contracts::typed_id::{AgentId, SandboxId, SandboxTemplateId, SessionId};

/// Template revision a Sandbox was created from.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxTemplateRef {
    /// Sandbox Template id.
    #[schema(example = "sbxtpl_01933b5a00007000800000000000001")]
    pub id: String,
    /// Template display name.
    #[schema(example = "Coding - Daytona")]
    pub display_name: Option<String>,
    /// Revision number pinned when the Session was created.
    #[schema(example = 4)]
    pub revision: Option<i32>,
}

/// One logical Sandbox in the organization.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxFleetItem {
    /// Durable logical Sandbox id. Stays the same when the provider resource is replaced.
    #[schema(example = "sandbox_01933b5a00007000800000000000001")]
    pub id: String,
    /// `running`, `paused`, `lost`, `starting`, `failed`, `not_started` or `deleted`.
    #[schema(example = "running")]
    pub state: String,
    /// Lifecycle intent the control plane is moving the Sandbox toward: `ready`, `paused` or `deleted`.
    #[schema(example = "ready")]
    pub desired_state: String,
    /// Provider id, such as `daytona`, `modal` or `docker`.
    #[schema(example = "daytona")]
    pub provider: String,
    /// Target kind from the pinned specification: `managed`, `container`, `machine`, `vfs` or `host`.
    #[schema(example = "managed")]
    pub target_kind: Option<String>,
    /// `primary` (addressed implicitly by shell and file tools) or `resource`.
    #[schema(example = "primary")]
    pub role: String,
    /// Owning Session; absent once the Session is deleted.
    pub session_id: Option<String>,
    /// Session title, kept after the Session is deleted.
    #[schema(example = "Fix flaky CI")]
    pub session_title: Option<String>,
    /// Agent the Session ran.
    pub agent_id: Option<String>,
    /// Agent display name.
    #[schema(example = "Coder")]
    pub agent_name: Option<String>,
    /// Template revision the Sandbox was created from.
    pub template: Option<SandboxTemplateRef>,
    /// Incarnation counter. Above 1 means the provider resource was lost and rebuilt.
    #[schema(example = 1)]
    pub generation: i64,
    /// Committed workspace checkpoints.
    #[schema(example = 12)]
    pub checkpoint_count: i64,
    /// Provider's id for the latest physical resource.
    #[schema(example = "dtn-7f3a91c2")]
    pub external_id: Option<String>,
    /// Working directory inside the Sandbox.
    #[schema(example = "/home/daytona/workspace")]
    pub workspace_path: Option<String>,
    /// Inactivity interval before the Sandbox is paused.
    #[schema(example = 300)]
    pub idle_after_seconds: Option<i64>,
    /// Error from the latest init commands, if they failed.
    pub last_init_error: Option<String>,
    /// Why this Sandbox needs attention: `lost`, `failed`, `init_failed`,
    /// `idle_running` (running with no activity for an hour) or `cleanup_failed`.
    pub attention: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Last recorded tool activity or state change.
    pub last_activity_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl From<SandboxFleetRow> for SandboxFleetItem {
    fn from(row: SandboxFleetRow) -> Self {
        Self {
            id: SandboxId::from_uuid(row.id).to_string(),
            state: row.fleet_state,
            desired_state: row.desired_state,
            provider: row.provider,
            target_kind: row.target_kind,
            role: row.role,
            session_id: row
                .session_id
                .map(|id| SessionId::from_uuid(id).to_string()),
            session_title: row.session_title,
            agent_id: row.agent_id.map(|id| AgentId::from_uuid(id).to_string()),
            agent_name: row.agent_name,
            template: row.template_id.map(|id| SandboxTemplateRef {
                id: SandboxTemplateId::from_uuid(id).to_string(),
                display_name: row.template_name,
                revision: row.template_revision,
            }),
            generation: row.generation,
            checkpoint_count: row.checkpoint_count,
            external_id: row.external_id,
            workspace_path: row.workspace_path,
            idle_after_seconds: row.idle_after_seconds,
            last_init_error: row.last_init_error,
            attention: row.attention,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_activity_at: row.last_activity_at,
            deleted_at: row.deleted_at,
        }
    }
}

/// A page of the fleet.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxFleetPage {
    pub items: Vec<SandboxFleetItem>,
    /// Sandboxes matching the filters.
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// A count keyed by a label.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxCount {
    #[schema(example = "running")]
    pub key: String,
    #[schema(example = 5)]
    pub count: i64,
}

/// Roll-ups for the fleet summary.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxFleetStats {
    /// Days covered by the `*_in_window` figures.
    #[schema(example = 7)]
    pub window_days: i64,
    /// Sandboxes per state.
    pub by_state: Vec<SandboxCount>,
    /// Live Sandboxes per provider.
    pub live_by_provider: Vec<SandboxCount>,
    pub created_in_window: i64,
    /// Created in the window before, for a trend.
    pub created_in_prior_window: i64,
    /// Total time Sandboxes spent running in the window.
    pub running_seconds_in_window: i64,
    /// Provider resources lost and rebuilt in the window.
    pub recoveries_in_window: i64,
    /// Sandboxes with at least one attention reason.
    pub needs_attention: i64,
}

/// A span of time a Sandbox spent in one state.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxStateSpan {
    /// Fleet state during the span.
    #[schema(example = "running")]
    pub state: String,
    /// Incarnation during the span.
    pub generation: i64,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The Sandbox is still in this state.
    pub current: bool,
}

impl From<SandboxTransitionRow> for SandboxStateSpan {
    fn from(row: SandboxTransitionRow) -> Self {
        Self {
            state: fleet_state(&row.state).to_string(),
            generation: row.generation,
            start: row.start_at,
            end: row.end_at,
            current: row.current,
        }
    }
}

/// One physical provider resource behind a logical Sandbox.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxIncarnation {
    pub generation: i64,
    pub external_id: String,
    /// Provider status the last time it was observed: `running`, `paused` or `lost`.
    pub status: String,
    pub last_init_error: Option<String>,
    pub created_at: DateTime<Utc>,
    /// When it was replaced or deleted.
    pub retired_at: Option<DateTime<Utc>>,
}

impl From<SandboxInstanceRow> for SandboxIncarnation {
    fn from(row: SandboxInstanceRow) -> Self {
        Self {
            generation: row.generation,
            external_id: row.external_id,
            status: row.status,
            last_init_error: row.last_init_error,
            created_at: row.created_at,
            retired_at: row.retired_at,
        }
    }
}

/// One Sandbox with its incarnations and lifecycle history.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxFleetDetail {
    #[serde(flatten)]
    pub sandbox: SandboxFleetItem,
    /// Provider resources, newest first.
    pub incarnations: Vec<SandboxIncarnation>,
    /// State history, oldest first.
    pub history: Vec<SandboxStateSpan>,
}

/// One Sandbox's lane on the timeline.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxTimelineLane {
    pub sandbox: SandboxFleetItem,
    /// Spans clipped to the window, oldest first.
    pub spans: Vec<SandboxStateSpan>,
    /// Seconds spent running within the window.
    pub running_seconds: i64,
}

/// How many Sandboxes were running from `at` until the next point.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxConcurrencyPoint {
    pub at: DateTime<Utc>,
    pub running: i64,
}

/// Lifecycle of the fleet over a window.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxTimeline {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Lanes ordered by running time, longest first.
    pub lanes: Vec<SandboxTimelineLane>,
    /// Sandboxes with activity in the window, including those beyond `limit`.
    pub total_lanes: i64,
    /// Step series of concurrently running Sandboxes across every lane.
    pub concurrency: Vec<SandboxConcurrencyPoint>,
    pub peak_running: i64,
    pub peak_at: Option<DateTime<Utc>>,
}
