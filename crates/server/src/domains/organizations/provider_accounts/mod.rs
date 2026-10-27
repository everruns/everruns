// Organization provider accounts (organization connections), read side.
//
// Decision: listing is a command so the CLI and MCP see the sandbox provider
// and web search accounts the UI manages. It returns non-secret metadata only:
// the encrypted credential and provider metadata never leave storage here.
// Creating, replacing and verifying accounts stay on REST
// (`api::organization_connections`), since they carry a credential.
// Decision: like the REST route (`OrgAdmin`), only an organization admin
// lists, and only for the caller's active organization; any other org is
// NotFound, which keeps org existence private.

use crate::domains::common::*;
use crate::domains::organizations::record::validate_org_public_id;
use chrono::{DateTime, Utc};
use everruns_core::{Permission, Policy, Rule};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Policy: list an org's provider accounts (organization admins, as REST).
pub const ORG_PROVIDER_ACCOUNTS_VIEW: Policy = Policy {
    id: "org.provider_accounts.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

#[derive(Debug, Clone, Serialize, ToSchema)]
/// Non-secret metadata for an organization-owned provider account.
pub struct OrganizationConnectionResponse {
    /// Stable connection identifier used by Sandbox Templates and web search.
    #[schema(example = "00000000-0000-0000-0000-000000000002")]
    pub id: Uuid,
    /// Human-readable account name.
    #[schema(example = "Production Daytona")]
    pub name: String,
    /// Connector identifier, e.g. `daytona` or `brave_search`.
    #[schema(example = "daytona")]
    pub provider: String,
    /// Provider identity returned while validating the credential, when available.
    #[schema(example = "platform-team")]
    pub provider_username: Option<String>,
    /// Time at which the provider account was first connected.
    #[schema(example = "2026-10-07T12:00:00Z")]
    pub connected_at: DateTime<Utc>,
    /// Time at which the account name or credential was last replaced.
    #[schema(example = "2026-10-07T12:30:00Z")]
    pub updated_at: DateTime<Utc>,
}

impl OrganizationConnectionResponse {
    pub fn from_row(row: crate::storage::VirtualUserConnectionRow) -> Self {
        Self {
            id: row.id,
            name: row.name.unwrap_or_else(|| row.provider.clone()),
            provider: row.provider,
            provider_username: row.provider_username,
            connected_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListOrganizationConnections {
    /// Organization public ID.
    pub org: String,
}

#[command(
    name = "list_organization_connections",
    category = "organizations",
    description = "List the organization's provider accounts (sandbox provider accounts such as Daytona or E2B, and web search accounts): id, name, provider, provider username, and timestamps. Metadata only, never credentials. Use an id as a Sandbox Template's connection or a web search account. Organization admins only.",
    method = "GET",
    path = "/v1/orgs/{org}/organization-connections",
    policy = ORG_PROVIDER_ACCOUNTS_VIEW,
    cli = CliRoute::new(&["orgs", "provider-accounts"], "list").with_examples(&[CliExample::new("Find the sandbox provider account to point a Sandbox Template at", "everruns orgs provider-accounts list --org org_01h9")]),
)]
impl Command for ListOrganizationConnections {
    type Output = Vec<OrganizationConnectionResponse>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<OrganizationConnectionResponse>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<OrganizationConnectionResponse>, CommandError> {
        if !validate_org_public_id(&self.org) || self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        Ok(ctx
            .db
            .list_organization_connections(ctx.org_id())
            .await?
            .into_iter()
            .map(OrganizationConnectionResponse::from_row)
            .collect())
    }
}

#[cfg(test)]
mod tests;

/// Editable history contains only the account's display name.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizationAccountMetadata {
    pub id: Uuid,
    pub name: String,
}
/// Read the editable name of an authorized organization account.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetOrganizationAccount {
    pub id: Uuid,
}
#[command(name = "get_organization_account", http = plain, tag = "Organizations", summary = "Get organization account name", category = "organizations", description = "Read an organization account's editable name. Credentials are never returned.", method = "GET", path = "/v1/organization-accounts/{id}", policy = ORG_PROVIDER_ACCOUNTS_VIEW, cli = CliRoute::new(&["organization-accounts"], "get").with_examples(&[CliExample::new("Read an organization account name without retrieving its credentials", "everruns organization-accounts get --id 00000000-0000-0000-0000-000000000001")]))]
impl Command for GetOrganizationAccount {
    type Output = OrganizationAccountMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = ctx
            .db
            .get_organization_connection(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Account"))?;
        Ok(OrganizationAccountMetadata {
            id: row.id,
            name: row.name.unwrap_or(row.provider),
        })
    }
}
/// Rename an organization account without replacing its credentials.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateOrganizationAccountName {
    pub id: Uuid,
    #[serde(flatten)]
    pub req: OrganizationAccountNameRequest,
}
/// A new display name; account credentials remain unchanged.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct OrganizationAccountNameRequest {
    pub name: String,
}
#[command(name = "update_organization_account_name", http = plain, request_body(OrganizationAccountNameRequest), tag = "Organizations", summary = "Rename organization account", category = "organizations", description = "Rename an organization account while preserving its current credentials.", method = "PATCH", path = "/v1/organization-accounts/{id}", policy = ORG_PROVIDER_ACCOUNTS_VIEW, cli = CliRoute::new(&["organization-accounts"], "update").with_examples(&[CliExample::new("Rename a connected account while preserving its credential", "everruns organization-accounts update --id 00000000-0000-0000-0000-000000000001 --name 'Northstar sandbox' --reason 'Clarify the account purpose'")]))]
impl Command for UpdateOrganizationAccountName {
    type Output = OrganizationAccountMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let name = self.req.name.trim();
        if name.is_empty() || name.len() > 255 {
            return Err(CommandError::bad_request(
                "Account name must be 1 to 255 characters",
            ));
        }
        let row = ctx
            .db
            .rename_organization_connection(ctx.org_id(), self.id, name)
            .await?
            .ok_or_else(|| CommandError::not_found("Account"))?;
        Ok(OrganizationAccountMetadata {
            id: row.id,
            name: row.name.unwrap_or(row.provider),
        })
    }
}
