// In-memory storage: Session Schedules, Leased Resources

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::{LeasedResourceId, ScheduleId, SessionId};

impl InMemoryDatabase {
    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        let now = Self::now();
        let id = ScheduleId::new();
        let public_id = id.to_string();

        let row = SessionScheduleRow {
            id,
            public_id,
            org_id: input.org_id,
            session_id: input.session_id,
            description: input.description,
            cron_expression: input.cron_expression,
            scheduled_at: input.scheduled_at,
            timezone: input.timezone,
            enabled: true,
            next_trigger_at: input.next_trigger_at,
            last_triggered_at: None,
            trigger_count: 0,
            created_at: now,
            updated_at: now,
        };
        self.session_schedules.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        Ok(self
            .session_schedules
            .read()
            .get(&schedule_id)
            .filter(|r| r.org_id == org_id)
            .cloned())
    }

    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        let schedules = self.session_schedules.read();
        let mut result: Vec<_> = schedules
            .values()
            .filter(|r| r.org_id == org_id && r.session_id == session_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        let mut schedules = self.session_schedules.write();
        let Some(row) = schedules.get_mut(&schedule_id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(enabled) = input.enabled {
            row.enabled = enabled;
        }
        input.next_trigger_at.apply(&mut row.next_trigger_at);
        if let Some(last) = input.last_triggered_at {
            row.last_triggered_at = Some(last);
        }
        if input.trigger_count_increment {
            row.trigger_count += 1;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        let mut schedules = self.session_schedules.write();
        if let Some(row) = schedules.get(&schedule_id) {
            if row.org_id != org_id {
                return Ok(false);
            }
            schedules.remove(&schedule_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        let schedules = self.session_schedules.read();
        let count = schedules
            .values()
            .filter(|r| r.session_id == session_id && r.enabled)
            .count();
        Ok(count as u32)
    }

    pub async fn claim_due_session_schedules(&self, limit: i32) -> Result<Vec<SessionScheduleRow>> {
        let now = Self::now();
        let schedules = self.session_schedules.read();
        let mut due: Vec<_> = schedules
            .values()
            .filter(|r| r.enabled && r.next_trigger_at.is_some_and(|t| t <= now))
            .cloned()
            .collect();
        due.sort_by(|a, b| a.next_trigger_at.cmp(&b.next_trigger_at));
        due.truncate(limit as usize);
        Ok(due)
    }

    // ============================================
    // Leased Resources
    // ============================================

    pub async fn get_session_organization_id(&self, session_id: SessionId) -> Result<Option<i64>> {
        Ok(self.sessions.read().get(&session_id).map(|row| row.org_id))
    }

    pub async fn upsert_leased_resource(
        &self,
        input: UpsertLeasedResourceRow,
    ) -> Result<LeasedResourceRow> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();

        if let Some(existing) = resources.values_mut().find(|row| {
            row.org_id == input.org_id
                && row.provider == input.provider
                && row.resource_type == input.resource_type
                && row.external_id == input.external_id
        }) {
            existing.session_id = Some(input.session_id);
            existing.display_name = input.display_name.or_else(|| existing.display_name.clone());
            existing.status = "active".to_string();
            existing.owner_user_id = input.owner_user_id.or(existing.owner_user_id);
            existing.lease_duration_seconds = input.lease_duration_seconds;
            existing.last_touched_at = now;
            existing.lease_expires_at = input.lease_expires_at;
            existing.cleanup_started_at = None;
            existing.cleanup_completed_at = None;
            existing.last_cleanup_error = None;
            existing.metadata = input.metadata;
            existing.updated_at = now;
            return Ok(existing.clone());
        }

        let id = LeasedResourceId::new();
        let row = LeasedResourceRow {
            id,
            public_id: id.to_string(),
            org_id: input.org_id,
            session_id: Some(input.session_id),
            provider: input.provider,
            resource_type: input.resource_type,
            external_id: input.external_id,
            display_name: input.display_name,
            status: "active".to_string(),
            owner_user_id: input.owner_user_id,
            lease_duration_seconds: input.lease_duration_seconds,
            last_touched_at: now,
            lease_expires_at: input.lease_expires_at,
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        resources.insert(id, row.clone());
        Ok(row)
    }

    pub async fn release_leased_resource(
        &self,
        input: ReleaseLeasedResourceRow,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let row = resources.values_mut().find(|row| {
            row.org_id == input.org_id
                && row.session_id == Some(input.session_id)
                && row.provider == input.provider
                && row.resource_type == input.resource_type
                && row.external_id == input.external_id
        });
        let Some(row) = row else {
            return Ok(None);
        };

        row.status = "released".to_string();
        row.cleanup_started_at = None;
        row.cleanup_completed_at = Some(now);
        row.lease_expires_at = now;
        row.last_cleanup_error = None;
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    pub async fn list_session_leased_resources(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<LeasedResourceRow>> {
        let resources = self.leased_resources.read();
        let mut result: Vec<_> = resources
            .values()
            .filter(|row| row.session_id == Some(session_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn claim_due_leased_resources(
        &self,
        limit: i32,
        stale_after_seconds: i32,
    ) -> Result<Vec<LeasedResourceRow>> {
        let now = Self::now();
        let stale_before = now - chrono::TimeDelta::seconds(stale_after_seconds as i64);
        let mut resources = self.leased_resources.write();
        let mut due_ids: Vec<_> = resources
            .values()
            .filter(|row| {
                (matches!(row.status.as_str(), "active" | "cleanup_failed")
                    && row.lease_expires_at <= now)
                    || (row.status == "cleaning"
                        && row.cleanup_started_at.is_some_and(|ts| ts <= stale_before))
            })
            .map(|row| row.id)
            .collect();
        due_ids.sort_by(|a, b| {
            let left = resources.get(a).expect("leased resource must exist");
            let right = resources.get(b).expect("leased resource must exist");
            (
                &left.lease_expires_at,
                &left.cleanup_started_at,
                &left.created_at,
            )
                .cmp(&(
                    &right.lease_expires_at,
                    &right.cleanup_started_at,
                    &right.created_at,
                ))
        });
        due_ids.truncate(limit as usize);

        let mut claimed = Vec::with_capacity(due_ids.len());
        for id in due_ids {
            if let Some(row) = resources.get_mut(&id) {
                row.status = "cleaning".to_string();
                row.cleanup_started_at = Some(now);
                row.cleanup_attempts += 1;
                row.last_cleanup_error = None;
                row.updated_at = now;
                claimed.push(row.clone());
            }
        }

        Ok(claimed)
    }

    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let Some(row) = resources.get_mut(&resource_id) else {
            return Ok(None);
        };
        if row.status != "cleaning" || row.cleanup_started_at != Some(expected_cleanup_started_at) {
            return Ok(None);
        }

        row.status = "released".to_string();
        row.cleanup_started_at = None;
        row.cleanup_completed_at = Some(now);
        row.lease_expires_at = now;
        row.last_cleanup_error = None;
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
        retry_after_seconds: i32,
        error: &str,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let Some(row) = resources.get_mut(&resource_id) else {
            return Ok(None);
        };
        if row.status != "cleaning" || row.cleanup_started_at != Some(expected_cleanup_started_at) {
            return Ok(None);
        }

        row.status = "cleanup_failed".to_string();
        row.cleanup_started_at = None;
        row.lease_expires_at = now + chrono::TimeDelta::seconds(retry_after_seconds as i64);
        row.last_cleanup_error = Some(error.to_string());
        row.updated_at = now;
        Ok(Some(row.clone()))
    }
}
