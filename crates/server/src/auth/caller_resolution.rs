use anyhow::{Context, Result};
use everruns_core::{Caller, OrgRole};
use uuid::Uuid;

use crate::storage::StorageBackend;

fn platform_user_from_roles(roles: &[String]) -> bool {
    roles.iter().any(|role| role == "admin")
}

pub async fn caller_for_user(db: &StorageBackend, org_id: i64, user_id: Uuid) -> Result<Caller> {
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

    // No selected project on these (non-HTTP) paths — scope to the org's
    // default project.
    let project_id = db
        .get_default_project(org_id)
        .await?
        .map(|p| p.project_id)
        .unwrap_or(everruns_core::DEFAULT_PROJECT_ID);

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
