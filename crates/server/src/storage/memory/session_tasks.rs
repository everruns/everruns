// In-memory storage: Session task registry
//
// Mirrors the PostgreSQL repository exactly; lifecycle invariants come from
// `everruns_core::session_task::apply_task_update` in both backends.

use super::super::models::{
    CreateSessionTaskPushConfig, NewSessionTaskMessageRow, SessionTaskMessageRow,
    SessionTaskPushConfigRow, SessionTaskRow,
};
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::session_task::{SessionTask, SessionTaskUpdate, apply_task_update};
use everruns_provider::typed_id::SessionId;

impl InMemoryDatabase {
    /// Insert a task. Idempotent on `id`: when the row already exists it is
    /// returned unchanged and the `bool` is false.
    pub async fn create_session_task(&self, task: &SessionTask) -> Result<(SessionTaskRow, bool)> {
        let mut row = SessionTaskRow::from_task(task)?;
        // EVE-680: denormalize the owning session's tree root onto the task,
        // mirroring the Postgres subquery. Read sessions before locking tasks to
        // keep a consistent lock order.
        row.root_session_id = self
            .sessions
            .read()
            .get(&task.session_id)
            .and_then(|s| s.root_session_id);
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

    /// List tasks across every session owned by `org_id`, newest-first, with
    /// optional kind/state/age filters and a bounded limit. Mirrors the
    /// PostgreSQL repository: org scoping is the authoritative multitenancy
    /// boundary — a task is only returned when its owning session belongs to
    /// the org.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_org_session_tasks(
        &self,
        org_id: i64,
        kind: Option<&str>,
        state: Option<&str>,
        created_after: Option<chrono::DateTime<chrono::Utc>>,
        root_session_id: Option<SessionId>,
        limit: i64,
    ) -> Result<Vec<SessionTaskRow>> {
        // Sessions owned by the org — the multitenancy boundary, mirroring the
        // SQL semijoin on sessions.org_id.
        let org_sessions: std::collections::HashSet<SessionId> = {
            let sessions = self.sessions.read();
            sessions
                .values()
                .filter(|s| s.org_id == org_id)
                .map(|s| s.id)
                .collect()
        };
        let tasks = self.session_tasks.read();
        let mut result: Vec<_> = tasks
            .values()
            .filter(|row| {
                org_sessions.contains(&row.session_id)
                    && kind.is_none_or(|k| row.kind == k)
                    && state.is_none_or(|s| row.state == s)
                    && created_after.is_none_or(|c| row.created_at >= c)
                    && root_session_id.is_none_or(|r| row.root_session_id == Some(r))
            })
            .cloned()
            .collect();
        // Newest-first, matching the PG `ORDER BY created_at DESC, id DESC`.
        result.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        result.truncate(limit.max(0) as usize);
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

    // ============================================
    // Per-task push-notification configs (EVE-682)
    // ============================================

    pub async fn create_task_push_config(
        &self,
        input: CreateSessionTaskPushConfig,
    ) -> Result<SessionTaskPushConfigRow> {
        let now = Self::now();
        let mut configs = self.session_task_push_configs.write();
        let id = configs.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let row = SessionTaskPushConfigRow {
            id,
            public_id: input.public_id,
            session_id: input.session_id,
            task_id: input.task_id,
            url: input.url,
            secret: input.secret,
            event_filter: input.event_filter,
            created_at: now,
            updated_at: now,
        };
        configs.push(row.clone());
        Ok(row)
    }

    pub async fn list_task_push_configs(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Vec<SessionTaskPushConfigRow>> {
        let mut rows: Vec<_> = self
            .session_task_push_configs
            .read()
            .iter()
            .filter(|c| c.session_id == session_id && c.task_id == task_id)
            .cloned()
            .collect();
        rows.sort_by_key(|c| (c.created_at, c.id));
        Ok(rows)
    }

    pub async fn delete_task_push_config(
        &self,
        session_id: SessionId,
        task_id: &str,
        public_id: &str,
    ) -> Result<bool> {
        let mut configs = self.session_task_push_configs.write();
        let before = configs.len();
        configs.retain(|c| {
            !(c.session_id == session_id && c.task_id == task_id && c.public_id == public_id)
        });
        Ok(configs.len() < before)
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
    /// tasks whose linked schedule is inactive.
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
                let schedule_id: everruns_provider::typed_id::ScheduleId =
                    schedule_id_str.parse().ok()?;

                // Fired one-shots are disabled before their monitor can be
                // marked Succeeded; only never-triggered disabled one-shots are
                // orphan candidates.
                let is_inactive = schedules
                    .get(&schedule_id)
                    .map(|s| {
                        !s.enabled
                            && (s.cron_expression.is_some()
                                || (s.last_triggered_at.is_none() && s.trigger_count == 0))
                    })
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

    /// Prune terminal session tasks whose `finished_at` is older than the TTL
    /// cutoff, in a bounded batch. Mirrors the PostgreSQL repository: deletes
    /// the task rows AND their messages (no FK cascade in-memory), and returns
    /// the `(session_id, task_id, result_path)` triples removed so the caller
    /// can clean up `result_path` artifacts (EVE-580).
    ///
    /// Only terminal states with a non-NULL `finished_at` strictly older than
    /// `cutoff` are eligible; live/running/queued tasks are never touched.
    pub async fn prune_terminal_session_tasks(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, Option<String>)>> {
        let mut tasks = self.session_tasks.write();

        // Select the eligible batch deterministically (oldest finished first),
        // matching the PG query's ORDER BY finished_at ASC.
        let mut eligible: Vec<(
            SessionId,
            String,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        )> = tasks
            .values()
            .filter(|row| {
                matches!(row.state.as_str(), "succeeded" | "failed" | "canceled")
                    && row.finished_at.is_some_and(|f| f < cutoff)
            })
            .map(|row| {
                (
                    row.session_id,
                    row.id.clone(),
                    row.result_path.clone(),
                    row.finished_at.expect("filtered to Some above"),
                )
            })
            .collect();
        eligible.sort_by(|a, b| (a.3, &a.1).cmp(&(b.3, &b.1)));
        eligible.truncate(limit.max(0) as usize);

        let pruned: Vec<(SessionId, String, Option<String>)> = eligible
            .into_iter()
            .map(|(sid, id, rp, _)| (sid, id, rp))
            .collect();

        let pruned_ids: std::collections::HashSet<&str> =
            pruned.iter().map(|(_, id, _)| id.as_str()).collect();
        for (_, id, _) in &pruned {
            tasks.remove(id);
        }
        drop(tasks);

        // Cascade messages for the pruned tasks.
        self.session_task_messages
            .write()
            .retain(|m| !pruned_ids.contains(m.task_id.as_str()));

        Ok(pruned)
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use everruns_provider::typed_id::SessionId;

    fn terminal_row(
        db: &InMemoryDatabase,
        session_id: SessionId,
        id: &str,
        state: &str,
        finished_at: Option<chrono::DateTime<Utc>>,
        result_path: Option<&str>,
    ) {
        let now = Utc::now();
        let row = SessionTaskRow {
            id: id.to_string(),
            session_id,
            root_session_id: Some(session_id),
            kind: "background_tool".to_string(),
            display_name: "t".to_string(),
            spec: serde_json::json!({}),
            state: state.to_string(),
            state_detail: None,
            progress: None,
            input_request: None,
            cancel_requested_at: None,
            summary: None,
            result_path: result_path.map(str::to_string),
            artifacts: serde_json::json!([]),
            error: None,
            attempt: 1,
            worker_id: None,
            heartbeat_at: None,
            links: serde_json::json!({}),
            wake_policy: "silent".to_string(),
            created_at: now,
            started_at: Some(now),
            finished_at,
            updated_at: now,
        };
        db.session_tasks.write().insert(id.to_string(), row);
    }

    fn message(db: &InMemoryDatabase, session_id: SessionId, id: &str, task_id: &str) {
        db.session_task_messages
            .write()
            .push(SessionTaskMessageRow {
                id: id.to_string(),
                task_id: task_id.to_string(),
                session_id,
                direction: "outbound".to_string(),
                content: serde_json::json!([]),
                in_reply_to: None,
                created_at: Utc::now(),
            });
    }

    #[test]
    fn to_task_surfaces_root_session_id() {
        // EVE-681: the delegation-tree root is denormalized on the row and must
        // now round-trip onto the core SessionTask so cross-session tooling
        // (the Work view) can group a whole tree by one id.
        let db = InMemoryDatabase::new();
        let session_id = SessionId::new();
        let root = SessionId::new();
        terminal_row(&db, session_id, "task_root", "running", None, None);
        db.session_tasks
            .write()
            .get_mut("task_root")
            .unwrap()
            .root_session_id = Some(root);

        let row = db.session_tasks.read().get("task_root").unwrap().clone();
        let task = row.to_task().expect("to_task");
        assert_eq!(task.root_session_id, Some(root));
        let round_tripped = SessionTaskRow::from_task(&task).expect("from_task");
        assert_eq!(round_tripped.root_session_id, Some(root));

        // A NULL root column surfaces as None (top-level session / unavailable).
        db.session_tasks
            .write()
            .get_mut("task_root")
            .unwrap()
            .root_session_id = None;
        let row = db.session_tasks.read().get("task_root").unwrap().clone();
        assert_eq!(row.to_task().expect("to_task").root_session_id, None);
    }

    #[tokio::test]
    async fn prune_removes_only_old_terminal_tasks_and_their_messages() {
        let db = InMemoryDatabase::new();
        let session_id = SessionId::new();
        let now = Utc::now();

        // Old terminal tasks (eligible) in each terminal state.
        let old = now - Duration::days(40);
        terminal_row(
            &db,
            session_id,
            "task_old_succ",
            "succeeded",
            Some(old),
            Some("/.tasks/task_old_succ/result.json"),
        );
        terminal_row(&db, session_id, "task_old_fail", "failed", Some(old), None);
        terminal_row(
            &db,
            session_id,
            "task_old_canc",
            "canceled",
            Some(old),
            None,
        );
        // Recent terminal task (not eligible).
        terminal_row(
            &db,
            session_id,
            "task_recent",
            "succeeded",
            Some(now - Duration::hours(1)),
            None,
        );
        // Live tasks (never eligible, even with NULL finished_at).
        terminal_row(&db, session_id, "task_running", "running", None, None);
        terminal_row(&db, session_id, "task_queued", "queued", None, None);

        // Messages: old task has messages (must be cascaded), recent keeps them.
        message(&db, session_id, "msg_old_1", "task_old_succ");
        message(&db, session_id, "msg_old_2", "task_old_succ");
        message(&db, session_id, "msg_recent", "task_recent");

        let cutoff = now - Duration::days(30);
        let pruned = db.prune_terminal_session_tasks(cutoff, 100).await.unwrap();

        // Only the three old terminal tasks pruned.
        let mut pruned_ids: Vec<&str> = pruned.iter().map(|(_, id, _)| id.as_str()).collect();
        pruned_ids.sort();
        assert_eq!(
            pruned_ids,
            vec!["task_old_canc", "task_old_fail", "task_old_succ"]
        );

        // result_path is surfaced for artifact cleanup.
        let succ = pruned
            .iter()
            .find(|(_, id, _)| id == "task_old_succ")
            .unwrap();
        assert_eq!(succ.2.as_deref(), Some("/.tasks/task_old_succ/result.json"));

        // Recent + live tasks survive.
        let remaining: Vec<String> = db.session_tasks.read().keys().cloned().collect();
        let mut remaining = remaining;
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                "task_queued".to_string(),
                "task_recent".to_string(),
                "task_running".to_string()
            ]
        );

        // Old task messages cascaded; recent task message kept.
        let msgs = db.session_task_messages.read();
        let msg_ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(msg_ids, vec!["msg_recent"]);
    }

    #[tokio::test]
    async fn prune_respects_batch_cap() {
        let db = InMemoryDatabase::new();
        let session_id = SessionId::new();
        let now = Utc::now();
        for i in 0..5 {
            terminal_row(
                &db,
                session_id,
                &format!("task_{i}"),
                "succeeded",
                // Stagger finished_at so the ORDER BY finished_at ASC is deterministic.
                Some(now - Duration::days(40) + Duration::seconds(i)),
                None,
            );
        }
        let cutoff = now - Duration::days(30);
        let pruned = db.prune_terminal_session_tasks(cutoff, 2).await.unwrap();
        assert_eq!(pruned.len(), 2, "batch cap bounds work per pass");
        assert_eq!(
            db.session_tasks.read().len(),
            3,
            "remaining drained next tick"
        );
        // Oldest-first: the two earliest finished_at are pruned.
        let mut ids: Vec<&str> = pruned.iter().map(|(_, id, _)| id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["task_0", "task_1"]);
    }
}
