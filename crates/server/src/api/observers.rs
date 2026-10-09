// Observer API routes, served by the generic `#[command(http = ..)]` handler — online scoring of production sessions.
// See knowledge/evaluation/online-evals.md. Gated behind the `observers` feature flag.

use crate::api::state::ApiState;
pub use crate::domains::observers::types::{
    CreateObserverRequest, ListObserversQuery, ListTraceScoresQuery, UpdateObserverRequest,
};
use axum::Router;

use crate::api::command_http::CommandRouterExt;
use crate::domains::observers::{
    CreateObserver, DeleteObserver, GetObserver, ListObserverScores, ListObservers, UpdateObserver,
};

// ============================================
// Routes
// ============================================

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .command::<CreateObserver>()
        .command::<ListObservers>()
        .command::<GetObserver>()
        .command::<UpdateObserver>()
        .command::<DeleteObserver>()
        .command::<ListObserverScores>()
        .with_state(state)
}
