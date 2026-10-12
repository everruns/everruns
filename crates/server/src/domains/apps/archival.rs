// Deprecated read-only access to frozen App records.

use super::queries as q;
use crate::domains::agent_channels::redact_channel_for_response;
use crate::domains::apps::record::App;
use crate::domains::common::*;
use everruns_contracts::typed_id::AppId;
use everruns_core::{Permission, Policy, Rule};
use serde::Deserialize;
use utoipa::ToSchema;

const APP_VIEW: Policy = Policy {
    id: "app.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAppsView)],
};

fn redact_app_for_response(mut app: App) -> App {
    app.channels = app
        .channels
        .into_iter()
        .map(redact_channel_for_response)
        .collect();
    app
}

/// List frozen App records for archival access.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListApps {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListApps {
    type Output = Vec<App>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_apps",
            category: "apps",
            description: "List frozen App records for archival access.",
            method: "GET",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<App>, CommandError> {
        let rows = ctx
            .db
            .list_apps(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await?;
        Ok(
            q::load_apps_list(&ctx.db, ctx.encryption.as_ref(), rows, ctx.org_id())
                .await?
                .into_iter()
                .map(redact_app_for_response)
                .collect(),
        )
    }
}

/// Get one frozen App record for archival access.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetApp {
    pub id: String,
}

impl Command for GetApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_app",
            category: "apps",
            description: "Get one frozen App record for archival access.",
            method: "GET",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        q::get_by_public_id(
            &ctx.db,
            ctx.encryption.as_ref(),
            ctx.org_id(),
            &app_id.to_string(),
        )
        .await?
        .map(redact_app_for_response)
        .ok_or_else(|| CommandError::not_found("App"))
    }
}
