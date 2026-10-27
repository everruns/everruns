use super::*;

/// Source direction. Existing bindings remain inbound-only until explicitly changed.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    /// Apply GitHub changes; keep authored fields read-only.
    #[default]
    Pull,
    /// Apply GitHub changes and publish local authored edits with conflict detection.
    TwoWay,
}
impl SourceMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::TwoWay => "two_way",
        }
    }
}

/// Continuous GitHub source binding, applied revision and reconciliation status.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentSourceStatus {
    /// Direction of synchronization: pull or two_way.
    #[schema(example = "two_way")]
    pub mode: String,
    /// Local edits are waiting for GitHub synchronization.
    #[schema(example = false)]
    pub local_dirty: bool,
    /// Binding generation used to detect a concurrent reconnect or disconnect.
    #[schema(example = "018f30c8-c970-7cb5-a8c9-9348aa71ba8c")]
    pub generation: Uuid,
    /// Selected GitHub repository in owner/name form.
    #[schema(example = "acme/agents")]
    pub repository: String,
    /// Branch monitored for source updates.
    #[schema(example = "main")]
    pub branch: String,
    /// Repository-relative package folder or manifest; empty means the root.
    #[schema(example = "agents/support")]
    pub path: String,
    /// Package channel names whose configuration is managed by the source.
    #[schema(example = json!(["chat"]))]
    pub managed_channels: Vec<String>,
    /// Last successfully applied commit; retained when a later revision fails.
    #[schema(example = "0123456789abcdef0123456789abcdef01234567")]
    pub applied_commit: Option<String>,
    /// Current reconciliation state, including conflict when both sides changed.
    #[schema(example = "synced")]
    pub status: String,
    /// Safe diagnostic for the latest failed check, without upstream credentials.
    #[schema(example = "Source package validation failed: missing capability")]
    pub error: Option<String>,
    /// UTC time when the source was last checked.
    #[schema(example = "2026-10-23T12:00:00Z")]
    pub last_checked_at: Option<DateTime<Utc>>,
    /// UTC time when a source revision was last applied successfully.
    #[schema(example = "2026-10-23T12:00:00Z")]
    pub last_synced_at: Option<DateTime<Utc>>,
}
impl From<AgentSourceRow> for AgentSourceStatus {
    fn from(r: AgentSourceRow) -> Self {
        Self {
            mode: r.mode,
            local_dirty: r.local_dirty || r.outbox.is_some(),
            generation: r.generation,
            repository: r.repository,
            branch: r.branch,
            path: r.path,
            managed_channels: r.managed_channels,
            applied_commit: r.applied_commit,
            status: r.status,
            error: r.error,
            last_checked_at: r.last_checked_at,
            last_synced_at: r.last_synced_at,
        }
    }
}
