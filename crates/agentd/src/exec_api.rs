//! The API a cell node serves to executors outside the cell (data plane v2
//! step 6). The wire types and the reasons are in `remote.rs`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};

use crate::node::Node;
use crate::remote::{
    ExecPropose, ExecProposed, PullReply, PullRequest, SessionDelta, ViewRequest, delta,
};

type ApiError = (StatusCode, String);

/// The longest a pull or a watch may hold its request.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// How long a pull that got some sessions waits for more before replying.
const LINGER: Duration = Duration::from_millis(2);

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/exec/propose", post(propose))
        .route("/cell/exec/view", post(view))
        .route("/cell/exec/pull", post(pull))
        .with_state(node)
}

fn unavailable(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::SERVICE_UNAVAILABLE, error.to_string())
}

/// The session as this replica holds it once it applied `index`.
async fn delta_at(
    node: &Node,
    session: everruns_contracts::typed_id::SessionId,
    since: Option<i32>,
    index: u64,
) -> Result<SessionDelta, ApiError> {
    tokio::time::timeout(Duration::from_secs(10), node.shared.wait_applied(index))
        .await
        .map_err(|_| unavailable("replica did not catch up"))?;
    let state = node.shared.state.read();
    let applied = *node.shared.applied.borrow();
    Ok(delta(&state, applied, session, since))
}

async fn propose(
    State(node): State<Arc<Node>>,
    Json(body): Json<ExecPropose>,
) -> Result<Json<ExecProposed>, ApiError> {
    let (response, index) = node.propose(body.command).await.map_err(unavailable)?;
    let delta = delta_at(&node, body.session, body.since, index).await?;
    Ok(Json(ExecProposed {
        response,
        index,
        delta,
        leader: node.leader_addr(),
    }))
}

async fn view(
    State(node): State<Arc<Node>>,
    Json(body): Json<ViewRequest>,
) -> Result<Json<SessionDelta>, ApiError> {
    if body.wait_ms > 0 {
        let mut changes = node.shared.watch(body.session);
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(body.wait_ms).min(MAX_WAIT);
        while node.shared.changed_at(body.session) <= body.changed_after {
            match tokio::time::timeout_at(deadline, changes.recv()).await {
                Err(_) => break,
                Ok(Ok(_)) => {}
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            }
        }
    }
    let index = body.min_index.max(node.shared.changed_at(body.session));
    Ok(Json(
        delta_at(&node, body.session, body.since, index).await?,
    ))
}

/// Hold an executor's credits until this leader has sessions for them, or
/// the wait runs out (the credits then go back with an empty reply).
async fn pull(
    State(node): State<Arc<Node>>,
    Json(body): Json<PullRequest>,
) -> Result<Json<PullReply>, ApiError> {
    if !node.is_leader() {
        return Ok(Json(PullReply {
            leader: node.leader_addr(),
            ..PullReply::default()
        }));
    }
    if node.cfg.executors == crate::node::ExecutorMode::InNode || body.credits == 0 {
        return Ok(Json(PullReply::default()));
    }
    let pull = Pull {
        ticket: node.dispatch.register(body.executor, body.credits),
        node: node.clone(),
    };
    let ticket = &pull.ticket;
    node.offer_waiting();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(body.wait_ms).min(MAX_WAIT);
    let mut lingering = false;
    while !node.dispatch.full(ticket) {
        let wait = if lingering {
            tokio::time::Instant::now() + LINGER
        } else {
            deadline
        };
        if tokio::time::timeout_at(wait.min(deadline), ticket.notify.notified())
            .await
            .is_err()
        {
            if lingering || tokio::time::Instant::now() >= deadline {
                break;
            }
            continue;
        }
        if !node.is_leader() {
            break;
        }
        lingering = true;
    }
    let sessions = node.dispatch.take(ticket);
    let tenants = {
        let state = node.shared.state.read();
        sessions
            .iter()
            .map(|s| state.sessions.get(s).and_then(|s| s.tenant.clone()))
            .collect()
    };
    Ok(Json(PullReply {
        sessions,
        leader: None,
        tenants,
    }))
}

/// A registered pull. Dropped before its reply was taken (axum drops the
/// handler when the executor's connection goes away, e.g. the executor
/// died), it withdraws the pull; once taken, withdrawing is a no-op.
struct Pull {
    node: Arc<Node>,
    ticket: crate::dispatch::Ticket,
}

impl Drop for Pull {
    fn drop(&mut self) {
        self.node.dispatch.abandon(&self.ticket);
    }
}
