use super::types::OrganizationResponse;
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::storage::{OrganizationRow, OrganizationWithRoleRow, StorageBackend};

pub fn require_user_id(ctx: &Ctx) -> Result<uuid::Uuid, CommandError> {
    ctx.caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Organizations require an authenticated user"))
}

pub async fn list_user_organizations(
    ctx: &Ctx,
) -> Result<Vec<OrganizationWithRoleRow>, CommandError> {
    ctx.db
        .list_user_organizations(require_user_id(ctx)?)
        .await
        .map_err(classify_anyhow)
}

pub async fn verify_membership(
    ctx: &Ctx,
    org_public_id: &str,
) -> Result<OrganizationWithRoleRow, CommandError> {
    list_user_organizations(ctx)
        .await?
        .into_iter()
        .find(|org| org.public_id == org_public_id)
        .ok_or_else(|| CommandError::not_found("Organization"))
}

pub async fn build_organization_response(
    db: &StorageBackend,
    org_id: i64,
    row: OrganizationRow,
) -> Result<OrganizationResponse, CommandError> {
    let settings = db
        .get_organization_settings(org_id)
        .await
        .map_err(classify_anyhow)?;

    Ok(OrganizationResponse {
        id: row.public_id,
        name: row.name,
        default_model_id: settings.as_ref().and_then(|s| s.default_model_id),
        default_harness_id: settings.as_ref().and_then(|s| s.default_harness_id),
        base_harness_id: settings.as_ref().and_then(|s| s.base_harness_id),
        default_provider_per_service: settings
            .as_ref()
            .map(|s| s.default_provider_per_service.0.clone())
            .unwrap_or_default(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
