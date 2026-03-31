// Audit logging types and trait (EVE-226)
//
// Decision: Two audit domains — Management (admin ops) and Agent (execution ops).
// Decision: AuditLogger trait is generic enough for SaaS to extend with custom event types.
// Decision: Fire-and-forget pattern — audit failures never block operations (TM-OBS-007).
// Decision: Domain-specific builders enforce type safety at compile time.
// See specs/audit-logging.md for full design.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Audit Domain
// ============================================================================

/// Top-level audit domain separating management ops from agent execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDomain {
    /// Org CRUD, member management, API keys, settings, role changes.
    Management,
    /// Agent runs, tool calls, LLM interactions.
    Agent,
}

impl AuditDomain {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AuditDomain::Management => "management",
            AuditDomain::Agent => "agent",
        }
    }
}

impl fmt::Display for AuditDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AuditDomain {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "management" => Ok(AuditDomain::Management),
            "agent" => Ok(AuditDomain::Agent),
            other => Err(format!("unknown audit domain: {other}")),
        }
    }
}

// ============================================================================
// Audit Action
// ============================================================================

/// Typed audit actions grouped by domain.
///
/// Convention: `domain.resource.verb` string form (e.g. `management.member.invited`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementAction {
    // Organization
    OrgCreated,
    OrgUpdated,
    OrgDeleted,
    // Members
    MemberInvited,
    MemberRemoved,
    MemberRoleChanged,
    // API keys
    ApiKeyCreated,
    ApiKeyRevoked,
    // Settings
    SettingsUpdated,
    // Agents
    AgentCreated,
    AgentUpdated,
    AgentDeleted,
    // Harnesses
    HarnessCreated,
    HarnessUpdated,
    HarnessDeleted,
    // LLM Providers
    LlmProviderCreated,
    LlmProviderUpdated,
    LlmProviderDeleted,
    // MCP Servers
    McpServerCreated,
    McpServerUpdated,
    McpServerDeleted,
    // Apps
    AppCreated,
    AppUpdated,
    AppDeleted,
    // Skills
    SkillCreated,
    SkillUpdated,
    SkillDeleted,
}

impl ManagementAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OrgCreated => "management.org.created",
            Self::OrgUpdated => "management.org.updated",
            Self::OrgDeleted => "management.org.deleted",
            Self::MemberInvited => "management.member.invited",
            Self::MemberRemoved => "management.member.removed",
            Self::MemberRoleChanged => "management.member.role_changed",
            Self::ApiKeyCreated => "management.api_key.created",
            Self::ApiKeyRevoked => "management.api_key.revoked",
            Self::SettingsUpdated => "management.settings.updated",
            Self::AgentCreated => "management.agent.created",
            Self::AgentUpdated => "management.agent.updated",
            Self::AgentDeleted => "management.agent.deleted",
            Self::HarnessCreated => "management.harness.created",
            Self::HarnessUpdated => "management.harness.updated",
            Self::HarnessDeleted => "management.harness.deleted",
            Self::LlmProviderCreated => "management.llm_provider.created",
            Self::LlmProviderUpdated => "management.llm_provider.updated",
            Self::LlmProviderDeleted => "management.llm_provider.deleted",
            Self::McpServerCreated => "management.mcp_server.created",
            Self::McpServerUpdated => "management.mcp_server.updated",
            Self::McpServerDeleted => "management.mcp_server.deleted",
            Self::AppCreated => "management.app.created",
            Self::AppUpdated => "management.app.updated",
            Self::AppDeleted => "management.app.deleted",
            Self::SkillCreated => "management.skill.created",
            Self::SkillUpdated => "management.skill.updated",
            Self::SkillDeleted => "management.skill.deleted",
        }
    }
}

impl fmt::Display for ManagementAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Agent-domain audit actions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAction {
    RunStarted,
    RunCompleted,
    RunFailed,
    ToolExecuted,
    LlmRequest,
}

impl AgentAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunStarted => "agent.run.started",
            Self::RunCompleted => "agent.run.completed",
            Self::RunFailed => "agent.run.failed",
            Self::ToolExecuted => "agent.tool.executed",
            Self::LlmRequest => "agent.llm.request",
        }
    }
}

impl fmt::Display for AgentAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unified action enum wrapping domain-specific actions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuditAction {
    Management(ManagementAction),
    Agent(AgentAction),
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Management(a) => a.as_str(),
            Self::Agent(a) => a.as_str(),
        }
    }

    pub fn domain(&self) -> AuditDomain {
        match self {
            Self::Management(_) => AuditDomain::Management,
            Self::Agent(_) => AuditDomain::Agent,
        }
    }
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ManagementAction> for AuditAction {
    fn from(a: ManagementAction) -> Self {
        Self::Management(a)
    }
}

impl From<AgentAction> for AuditAction {
    fn from(a: AgentAction) -> Self {
        Self::Agent(a)
    }
}

// ============================================================================
// Audit Target
// ============================================================================

/// Identifies the resource affected by an audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditTarget {
    /// Resource type (e.g. "member", "agent", "harness", "session").
    pub target_type: String,
    /// Resource identifier (public ID or UUID string).
    pub target_id: String,
}

impl AuditTarget {
    pub fn new(target_type: impl Into<String>, target_id: impl Into<String>) -> Self {
        Self {
            target_type: target_type.into(),
            target_id: target_id.into(),
        }
    }
}

// ============================================================================
// Audit Event
// ============================================================================

/// A complete audit event ready for storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub domain: AuditDomain,
    pub action: AuditAction,
    pub actor_user_id: Option<Uuid>,
    pub org_id: i64,
    pub target: Option<AuditTarget>,
    pub details: serde_json::Value,
    pub ip_address: Option<String>,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Builders
// ============================================================================

/// Builder for constructing audit events with type safety.
pub struct AuditEventBuilder {
    domain: AuditDomain,
    action: AuditAction,
    actor_user_id: Option<Uuid>,
    org_id: i64,
    target: Option<AuditTarget>,
    details: serde_json::Map<String, serde_json::Value>,
    ip_address: Option<String>,
}

impl AuditEventBuilder {
    pub fn target(mut self, target_type: impl Into<String>, target_id: impl Into<String>) -> Self {
        self.target = Some(AuditTarget::new(target_type, target_id));
        self
    }

    pub fn detail(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub fn ip(mut self, ip: impl Into<String>) -> Self {
        self.ip_address = Some(ip.into());
        self
    }

    pub fn build(self) -> AuditEvent {
        AuditEvent {
            domain: self.domain,
            action: self.action,
            actor_user_id: self.actor_user_id,
            org_id: self.org_id,
            target: self.target,
            details: serde_json::Value::Object(self.details),
            ip_address: self.ip_address,
            timestamp: Utc::now(),
        }
    }
}

impl AuditEvent {
    /// Start building a management audit event.
    pub fn management(
        action: ManagementAction,
        org_id: i64,
        actor: Option<Uuid>,
    ) -> AuditEventBuilder {
        AuditEventBuilder {
            domain: AuditDomain::Management,
            action: AuditAction::Management(action),
            actor_user_id: actor,
            org_id,
            target: None,
            details: serde_json::Map::new(),
            ip_address: None,
        }
    }

    /// Start building an agent audit event.
    pub fn agent(action: AgentAction, org_id: i64, actor: Option<Uuid>) -> AuditEventBuilder {
        AuditEventBuilder {
            domain: AuditDomain::Agent,
            action: AuditAction::Agent(action),
            actor_user_id: actor,
            org_id,
            target: None,
            details: serde_json::Map::new(),
            ip_address: None,
        }
    }
}

// ============================================================================
// HasAuditTargetId — extract target ID from service return values
// ============================================================================

/// Trait for extracting an audit target ID from a service method result.
///
/// Implement on domain types (e.g. `Harness`, `Agent`) so the `#[audit]` macro
/// can automatically capture the target ID from the `Ok(value)` return.
pub trait HasAuditTargetId {
    fn audit_target_id(&self) -> Option<String>;
}

/// Blanket: if there's no meaningful target, return None.
impl HasAuditTargetId for () {
    fn audit_target_id(&self) -> Option<String> {
        None
    }
}

impl HasAuditTargetId for bool {
    fn audit_target_id(&self) -> Option<String> {
        None
    }
}

impl HasAuditTargetId for String {
    fn audit_target_id(&self) -> Option<String> {
        Some(self.clone())
    }
}

impl<T: HasAuditTargetId> HasAuditTargetId for Vec<T> {
    fn audit_target_id(&self) -> Option<String> {
        None
    }
}

// Domain type implementations
impl HasAuditTargetId for crate::Harness {
    fn audit_target_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}

impl HasAuditTargetId for crate::Agent {
    fn audit_target_id(&self) -> Option<String> {
        Some(self.public_id.to_string())
    }
}

impl HasAuditTargetId for crate::Session {
    fn audit_target_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}

// ============================================================================
// AuditLogger Trait
// ============================================================================

/// Contract for audit log persistence.
///
/// Implementations must be non-blocking and failure-tolerant (TM-OBS-007).
/// The default strategy is fire-and-forget via `tokio::spawn`.
#[async_trait::async_trait]
pub trait AuditLogger: Send + Sync {
    /// Persist an audit event. Implementations should not propagate errors
    /// to callers — log warnings internally on failure.
    async fn log_event(&self, event: AuditEvent) -> anyhow::Result<()>;

    /// Fire-and-forget helper: spawns `log_event` on a background task.
    fn emit(&self, event: AuditEvent)
    where
        Self: 'static + Clone,
    {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.log_event(event).await {
                tracing::warn!(error = %e, "Failed to write audit log");
            }
        });
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_builder_produces_correct_domain() {
        let event = AuditEvent::management(ManagementAction::MemberInvited, 1, Some(Uuid::nil()))
            .target("member", "usr_abc123")
            .detail("role", "admin")
            .build();

        assert_eq!(event.domain, AuditDomain::Management);
        assert_eq!(event.action.as_str(), "management.member.invited");
        assert_eq!(event.org_id, 1);
        assert_eq!(event.actor_user_id, Some(Uuid::nil()));
        assert!(event.target.is_some());
        let target = event.target.unwrap();
        assert_eq!(target.target_type, "member");
        assert_eq!(target.target_id, "usr_abc123");
        assert_eq!(event.details["role"], "admin");
    }

    #[test]
    fn agent_builder_produces_correct_domain() {
        let event = AuditEvent::agent(AgentAction::RunStarted, 1, None)
            .target("session", "ses_abc123")
            .build();

        assert_eq!(event.domain, AuditDomain::Agent);
        assert_eq!(event.action.as_str(), "agent.run.started");
        assert!(event.target.is_some());
    }

    #[test]
    fn action_domain_derivation() {
        let mgmt: AuditAction = ManagementAction::ApiKeyCreated.into();
        assert_eq!(mgmt.domain(), AuditDomain::Management);

        let agent: AuditAction = AgentAction::ToolExecuted.into();
        assert_eq!(agent.domain(), AuditDomain::Agent);
    }

    #[test]
    fn domain_roundtrip() {
        assert_eq!(
            "management".parse::<AuditDomain>().unwrap(),
            AuditDomain::Management
        );
        assert_eq!("agent".parse::<AuditDomain>().unwrap(), AuditDomain::Agent);
        assert!("unknown".parse::<AuditDomain>().is_err());
    }

    #[test]
    fn event_serialization() {
        let event =
            AuditEvent::management(ManagementAction::OrgCreated, 1, Some(Uuid::nil())).build();

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["domain"], "management");
    }
}
