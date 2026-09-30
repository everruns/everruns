// Virtual user domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod lifecycle;
pub mod queries;
pub mod types;

pub use commands::*;

pub const VIRTUAL_USER_VIEW: Policy = Policy {
    id: "virtual_user.view",
    rules: &[Rule::UserHasPermission(Permission::OrgVirtualUsersView)],
};

pub const VIRTUAL_USER_MANAGE: Policy = Policy {
    id: "virtual_user.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgVirtualUsersManage)],
};

// Dangerous virtual-user ops reuse OrgAppsDangerous (Owner-only) as the danger
// tier — a pre-existing choice kept here to preserve the Owner-only gate.
pub const VIRTUAL_USER_DANGEROUS: Policy = Policy {
    id: "virtual_user.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgVirtualUsersManage),
        Rule::UserHasPermission(Permission::OrgAppsDangerous),
    ],
};

/// Private end-user grants require the proved console self binding. Service
/// grants require management policy. Profile administration grants neither.
pub async fn connection_target(
    db: &crate::storage::StorageBackend,
    resolver: &dyn everruns_core::PermissionResolver,
    caller: &everruns_core::Caller,
    raw: &str,
) -> Result<everruns_provider::typed_id::VirtualUserId, crate::domains::common::CommandError> {
    use crate::domains::common::{CommandError, classify_anyhow};
    let id = if raw == "me" {
        let uid = caller
            .user_id
            .ok_or_else(|| CommandError::forbidden("Authentication required"))?;
        db.default_virtual_user(caller.org_id, uid)
            .await
            .map_err(classify_anyhow)?
            .id
    } else {
        raw.parse()
            .map_err(|_| CommandError::bad_request("Invalid virtual user ID"))?
    };
    let v = db
        .get_virtual_user(caller.org_id, id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Virtual user"))?;
    if v.status != "active" {
        return Err(CommandError::forbidden("Runtime account is not active"));
    }
    if v.usage == "end_user" {
        let bindings = db
            .list_virtual_user_bindings(caller.org_id, id)
            .await
            .map_err(classify_anyhow)?;
        if caller.user_id.is_none()
            || !bindings
                .iter()
                .any(|b| b.management_user_id == caller.user_id)
        {
            return Err(CommandError::forbidden("Private end-user connection"));
        }
    } else {
        VIRTUAL_USER_MANAGE
            .evaluate_with(resolver, caller)
            .map_err(|_| CommandError::forbidden("Permission denied"))?;
    }
    Ok(id)
}
