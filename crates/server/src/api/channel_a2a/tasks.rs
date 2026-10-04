// `ListTasks` and `SubscribeToTask` (A2A 1.0 §3.1.4, §3.1.6; 0.3 `tasks/list`,
// `tasks/resubscribe`).
//
// THREAT[TM-A2A-012]: both read sessions, so both are fenced to the calling
// channel exactly like `tasks/get`. `ListTasks` filters on the channel's own
// routing tags (`app:` + `app_channel:`), the same pair
// `session_belongs_to_a2a_channel` checks, so a key never lists another
// channel's tasks; a `contextId` from elsewhere lists nothing.
//
// Design Decisions:
// - A task is a session (task id = context id = session id), so listing is a
//   keyset page over the channel's sessions ordered by `updated_at`, the
//   session's last status change. The page token is opaque base64 of the last
//   row's `(updated_at, id)`, which the spec's cursor pagination asks for.
// - The `status` filter runs in SQL on the session's derived activity, which
//   is coarser than the task state (it cannot tell `failed` from `canceled`,
//   and `submitted` or `working` can sit in either `idle` or `running` while a
//   turn is queued), so each state maps to every activity that can hold it.
//   The page is then filtered on the exact projected state, so a page can be
//   shorter than `pageSize` while `totalSize` counts the coarser match.
// - Subscribing to a terminal task is UnsupportedOperation, as the spec says.
//   The subscription is opened before the task is read, so no event between
//   the read and the first frame is lost; the first frame is the full task.

use crate::records::SessionActivity;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::wire::{self, WireVersion};
use super::{
    AuthorizedA2a, ChannelA2aState, JsonRpcRequest, RpcRejection, bound_task_session,
    internal_error, rpc_error, rpc_success, session_belongs_to_a2a_channel, stream, task_view,
};
use crate::api::common::ErrorResponse;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;
const TERMINAL_STATES: [&str; 4] = ["completed", "failed", "canceled", "rejected"];

pub(super) fn is_terminal(state: &str) -> bool {
    TERMINAL_STATES.contains(&state)
}

/// The parsed `ListTasksRequest`.
#[derive(Debug, PartialEq)]
struct ListQuery {
    context_id: Option<String>,
    /// Exact 0.3 state label to keep, and the coarse activities to query.
    status: Option<(&'static str, &'static [SessionActivity])>,
    page_size: u32,
    after: Option<(DateTime<Utc>, Uuid)>,
    updated_after: Option<DateTime<Utc>>,
    include_artifacts: bool,
}

fn parse_list_query(params: &Value) -> Result<ListQuery, &'static str> {
    let text = |name: &str| {
        params
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let status = match text("status") {
        None | Some("TASK_STATE_UNSPECIFIED") => None,
        Some(raw) => Some(state_filter(raw).ok_or("Invalid params: unknown `status`")?),
    };
    let page_size = match params.get("pageSize") {
        None | Some(Value::Null) => DEFAULT_PAGE_SIZE,
        Some(value) => value
            .as_u64()
            .filter(|size| (1..=u64::from(MAX_PAGE_SIZE)).contains(size))
            .ok_or("Invalid params: `pageSize` must be between 1 and 100")?
            as u32,
    };
    let after = match text("pageToken") {
        None => None,
        Some(token) => Some(decode_page_token(token).ok_or("Invalid params: bad `pageToken`")?),
    };
    let updated_after = match text("statusTimestampAfter") {
        None => None,
        Some(raw) => Some(
            DateTime::parse_from_rfc3339(raw)
                .map_err(|_| "Invalid params: `statusTimestampAfter` must be RFC 3339")?
                .with_timezone(&Utc),
        ),
    };
    Ok(ListQuery {
        context_id: text("contextId").map(str::to_owned),
        status,
        page_size,
        after,
        updated_after,
        include_artifacts: params
            .get("includeArtifacts")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// A requested state (1.0 or 0.3 spelling) as the exact label tasks carry and
/// the session activities that can hold it.
fn state_filter(raw: &str) -> Option<(&'static str, &'static [SessionActivity])> {
    let label = match raw {
        "TASK_STATE_SUBMITTED" | "submitted" => "submitted",
        "TASK_STATE_WORKING" | "working" => "working",
        "TASK_STATE_COMPLETED" | "completed" => "completed",
        "TASK_STATE_FAILED" | "failed" => "failed",
        "TASK_STATE_CANCELED" | "canceled" => "canceled",
        "TASK_STATE_INPUT_REQUIRED" | "input-required" => "input-required",
        "TASK_STATE_AUTH_REQUIRED" | "auth-required" => "auth-required",
        _ => return None,
    };
    use SessionActivity::{Completed, Failed, Idle, Paused, Running};
    let activities: &'static [SessionActivity] = match label {
        "completed" => &[Completed],
        "failed" | "canceled" => &[Failed],
        "submitted" | "working" => &[Idle, Running],
        // A parked `ask_user` call holds the session in a running status.
        _ => &[Idle, Running, Paused],
    };
    Some((label, activities))
}

fn encode_page_token(updated_at: DateTime<Utc>, id: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(format!("{}|{id}", updated_at.to_rfc3339()))
}

fn decode_page_token(token: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let raw = String::from_utf8(URL_SAFE_NO_PAD.decode(token).ok()?).ok()?;
    let (at, id) = raw.split_once('|')?;
    Some((
        DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc),
        id.parse().ok()?,
    ))
}

pub(super) async fn handle_list_tasks(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    let query = match parse_list_query(&parsed.params) {
        Ok(query) => query,
        Err(msg) => return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response(),
    };
    let page = match list_page(state, &auth, &query).await {
        Ok(page) => page,
        Err(err) => return internal_error(err).into_response(),
    };
    let tasks: Vec<Value> = page
        .tasks
        .into_iter()
        .map(|task| wire::task(version, task))
        .collect();
    let result = json!({
        "tasks": tasks,
        "nextPageToken": page.next_page_token,
        "pageSize": query.page_size,
        "totalSize": page.total,
    });
    (StatusCode::OK, rpc_success(rpc_id, result)).into_response()
}

struct TaskPage {
    tasks: Vec<Value>,
    next_page_token: String,
    total: u32,
}

async fn list_page(
    state: &ChannelA2aState,
    auth: &AuthorizedA2a,
    query: &ListQuery,
) -> anyhow::Result<TaskPage> {
    let (sessions, total, last) = match &query.context_id {
        // A context is one session, so the filter is a lookup.
        Some(context_id) => {
            let session = match context_id.parse() {
                Ok(id) => state.db.get_session(auth.org_id, id).await?,
                Err(_) => None,
            }
            .filter(|session| session_belongs_to_a2a_channel(session, auth))
            .filter(|session| {
                query
                    .updated_after
                    .is_none_or(|after| session.updated_at >= after)
            });
            let total = u32::from(session.is_some());
            (session.into_iter().collect::<Vec<_>>(), total, None)
        }
        None => {
            let tags = [
                format!("app:{}", auth.app_public_id),
                format!("app_channel:{}", auth.channel_public_id),
            ];
            let activities = query.status.map_or(&[][..], |(_, activities)| activities);
            let (rows, total) = state
                .db
                .list_sessions_by_tags(
                    auth.org_id,
                    &tags,
                    activities,
                    query.updated_after,
                    query.after,
                    query.page_size,
                )
                .await?;
            let last = (rows.len() == query.page_size as usize)
                .then(|| rows.last().map(|row| (row.updated_at, row.id.uuid())))
                .flatten();
            (rows, total, last)
        }
    };

    let mut tasks = Vec::with_capacity(sessions.len());
    for session in &sessions {
        let mut task = task_view::load_task(state, auth, session).await?;
        let label = task["status"]["state"].as_str().unwrap_or_default();
        if query.status.is_some_and(|(wanted, _)| wanted != label) {
            continue;
        }
        if !query.include_artifacts
            && let Some(task) = task.as_object_mut()
        {
            task.remove("artifacts");
        }
        tasks.push(task);
    }
    Ok(TaskPage {
        tasks,
        next_page_token: last
            .map(|(at, id)| encode_page_token(at, id))
            .unwrap_or_default(),
        total,
    })
}

/// `SubscribeToTask` / `tasks/resubscribe`: the task, then its live updates
/// until it settles.
pub(super) async fn handle_subscribe(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    let session = match bound_task_session(state, &auth, &parsed.params).await {
        Ok(session) => session,
        Err(rejection) => return rejection.into_response_with(rpc_id),
    };
    let subscription = match state.event_delivery.subscribe(session.id.uuid()).await {
        Ok(subscription) => subscription,
        Err(err) => return internal_error(err).into_response(),
    };
    let task = match task_view::load_task(state, &auth, &session).await {
        Ok(task) => task,
        Err(err) => return internal_error(err).into_response(),
    };
    if task["status"]["state"].as_str().is_some_and(is_terminal) {
        return RpcRejection(-32004, "The task is in a terminal state").into_response_with(rpc_id);
    }
    let sse_guard = match state
        .sse_tracker
        .try_acquire(auth.org_id, session.id.uuid())
    {
        Ok(guard) => guard,
        Err(rejection) => {
            return ErrorResponse::new(rejection.report("a2a", auth.org_id, &session.id.uuid()))
                .into_response(StatusCode::TOO_MANY_REQUESTS)
                .into_response();
        }
    };
    let task_id = session.id.to_string();
    stream::sse_response(
        stream::StreamContext {
            subscription,
            rpc_id,
            context_id: task_id.clone(),
            task_id,
            session_id: session.id.uuid(),
            frontend_url: state.frontend_url.clone(),
            version,
            initial_task: task,
        },
        sse_guard,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_query_defaults() {
        let query = parse_list_query(&json!({})).unwrap();
        assert_eq!(query.page_size, 50);
        assert_eq!(query.status, None);
        assert!(!query.include_artifacts);
        assert!(query.after.is_none());
    }

    #[test]
    fn list_query_reads_both_state_spellings() {
        for raw in ["TASK_STATE_CANCELED", "canceled"] {
            let query = parse_list_query(&json!({ "status": raw })).unwrap();
            assert_eq!(
                query.status,
                Some(("canceled", &[SessionActivity::Failed][..]))
            );
        }
        assert!(parse_list_query(&json!({ "status": "TASK_STATE_BOGUS" })).is_err());
    }

    #[test]
    fn list_query_bounds_page_size() {
        assert!(parse_list_query(&json!({ "pageSize": 0 })).is_err());
        assert!(parse_list_query(&json!({ "pageSize": 101 })).is_err());
        assert_eq!(
            parse_list_query(&json!({ "pageSize": 100 }))
                .unwrap()
                .page_size,
            100
        );
    }

    #[test]
    fn page_token_round_trips_and_rejects_garbage() {
        let at = DateTime::parse_from_rfc3339("2026-10-02T10:00:00.123456Z")
            .unwrap()
            .with_timezone(&Utc);
        let id = Uuid::now_v7();
        let token = encode_page_token(at, id);
        assert_eq!(decode_page_token(&token), Some((at, id)));
        assert!(parse_list_query(&json!({ "pageToken": "not-a-token" })).is_err());
    }
}
