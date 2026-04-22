use super::queries as q;
use super::types::ListUsersResponse;
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListUsers {
    #[serde(default)]
    pub search: Option<String>,
}

impl Command for ListUsers {
    type Output = ListUsersResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_users",
            category: "users",
            description: "List users in the current organization. Supports search filtering.",
            method: "GET",
            path: "/v1/users",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListUsersResponse, CommandError> {
        if ctx.caller.user_id.is_none() {
            return Err(CommandError::Forbidden(
                "Users require an authenticated user".into(),
            ));
        }

        let rows = ctx
            .db
            .list_users_by_org(ctx.org_id(), self.search.as_deref())
            .await
            .map_err(classify_anyhow)?;

        Ok(crate::api::common::ListResponse::new(
            rows.into_iter().map(q::row_to_user).collect(),
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<ListUsers>() }
