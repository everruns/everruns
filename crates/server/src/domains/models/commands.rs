use super::queries as q;
use super::{LLM_MODEL_MANAGE, LLM_MODEL_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    contracts::model::Model, contracts::model::ModelSource, contracts::model::ModelWithProvider,
    contracts::typed_id::ProviderId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize)]
pub struct DeleteModelResult {
    pub deleted: bool,
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateModel {
    /// LLM provider's prefixed public identifier.
    pub provider_id: String,
    /// LLM model's prefixed public identifier.
    pub model_id: String,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: String,
    #[serde(default)]
    /// Service the model provides: chat, decisions, embeddings, realtime, ...
    pub service: Option<everruns_contracts::ServiceKind>,
    #[serde(default)]
    /// Stable model profile to assign to the model.
    pub profile_key: Option<String>,
    #[serde(default)]
    /// Capability labels for the model.
    pub capabilities: Vec<String>,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: bool,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Show the model first in pickers.
    pub is_favorite: bool,
}

#[command(
    name = "create_model",
    category = "models",
    description = "Create a new model for a provider.",
    method = "POST",
    cli = CliRoute::new(&["providers", "models"], "create").with_examples(&[CliExample::new("Register a model that provider discovery does not list", "everruns providers models create --provider-id provider_01h9 --model-id gpt-5.1 --display-name 'GPT-5.1' --enabled true --reason 'Make the new model selectable'")]),
    path = "/v1/providers/{provider_id}/models",
    policy = LLM_MODEL_MANAGE,
)]
impl Command for CreateModel {
    type Output = Model;

    async fn execute(self, ctx: &Ctx) -> Result<Model, CommandError> {
        let provider_id = q::parse_provider_id(&self.provider_id)?;
        q::service(ctx)
            .create(
                &ctx.caller,
                provider_id,
                crate::domains::models::types::CreateModelRequest {
                    model_id: self.model_id,
                    display_name: self.display_name,
                    service: self.service,
                    profile_key: self.profile_key,
                    capabilities: self.capabilities,
                    enabled: self.enabled,
                    is_favorite: self.is_favorite,
                },
            )
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListProviderModels {
    /// LLM provider's prefixed public identifier.
    pub provider_id: String,
}

#[command(
    name = "list_provider_models",
    category = "models",
    description = "List models for a specific provider.",
    method = "GET",
    cli = CliRoute::new(&["providers", "models"], "list").with_examples(&[CliExample::new("See which models one provider offers", "everruns providers models list --provider-id provider_01h9")]),
    path = "/v1/providers/{provider_id}/models",
    policy = LLM_MODEL_VIEW,
)]
impl Command for ListProviderModels {
    type Output = Vec<Model>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Model>, CommandError> {
        let provider_id = q::parse_provider_id(&self.provider_id)?;
        q::service(ctx)
            .list_for_provider(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListModels {
    /// Only models providing this service.
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Only models from this source: manual, discovered, or predefined.
    pub source: Option<ModelSource>,
    #[serde(
        default = "default_true",
        deserialize_with = "deserialize_bool_lenient"
    )]
    /// Include models the provider no longer lists. Defaults to true.
    pub include_stale: bool,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Only models marked as favorites.
    pub favorites_only: bool,
}

const fn default_true() -> bool {
    true
}

#[command(
    name = "list_models",
    category = "models",
    description = "List all models across all providers.",
    method = "GET",
    cli = CliRoute::new(&["models"], "list").with_examples(&[CliExample::new("Find the models you have starred", "everruns models list --favorites-only true")]),
    path = "/v1/models",
    policy = LLM_MODEL_VIEW,
)]
impl Command for ListModels {
    type Output = Vec<ModelWithProvider>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<ModelWithProvider>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<ModelWithProvider>, CommandError> {
        let models = q::service(ctx)
            .list_all_with_filters(
                &ctx.caller,
                self.source,
                self.include_stale,
                self.favorites_only,
            )
            .await?;
        Ok(models
            .into_iter()
            .filter(|model| self.service.is_none_or(|service| model.service == service))
            .collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_model",
    category = "models",
    description = "Get a specific model with provider information.",
    method = "GET",
    cli = CliRoute::new(&["models"], "get").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Show a model with its provider and capabilities", "everruns models get model_01h9")]),
    path = "/v1/models/{id}",
    policy = LLM_MODEL_VIEW,
    positional = "id",
)]
impl Command for GetModel {
    type Output = ModelWithProvider;

    async fn execute(self, ctx: &Ctx) -> Result<ModelWithProvider, CommandError> {
        let model_id = q::parse_model_id(&self.id)?;
        q::service(ctx)
            .get_with_provider(&ctx.caller, model_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Model"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// LLM provider's prefixed public identifier.
    pub provider_id: Option<String>,
    /// LLM model's prefixed public identifier.
    pub model_id: Option<String>,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: Option<String>,
    /// Change the selected service; it must match the assigned profile.
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Explicitly reassign the stable profile; preference-only edits preserve it.
    pub profile_key: Option<String>,
    /// Replacement capability labels.
    pub capabilities: Option<Vec<String>>,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Show the model first in pickers.
    pub is_favorite: Option<bool>,
}

#[command(
    name = "update_model",
    category = "models",
    description = "Update a model.",
    method = "PATCH",
    cli = CliRoute::new(&["models"], "update").with_examples(&[CliExample::new("Star a model so it appears first in pickers", "everruns models update --id model_01h9 --is-favorite true --reason 'Team default'")]),
    path = "/v1/models/{id}",
    policy = LLM_MODEL_MANAGE,
)]
impl Command for UpdateModel {
    type Output = Model;

    async fn execute(self, ctx: &Ctx) -> Result<Model, CommandError> {
        let model_id = q::parse_model_id(&self.id)?;
        let provider_id = self
            .provider_id
            .as_deref()
            .map(q::parse_provider_id)
            .transpose()?;
        q::service(ctx)
            .update(
                &ctx.caller,
                model_id,
                crate::domains::models::types::UpdateModelRequest {
                    provider_id: provider_id.map(|id| ProviderId::from_uuid(id).to_string()),
                    model_id: self.model_id,
                    display_name: self.display_name,
                    service: self.service,
                    profile_key: self.profile_key,
                    capabilities: self.capabilities,
                    enabled: self.enabled,
                    is_favorite: self.is_favorite,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Model"))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "delete_model",
    category = "models",
    description = "Delete a model.",
    method = "DELETE",
    cli = CliRoute::new(&["models"], "delete").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Remove a model that was registered by mistake", "everruns models delete model_01h9 --reason 'Duplicate of an existing entry'")]),
    path = "/v1/models/{id}",
    policy = LLM_MODEL_MANAGE,
    positional = "id",
)]
impl Command for DeleteModel {
    type Output = DeleteModelResult;

    async fn execute(self, ctx: &Ctx) -> Result<DeleteModelResult, CommandError> {
        let model_id = q::parse_model_id(&self.id)?;
        let deleted = q::service(ctx).delete(&ctx.caller, model_id).await?;
        if !deleted {
            return Err(CommandError::not_found("Model"));
        }
        Ok(DeleteModelResult { deleted })
    }
}

#[cfg(test)]
mod parse_tests {
    //! Regression coverage for EVE-418: bashkit's MCP flag parser forwards
    //! booleans as JSON strings (e.g. `--enabled true` becomes
    //! `{"enabled": "true"}`). Without lenient bool coercion, deserialization
    //! used to fail with a generic `BadRequest`/`callback failed` and leave the
    //! record unchanged.

    use super::*;
    use serde_json::json;

    #[test]
    fn update_model_accepts_string_bools_for_enabled_and_is_favorite() {
        let cmd: UpdateModel = serde_json::from_value(json!({
            "id": "model_019df670b5af7db7a5685a4ad18a544a",
            "enabled": "true",
            "is_favorite": "false",
        }))
        .expect("string bools must be accepted via lenient coercion");
        assert_eq!(cmd.enabled, Some(true));
        assert_eq!(cmd.is_favorite, Some(false));
        assert_eq!(cmd.id, "model_019df670b5af7db7a5685a4ad18a544a");
    }

    #[test]
    fn update_model_accepts_native_bools() {
        let cmd: UpdateModel = serde_json::from_value(json!({
            "id": "model_x",
            "enabled": true,
            "is_favorite": true,
        }))
        .unwrap();
        assert_eq!(cmd.enabled, Some(true));
        assert_eq!(cmd.is_favorite, Some(true));
    }

    #[test]
    fn update_model_keeps_missing_bool_fields_none() {
        let cmd: UpdateModel = serde_json::from_value(json!({"id": "model_x"})).unwrap();
        assert_eq!(cmd.enabled, None);
        assert_eq!(cmd.is_favorite, None);
    }

    #[test]
    fn create_model_accepts_string_bools() {
        let cmd: CreateModel = serde_json::from_value(json!({
            "provider_id": "provider_x",
            "model_id": "gpt-5.5",
            "display_name": "GPT-5.5",
            "enabled": "true",
            "is_favorite": "1",
        }))
        .unwrap();
        assert!(cmd.enabled);
        assert!(cmd.is_favorite);
    }

    #[test]
    fn list_models_accepts_string_bools() {
        let cmd: ListModels = serde_json::from_value(json!({
            "include_stale": "false",
            "favorites_only": "true",
        }))
        .unwrap();
        assert!(!cmd.include_stale);
        assert!(cmd.favorites_only);
    }

    #[test]
    fn list_models_omitted_fields_keep_default_true_for_include_stale() {
        // `include_stale` mixes a custom deserializer with `default = "default_true"`;
        // omitting it must still resolve to `true`, not silently flip to `false`.
        let cmd: ListModels = serde_json::from_value(json!({})).unwrap();
        assert!(cmd.include_stale);
        assert!(!cmd.favorites_only);
    }
}

/// Resolve the organization's default without creating a session.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetDefaultModel {}
#[command(
    name = "get_default_model",
    category = "models",
    description = "Get the effective organization default model.",
    method = "GET",
    cli = CliRoute::new(&["models", "default"], "get").with_examples(&[CliExample::new("Check which model new sessions use when none is chosen", "everruns models default get")]),
    path = "/v1/models/default",
    policy = LLM_MODEL_VIEW,
)]
impl Command for GetDefaultModel {
    type Output = Option<ModelWithProvider>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        q::service(ctx)
            .get_default(&ctx.caller)
            .await
            .map_err(classify_anyhow)
    }
}

/// Read or change the organization's explicit decision model selection.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetDefaultDecisionModel {}
#[command(
    name = "get_default_decision_model",
    category = "models",
    description = "Get the selected decision model.",
    method = "GET",
    cli = CliRoute::new(&["models", "decision-default"], "get").with_examples(&[CliExample::new("Check which model handles typed decisions", "everruns models decision-default get")]),
    path = "/v1/models/decision-default",
    policy = LLM_MODEL_VIEW,
)]
impl Command for GetDefaultDecisionModel {
    type Output = Option<ModelWithProvider>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let id = ctx.db.get_decision_default(ctx.caller.org_id).await?;
        let models = q::service(ctx).list_all(&ctx.caller).await?;
        Ok(id.and_then(|id| models.into_iter().find(|m| m.id.uuid() == id)))
    }
}

/// Select or clear the organization decision default.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SetDefaultDecisionModel {
    /// Prefixed saved model ID; omit or pass null to clear the default.
    pub model_id: Option<String>,
}

#[command(
    name = "set_default_decision_model",
    category = "models",
    description = "Set the selected decision model.",
    method = "PUT",
    cli = CliRoute::new(&["models", "decision-default"], "set").with_examples(&[CliExample::new("Choose the model that handles typed decisions", "everruns models decision-default set --model-id model_01h9 --reason 'Cheaper model is accurate enough'")]),
    path = "/v1/models/decision-default",
    policy = LLM_MODEL_MANAGE,
)]
impl Command for SetDefaultDecisionModel {
    type Output = Option<ModelWithProvider>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let model = match self.model_id {
            None => None,
            Some(id) => {
                let models = q::service(ctx).list_all(&ctx.caller).await?;
                let model = models
                    .into_iter()
                    .find(|m| m.id.to_string() == id)
                    .ok_or_else(|| CommandError::bad_request("Decision model not found"))?;
                if model.service != everruns_contracts::ServiceKind::Decisions
                    || !model.enabled
                    || !model.healthy
                    || model
                        .profile
                        .as_ref()
                        .and_then(|p| p.decisions.as_ref())
                        .is_none_or(|profile| {
                            !profile.calibrated
                                || ["noul", "choice", "score"]
                                    .iter()
                                    .any(|kind| !profile.primitives.iter().any(|p| p == kind))
                        })
                {
                    return Err(CommandError::bad_request(
                        "Enabled, healthy decision model required",
                    ));
                }
                Some(model)
            }
        };
        ctx.db
            .set_decision_default(ctx.caller.org_id, model.as_ref().map(|m| m.id.uuid()))
            .await?;
        Ok(model)
    }
}
