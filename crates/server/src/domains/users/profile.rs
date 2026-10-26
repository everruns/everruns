//! The account owner's editable profile, independent of the active tenant.
use crate::domains::common::*;
use everruns_core::Policy;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Ownership is checked below and by the history reader; an organization role
// must never grant access to another person's account history.
pub const OWN_PROFILE: Policy = Policy {
    id: "profile.self",
    rules: &[],
};
fn owner(ctx: &Ctx, id: &str) -> Result<Uuid, CommandError> {
    let id = Uuid::parse_str(id).map_err(|_| CommandError::not_found("Profile"))?;
    if Some(id) != ctx.caller.user_id {
        return Err(CommandError::not_found("Profile"));
    }
    Ok(id)
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountProfile {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetAccountProfile {
    pub id: String,
}
#[command(name = "get_account_profile", category = "users", description = "Read your own editable profile, without email, tokens or credentials.", method = "GET", path = "/v1/users/{id}/profile", policy = OWN_PROFILE)]
impl Command for GetAccountProfile {
    type Output = AccountProfile;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let id = owner(ctx, &self.id)?;
        let user = ctx
            .db
            .get_user(id)
            .await?
            .ok_or_else(|| CommandError::not_found("Profile"))?;
        Ok(AccountProfile {
            id: id.to_string(),
            name: user.name,
        })
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateAccountProfile {
    pub id: String,
    pub name: String,
}
#[command(name = "update_account_profile", category = "users", description = "Update your own account display name.", method = "PATCH", path = "/v1/users/me", policy = OWN_PROFILE)]
impl Command for UpdateAccountProfile {
    type Output = AccountProfile;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let id = owner(ctx, &self.id)?;
        let name = self.name.trim();
        if name.is_empty() || name.len() > 255 {
            return Err(CommandError::bad_request(
                "Name must be 1 to 255 characters",
            ));
        }
        let user = ctx
            .db
            .update_user(
                id,
                crate::storage::UpdateUser {
                    name: Some(name.into()),
                    ..Default::default()
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Profile"))?;
        Ok(AccountProfile {
            id: id.to_string(),
            name: user.name,
        })
    }
}
