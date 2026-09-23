use anyhow::{Context, Result};
use everruns_core::{Caller, OrgRole};
use uuid::Uuid;

use crate::storage::StorageBackend;

fn platform_user_from_roles(roles: &[String]) -> bool {
    roles.iter().any(|role| role == "admin")
}

/// Resolve a user's caller identity for a non-HTTP path.
///
/// `project_id` is the project the call runs in: platform tools a session
/// invokes pass the session's project, so an agent only reaches agents in its
/// own project. `None` means the org's default project.
pub async fn caller_for_user(
    db: &StorageBackend,
    org_id: i64,
    user_id: Uuid,
    project_id: Option<i64>,
) -> Result<Caller> {
    let user = db
        .get_user(user_id)
        .await?
        .with_context(|| format!("User {user_id} not found"))?;
    let membership = db
        .list_user_organizations(user_id)
        .await?
        .into_iter()
        .find(|org| org.org_id == org_id)
        .with_context(|| format!("User {user_id} is not a member of organization {org_id}"))?;
    let roles: Vec<String> = serde_json::from_value(user.roles).unwrap_or_default();

    let project_id = match project_id {
        Some(project_id) => project_id,
        None => db
            .get_default_project(org_id)
            .await?
            .map(|p| p.project_id)
            .unwrap_or(everruns_core::DEFAULT_PROJECT_ID),
    };

    Ok(Caller {
        org_id,
        org_public_id: membership.public_id,
        project_id,
        user_id: Some(user_id),
        role: membership
            .role
            .parse::<OrgRole>()
            .unwrap_or(OrgRole::Member),
        is_platform_user: platform_user_from_roles(&roles),
        is_internal: false,
    })
}
