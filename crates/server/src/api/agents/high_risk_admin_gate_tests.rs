use super::*;
use crate::records::{ChannelStatus, ChannelType};
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use everruns_core::{DefaultPermissionResolver, Permission};
use std::sync::Arc;

struct AgentsOnlyResolver;

impl PermissionResolver for AgentsOnlyResolver {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        matches!(permission, Permission::OrgAgentsManage)
    }

    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        vec![Permission::OrgAgentsManage]
    }
}

fn capability_service() -> CapabilityService {
    let db = Arc::new(StorageBackend::in_memory());
    CapabilityService::with_registry(db, None, crate::platform::oss_capability_registry())
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
    }
}

fn caps(refs: &[&str]) -> Vec<everruns_contracts::CapabilityRef> {
    refs.iter()
        .map(|r| everruns_contracts::CapabilityRef::new((*r).to_string()))
        .collect()
}

#[test]
fn member_blocked_from_assigning_bashkit_shell() {
    let svc = capability_service();
    let result = require_admin_for_high_risk(
        &org_with_role(OrgRole::Member),
        &caps(&["bashkit_shell"]),
        &svc,
    );
    let (status, body) = result.expect_err("member must not assign bashkit_shell");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.0
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("bashkit_shell")
    );
}

#[test]
fn member_blocked_from_assigning_legacy_virtual_bash_alias() {
    // The pre-rename `virtual_bash` ID resolves to `bashkit_shell` via
    // registry aliasing; the admin gate must cover it identically.
    let svc = capability_service();
    let result = require_admin_for_high_risk(
        &org_with_role(OrgRole::Member),
        &caps(&["virtual_bash"]),
        &svc,
    );
    let (status, _body) = result.expect_err("member must not assign via legacy alias");
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn member_blocked_from_assigning_web_fetch() {
    let svc = capability_service();
    let result =
        require_admin_for_high_risk(&org_with_role(OrgRole::Member), &caps(&["web_fetch"]), &svc);
    let (status, body) = result.expect_err("member must not assign web_fetch");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.0.detail.as_deref().unwrap_or("").contains("web_fetch"));
}

#[test]
fn member_allowed_for_low_risk_capability() {
    let svc = capability_service();
    let result = require_admin_for_high_risk(
        &org_with_role(OrgRole::Member),
        &caps(&["current_time"]),
        &svc,
    );
    assert!(
        result.is_ok(),
        "low-risk capabilities must remain assignable by members"
    );
}

#[test]
fn admin_allowed_for_bashkit_shell() {
    let svc = capability_service();
    let result = require_admin_for_high_risk(
        &org_with_role(OrgRole::Admin),
        &caps(&["bashkit_shell"]),
        &svc,
    );
    assert!(result.is_ok());
}

#[test]
fn owner_allowed_for_web_fetch() {
    let svc = capability_service();
    let result =
        require_admin_for_high_risk(&org_with_role(OrgRole::Owner), &caps(&["web_fetch"]), &svc);
    assert!(result.is_ok());
}

#[test]
fn empty_capability_list_allowed_for_members() {
    let svc = capability_service();
    let result = require_admin_for_high_risk(&org_with_role(OrgRole::Member), &[], &svc);
    assert!(result.is_ok());
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
