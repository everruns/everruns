// Feature flags API endpoint
//
// Decision: GET /v1/feature-flags is public (no auth required).
//   Feature flags are non-sensitive system config needed before auth completes.
// Decision: Flags computed once at startup and served from memory (no DB query).

use axum::{Json, Router, extract::State, routing::get};
use everruns_core::FeatureFlags;

#[derive(Clone)]
pub struct AppState {
    pub flags: FeatureFlags,
}

pub fn routes(state: AppState) -> Router {
    Router::new().route(
        "/v1/feature-flags",
        get(get_feature_flags).with_state(state),
    )
}

async fn get_feature_flags(State(state): State<AppState>) -> Json<FeatureFlags> {
    Json(state.flags.clone())
}
