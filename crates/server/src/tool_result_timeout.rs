// Time out turns parked in `waiting_for_tool_results`.
//
// Decision: each park arms one delayed durable task (`arm_parked_turn`, called
// where the worker's status write lands) that becomes claimable at the park's
// earliest deadline: an `ask_user` call's own `expires_at`, a tool-approval
// batch's, or the generic timeout. One server replica's job pool claims it
// (`crate::cluster_jobs`), so the deadline fires once, on time, without a scan.
// The task carries the `tool.call_requested` event it was armed for; a later
// park arms its own, and a task whose park has ended or moved on does nothing.
// Decision: tasks are standalone, not on the turn's workflow: a parked turn's
// workflow is finished, and a deadline must not add to its event log.
// Decision: a backstop sweep still runs once per cluster
// (TOOL_RESULT_TIMEOUT_SWEEP_INTERVAL_SECS, default 60s) for parks that armed
// nothing: written before this change, by a path without a durable store, or a
// lost enqueue. It also recovers expired `resolving_tool_results` leases.
// Decision: timeout is 5 minutes per knowledge/execution/client-side-tools.md, configurable via env var.

use crate::services::EventService;
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::StorageBackend;
use crate::storage::models::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{EventId, MessageId, SessionId, TurnId};
use everruns_core::builtins::ask_user::{AskUserAnsweredBy, AskUserStatus};
use everruns_core::events::{
    EventContext, EventData, EventRequest, ToolCompletedData, deserialize_event_data,
};
use everruns_core::host::TurnBackend;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Default timeout for waiting_for_tool_results sessions (5 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// How often the backstop sweep runs by default (60 seconds).
const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 60;

/// How soon a deadline task looks again when its resolution did not end the
/// park (a transient failure), so a stuck park is retried without a scan.
const RETRY_AFTER_SECS: i64 = 30;

/// Activity type of the per-park deadline task.
pub const TOOL_RESULT_DEADLINE_ACTIVITY: &str = "tool_result_deadline";
/// Activity type of the backstop sweep's schedule.
pub const TOOL_RESULT_TIMEOUT_SWEEP_ACTIVITY: &str = "tool_result_timeout_sweep";
const TOOL_RESULT_TIMEOUT_SWEEP_SCHEDULE: &str = "tool-result-timeout-sweep";

fn timeout_secs_from_env() -> u64 {
    std::env::var("TOOL_RESULT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

fn sweep_interval_secs_from_env() -> u64 {
    std::env::var("TOOL_RESULT_TIMEOUT_SWEEP_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|secs: &u64| *secs > 0)
        .unwrap_or(DEFAULT_SWEEP_INTERVAL_SECS)
}

/// What the server wires in: the per-park deadline task and the backstop
/// sweep, both over the background pool.
pub struct ToolResultTimeouts {
    db: Arc<StorageBackend>,
    runner: Arc<dyn TurnBackend>,
    event_service: Arc<EventService>,
    event_delivery: crate::event_delivery::EventDelivery,
    timeout_secs: u64,
}

impl ToolResultTimeouts {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn TurnBackend>,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            event_service: Arc::new(EventService::new(db.clone(), event_delivery.clone())),
            db,
            runner,
            event_delivery,
            timeout_secs: timeout_secs_from_env(),
        }
    }

    /// The backstop sweep as a cluster-once job.
    pub fn backstop_job(&self) -> crate::cluster_jobs::ClusterJob {
        let (db, runner, event_service) = (
            self.db.clone(),
            self.runner.clone(),
            self.event_service.clone(),
        );
        let timeout_secs = self.timeout_secs;
        crate::cluster_jobs::ClusterJob::every(
            TOOL_RESULT_TIMEOUT_SWEEP_SCHEDULE,
            TOOL_RESULT_TIMEOUT_SWEEP_ACTIVITY,
            "Time out parked turns that armed no deadline task",
            std::time::Duration::from_secs(sweep_interval_secs_from_env()),
            move || {
                let (db, runner, event_service) =
                    (db.clone(), runner.clone(), event_service.clone());
                Box::pin(async move {
                    if let Err(e) =
                        sweep_timed_out_sessions(&db, &runner, &event_service, timeout_secs).await
                    {
                        tracing::warn!(error = %e, "Tool result timeout sweep error");
                    }
                })
            },
        )
    }

    /// The deadline task's handler, served by the cluster job pool, which
    /// re-arms through `store` while the park is still waiting.
    pub fn deadline_task(
        &self,
        store: Arc<dyn everruns_durable::WorkflowEventStore + Send + Sync>,
    ) -> crate::cluster_jobs::ClusterTask {
        let (db, runner, event_service) = (
            self.db.clone(),
            self.runner.clone(),
            self.event_service.clone(),
        );
        let timeout_secs = self.timeout_secs;
        crate::cluster_jobs::ClusterTask::new(TOOL_RESULT_DEADLINE_ACTIVITY, move |input| {
            let (db, runner, event_service, store) = (
                db.clone(),
                runner.clone(),
                event_service.clone(),
                store.clone(),
            );
            Box::pin(async move {
                let input: DeadlineInput = match serde_json::from_value(input) {
                    Ok(input) => input,
                    Err(error) => {
                        tracing::warn!(%error, "Malformed tool result deadline task");
                        return;
                    }
                };
                match settle_deadline(&db, &runner, &event_service, &input, timeout_secs).await {
                    Ok(Some(next)) => {
                        if let Err(error) = enqueue_deadline(store.as_ref(), &input, next).await {
                            tracing::warn!(session_id = %input.session_id, %error, "Failed to re-arm tool result deadline");
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(session_id = %input.session_id, %error, "Tool result deadline failed");
                    }
                }
            })
        })
    }

    /// Fallback for a server without a durable store: the backstop sweep as a
    /// local loop on this replica.
    pub fn spawn_local_sweep(self) -> JoinHandle<()> {
        spawn_tool_result_timeout_sweep(self.db, self.runner, self.event_delivery)
    }
}

fn spawn_tool_result_timeout_sweep(
    db: Arc<StorageBackend>,
    runner: Arc<dyn TurnBackend>,
    event_delivery: crate::event_delivery::EventDelivery,
) -> JoinHandle<()> {
    let timeout_secs = timeout_secs_from_env();
    let sweep_interval_secs = sweep_interval_secs_from_env();

    tokio::spawn(async move {
        tracing::info!(
            timeout_secs,
            sweep_interval_secs,
            "Tool result timeout sweep started"
        );

        let event_service = EventService::new(db.clone(), event_delivery);

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(sweep_interval_secs)).await;

            if let Err(e) =
                sweep_timed_out_sessions(&db, &runner, &event_service, timeout_secs).await
            {
                tracing::warn!(error = %e, "Tool result timeout sweep error");
            }
        }
    })
}

/// One pass of the sweep. Public so contract tests can drive a pass without
/// waiting on the background interval.
pub async fn sweep_timed_out_sessions(
    db: &Arc<StorageBackend>,
    runner: &Arc<dyn TurnBackend>,
    event_service: &EventService,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let duration = chrono::Duration::try_seconds(timeout_secs.min(i64::MAX as u64) as i64)
        .unwrap_or(chrono::Duration::seconds(DEFAULT_TIMEOUT_SECS as i64));

    // Two passes over the same small set, each with the cutoff its own rule
    // needs. An `ask_user` call carries a per-call `expires_at` that can be far
    // shorter than the generic timeout, so it has to be considered from the
    // moment the session parks rather than only once the global cutoff passes.
    for (session_id, org_id) in db.list_sessions_waiting_tool_results_before(now).await? {
        if let Err(e) =
            resolve_expired_question(db, event_service, runner, session_id, org_id).await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to resolve an expired ask_user call"
            );
        }
        if let Err(e) =
            resolve_expired_approvals(db, event_service, runner, session_id, org_id).await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to resolve an expired tool approval request"
            );
        }
    }

    let timed_out = db
        .list_sessions_waiting_tool_results_before(now - duration)
        .await?;
    if timed_out.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = timed_out.len(),
        "Found timed-out waiting_for_tool_results sessions"
    );

    for (session_id, org_id) in timed_out {
        // A session parked on `ask_user` is never resolved with the generic
        // timeout error (EVE-1056). Its deadline belongs to the call, and the
        // pass above owns it; the generic payload would tell the model the
        // client went away when in fact nobody answered a question.
        if let Ok(Some(pending)) = pending_ask_user(db, session_id).await
            && pending.expires_at.is_some()
        {
            continue;
        }
        // Same for a hard tool-approval request (EVE-1140): its own deadline,
        // resolved by the pass above as not approved, never as a client that
        // went away.
        if let Ok(pending) =
            crate::api::tool_approvals::pending_tool_approvals(db, session_id).await
            && pending.iter().any(|approval| approval.expires_at.is_some())
        {
            continue;
        }
        if let Err(e) = timeout_session(db, event_service, runner, session_id, org_id).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to timeout session"
            );
        }
    }

    Ok(())
}

/// The pending `ask_user` call on a session, if that is what it is parked on.
async fn pending_ask_user(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
) -> anyhow::Result<Option<crate::api::question_answers::PendingQuestions>> {
    let requested = db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1),
        )
        .await?;
    if requested.is_empty() {
        return Ok(None);
    }
    Ok(crate::api::question_answers::pending_from_events(
        &requested, None,
    ))
}

/// Resolve a parked `ask_user` call whose own deadline has passed.
///
/// Goes through the shared resolution operation rather than writing a
/// completion event directly, so a human answering at the same instant and this
/// sweep race on one claim and the first writer wins (EVE-1054).
async fn resolve_expired_question(
    db: &Arc<StorageBackend>,
    event_service: &EventService,
    runner: &Arc<dyn TurnBackend>,
    session_id: SessionId,
    org_id: i64,
) -> anyhow::Result<()> {
    let Some(pending) = pending_ask_user(db, session_id).await? else {
        return Ok(());
    };
    let Some(expires_at) = pending.expires_at else {
        // Recorded before the server stamped deadlines. Leave it to the
        // generic timeout rather than inventing a deadline for it.
        return Ok(());
    };
    if Utc::now() < expires_at {
        return Ok(());
    }

    // Free-form text and credentials have no default worth applying, so an
    // unanswered call containing either declines rather than claiming a value
    // nobody supplied.
    let (status, answers) = if everruns_core::builtins::ask_user::questions_have_no_default_answer(
        &pending.questions,
    ) {
        (AskUserStatus::Declined, Vec::new())
    } else {
        (
            AskUserStatus::TimedOut,
            everruns_core::builtins::ask_user::declared_defaults(&pending.questions),
        )
    };

    let session_service = crate::domains::sessions::SessionService::new(db.clone());
    let resolver = crate::api::question_answers::QuestionResolver {
        db,
        session_service: &session_service,
        event_service,
        runner: runner.clone(),
    };
    let caller = everruns_core::Caller::internal(org_id);
    match crate::api::question_answers::resolve_question_answers_with_source(
        &resolver,
        &caller,
        session_id,
        Some(&pending.tool_call_id),
        status,
        AskUserAnsweredBy::Timeout,
        &answers,
    )
    .await
    {
        Ok(_) => {
            tracing::info!(
                session_id = %session_id,
                tool_call_id = %pending.tool_call_id,
                resolution = ?status,
                "Resolved an ask_user call at its deadline"
            );
            Ok(())
        }
        // A human got there first, or the turn moved on. Either way the
        // question is answered and there is nothing for the sweep to do.
        Err(
            crate::api::question_answers::ResolveError::AlreadyResolved
            | crate::api::question_answers::ResolveError::NoPendingQuestions
            | crate::api::question_answers::ResolveError::WrongPendingCall,
        ) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("{error:?}")),
    }
}

/// Resolve a parked tool-approval batch once its deadline has passed.
///
/// Fails closed: an unanswered request is not approved and nothing is recorded,
/// so a retried call asks again rather than inheriting a decision nobody made.
/// Goes through the shared resolution operation, so a person answering at the
/// same instant and this sweep race on one claim and the first writer wins.
async fn resolve_expired_approvals(
    db: &Arc<StorageBackend>,
    event_service: &EventService,
    runner: &Arc<dyn TurnBackend>,
    session_id: SessionId,
    org_id: i64,
) -> anyhow::Result<()> {
    let pending = crate::api::tool_approvals::pending_tool_approvals(db, session_id).await?;
    let now = Utc::now();
    // The batch shares one deadline in practice; resolve once the earliest
    // passes, since one resume settles all of it.
    let Some(earliest) = pending
        .iter()
        .filter_map(|approval| approval.expires_at)
        .min()
    else {
        return Ok(());
    };
    if now < earliest {
        return Ok(());
    }
    use crate::api::tool_approvals::{
        ApprovalOutcome, ApprovalResolveError, ApprovalServices, resolve_tool_approvals,
    };
    match resolve_tool_approvals(
        &ApprovalServices {
            db,
            event_service,
            runner,
        },
        org_id,
        session_id,
        &pending,
        &std::collections::HashMap::new(),
        ApprovalOutcome::Expired,
        "tool_approval_timeout",
    )
    .await
    {
        Ok(_) => Ok(()),
        // A person got there first, or the turn moved on.
        Err(
            ApprovalResolveError::AlreadyResolved
            | ApprovalResolveError::NotWaiting(_)
            | ApprovalResolveError::NotFound(_),
        ) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("{error:?}")),
    }
}

async fn timeout_session(
    db: &Arc<StorageBackend>,
    event_service: &EventService,
    runner: &Arc<dyn TurnBackend>,
    session_id: SessionId,
    org_id: i64,
) -> anyhow::Result<()> {
    let tool_call_ids = find_pending_tool_call_ids(db, session_id).await?;
    if tool_call_ids.is_empty() {
        tracing::warn!(
            session_id = %session_id,
            "No tool.call_requested event found for timed-out session"
        );
    }
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let message_id = MessageId::from_uuid(session_id.uuid());
    let events = tool_call_ids
        .into_iter()
        .map(|tool_call_id| {
            EventRequest::new(
                session_id,
                EventContext::turn(turn_id, message_id),
                ToolCompletedData::failure(
                    tool_call_id,
                    String::new(),
                    "timeout".to_string(),
                    "Timed out waiting for client tool results".to_string(),
                    None,
                ),
            )
        })
        .collect();
    let plan = WaitingTurnResolutionPlan {
        kind: "timeout".to_string(),
        events,
        session_values: Vec::new(),
        response: serde_json::Value::Null,
    };
    let claim = match db.recover_waiting_turn(org_id, session_id, plan).await? {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { .. } | ClaimWaitingTurnResult::SessionNotFound => {
            return Ok(());
        }
    };
    execute_waiting_turn_resolution(db, event_service, runner, org_id, session_id, &claim).await?;
    tracing::info!(
        session_id = %session_id,
        resolution_kind = %claim.plan.kind,
        "Workflow resumed after parked-turn recovery"
    );
    Ok(())
}

/// Find pending tool call IDs from the most recent tool.call_requested event.
async fn find_pending_tool_call_ids(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
) -> anyhow::Result<Vec<String>> {
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1), // Only need the most recent one
        )
        .await?;

    // list_events with limit returns the LAST N events, so this is the most recent
    if let Some(event) = events.last() {
        let data = deserialize_event_data(&event.event_type, event.data.clone());
        if let EventData::ToolCallRequested(req) = data {
            return Ok(req.tool_calls.iter().map(|tc| tc.id.clone()).collect());
        }
    }

    Ok(vec![])
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DeadlineInput {
    org_id: i64,
    session_id: SessionId,
    /// The `tool.call_requested` event the park waits on.
    call_event_id: EventId,
}

/// Where a parked session stands against its deadlines.
struct ParkedTurn {
    call_event_id: EventId,
    /// Earliest deadline: the per-call one when the call carries one, else
    /// the generic timeout from when the session parked.
    due: DateTime<Utc>,
}

/// The park the session is in now, `None` when it is not waiting.
async fn parked_turn(
    db: &StorageBackend,
    org_id: i64,
    session_id: SessionId,
    timeout_secs: u64,
) -> anyhow::Result<Option<ParkedTurn>> {
    let Some(session) = db.get_session(org_id, session_id).await? else {
        return Ok(None);
    };
    if session.status != "waiting_for_tool_results" {
        return Ok(None);
    }
    let requested = db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1),
        )
        .await?;
    let Some(event) = requested.last() else {
        return Ok(None);
    };
    // The same precedence as the sweep: a per-call deadline owns the park, and
    // the generic timeout applies only when no call carries one.
    let ask_user = crate::api::question_answers::pending_from_events(&requested, None)
        .and_then(|pending| pending.expires_at);
    let approval = crate::api::tool_approvals::pending_approvals_from_events(&requested)
        .iter()
        .filter_map(|approval| approval.expires_at)
        .min();
    let due = match (ask_user, approval) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => {
            session.updated_at
                + chrono::Duration::try_seconds(timeout_secs.min(i64::MAX as u64) as i64)
                    .unwrap_or(chrono::Duration::seconds(DEFAULT_TIMEOUT_SECS as i64))
        }
    };
    Ok(Some(ParkedTurn {
        call_event_id: event.id,
        due,
    }))
}

async fn enqueue_deadline<S>(
    store: &S,
    input: &DeadlineInput,
    due: DateTime<Utc>,
) -> anyhow::Result<()>
where
    S: everruns_durable::TaskQueue + ?Sized,
{
    let delay = (due - Utc::now()).to_std().unwrap_or_default();
    store
        .enqueue_task(everruns_durable::TaskDefinition {
            workflow_id: None,
            activity_id: format!(
                "{TOOL_RESULT_DEADLINE_ACTIVITY}:{}:{}",
                input.session_id, input.call_event_id
            ),
            activity_type: TOOL_RESULT_DEADLINE_ACTIVITY.to_string(),
            input: serde_json::to_value(input)?,
            options: everruns_durable::ActivityOptions::default().with_start_delay(delay),
        })
        .await?;
    Ok(())
}

/// Arm the deadline task for a turn that just parked. Called where the
/// worker's `waiting_for_tool_results` status write lands; failures are
/// logged and left to the backstop sweep, never surfaced to the turn.
pub async fn arm_parked_turn<S>(
    db: &StorageBackend,
    store: Option<&S>,
    org_id: i64,
    session_id: SessionId,
) where
    S: everruns_durable::TaskQueue + ?Sized,
{
    let Some(store) = store else {
        return;
    };
    let armed = async {
        let Some(parked) = parked_turn(db, org_id, session_id, timeout_secs_from_env()).await?
        else {
            return Ok(());
        };
        let input = DeadlineInput {
            org_id,
            session_id,
            call_event_id: parked.call_event_id,
        };
        enqueue_deadline(store, &input, parked.due).await
    };
    if let Err(error) = armed.await {
        tracing::warn!(session_id = %session_id, %error, "Failed to arm tool result deadline");
    }
}

/// Resolve the park if its deadline has passed. Returns when to look again
/// while the same park is still waiting, `None` once it has ended.
async fn settle_deadline(
    db: &Arc<StorageBackend>,
    runner: &Arc<dyn TurnBackend>,
    event_service: &EventService,
    input: &DeadlineInput,
    timeout_secs: u64,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    let (org_id, session_id) = (input.org_id, input.session_id);
    let same_park = |parked: Option<ParkedTurn>| {
        parked.filter(|parked| parked.call_event_id == input.call_event_id)
    };
    let Some(parked) = same_park(parked_turn(db, org_id, session_id, timeout_secs).await?) else {
        // The park ended, or a later park armed its own deadline.
        return Ok(None);
    };
    if Utc::now() < parked.due {
        // The session was touched after parking, which moves the generic
        // timeout; look again then.
        return Ok(Some(parked.due));
    }

    resolve_expired_question(db, event_service, runner, session_id, org_id).await?;
    resolve_expired_approvals(db, event_service, runner, session_id, org_id).await?;
    let per_call = pending_ask_user(db, session_id)
        .await?
        .is_some_and(|pending| pending.expires_at.is_some())
        || crate::api::tool_approvals::pending_tool_approvals(db, session_id)
            .await?
            .iter()
            .any(|approval| approval.expires_at.is_some());
    if !per_call {
        timeout_session(db, event_service, runner, session_id, org_id).await?;
    }

    Ok(
        same_park(parked_turn(db, org_id, session_id, timeout_secs).await?).map(|parked| {
            parked
                .due
                .max(Utc::now() + chrono::Duration::seconds(RETRY_AFTER_SECS))
        }),
    )
}
