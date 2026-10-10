use super::credential_check::CredentialCheckResult;
use super::queries as q;
use super::types::SyncModelsResponse;
use super::{LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW};
use crate::domains::common::*;
use crate::domains::providers::record::Provider;
use crate::kernel_imports::{contracts::provider::DriverId, contracts::provider::ProviderStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize)]
pub struct DeleteProviderResult {
    pub deleted: bool,
}

fn sync_service(
    ctx: &Ctx,
) -> Result<&std::sync::Arc<crate::domains::models::ModelSyncService>, CommandError> {
    ctx.model_sync_service
        .as_ref()
        // A transport whose context lacks the service is a deployment gap, not
        // an internal fault: answer 503 rather than paging on a 500.
        .ok_or_else(|| CommandError::unavailable("Model sync is not available on this endpoint"))
}

/// Make a provider that just gained a credential immediately usable: discover
/// its models, then bootstrap the org's enabled models and default model.
///
/// Best-effort by design. The provider row is already written and valid; a
/// provider API that is slow, unreachable, or rejects the key must not fail the
/// create/update. The UI surfaces the resulting state (and a "no intelligence
/// configured" notice) from the models it can see.
async fn provision_provider_models(ctx: &Ctx, provider: &Provider) {
    let provider_uuid = provider.id.uuid();

    if let Some(sync) = ctx.model_sync_service.as_ref() {
        match tokio::time::timeout(
            PROVISION_TIMEOUT,
            sync.sync_provider(ctx.org_id(), provider_uuid),
        )
        .await
        {
            // A provider-reported failure is a warning, not routine info: the
            // key may be wrong. Its message is an upstream string, so it is
            // logged at the same level and shape as any other sync failure
            // rather than folded into a success line.
            Ok(Ok(crate::domains::models::SyncResult::Failed { error })) => tracing::warn!(
                org_id = ctx.org_id(),
                provider_id = %provider.id,
                %error,
                "Provider rejected model discovery after a credential change (non-fatal)"
            ),
            Ok(Ok(result)) => tracing::info!(
                org_id = ctx.org_id(),
                provider_id = %provider.id,
                ?result,
                "Synced models for newly credentialed provider"
            ),
            Ok(Err(error)) => tracing::warn!(
                org_id = ctx.org_id(),
                provider_id = %provider.id,
                %error,
                "Model sync after provider credential change failed (non-fatal)"
            ),
            Err(_) => tracing::warn!(
                org_id = ctx.org_id(),
                provider_id = %provider.id,
                "Model sync after provider credential change timed out (non-fatal)"
            ),
        }
    }

    if let Some(models) = ctx.model_service.as_ref()
        && let Err(error) = models
            .bootstrap_intelligence(ctx.org_id(), provider_uuid)
            .await
    {
        tracing::warn!(
            org_id = ctx.org_id(),
            provider_id = %provider.id,
            %error,
            "Intelligence bootstrap after provider credential change failed (non-fatal)"
        );
    }
}

/// Upper bound on the inline discovery call a create/update waits for. Long
/// enough for a cold provider API, short enough that the onboarding request
/// still returns.
const PROVISION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateProvider {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub provider_type: DriverId,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Trace/observability link configuration override (driver defaults apply
    /// when omitted).
    #[serde(default)]
    pub trace: Option<everruns_contracts::provider::ProviderTraceConfig>,
    pub request_options: Option<everruns_contracts::provider::ProviderRequestOptions>,
}

#[command(
    name = "create_provider",
    category = "providers",
    description = "Create a new LLM provider.",
    method = "POST",
    path = "/v1/providers",
    policy = LLM_PROVIDER_MANAGE,
)]
impl Command for CreateProvider {
    type Output = Provider;

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        if self.provider_type.as_str() == "mistral" && !ctx.feature_flags.mistral {
            return Err(CommandError::feature_not_enabled("mistral"));
        }
        let has_credential = self.api_key.is_some();
        let provider = q::service(ctx)
            .create(
                &ctx.caller,
                crate::domains::providers::types::CreateProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                    // Typed credentials are resolved into `api_key` at the HTTP
                    // boundary; the command carries the assembled document.
                    credentials: None,
                    trace: self.trace,
                    request_options: self.request_options,
                },
            )
            .await?;

        if has_credential {
            provision_provider_models(ctx, &provider).await;
        }
        Ok(provider)
    }
}

// Empty-braces (not a unit struct) so serde deserializes the empty `{}` params
// object the MCP/command dispatcher passes; a unit struct rejects a map with
// "invalid type: map, expected unit struct ListProviders".
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListProviders {}

#[command(
    name = "list_providers",
    category = "providers",
    description = "List all LLM providers.",
    method = "GET",
    path = "/v1/providers",
    policy = LLM_PROVIDER_VIEW,
)]
impl Command for ListProviders {
    type Output = Vec<Provider>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Provider>, CommandError> {
        q::service(ctx)
            .list(&ctx.caller)
            .await
            .map_err(classify_anyhow)
    }
}

/// Check a candidate credential against the provider before it is stored.
///
/// Deliberately takes the credential inline rather than a provider id: the
/// point is to find out whether a key works *before* any provider row exists
/// (setup) or before an edit overwrites a working one. Nothing is persisted.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CheckProviderCredentials {
    pub provider_type: DriverId,
    /// The credential to probe. Typed multi-field credentials are assembled
    /// into this single document at the HTTP boundary, as for create.
    pub api_key: String,
    /// Optional custom endpoint. Validated exactly as on create — this issues
    /// a real outbound request.
    #[serde(default)]
    pub base_url: Option<String>,
}

#[command(
    name = "check_provider_credentials",
    category = "providers",
    description = "Check a provider API key without storing it.",
    method = "POST",
    path = "/v1/providers/check-credentials",
    policy = LLM_PROVIDER_MANAGE,
)]
impl Command for CheckProviderCredentials {
    type Output = CredentialCheckResult;

    async fn execute(self, ctx: &Ctx) -> Result<CredentialCheckResult, CommandError> {
        if self.api_key.trim().is_empty() {
            return Err(CommandError::bad_request("api_key must not be empty"));
        }
        crate::domains::providers::service::validate_provider_type(&self.provider_type)?;
        crate::domains::providers::service::validate_provider_base_url(
            self.provider_type.clone(),
            self.base_url.as_deref(),
        )?;

        Ok(crate::domains::providers::check_credentials(
            &ctx.driver_registry,
            self.provider_type,
            self.api_key,
            self.base_url,
        )
        .await)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_provider",
    category = "providers",
    description = "Get a specific LLM provider.",
    method = "GET",
    path = "/v1/providers/{id}",
    policy = LLM_PROVIDER_VIEW,
    positional = "id",
)]
impl Command for GetProvider {
    type Output = Provider;

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        q::service(ctx)
            .get(&ctx.caller, provider_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Provider"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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
    pub trace: Option<everruns_contracts::provider::ProviderTraceConfig>,
    pub request_options: Option<everruns_contracts::provider::ProviderRequestOptions>,
}

#[command(
    name = "update_provider",
    category = "providers",
    description = "Update an LLM provider.",
    method = "PATCH",
    path = "/v1/providers/{id}",
    policy = LLM_PROVIDER_MANAGE,
)]
impl Command for UpdateProvider {
    type Output = Provider;

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        let has_credential = self.api_key.is_some();
        let provider = q::service(ctx)
            .update(
                &ctx.caller,
                provider_id,
                crate::domains::providers::types::UpdateProviderRequest {
                    name: self.name,
                    provider_type: self.provider_type,
                    base_url: self.base_url,
                    api_key: self.api_key,
                    // Typed credentials are resolved into `api_key` at the HTTP
                    // boundary; the command carries the assembled document.
                    credentials: None,
                    status: self.status,
                    trace: self.trace,
                    request_options: self.request_options,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Provider"))?;

        // A key arriving on an existing provider is the same event as one
        // arriving with a new provider: the org may only now have intelligence.
        if has_credential {
            provision_provider_models(ctx, &provider).await;
        }
        Ok(provider)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "delete_provider",
    category = "providers",
    description = "Delete an LLM provider.",
    method = "DELETE",
    path = "/v1/providers/{id}",
    policy = LLM_PROVIDER_MANAGE,
    positional = "id",
)]
impl Command for DeleteProvider {
    type Output = DeleteProviderResult;

    async fn execute(self, ctx: &Ctx) -> Result<DeleteProviderResult, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        let deleted = q::service(ctx).delete(&ctx.caller, provider_id).await?;
        if !deleted {
            return Err(CommandError::not_found("Provider"));
        }
        Ok(DeleteProviderResult { deleted })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ReviewProviderModels {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "review_provider_models",
    category = "providers",
    description = "Mark a provider's discovered models as reviewed, clearing their new flag.",
    method = "POST",
    path = "/v1/providers/{id}/models/review",
    policy = LLM_PROVIDER_MANAGE,
)]
impl Command for ReviewProviderModels {
    type Output = Provider;

    async fn execute(self, ctx: &Ctx) -> Result<Provider, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        let service = q::service(ctx);
        if !service
            .mark_models_reviewed(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Provider"));
        }
        service
            .get(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Provider"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SyncProviderModels {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "sync_provider_models",
    category = "providers",
    description = "Discover and sync models from a provider.",
    method = "POST",
    path = "/v1/providers/{id}/sync-models",
    policy = LLM_PROVIDER_MANAGE,
)]
impl Command for SyncProviderModels {
    type Output = SyncModelsResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SyncModelsResponse, CommandError> {
        let provider_id = q::parse_provider_id(&self.id)?;
        q::service(ctx)
            .get(&ctx.caller, provider_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Provider"))?;
        let result = sync_service(ctx)?
            .sync_provider(ctx.org_id(), provider_id)
            .await?;

        // A manual refresh is also a chance to make the org usable: an org that
        // still has no enabled chat model or no resolvable default gets one.
        if let Some(models) = ctx.model_service.as_ref()
            && let Err(error) = models
                .bootstrap_intelligence(ctx.org_id(), provider_id)
                .await
        {
            tracing::warn!(
                org_id = ctx.org_id(),
                %provider_id,
                %error,
                "Intelligence bootstrap after model sync failed (non-fatal)"
            );
        }

        match result {
            crate::domains::models::SyncResult::Success {
                created,
                updated,
                stale,
            } => Ok(SyncModelsResponse::Success {
                created,
                updated,
                stale,
            }),
            crate::domains::models::SyncResult::NotSupported => {
                Ok(SyncModelsResponse::NotSupported)
            }
            crate::domains::models::SyncResult::Failed { error } => {
                Err(CommandError::internal(anyhow::anyhow!(error)))
            }
        }
    }
}
