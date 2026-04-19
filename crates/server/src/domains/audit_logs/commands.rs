// Audit log commands — user-facing operations.
//
// Write path (insert) stays on the AuditLogger trait — it is a side-effect
// of other operations, not a user-invoked command. Only the read command
// is exposed here.

use super::AUDIT_LOG_VIEW;
use super::types::{AuditLogEntry, AuditLogQuery};
use crate::domains::common::*;
use chrono::{DateTime, Utc};
use everruns_core::Policy;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// ListAuditLogs
// ============================================================================

/// List audit logs for the caller's organization. Policy-gated read.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListAuditLogs {
    /// Max entries to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Cursor: return entries created before this timestamp.
    pub before: Option<DateTime<Utc>>,
    /// Filter by event type prefix (e.g. "auth.login") — legacy.
    pub event_type: Option<String>,
    /// Filter by actor UUID.
    pub actor_id: Option<Uuid>,
    /// Filter by audit domain ("management" or "agent").
    pub domain: Option<String>,
    /// Filter by action string (e.g. "management.member.invited").
    pub action: Option<String>,
}

impl Command for ListAuditLogs {
    type Output = Vec<AuditLogEntry>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_audit_logs",
            category: "audit_logs",
            description: "List audit logs for the caller's organization. Supports domain, action, actor, and event-type filters.",
            method: "GET",
            path: "/v1/orgs/{org}/audit-logs",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AUDIT_LOG_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AuditLogEntry>, CommandError> {
        // Enforce policy inside execute so HTTP and MCP paths are consistent.
        // dispatch_for also evaluates Self::policy() for MCP; double-checking
        // here costs nothing and makes this command safe to call directly.
        AUDIT_LOG_VIEW
            .evaluate(&ctx.caller)
            .map_err(|e| CommandError::Forbidden(e.message))?;

        let limit = self.limit.unwrap_or(50).clamp(1, 200);

        // THREAT[TM-TENANT]: Always use caller's org_id, never trust client-supplied value.
        let query = AuditLogQuery {
            org_id: ctx.org_id(),
            limit,
            before: self.before,
            event_type_prefix: self.event_type.as_deref(),
            actor_id: self.actor_id,
            domain: self.domain.as_deref(),
            action: self.action.as_deref(),
        };

        let rows = ctx
            .db
            .list_audit_logs(query)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(AuditLogEntry::from).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAuditLogs>() }

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateAuditLogRow;
    use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
    use std::sync::Arc;

    fn caller_with_role(role: OrgRole) -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::nil()),
            role,
            is_platform_user: false,
        }
    }

    async fn make_ctx(role: OrgRole) -> Ctx {
        let db = Arc::new(StorageBackend::in_memory());
        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "management.member.invited".to_string(),
            ip_address: None,
            metadata: serde_json::json!({}),
            domain: "management".to_string(),
            action: "management.member.invited".to_string(),
            target_type: Some("member".to_string()),
            target_id: Some("usr_001".to_string()),
        })
        .await
        .unwrap();
        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "agent.run.started".to_string(),
            ip_address: None,
            metadata: serde_json::json!({}),
            domain: "agent".to_string(),
            action: "agent.run.started".to_string(),
            target_type: Some("session".to_string()),
            target_id: Some("ses_001".to_string()),
        })
        .await
        .unwrap();

        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        Ctx::new(caller_with_role(role), db, capability_service, None)
    }

    #[tokio::test]
    async fn owner_can_list_audit_logs() {
        let ctx = make_ctx(OrgRole::Owner).await;
        let logs = ListAuditLogs::default().execute(&ctx).await.unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn admin_can_list_audit_logs() {
        let ctx = make_ctx(OrgRole::Admin).await;
        let logs = ListAuditLogs::default().execute(&ctx).await.unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn member_cannot_list_audit_logs() {
        let ctx = make_ctx(OrgRole::Member).await;
        let err = ListAuditLogs::default()
            .execute(&ctx)
            .await
            .expect_err("member should be rejected by AUDIT_LOG_VIEW policy");
        assert!(matches!(err, CommandError::Forbidden(_)));
    }

    #[tokio::test]
    async fn domain_filter() {
        let ctx = make_ctx(OrgRole::Owner).await;

        let mgmt = ListAuditLogs {
            domain: Some("management".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(mgmt.len(), 1);
        assert_eq!(mgmt[0].domain, "management");

        let agent = ListAuditLogs {
            domain: Some("agent".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].domain, "agent");
    }

    #[tokio::test]
    async fn action_filter() {
        let ctx = make_ctx(OrgRole::Owner).await;
        let logs = ListAuditLogs {
            action: Some("management.member.invited".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].target_type.as_deref(), Some("member"));
    }

    #[tokio::test]
    async fn limit_is_clamped() {
        let ctx = make_ctx(OrgRole::Owner).await;
        let logs = ListAuditLogs {
            limit: Some(10_000),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        // Limit clamped to 200; storage backend returns all available rows.
        assert_eq!(logs.len(), 2);
    }
}
