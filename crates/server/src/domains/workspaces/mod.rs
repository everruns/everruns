// Workspace domain.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod record;
pub mod types;

pub use commands::*;

pub const WORKSPACE_VIEW: Policy = Policy {
    id: "workspace.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const WORKSPACE_MANAGE: Policy = Policy {
    id: "workspace.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Insert a workspace for tests that need a plain org entity to rename,
/// archive or attach history to. There is no create command: a real workspace
/// is only ever made by the session that owns it.
#[cfg(test)]
pub(crate) async fn seed_workspace(ctx: &crate::domains::common::Ctx, name: &str) -> String {
    let row = ctx
        .db
        .create_workspace(
            ctx.org_id(),
            crate::storage::CreateWorkspaceRow {
                id: None,
                public_id: everruns_contracts::typed_id::WorkspaceId::new().to_string(),
                name: name.to_string(),
                description: None,
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("seed workspace");
    row.public_id
}
