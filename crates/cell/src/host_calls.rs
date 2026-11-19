//! The cell's side of host calls (`everruns_contracts::host_calls`): the
//! one path from a tool on an executor to the control plane at call time.
//!
//! Decisions:
//! - An executor never calls the control plane. It sends a host call to the
//!   session's directory cell (`/cell/orgs/{org}/host-calls/{name}`), which
//!   passes it on over the link
//!   (`POST /dp/v1/tenants/{org}/host-calls/{name}`). The answer comes back
//!   sealed with the org's data key and goes to the executor unopened.
//! - Only a declared call goes; anything else is refused here.
//! - The cell stamps what it verified: a call that acts as a person gets
//!   `acting_user` from the session's own record of who sent the turn's
//!   input message (`senders.rs`), whatever the executor put there
//!   (THREAT[TM-AGENT-017]).
//! - Each org has at most [`IN_FLIGHT`] calls in flight on a node, so one
//!   org's burst cannot take the link from the others.
//! - After [`TRIP_AFTER`] calls in a row the control plane did not answer,
//!   calls fail at once for [`OPEN_FOR`] instead of piling onto a control
//!   plane that is struggling. Its refusals (an answer) do not count.
//! - Each call is counted by name and outcome, with its round trip
//!   (`cell_host_calls_total`, `cell_host_call_duration_seconds`).
//! - THREAT[TM-TENANT]: the control plane runs the call as the path org and
//!   checks that the session is that org's.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use everruns_contracts::dataplane::SealedSecret;
use everruns_contracts::host_calls::{ActsAs, HostCall, HostCallRequest, OPEN_TURN, by_name};
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::node::Node;

type ApiError = (StatusCode, String);

/// Host calls one org may have in flight on a node.
pub const IN_FLIGHT: usize = 64;
/// Unanswered calls in a row that open the breaker.
pub const TRIP_AFTER: u32 = 5;
/// How long an open breaker fails calls at once.
pub const OPEN_FOR: Duration = Duration::from_secs(10);

/// The node's limits on host calls.
#[derive(Default)]
pub struct Limits {
    orgs: Mutex<HashMap<String, Arc<Semaphore>>>,
    unanswered: AtomicU32,
    open_until: Mutex<Option<Instant>>,
}

impl Limits {
    fn org(&self, org: &str) -> Arc<Semaphore> {
        self.orgs
            .lock()
            .entry(org.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(IN_FLIGHT)))
            .clone()
    }

    /// Whether calls fail at once now.
    fn open(&self, now: Instant) -> bool {
        self.open_until.lock().is_some_and(|until| now < until)
    }

    /// The control plane answered (or refused): the breaker closes.
    fn answered(&self) {
        self.unanswered.store(0, Ordering::SeqCst);
        *self.open_until.lock() = None;
    }

    /// The control plane did not answer.
    fn unanswered(&self, now: Instant) {
        let n = self.unanswered.fetch_add(1, Ordering::SeqCst) + 1;
        if n >= TRIP_AFTER {
            *self.open_until.lock() = Some(now + OPEN_FOR);
        }
    }
}

/// `POST /cell/orgs/{org}/host-calls/{name}`: an executor's host call for
/// `org`, passed on to the control plane.
pub(crate) async fn cell_host_call(
    State(node): State<Arc<Node>>,
    Path((org, name)): Path<(String, String)>,
    Json(mut request): Json<HostCallRequest>,
) -> Result<Json<SealedSecret>, ApiError> {
    let call = by_name(&name).ok_or((StatusCode::NOT_FOUND, format!("no host call {name}")))?;
    let started = Instant::now();
    let outcome = pass_on(&node, &org, call, &mut request).await;
    let label = match &outcome {
        Ok(_) => "ok",
        Err((StatusCode::TOO_MANY_REQUESTS, _)) => "limited",
        Err((StatusCode::SERVICE_UNAVAILABLE, _)) => "unavailable",
        Err((StatusCode::BAD_REQUEST, _)) => "bad_request",
        Err(_) => "failed",
    };
    node.prom
        .host_calls
        .inc(&[("call", call.name), ("outcome", label)]);
    node.prom
        .host_call_seconds
        .observe(&[("call", call.name)], started.elapsed());
    outcome.map(Json)
}

async fn pass_on(
    node: &Node,
    org: &str,
    call: &HostCall,
    request: &mut HostCallRequest,
) -> Result<SealedSecret, ApiError> {
    let cp = node.cfg.control_plane.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "this cell has no control plane".to_owned(),
    ))?;
    let limits = &node.host_call_limits;
    if limits.open(Instant::now()) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "the control plane is not answering host calls".to_owned(),
        ));
    }
    let org_limit = limits.org(org);
    let _slot = org_limit.try_acquire().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            format!("too many host calls in flight for org {org}"),
        )
    })?;
    if call.acts_as == ActsAs::Person {
        act_as_sender(node, &mut request.params).await?;
    }
    let sent = cp
        .post(
            &node.http,
            &format!("/dp/v1/tenants/{org}/host-calls/{}", call.name),
        )
        .json(&*request)
        .timeout(call.timeout())
        .send()
        .await;
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => {
            limits.unanswered(Instant::now());
            tracing::warn!(%org, call = call.name, %error, "a host call did not reach the control plane");
            return Err((
                StatusCode::BAD_GATEWAY,
                "the control plane did not answer the host call".to_owned(),
            ));
        }
    };
    let status = reply.status();
    if status.is_server_error() {
        limits.unanswered(Instant::now());
    } else {
        limits.answered();
    }
    if !status.is_success() {
        tracing::warn!(%org, %status, call = call.name, "the control plane refused a host call");
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("the control plane refused the host call ({status})"),
        ));
    }
    reply.json().await.map_err(|error| {
        tracing::warn!(%org, call = call.name, %error, "a host call's answer did not parse");
        (
            StatusCode::BAD_GATEWAY,
            "the control plane's answer did not parse".to_owned(),
        )
    })
}

/// Set a call's `acting_user` to the person who sent its input message,
/// from the session's own record (`senders.rs`), whatever the executor put
/// there (THREAT[TM-AGENT-017]): `null` when no one did, or when the call
/// names no input message (a lookup outside a turn), which the control
/// plane treats as acting as no one.
///
/// An input message of [`OPEN_TURN`] (an MCP server's credential, asked
/// for by session) is replaced with the input of the turn the session has
/// open, as a bare UUID; with no turn open the call names none.
pub(crate) async fn act_as_sender(
    node: &Node,
    params: &mut serde_json::Value,
) -> Result<(), ApiError> {
    params["acting_user"] = serde_json::Value::Null;
    let Some(message) = params["input_message_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    else {
        return Ok(());
    };
    let bad = |what: &str| {
        (
            StatusCode::BAD_REQUEST,
            format!("a call acting as a person needs {what}"),
        )
    };
    let message = if message == OPEN_TURN {
        None
    } else {
        Some(input_message(&message).ok_or_else(|| bad("a valid input message"))?)
    };
    let session = params["session_id"]
        .as_str()
        .and_then(|id| everruns_contracts::typed_id::SessionId::parse(id).ok())
        .ok_or_else(|| bad("its session"))?;
    let gateway = node.gateway.get().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "this cell has no gateway".to_owned(),
    ))?;
    let found = gateway
        .session_command(
            session,
            "sender",
            &serde_json::json!({ "message": message }),
        )
        .await?;
    if message.is_none() {
        params["input_message_id"] = match found["message"]
            .as_str()
            .and_then(|id| everruns_contracts::typed_id::MessageId::parse(id).ok())
        {
            Some(open) => serde_json::json!(open.uuid()),
            None => serde_json::Value::Null,
        };
    }
    params["acting_user"] = found["user"].clone();
    Ok(())
}

/// An input message id as a call names it: prefixed (`msg_...`, the
/// platform tools) or a bare UUID (the connection resolver).
fn input_message(id: &str) -> Option<everruns_contracts::typed_id::MessageId> {
    use everruns_contracts::typed_id::MessageId;
    MessageId::parse(id)
        .ok()
        .or_else(|| uuid::Uuid::parse_str(id).ok().map(MessageId::from_uuid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breaker_opens_after_unanswered_calls_and_an_answer_closes_it() {
        let limits = Limits::default();
        let now = Instant::now();
        for _ in 0..TRIP_AFTER - 1 {
            limits.unanswered(now);
        }
        assert!(!limits.open(now));
        limits.unanswered(now);
        assert!(limits.open(now));
        assert!(!limits.open(now + OPEN_FOR));
        limits.answered();
        assert!(!limits.open(now));
    }

    #[test]
    fn an_input_message_is_prefixed_or_a_bare_uuid() {
        let id = everruns_contracts::typed_id::MessageId::new();
        assert_eq!(input_message(&id.to_string()), Some(id));
        assert_eq!(input_message(&id.uuid().to_string()), Some(id));
        assert_eq!(input_message("sess_nope"), None);
    }

    #[test]
    fn each_org_has_its_own_slots() {
        let limits = Limits::default();
        let a = limits.org("org_a");
        let held: Vec<_> = (0..IN_FLIGHT)
            .map(|_| a.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(a.try_acquire().is_err());
        assert!(limits.org("org_b").try_acquire().is_ok());
        drop(held);
        assert!(limits.org("org_a").try_acquire().is_ok());
    }
}
