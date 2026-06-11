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

    /// Last `limit` messages, returned oldest first.
    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        let messages = self.session_task_messages.read();
        let mut result: Vec<_> = messages
            .iter()
            .filter(|m| m.session_id == session_id && m.task_id == task_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        if let Some(limit) = limit {
            let skip = result.len().saturating_sub(limit as usize);
            result.drain(..skip);
        }
        Ok(result)
    }
}
