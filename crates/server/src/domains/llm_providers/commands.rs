use super::queries as q;
use super::types::SyncModelsResponse;
use super::{LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW};
use crate::domains::common::*;
use everruns_core::llm_models::LlmProvider;
use everruns_core::{LlmProviderStatus, LlmProviderType, Policy};
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
        .ok_or_else(|| CommandError::Internal(anyhow::anyhow!("Model sync service not configured")))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProvider {
    pub name: String,
    pub provider_type: LlmProviderType,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

impl Command for CreateProvider {
    type Output = LlmProvider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_provider",
            category: "llm_providers",
            description: "Create a new LLM provider.",
            method: "POST",
            path: "/v1/llm-providers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<LlmProvider, CommandError> {
        q::service(ctx)
            .create(
                &ctx.caller,
                crate::api::llm_providers::CreateLlmProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                },
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateProvider>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListProviders;

impl Command for ListProviders {
    type Output = Vec<LlmProvider>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_providers",
            category: "llm_providers",
            description: "List all LLM providers.",
            method: "GET",
            path: "/v1/llm-providers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<LlmProvider>, CommandError> {
        q::service(ctx)
            .list(&ctx.caller)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListProviders>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetProvider {
    pub id: String,
}

impl Command for GetProvider {
    type Output = LlmProvider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_provider",
            category: "llm_providers",
            description: "Get a specific LLM provider.",
            method: "GET",
            path: "/v1/llm-providers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<LlmProvider, CommandError> {
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
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<LlmProviderType>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub status: Option<LlmProviderStatus>,
}

impl Command for UpdateProvider {
    type Output = LlmProvider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_provider",
            category: "llm_providers",
            description: "Update an LLM provider.",
            method: "PATCH",
            path: "/v1/llm-providers/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_PROVIDER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<LlmProvider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        q::service(ctx)
            .update(
                &ctx.caller,
                provider_id,
                crate::api::llm_providers::UpdateLlmProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                    status: self.status,
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
    pub id: String,
}

impl Command for DeleteProvider {
    type Output = DeleteProviderResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_provider",
            category: "llm_providers",
            description: "Delete an LLM provider.",
            method: "DELETE",
            path: "/v1/llm-providers/{id}",
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
    pub id: String,
}

impl Command for SyncProviderModels {
    type Output = SyncModelsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "sync_provider_models",
            category: "llm_providers",
            description: "Discover and sync models from a provider.",
            method: "POST",
            path: "/v1/llm-providers/{id}/sync-models",
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
                Err(CommandError::Internal(anyhow::anyhow!(error)))
            }
        }
    }
}

inventory::submit! { CommandDescriptor::of::<SyncProviderModels>() }
