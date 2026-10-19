use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use everruns_contracts::runtime::credential_services::CredentialInvocation;
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Internal credential operation tied to a persisted invocation, never arbitrary caller identity.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct WorkerCredentials {
    /// Session containing the persisted invocation.
    pub session_id: String,
    /// Persisted input message establishing the invocation identity.
    pub input_message_id: Uuid,
    /// Internal credential service operation to execute.
    pub operation: String,
    /// Operation-specific parameters; excluded from debug output.
    pub params: Value,
}
impl std::fmt::Debug for WorkerCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerCredentials")
            .field("operation", &self.operation)
            .field("params", &"<redacted>")
            .finish()
    }
}

#[command(name="worker_credentials",category="credentials",description="Internal: authorize, resolve or capture write-only credentials for a persisted invocation.",method="POST",path="/internal/sessions/{session_id}/credentials",policy=SESSION_VIEW)]
impl Command for WorkerCredentials {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let session_id: SessionId = self
            .session_id
            .parse()
            .map_err(|_| CommandError::bad_request("Invalid session ID"))?;
        let invocation = CredentialInvocation {
            session_id,
            input_message_id: self.input_message_id,
        };
        super::service::execute(ctx, invocation, &self.operation, self.params).await
    }
}

use super::{CREDENTIAL_MANAGE, CREDENTIAL_VIEW, management as domain};
/// Select an authorized owner whose credential metadata should be listed.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListScopedCredentials {
    /// Owner and required invocation usage.
    #[serde(flatten)]
    pub owner: domain::CredentialOwnerInput,
}
#[command(
    name="list_scoped_credentials",category="credentials",description="List credential metadata for an authorized owner. Values are never returned.",method="GET",path="/v1/credential-bindings",policy=CREDENTIAL_VIEW,
    cli = CliRoute::new(&["credential-bindings"], "list").with_examples(&[CliExample::new("Review organization credential metadata before granting access", "everruns credential-bindings list --scope organization --usage service")]),
)]
impl Command for ListScopedCredentials {
    type Output = Vec<domain::ManagedCredential>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::list(ctx, &self.owner).await
    }
}
#[command(
    name="create_scoped_credential",category="credentials",description="Declare an HTTP or MCP write-only credential binding. The owner provisions its value in the secure Credentials UI.",method="POST",path="/v1/credential-bindings",policy=CREDENTIAL_MANAGE,
    cli = CliRoute::new(&["credential-bindings"], "create").with_examples(&[CliExample::new("Declare personal mailbox access without putting a value in the command", "everruns credential-bindings create --scope virtual_user --owner-id me --usage user --name MAIL_TOKEN --label 'Mailbox API' --binding '{\"url\":\"https://mail.example/api\",\"auth\":{\"type\":\"bearer\"}}'")]),
)]
impl Command for domain::CreateScopedCredential {
    type Output = Uuid;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::create(ctx, self).await
    }
}
/// Revoke a credential using an optimistic concurrency precondition.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RevokeScopedCredential {
    /// Credential binding identifier.
    pub id: Uuid,
    /// Last observed resource revision; stale revocations are rejected.
    pub revision: i64,
}
#[command(
    name="revoke_scoped_credential",category="credentials",description="Revoke a credential at the observed revision; future sends fail closed.",method="DELETE",path="/v1/credential-bindings/{id}",policy=CREDENTIAL_MANAGE,
    cli = CliRoute::new(&["credential-bindings"], "revoke").with_examples(&[CliExample::new("Revoke a credential using the revision returned by the metadata list", "everruns credential-bindings revoke --id 00000000-0000-0000-0000-000000000001 --revision 3")]),
)]
impl Command for RevokeScopedCredential {
    type Output = ();
    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        domain::revoke(ctx, self.id, self.revision).await
    }
}
/// Select a credential or connection whose explicit use grants should be listed.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListResourceGrants {
    /// Credential binding or native connection resource identifier.
    pub id: Uuid,
}
#[command(
    name="list_resource_grants",category="credentials",description="List explicit use grants for an authorized resource owner.",method="GET",path="/v1/resources/{id}/grants",policy=CREDENTIAL_VIEW,
    cli = CliRoute::new(&["resources", "grants"], "list").with_examples(&[CliExample::new("Review who can use a credential or connection", "everruns resources grants list --id 00000000-0000-0000-0000-000000000001")]),
)]
impl Command for ListResourceGrants {
    type Output = Vec<domain::ManagedResourceGrant>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::grants(ctx, self.id).await
    }
}
/// Grant bounded resource use through an explicit owner action.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GrantResource {
    /// Credential binding or native connection resource identifier.
    pub id: Uuid,
    /// Consumer, required usage, expiry, and optional access bounds.
    #[serde(flatten)]
    pub grant: domain::GrantCredential,
}
#[command(
    name="grant_resource_access",category="credentials",description="Explicitly grant bounded resource use to a consumer. Use does not grant management or resharing.",method="POST",path="/v1/resources/{id}/grants",policy=CREDENTIAL_MANAGE,
    cli = CliRoute::new(&["resources", "grants"], "grant").with_examples(&[CliExample::new("Allow one service agent to use an organization credential", "everruns resources grants grant --id 00000000-0000-0000-0000-000000000001 --consumer-scope agent --consumer-id agent_00000000000000000000000000000002 --usage service")]),
)]
impl Command for GrantResource {
    type Output = Uuid;
    async fn execute(self, ctx: &Ctx) -> Result<Uuid, CommandError> {
        domain::grant(ctx, self.id, self.grant).await
    }
}
/// Revoke one consumer's explicit grant without revoking the resource itself.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RevokeResourceGrant {
    /// Credential binding or native connection resource identifier.
    pub id: Uuid,
    /// Explicit use grant identifier.
    pub grant_id: Uuid,
}
#[command(
    name="revoke_resource_grant",category="credentials",description="Revoke an explicit resource use grant.",method="DELETE",path="/v1/resources/{id}/grants/{grant_id}",policy=CREDENTIAL_MANAGE,
    cli = CliRoute::new(&["resources", "grants"], "revoke").with_examples(&[CliExample::new("Stop a consumer from using a resource without revoking its other grants", "everruns resources grants revoke --id 00000000-0000-0000-0000-000000000001 --grant-id 00000000-0000-0000-0000-000000000003")]),
)]
impl Command for RevokeResourceGrant {
    type Output = ();
    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        domain::revoke_grant(ctx, self.id, self.grant_id).await
    }
}
