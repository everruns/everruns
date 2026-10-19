use super::*;
use crate::domains::agent_channels::record::{ChannelStatus, ChannelType};
use everruns_core::{DefaultPermissionResolver, OrgRole, Permission};

struct AgentsOnlyResolver;

impl PermissionResolver for AgentsOnlyResolver {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        matches!(permission, Permission::OrgAgentsManage)
    }

    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        vec![Permission::OrgAgentsManage]
    }
}

fn org_with_role(role: OrgRole) -> ResolvedOrg {
    ResolvedOrg {
        org_id: 1,
        public_id: "org_test".to_string(),
        name: "Test".to_string(),
        user_id: None,
        role,
        is_platform_user: false,
        feature_flags: crate::records::FeatureFlags::default(),
        read_only: false,
    }
}

#[test]
fn effective_harness_metadata_requires_harness_view() {
    let caller = Caller::from(&org_with_role(OrgRole::Owner));
    let (status, _) = authorize_effective_harness_view(&AgentsOnlyResolver, &caller)
        .expect_err("agent permission must not grant access to harness metadata");

    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn effective_harness_metadata_allows_harness_view() {
    let caller = Caller::from(&org_with_role(OrgRole::Owner));

    assert!(authorize_effective_harness_view(&DefaultPermissionResolver, &caller).is_ok());
}

#[test]
fn agent_channel_summary_serializes_only_non_secret_metadata() {
    let summary = AgentChannelSummary {
        id: "aep_example".into(),
        channel_type: ChannelType::Webhook,
        enabled: true,
        status: ChannelStatus::Draft,
    };
    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        serde_json::json!({
            "id": "aep_example", "channel_type": "webhook", "enabled": true, "status": "draft"
        })
    );
}
