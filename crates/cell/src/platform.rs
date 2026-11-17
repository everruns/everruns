//! What a session owns on a cell besides its log and workspace (data plane
//! v2 step 11.4): its schedules, and the commands of its detached tool
//! calls (`detach.rs`).
//!
//! Decisions:
//! - Schedules are part of the session's replicated state, so they move,
//!   snapshot and restore with it. Nothing about them lives outside the
//!   session's cell, and no central scheduler exists.
//! - The cell's leader fires them ([`fire_due`]). On every scheduler tick it
//!   proposes `FireSchedule` for each schedule due by now. Applying it admits
//!   the occurrence and advances the schedule in one step, and only over the
//!   time the schedule was due at, so a fire proposed twice (a retry, or an
//!   old leader and a new one) admits once.
//! - An occurrence's message id derives from the schedule, its run number and
//!   its time, as on Engine's local host, so it is the same id wherever it
//!   is computed.
//! - A recurring schedule's next time is computed from the fire time, not
//!   from the time it was due. A cell that was down does not fire a burst of
//!   missed runs when it comes back.
//! - An occurrence the session refuses (it is suspended, say) is skipped: the
//!   schedule moves on, so the leader does not propose it again every tick.
//! - Limits: five active schedules per session, as on every host, checked
//!   when the create applies, so two concurrent creates cannot both pass.
//!   The org-wide cap needs a count across cells; it comes with the
//!   control plane's tenant limits (11.6).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use everruns_contracts::runtime::session_schedule::{
    MAX_ACTIVE_SCHEDULES_PER_SESSION, SessionSchedule, parse_session_cron, stop_reason,
};
use everruns_contracts::typed_id::{EventId, MessageId, ScheduleId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::node::Node;
use crate::state::{CellState, Change, Command, Placement, Response, SessionState};

/// A session's platform state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Platform {
    /// Its schedules by id, deleted ones included (they stay for the
    /// session's record, and never fire).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schedules: BTreeMap<String, SessionSchedule>,
    /// Its tasks by id, as JSON with their raw spec (`tasks.rs`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, serde_json::Value>,
    /// Each task's message channel, oldest first.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_messages: BTreeMap<String, Vec<everruns_core::session_task::TaskMessage>>,
}

impl Platform {
    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty() && self.tasks.is_empty() && self.task_messages.is_empty()
    }

    /// Schedules due at `now_ms`, with the time each was due.
    pub fn due(&self, now_ms: i64) -> impl Iterator<Item = (ScheduleId, i64)> + '_ {
        self.schedules.values().filter_map(move |s| {
            let due = s.next_trigger_at?.timestamp_millis();
            (live(s) && due <= now_ms).then_some((s.id, due))
        })
    }

    fn active(&self) -> u32 {
        self.schedules.values().filter(|s| live(s)).count() as u32
    }
}

fn live(schedule: &SessionSchedule) -> bool {
    schedule.enabled && schedule.deleted_at.is_none()
}

/// A change to a session's platform state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Add a schedule, built in full by the proposer (its id and times
    /// travel in the command, so every replica applies the same one).
    CreateSchedule { schedule: Box<SessionSchedule> },
    /// Change a schedule.
    UpdateSchedule {
        id: ScheduleId,
        #[serde(default)]
        change: ScheduleChange,
    },
    /// Fire `id` if it is still due at `due_ms`.
    FireSchedule { id: ScheduleId, due_ms: i64 },
    /// Fire `id` now on a person's request; its next time stays.
    TriggerSchedule { id: ScheduleId },
    /// A detached tool call's record or events (`detach.rs`).
    Call(crate::detach::CallOp),
    /// A change to the session's tasks (`tasks.rs`).
    Task(crate::tasks::TaskOp),
}

/// What a person or the agent changes on a schedule (the control plane's
/// `PATCH` shape: an `ends_at` or `max_runs` of null removes it).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub ends_at: Option<Option<DateTime<Utc>>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_runs: Option<Option<u32>>,
    /// Delete it: it stops for good and leaves the agent's view.
    #[serde(default)]
    pub deleted: bool,
}

/// A field that is absent (`None`), null (`Some(None)`) or set.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// What a change to a schedule that is not there answers.
pub const SCHEDULE_NOT_FOUND: &str = "schedule not found";

fn no_schedule() -> Response {
    Response::Rejected {
        reason: SCHEDULE_NOT_FOUND.to_owned(),
    }
}

fn at(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// The next time of recurring `schedule` after `after`, in its timezone.
pub fn next_cron(
    schedule: &SessionSchedule,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let expression = schedule
        .cron_expression
        .as_deref()
        .ok_or("a recurring schedule has no cron expression")?;
    let cron = parse_session_cron(expression)?;
    let zone = if schedule.timezone.trim().is_empty() {
        chrono_tz::UTC
    } else {
        schedule
            .timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|error| format!("invalid IANA timezone '{}': {error}", schedule.timezone))?
    };
    // cron yields both offsets of an ambiguous wall time; never the past.
    cron.after(&after.with_timezone(&zone))
        .map(|next| next.with_timezone(&Utc))
        .find(|next| *next > after)
        .ok_or_else(|| "the cron expression has no future time".to_owned())
}

/// Namespace of occurrence ids (the same as Engine's local host).
const OCCURRENCE: uuid::Uuid = uuid::Uuid::from_u128(0x6f1d_2c4e_8a3b_5d17_9e0f_4b2a_7c61_d385);

/// The admitted message id of `schedule`'s next occurrence.
pub fn occurrence_id(schedule: &SessionSchedule) -> MessageId {
    let key = format!(
        "{}:{}:{}",
        schedule.id,
        schedule.trigger_count,
        schedule.next_trigger_at.map_or(0, |t| t.timestamp_millis())
    );
    MessageId::from_uuid(uuid::Uuid::new_v5(&OCCURRENCE, key.as_bytes()))
}

fn not_fired(reason: &str) -> Response {
    Response::Value {
        value: json!({ "fired": false, "reason": reason }),
    }
}

impl CellState {
    /// Apply [`Command::Platform`].
    pub(crate) fn apply_platform(
        &mut self,
        session: SessionId,
        op: Op,
        now_ms: i64,
        seed: EventId,
        changes: &mut Vec<Change>,
    ) -> Response {
        let Some(state) = self.session_mut(session) else {
            return self.missing(session);
        };
        if state.placement != Placement::Live {
            return Response::Frozen;
        }
        if let Op::Call(op) = op {
            return crate::detach::apply(state, session, op, now_ms, changes);
        }
        if let Op::Task(op) = op {
            return crate::tasks::apply(state, session, op, now_ms, seed, changes);
        }
        let now = at(now_ms);
        let schedules = &mut state.platform.schedules;
        let response = match op {
            Op::CreateSchedule { schedule } => {
                if let Some(existing) = schedules.get(&schedule.id.to_string()) {
                    // A retried create answers what it made.
                    return Response::Value {
                        value: json!(existing),
                    };
                }
                if state.platform.active() >= MAX_ACTIVE_SCHEDULES_PER_SESSION {
                    return Response::Rejected {
                        reason: format!(
                            "Maximum {MAX_ACTIVE_SCHEDULES_PER_SESSION} active schedules per session. Cancel an existing schedule first."
                        ),
                    };
                }
                let value = json!(schedule);
                state
                    .platform
                    .schedules
                    .insert(schedule.id.to_string(), *schedule);
                Response::Value { value }
            }
            Op::UpdateSchedule { id, change } => {
                let Some(schedule) = schedules.get_mut(&id.to_string()) else {
                    return no_schedule();
                };
                if change.deleted && schedule.deleted_at.is_none() {
                    schedule.deleted_at = Some(now);
                    schedule.enabled = false;
                }
                if let Some(enabled) = change.enabled
                    && schedule.deleted_at.is_none()
                {
                    schedule.enabled = enabled;
                    if enabled {
                        schedule.stop_reason = None;
                    }
                    if enabled && schedule.next_trigger_at.is_none_or(|next| next < now) {
                        // Resumed: from now on, not the runs it missed.
                        schedule.next_trigger_at = match schedule.cron_expression {
                            Some(_) => next_cron(schedule, now).ok(),
                            None => schedule.scheduled_at.filter(|t| *t > now),
                        };
                        schedule.enabled = schedule.next_trigger_at.is_some();
                    }
                }
                if let Some(ends_at) = change.ends_at {
                    schedule.ends_at = ends_at;
                }
                if let Some(max_runs) = change.max_runs {
                    schedule.max_runs = max_runs;
                }
                schedule.updated_at = now;
                Response::Value {
                    value: json!(schedule),
                }
            }
            Op::FireSchedule { id, due_ms } => {
                let Some(schedule) = schedules.get(&id.to_string()).cloned() else {
                    return not_fired("not found");
                };
                let due = schedule.next_trigger_at.map(|t| t.timestamp_millis());
                if !live(&schedule) || due != Some(due_ms) {
                    return not_fired("not due");
                }
                fire(state, session, schedule, now_ms, seed, changes, true)
            }
            Op::TriggerSchedule { id } => {
                let Some(schedule) = schedules.get(&id.to_string()).cloned() else {
                    return no_schedule();
                };
                if schedule.deleted_at.is_some() {
                    return no_schedule();
                }
                fire(state, session, schedule, now_ms, seed, changes, false)
            }
            Op::Call(_) | Op::Task(_) => return Response::Ok,
        };
        changes.push(Change::Session(session));
        response
    }
}

/// Admit `schedule`'s occurrence into `state` and, when `advance`, move it
/// to its next time. A manual trigger (`advance` false) keeps the next time
/// and still counts as a run.
fn fire(
    state: &mut SessionState,
    session: SessionId,
    mut schedule: SessionSchedule,
    now_ms: i64,
    seed: EventId,
    changes: &mut Vec<Change>,
    advance: bool,
) -> Response {
    use everruns::hosting::spi::session_commands::SessionCommand;
    let now = at(now_ms);
    let ended = schedule.ends_at.is_some_and(|end| now >= end)
        || schedule
            .max_runs
            .is_some_and(|max| schedule.trigger_count >= max);
    if advance && ended {
        schedule.enabled = false;
        schedule.next_trigger_at = None;
        schedule.stop_reason = Some(stop_reason::ENDED.to_owned());
        schedule.updated_at = now;
        state
            .platform
            .schedules
            .insert(schedule.id.to_string(), schedule);
        return not_fired(stop_reason::ENDED);
    }
    let occurrence = if advance {
        occurrence_id(&schedule)
    } else {
        MessageId::from_uuid(uuid::Uuid::new_v5(&OCCURRENCE, seed.to_string().as_bytes()))
    };
    let mut fired = schedule.clone();
    fired.trigger_count = fired.trigger_count.saturating_add(1);
    let mut input = everruns_core::InputMessage::user(schedule.description.clone());
    fired.fire_origin().write(&mut input.metadata);
    let mut message = everruns_core::host::AcceptedTurnInput::new(input).into_message();
    message.id = occurrence;
    // Every replica builds the same message: no clock but the command's.
    message.created_at = now;
    let admit = SessionCommand::Admit {
        message: Box::new(message),
        cancelled: false,
        context: None,
    };
    let refused = match state.command(session, admit, now_ms, seed) {
        Ok((_, events)) => {
            for event in events {
                changes.push(Change::Event(session, Arc::new(event)));
            }
            None
        }
        Err(reason) => Some(reason),
    };
    let due_ms = schedule.next_trigger_at.map(|t| t.timestamp_millis());
    if refused.is_none() {
        schedule.trigger_count = fired.trigger_count;
        schedule.last_triggered_at = Some(now);
    }
    schedule.updated_at = now;
    if advance {
        match schedule.cron_expression {
            Some(_) => match next_cron(&schedule, now) {
                Ok(next) => schedule.next_trigger_at = Some(next),
                Err(reason) => {
                    schedule.enabled = false;
                    schedule.next_trigger_at = None;
                    schedule.stop_reason = Some(reason);
                }
            },
            None => {
                schedule.enabled = false;
                schedule.next_trigger_at = None;
            }
        }
    }
    let stored = json!(schedule);
    state
        .platform
        .schedules
        .insert(schedule.id.to_string(), schedule);
    Response::Value {
        value: match refused {
            None => json!({
                "fired": true,
                "message_id": occurrence,
                "lag_ms": due_ms.map(|due| now_ms - due),
                "schedule": stored,
            }),
            Some(reason) => json!({ "fired": false, "reason": reason, "schedule": stored }),
        },
    }
}

/// Fires proposed in one Raft entry.
const FIRE_BATCH: usize = 256;

/// While `node` leads its cell: fire every schedule due now. Called on each
/// scheduler tick; a proposal that does not commit within a tick's worth of
/// time is left to the next tick (applying a fire twice admits once).
pub(crate) async fn fire_due(node: &Arc<Node>) {
    let now = crate::node::now_ms();
    let due: Vec<(SessionId, ScheduleId, i64)> = {
        let state = node.shared.state.read();
        state
            .sessions
            .iter()
            .filter(|(_, s)| s.placement == Placement::Live && !s.platform.is_empty())
            .flat_map(|(session, s)| s.platform.due(now).map(|(id, due)| (*session, id, due)))
            .collect()
    };
    for chunk in due.chunks(FIRE_BATCH) {
        let commands = chunk
            .iter()
            .map(|(session, id, due_ms)| Command::Platform {
                session: *session,
                action: Op::FireSchedule {
                    id: *id,
                    due_ms: *due_ms,
                },
                now_ms: now,
                seed: EventId::new(),
            })
            .collect();
        let proposed = tokio::time::timeout(
            Duration::from_secs(1),
            node.propose(Command::Batch { commands }),
        )
        .await;
        let Ok(Ok((Response::Batch { responses }, _))) = proposed else {
            return;
        };
        let wall = crate::node::now_ms();
        for ((_, _, due_ms), response) in chunk.iter().zip(responses) {
            if let Response::Value { value } = response
                && value["fired"] == true
            {
                node.prom.schedules_fired.inc(&[]);
                let lag = Duration::from_millis((wall - due_ms).max(0) as u64);
                node.prom.schedule_lag_seconds.observe(&[], lag);
            }
        }
    }
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;
