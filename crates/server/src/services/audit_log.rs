// Audit log service (EVE-226)
//
// Provides policy-gated read access to audit logs.
// Write path is via AuditLogger trait (fire-and-forget from service layer).

use crate::storage::StorageBackend;
use crate::storage::models::{AuditLogQuery, AuditLogRow};
use anyhow::Result;
use everruns_core::{Caller, Permission, Policy, Rule};
use everruns_macros::policy;
use std::sync::Arc;

/// Policy: View audit logs (read-only, admin+owner).
pub const AUDIT_LOG_VIEW: Policy = Policy {
    id: "audit_log.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAuditLogsView)],
};

/// Re-export for API config endpoint.
pub const fn policies() -> [&'static Policy; 1] {
    [&AUDIT_LOG_VIEW]
}

pub struct AuditLogService {
    db: Arc<StorageBackend>,
}

impl AuditLogService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    #[policy(AUDIT_LOG_VIEW)]
    pub async fn list(
        &self,
        caller: &Caller,
        query: AuditLogQuery<'_>,
    ) -> Result<Vec<AuditLogRow>> {
        self.db.list_audit_logs(query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateAuditLogRow;
    use everruns_core::{DEFAULT_ORG_ID, organization::OrgRole};
    use uuid::Uuid;

    fn owner_caller() -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::nil()),
            role: OrgRole::Owner,
            is_platform_user: false,
        }
    }

    fn member_caller() -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::nil()),
            role: OrgRole::Member,
            is_platform_user: false,
        }
    }

    async fn make_service() -> AuditLogService {
        let db = Arc::new(StorageBackend::in_memory());
        // Seed some audit logs
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

        AuditLogService::new(db)
    }

    #[tokio::test]
    async fn test_owner_can_list_audit_logs() {
        let svc = make_service().await;
        let caller = owner_caller();
        let logs = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn test_admin_can_list_audit_logs() {
        let svc = make_service().await;
        let caller = Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::nil()),
            role: OrgRole::Admin,
            is_platform_user: false,
        };
        let logs = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn test_member_cannot_list_audit_logs() {
        let svc = make_service().await;
        let caller = member_caller();
        let result = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.downcast_ref::<everruns_core::PolicyError>().is_some());
    }

    #[tokio::test]
    async fn test_domain_filter() {
        let svc = make_service().await;
        let caller = owner_caller();
        let mgmt = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    domain: Some("management"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(mgmt.len(), 1);
        assert_eq!(mgmt[0].domain, "management");

        let agent = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    domain: Some("agent"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].domain, "agent");
    }

    #[tokio::test]
    async fn test_action_filter() {
        let svc = make_service().await;
        let caller = owner_caller();
        let logs = svc
            .list(
                &caller,
                AuditLogQuery {
                    org_id: caller.org_id,
                    limit: 50,
                    action: Some("management.member.invited"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].target_type.as_deref(), Some("member"));
    }
}
