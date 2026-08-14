use super::queries as q;
use super::{LLM_MODEL_MANAGE, LLM_MODEL_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    Policy, everruns_provider::model::Model, everruns_provider::model::ModelSource,
    everruns_provider::model::ModelWithProvider, everruns_provider::typed_id::ProviderId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize)]
pub struct DeleteModelResult {
    pub deleted: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateModel {
    /// LLM provider's prefixed public identifier.
    pub provider_id: String,
    /// LLM model's prefixed public identifier.
    pub model_id: String,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: bool,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub is_favorite: bool,
}

impl Command for CreateModel {
    type Output = Model;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_model",
            category: "models",
            description: "Create a new model for a provider.",
            method: "POST",
            path: "/v1/providers/{provider_id}/models",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Model, CommandError> {
        let provider_id = q::parse_provider_id(&self.provider_id)?;
        q::service(ctx)
            .create(
                &ctx.caller,
                provider_id,
                crate::api::models::CreateModelRequest {
                    model_id: self.model_id,
                    display_name: self.display_name,
                    capabilities: self.capabilities,
                    enabled: self.enabled,
                    is_favorite: self.is_favorite,
                },
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateModel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListProviderModels {
    /// LLM provider's prefixed public identifier.
    pub provider_id: String,
}

impl Command for ListProviderModels {
    type Output = Vec<Model>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_provider_models",
            category: "models",
            description: "List models for a specific provider.",
            method: "GET",
            path: "/v1/providers/{provider_id}/models",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Model>, CommandError> {
        let provider_id = q::parse_provider_id(&self.provider_id)?;
        q::service(ctx)
            .list_for_provider(&ctx.caller, provider_id)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListProviderModels>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListModels {
    pub source: Option<ModelSource>,
    #[serde(
        default = "default_true",
        deserialize_with = "deserialize_bool_lenient"
    )]
    pub include_stale: bool,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub favorites_only: bool,
}

const fn default_true() -> bool {
    true
}

impl Command for ListModels {
    type Output = Vec<ModelWithProvider>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_models",
            category: "models",
            description: "List all models across all providers.",
            method: "GET",
            path: "/v1/models",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<ModelWithProvider>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<ModelWithProvider>, CommandError> {
        q::service(ctx)
            .list_all_with_filters(
                &ctx.caller,
                self.source,
                self.include_stale,
                self.favorites_only,
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListModels>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for GetModel {
    type Output = ModelWithProvider;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_model",
            category: "models",
            description: "Get a specific model with provider information.",
            method: "GET",
            path: "/v1/models/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ModelWithProvider, CommandError> {
        let model_id = q::parse_model_id(&self.id)?;
        q::service(ctx)
            .get_with_provider(&ctx.caller, model_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Model"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetModel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// LLM provider's prefixed public identifier.
    pub provider_id: Option<String>,
    /// LLM model's prefixed public identifier.
    pub model_id: Option<String>,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub is_favorite: Option<bool>,
}

impl Command for UpdateModel {
    type Output = Model;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_model",
            category: "models",
            description: "Update a model.",
            method: "PATCH",
            path: "/v1/models/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_MANAGE)
    }

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
                crate::api::models::UpdateModelRequest {
                    provider_id: provider_id.map(|id| ProviderId::from_uuid(id).to_string()),
                    model_id: self.model_id,
                    display_name: self.display_name,
                    capabilities: self.capabilities,
                    enabled: self.enabled,
                    is_favorite: self.is_favorite,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Model"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateModel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DeleteModel {
    type Output = DeleteModelResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_model",
            category: "models",
            description: "Delete a model.",
            method: "DELETE",
            path: "/v1/models/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&LLM_MODEL_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeleteModelResult, CommandError> {
        let model_id = q::parse_model_id(&self.id)?;
        let deleted = q::service(ctx)
            .delete(&ctx.caller, model_id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Model"));
        }
        Ok(DeleteModelResult { deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteModel>() }

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
