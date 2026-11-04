// Observer API routes, served by the generic `#[command(http = ..)]` handler — online scoring of production sessions.
// See knowledge/evaluation/online-evals.md. Gated behind the `observers` feature flag.

use crate::api::state::ApiState;
pub use crate::domains::observers::types::{
    BackfillObserverRequest, CreateObserverRequest, ListObserversQuery, ListTraceScoresQuery,
    QualitySummaryQuery, ReviewScoreRequest, UpdateObserverRequest,
};
use axum::Router;

use crate::api::command_http::CommandRouterExt;
use crate::domains::observers::{
    BackfillObserver, CreateObserver, DeleteObserver, GetObserver, GetObserverSummary,
    ListObserverScores, ListObservers, ReviewObserverScore, UpdateObserver,
};

// ============================================
// Routes
// ============================================

pub fn routes(state: ApiState) -> Router {
    Router::new()
        // Static `/summary` before `/{observer_id}` for readability; the
        // router prefers static segments either way.
        .command::<GetObserverSummary>()
        .command::<CreateObserver>()
        .command::<ListObservers>()
        .command::<GetObserver>()
        .command::<UpdateObserver>()
        .command::<DeleteObserver>()
        .command::<ListObserverScores>()
        .command::<ReviewObserverScore>()
        .command::<BackfillObserver>()
        .with_state(state)
}
