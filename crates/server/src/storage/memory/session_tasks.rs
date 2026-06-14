// In-memory storage: Session task registry
//
// Mirrors the PostgreSQL repository exactly; lifecycle invariants come from
// `everruns_core::session_task::apply_task_update` in both backends.

use super::super::models::{NewSessionTaskMessageRow, SessionTaskMessageRow, SessionTaskRow};
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::SessionId;
use everruns_core::session_task::{SessionTask, SessionTaskUpdate, apply_task_update};

impl InMemoryDatabase {
    /// Insert a task. Idempotent on `id`: when the row already exists it is
    /// returned unchanged and the `bool` is false.
    pub async fn create_session_task(&self, task: &SessionTask) -> Result<(SessionTaskRow, bool)> {
        let row = SessionTaskRow::from_task(task)?;
        let mut tasks = self.session_tasks.write();
        if let Some(existing) = tasks.get(&row.id) {
            // Idempotency is scoped to the owning session: an id that exists
            // under a different session is a caller bug, not a replay.
            if existing.session_id != row.session_id {
                anyhow::bail!(
                    "session task id {} already exists in a different session",
                    row.id
                );
            }
            return Ok((existing.clone(), false));
        }
        tasks.insert(row.id.clone(), row.clone());
        Ok((row, true))
    }

    pub async fn get_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTaskRow>> {
        let tasks = self.session_tasks.read();
        Ok(tasks
            .get(task_id)
            .filter(|row| row.session_id == session_id)
            .cloned())
    }

    pub async fn list_session_tasks(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<SessionTaskRow>> {
        let tasks = self.session_tasks.read();
        let mut result: Vec<_> = tasks
            .values()
            .filter(|row| {
                row.session_id == session_id
                    && kind.is_none_or(|k| row.kind == k)
                    && state.is_none_or(|s| row.state == s)
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        Ok(result)
    }

    /// Load the task, apply the update through core invariants, write back.
    pub async fn update_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTaskRow>> {
        let mut tasks = self.session_tasks.write();
        let Some(row) = tasks
            .get(task_id)
            .filter(|row| row.session_id == session_id)
        else {
            return Ok(None);
        };

        let mut task = row.to_task()?;
        apply_task_update(&mut task, update, Self::now());
        let updated = SessionTaskRow::from_task(&task)?;
        tasks.insert(task_id.to_string(), updated.clone());
        Ok(Some(updated))
    }

    /// Record cooperative cancel intent. Idempotent: sets
    /// `cancel_requested_at` only when NULL. Returns the row plus whether
    /// this call changed it.
    pub async fn request_cancel_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<(SessionTaskRow, bool)>> {
        let mut tasks = self.session_tasks.write();
        let Some(row) = tasks
            .get_mut(task_id)
            .filter(|row| row.session_id == session_id)
        else {
            return Ok(None);
        };

        if row.cancel_requested_at.is_some() {
            return Ok(Some((row.clone(), false)));
        }
        let now = Self::now();
        row.cancel_requested_at = Some(now);
        row.updated_at = now;
        Ok(Some((row.clone(), true)))
    }

    pub async fn insert_session_task_message(
        &self,
        input: NewSessionTaskMessageRow,
    ) -> Result<SessionTaskMessageRow> {
        let row = SessionTaskMessageRow {
            id: input.id,
            task_id: input.task_id,
            session_id: input.session_id,
            direction: input.direction,
            content: input.content,
            in_reply_to: input.in_reply_to,
            created_at: Self::now(),
        };
        self.session_task_messages.write().push(row.clone());
        Ok(row)
    }

    /// Return (session_id, task_id, schedule_id) triples for running monitor
    /// tasks whose linked schedule is inactive (missing or enabled=false).
    pub async fn list_monitor_tasks_with_inactive_schedules(
        &self,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, String)>> {
        let tasks = self.session_tasks.read();
        let schedules = self.session_schedules.read();

        let mut result: Vec<(SessionId, String, String)> = tasks
            .values()
            .filter_map(|row| {
                if row.kind != "monitor" || row.state != "running" {
                    return None;
                }
                let schedule_id_str = row
                    .spec
                    .get("schedule_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)?;

                // Parse the schedule_id as a UUID/ScheduleId.
                let schedule_id: everruns_core::ScheduleId = schedule_id_str.parse().ok()?;

                // Inactive = missing row OR enabled=false.
                let is_inactive = schedules
                    .get(&schedule_id)
                    .map(|s| !s.enabled)
                    .unwrap_or(true); // missing row → inactive

                if is_inactive {
                    Some((row.session_id, row.id.clone(), schedule_id_str))
                } else {
                    None
                }
            })
            .collect();

        result.sort_by(|(_, a, _), (_, b, _)| a.cmp(b));
        result.truncate(limit as usize);
        Ok(result)
    }

    /// Return IDs of tasks that have a stale heartbeat (orphaned workers).
    ///
    /// A task is stale when:
    /// - state IN ('queued', 'running'), AND
    /// - heartbeat_at IS NOT NULL, AND
    /// - heartbeat_at < now - stale_after.
    ///
    /// Tasks with NULL heartbeat_at are foreground subagent tasks (EVE-535
    /// spawn-handle coverage) and are never reaped.
    pub async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(SessionId, String)>> {
        let cutoff = Self::now() - stale_after;
        let tasks = self.session_tasks.read();
        let mut result: Vec<(SessionId, String)> = tasks
            .values()
            .filter(|row| {
                (row.state == "queued" || row.state == "running")
                    && row.heartbeat_at.is_some_and(|hb| hb < cutoff)
            })
            .map(|row| (row.session_id, row.id.clone()))
            .collect();
        result.sort_by(|(_, a), (_, b)| a.cmp(b));
        result.truncate(limit as usize);
        Ok(result)
    }

    /// Messages on the task channel, oldest first.
    ///
    /// When `after_id` is `Some`, only messages after that cursor ID are
    /// returned. When `after_id` is `None`, the latest `limit` messages are
    /// returned (current default behaviour).
    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        let messages = self.session_task_messages.read();
        let mut result: Vec<_> = messages
            .iter()
            .filter(|m| m.session_id == session_id && m.task_id == task_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));

        if let Some(cursor_id) = after_id {
            let pos = result.iter().position(|m| m.id == cursor_id);
            if let Some(idx) = pos {
                result.drain(..=idx);
            } else {
                result.clear();
            }
            if let Some(limit) = limit {
                result.truncate(limit as usize);
            }
        } else if let Some(limit) = limit {
            let skip = result.len().saturating_sub(limit as usize);
            result.drain(..skip);
        }
        Ok(result)
    }
}
