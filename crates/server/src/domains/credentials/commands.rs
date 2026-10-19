use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use everruns_contracts::runtime::credential_services::CredentialInvocation;
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct WorkerCredentials {
    pub session_id: String,
    pub input_message_id: Uuid,
    pub operation: String,
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
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListScopedCredentials {
    #[serde(flatten)]
    pub owner: domain::CredentialOwnerInput,
}
#[command(name="list_scoped_credentials",category="credentials",description="List credential metadata for an authorized owner. Values are never returned.",method="GET",path="/v1/credential-bindings",policy=CREDENTIAL_VIEW)]
impl Command for ListScopedCredentials {
    type Output = Vec<domain::ManagedCredential>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::list(ctx, &self.owner).await
    }
}
#[command(name="create_scoped_credential",category="credentials",description="Declare an HTTP or MCP write-only credential binding. The owner provisions its value in the secure Credentials UI.",method="POST",path="/v1/credential-bindings",policy=CREDENTIAL_MANAGE)]
impl Command for domain::CreateScopedCredential {
    type Output = Uuid;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::create(ctx, self).await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RevokeScopedCredential {
    pub id: Uuid,
    pub revision: i64,
}
#[command(name="revoke_scoped_credential",category="credentials",description="Revoke a credential at the observed revision; future sends fail closed.",method="DELETE",path="/v1/credential-bindings/{id}",policy=CREDENTIAL_MANAGE)]
impl Command for RevokeScopedCredential {
    type Output = ();
    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        domain::revoke(ctx, self.id, self.revision).await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListResourceGrants {
    pub id: Uuid,
}
#[command(name="list_resource_grants",category="credentials",description="List explicit use grants for an authorized resource owner.",method="GET",path="/v1/resources/{id}/grants",policy=CREDENTIAL_VIEW)]
impl Command for ListResourceGrants {
    type Output = Vec<domain::ManagedResourceGrant>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        domain::grants(ctx, self.id).await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GrantResource {
    pub id: Uuid,
    #[serde(flatten)]
    pub grant: domain::GrantCredential,
}
#[command(name="grant_resource_access",category="credentials",description="Explicitly grant bounded resource use to a consumer. Use does not grant management or resharing.",method="POST",path="/v1/resources/{id}/grants",policy=CREDENTIAL_MANAGE)]
impl Command for GrantResource {
    type Output = Uuid;
    async fn execute(self, ctx: &Ctx) -> Result<Uuid, CommandError> {
        domain::grant(ctx, self.id, self.grant).await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RevokeResourceGrant {
    pub id: Uuid,
    pub grant_id: Uuid,
}
#[command(name="revoke_resource_grant",category="credentials",description="Revoke an explicit resource use grant.",method="DELETE",path="/v1/resources/{id}/grants/{grant_id}",policy=CREDENTIAL_MANAGE)]
impl Command for RevokeResourceGrant {
    type Output = ();
    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        domain::revoke_grant(ctx, self.id, self.grant_id).await
    }
}
