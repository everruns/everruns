use super::queries as q;
use super::types::SyncModelsResponse;
use super::{LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    Policy, everruns_provider::provider::DriverId, everruns_provider::provider::ProviderStatus,
};
use everruns_provider::provider::Provider;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize)]
pub struct DeleteProviderResult {
    pub deleted: bool,
}

fn sync_service(
    ctx: &Ctx,
) -> Result<&std::sync::Arc<crate::services::ModelSyncService>, CommandError> {
    ctx.model_sync_service
        .as_ref()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Model sync service not configured")))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProvider {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub provider_type: DriverId,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Trace/observability link configuration override (driver defaults apply
    /// when omitted).
    #[serde(default)]
    pub trace: Option<everruns_provider::provider::ProviderTraceConfig>,
}

impl Command for CreateProvider {
    type Output = Provider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_provider",
            category: "providers",
            description: "Create a new LLM provider.",
            method: "POST",
            path: "/v1/providers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        q::service(ctx)
            .create(
                &ctx.caller,
                crate::api::providers::CreateProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                    // Typed credentials are resolved into `api_key` at the HTTP
                    // boundary; the command carries the assembled document.
                    credentials: None,
                    trace: self.trace,
                },
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateProvider>() }

// Empty-braces (not a unit struct) so serde deserializes the empty `{}` params
// object the MCP/command dispatcher passes; a unit struct rejects a map with
// "invalid type: map, expected unit struct ListProviders".
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListProviders {}

impl Command for ListProviders {
    type Output = Vec<Provider>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_providers",
            category: "providers",
            description: "List all LLM providers.",
            method: "GET",
            path: "/v1/providers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Provider>, CommandError> {
        q::service(ctx)
            .list(&ctx.caller)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListProviders>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for GetProvider {
    type Output = Provider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_provider",
            category: "providers",
            description: "Get a specific LLM provider.",
            method: "GET",
            path: "/v1/providers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        q::service(ctx)
            .get(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Provider"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetProvider>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    pub provider_type: Option<DriverId>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Current lifecycle status.
    pub status: Option<ProviderStatus>,
    /// Trace/observability link configuration override (merged into stored
    /// settings, preserving other keys).
    #[serde(default)]
    pub trace: Option<everruns_provider::provider::ProviderTraceConfig>,
}

impl Command for UpdateProvider {
    type Output = Provider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_provider",
            category: "providers",
            description: "Update an LLM provider.",
            method: "PATCH",
            path: "/v1/providers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        q::service(ctx)
            .update(
                &ctx.caller,
                provider_id,
                crate::api::providers::UpdateProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                    // Typed credentials are resolved into `api_key` at the HTTP
                    // boundary; the command carries the assembled document.
                    credentials: None,
                    status: self.status,
                    trace: self.trace,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Provider"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateProvider>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DeleteProvider {
    type Output = DeleteProviderResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_provider",
            category: "providers",
            description: "Delete an LLM provider.",
            method: "DELETE",
            path: "/v1/providers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeleteProviderResult, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        let deleted = q::service(ctx)
            .delete(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Provider"));
        }
        Ok(DeleteProviderResult { deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteProvider>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncProviderModels {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for SyncProviderModels {
    type Output = SyncModelsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "sync_provider_models",
            category: "providers",
            description: "Discover and sync models from a provider.",
            method: "POST",
            path: "/v1/providers/{id}/sync-models",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SyncModelsResponse, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        let result = sync_service(ctx)?
            .sync_provider(ctx.org_id(), provider_id)
            .await
            .map_err(classify_anyhow)?;
        match result {
            crate::services::SyncResult::Success {
                created,
                updated,
                stale,
            } => Ok(SyncModelsResponse::Success {
                created,
                updated,
                stale,
            }),
            crate::services::SyncResult::NotSupported => Ok(SyncModelsResponse::NotSupported),
            crate::services::SyncResult::Failed { error } => {
                Err(CommandError::internal(anyhow::anyhow!(error)))
            }
        }
    }
}

inventory::submit! { CommandDescriptor::of::<SyncProviderModels>() }
