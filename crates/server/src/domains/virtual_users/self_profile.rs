//! Console self-service uses the same entity lock and journal as management.
use super::record::VirtualUser;
use super::types::UpdateVirtualUserRequest;
use crate::domains::common::*;
use everruns_core::Policy;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const OWN_PROFILE: Policy = Policy {
    id: "virtual_user.self_profile",
    rules: &[],
};

pub async fn owns(ctx: &Ctx, id: &str) -> bool {
    let (Some(user), Ok(id)) = (
        ctx.caller.user_id,
        id.parse::<everruns_contracts::typed_id::VirtualUserId>(),
    ) else {
        return false;
    };
    // Join the command transaction; using the raw pool here would deadlock a
    // one-connection pool while a save or inverse owns that connection.
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM virtual_user_bindings WHERE org_id=$1 AND virtual_user_id=$2 AND management_user_id=$3 AND status='active')")
        .bind(ctx.org_id()).bind(id).bind(user).fetch_one(ctx.db.tx_pool()).await.unwrap_or(false)
}

/// Read the active organization profile bound to the signed-in account.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetOwnVirtualUserProfile {
    pub id: String,
}
#[command(name = "get_own_virtual_user_profile", http = plain, tag = "virtual-users", summary = "Get your organization profile", category = "virtual_users", description = "Read your own organization profile.", method = "GET", path = "/v1/virtual-users/{id}/self-profile", policy = OWN_PROFILE, cli = CliRoute::new(&["virtual-users", "self-profile"], "get").with_examples(&[CliExample::new("Read the organization profile bound to your signed-in account", "everruns virtual-users self-profile get --id identity_01h9")]))]
impl Command for GetOwnVirtualUserProfile {
    type Output = VirtualUser;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if !owns(ctx, &self.id).await {
            return Err(CommandError::not_found("Profile"));
        }
        super::GetVirtualUser { id: self.id }.execute(ctx).await
    }
}

/// Update the signed-in organization profile without changing its lifecycle.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateOwnVirtualUserProfile {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateVirtualUserRequest,
}
#[command(name = "update_own_virtual_user_profile", http = plain, request_body(UpdateVirtualUserRequest), tag = "virtual-users", summary = "Update your organization profile", category = "virtual_users", description = "Update your own organization profile without changing lifecycle.", method = "PATCH", path = "/v1/virtual-users/{id}/self-profile", policy = OWN_PROFILE, cli = CliRoute::new(&["virtual-users", "self-profile"], "update").with_examples(&[CliExample::new("Set your own organization profile timezone without management authority", "everruns virtual-users self-profile update --id identity_01h9 --timezone Europe/London --reason 'Use my local timezone'")]))]
impl Command for UpdateOwnVirtualUserProfile {
    type Output = VirtualUser;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if !owns(ctx, &self.id).await {
            return Err(CommandError::not_found("Profile"));
        }
        if self.req.status.is_some() {
            return Err(CommandError::forbidden(
                "Self-service cannot change lifecycle",
            ));
        }
        if self
            .req
            .name
            .as_ref()
            .is_some_and(|n| n.trim().is_empty() || n.len() > 255)
        {
            return Err(CommandError::bad_request("Invalid name"));
        }
        // Share validation and storage; authority is the proved self binding above.
        super::UpdateVirtualUserCmd {
            id: self.id,
            req: self.req,
        }
        .execute(ctx)
        .await
    }
}
