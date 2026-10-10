//! Local schedules as timers in the session log.
//!
//! Decision (actor-based design, step 3): the session log is the record of a
//! schedule. Creating one appends `timer.set`, each delivered occurrence
//! `timer.fired`, cancelling `timer.cancelled`. The `local_schedules` table is
//! the index the runner polls, and `LocalScheduleStore::reconcile_journal`
//! makes it agree with the log when the runner starts:
//!
//! - a timer in the log with no row (the process died between the two writes,
//!   or the database was replaced) gets its row back;
//! - a row behind the log (cancelled or fired in the log only) catches up, so
//!   an occurrence the log already records is not delivered twice;
//! - an enabled row with no timer in the log (created before schedules were
//!   journaled) is written into the log, so from then on the log alone is
//!   enough to rebuild it.
//!
//! Writes go to the log first and the index second, except a create that must
//! pass the per-session and per-org limits: the limit check and the row insert
//! share one SQLite transaction, so the row comes first and is removed again
//! if the log refuses the entry.
//!
//! Delivery stays at least once, as before: an occurrence is sent, then
//! recorded. A crash between the two sends it again on restart.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::events::{
    EventContext, EventData, EventRequest, TimerCancelledData, TimerFiredData, TimerPurpose,
    TimerSetData,
};
use everruns_contracts::typed_id::{PrincipalId, ScheduleId, SessionId};
use everruns_core::host::{EventLog, EventReadLimit, EventReadRequest, MAX_EVENT_PAGE_SIZE};
use everruns_core::session_schedule::SessionSchedule;
use serde_json::Value;

/// Append one timer entry to `session_id`'s log.
pub(crate) async fn append(
    log: &dyn EventLog,
    session_id: SessionId,
    data: impl Into<EventData>,
) -> Result<()> {
    log.append(EventRequest::new(session_id, EventContext::empty(), data))
        .await
        .map(|_| ())
        .map_err(|error| AgentLoopError::store(format!("schedule journal append failed: {error}")))
}

/// The `timer.set` entry recording `schedule`.
pub(crate) fn timer_set(schedule: &SessionSchedule, metadata: &Value) -> TimerSetData {
    let metadata = match metadata {
        Value::Object(map) if map.is_empty() => Value::Null,
        other => other.clone(),
    };
    TimerSetData {
        timer_id: schedule.id.to_string(),
        purpose: TimerPurpose::Schedule,
        fire_at: schedule
            .next_trigger_at
            .or(schedule.scheduled_at)
            .unwrap_or(schedule.created_at),
        cron_expression: schedule.cron_expression.clone(),
        timezone: Some(schedule.timezone.clone()),
        description: Some(schedule.description.clone()),
        metadata,
    }
}

pub(crate) fn timer_fired(schedule: &SessionSchedule, fired_at: DateTime<Utc>) -> TimerFiredData {
    TimerFiredData {
        timer_id: schedule.id.to_string(),
        fired_at,
        next_fire_at: schedule.next_trigger_at,
    }
}

pub(crate) fn timer_cancelled(schedule_id: ScheduleId) -> TimerCancelledData {
    TimerCancelledData {
        timer_id: schedule_id.to_string(),
    }
}

/// A schedule as the session log describes it.
#[derive(Debug, Clone)]
pub(crate) struct JournalTimer {
    pub set: TimerSetData,
    pub set_at: DateTime<Utc>,
    pub fired_at: Vec<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub cancelled: bool,
}

impl JournalTimer {
    /// Whether the timer can still fire.
    pub fn active(&self) -> bool {
        !self.cancelled && self.next_fire_at.is_some()
    }

    /// Occurrences recorded after `after` (all of them when `None`).
    pub fn fired_since(&self, after: Option<DateTime<Utc>>) -> u32 {
        let count = self
            .fired_at
            .iter()
            .filter(|at| after.is_none_or(|after| **at > after))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Rebuild the schedule record from the log.
    pub fn schedule(
        &self,
        session_id: SessionId,
        owner_principal_id: PrincipalId,
    ) -> Result<SessionSchedule> {
        let id = ScheduleId::from_str(&self.set.timer_id).map_err(|error| {
            AgentLoopError::store(format!(
                "timer {} is not a schedule id: {error}",
                self.set.timer_id
            ))
        })?;
        let cron_expression = self.set.cron_expression.clone();
        let last_triggered_at = self.fired_at.last().copied();
        Ok(SessionSchedule {
            id,
            session_id,
            owner_principal_id,
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            description: self.set.description.clone().unwrap_or_default(),
            scheduled_at: cron_expression.is_none().then_some(self.set.fire_at),
            schedule_type: SessionSchedule::derive_type(&cron_expression),
            cron_expression,
            timezone: self.set.timezone.clone().unwrap_or_else(|| "UTC".into()),
            enabled: self.active(),
            next_trigger_at: self.next_fire_at,
            last_triggered_at,
            trigger_count: self.fired_since(None),
            created_at: self.set_at,
            updated_at: last_triggered_at.unwrap_or(self.set_at),
        })
    }

    /// The host fields kept with the timer, as the store's metadata bag.
    pub fn metadata(&self) -> Value {
        match &self.set.metadata {
            Value::Null => Value::Object(Default::default()),
            other => other.clone(),
        }
    }
}

/// Fold every schedule timer in `session_id`'s log, in the order they were set.
pub(crate) async fn read_timers(
    log: &dyn EventLog,
    session_id: SessionId,
) -> Result<Vec<JournalTimer>> {
    let limit = EventReadLimit::new(MAX_EVENT_PAGE_SIZE)
        .map_err(|error| AgentLoopError::store(error.to_string()))?;
    let mut request = EventReadRequest::new(session_id, limit);
    let mut order: Vec<String> = Vec::new();
    let mut timers: HashMap<String, JournalTimer> = HashMap::new();
    loop {
        let page = log.read_page(request).await.map_err(|error| {
            AgentLoopError::store(format!("schedule journal read failed: {error}"))
        })?;
        for event in page.events {
            match event.data {
                EventData::TimerSet(set) if set.purpose == TimerPurpose::Schedule => {
                    let id = set.timer_id.clone();
                    if !timers.contains_key(&id) {
                        order.push(id.clone());
                    }
                    timers.insert(
                        id,
                        JournalTimer {
                            next_fire_at: Some(set.fire_at),
                            set,
                            set_at: event.ts,
                            fired_at: Vec::new(),
                            cancelled: false,
                        },
                    );
                }
                EventData::TimerFired(fired) => {
                    if let Some(timer) = timers.get_mut(&fired.timer_id) {
                        timer.fired_at.push(fired.fired_at);
                        timer.next_fire_at = fired.next_fire_at;
                    }
                }
                EventData::TimerCancelled(cancelled) => {
                    if let Some(timer) = timers.get_mut(&cancelled.timer_id) {
                        timer.cancelled = true;
                    }
                }
                _ => {}
            }
        }
        match page.next_cursor {
            Some(cursor) => request = EventReadRequest::from_cursor(cursor, limit),
            None => break,
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|id| timers.remove(&id))
        .collect())
}
