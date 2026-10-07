use super::queries as q;
use super::types::ListUsersResponse;
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListUsers {
    #[serde(default)]
    pub search: Option<String>,
}

#[command(
    name = "list_users",
    category = "users",
    description = "List users in the current organization. Supports search filtering.",
    method = "GET",
    path = "/v1/users"
)]
impl Command for ListUsers {
    type Output = ListUsersResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ListUsersResponse, CommandError> {
        if ctx.caller.user_id.is_none() {
            return Err(CommandError::forbidden(
                "Users require an authenticated user",
            ));
        }

        let rows = ctx
            .db
            .list_users_by_org(ctx.org_id(), self.search.as_deref())
            .await?;

        Ok(crate::api::common::ListResponse::new(
            rows.into_iter().map(q::row_to_user).collect(),
        ))
    }
}
