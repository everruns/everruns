// Session trace API: the bounded reads behind the session Trace view.
//
// Business logic lives in `crate::domains::session_trace`; this file only
// mounts its commands on the generic command handler.
// See knowledge/ui/session-trace.md.

use crate::api::command_http::CommandRouterExt;
use crate::api::state::ApiState;
use crate::domains::session_trace::{
    GetSessionTrace, GetSessionTraceStep, ListSessionTraceRequest, ListSessionTraceSteps,
    ListSessionTraceTurnEvents, ListSessionTraceTurns,
};
use axum::Router;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .command::<GetSessionTrace>()
        .command::<ListSessionTraceTurns>()
        .command::<ListSessionTraceSteps>()
        .command::<GetSessionTraceStep>()
        .command::<ListSessionTraceRequest>()
        .command::<ListSessionTraceTurnEvents>()
        .with_state(state)
}
