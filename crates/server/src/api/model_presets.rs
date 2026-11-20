// Model preset API routes, served by the generic `#[command(http = ..)]` handler.
// See knowledge/foundations/model-presets.md.

use crate::api::command_http::CommandRouterExt;
use crate::api::state::ApiState;
use crate::domains::model_presets::{
    ApplyModelPreset, CreateModelPreset, DeleteModelPreset, GetModelPreset, ListModelPresets,
    UpdateModelPreset,
};
use axum::Router;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .command::<ListModelPresets>()
        .command::<CreateModelPreset>()
        .command::<GetModelPreset>()
        .command::<UpdateModelPreset>()
        .command::<DeleteModelPreset>()
        .command::<ApplyModelPreset>()
        .with_state(state)
}
