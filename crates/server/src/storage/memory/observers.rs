// In-memory observer storage. Same queue semantics as the PostgreSQL
// implementation, minus durability: a single write lock replaces
// FOR UPDATE SKIP LOCKED.

use anyhow::Result;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::*;

impl InMemoryDatabase {
    // ============================================
    // Observer CRUD
    // ============================================

    pub async fn create_observer(
        &self,
        org_id: i64,
        input: CreateObserverRow,
    ) -> Result<ObserverRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = ObserverRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            match_config: input.match_config,
            sampling_rate: input.sampling_rate,
            scorers: input.scorers,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        self.observers.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_observer_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ObserverRow>> {
        let observers = self.observers.read();
        Ok(observers
            .values()
            .find(|o| o.org_id == org_id && o.public_id == public_id)
            .cloned())
    }

    pub async fn get_observer(&self, id: Uuid) -> Result<Option<ObserverRow>> {
        Ok(self.observers.read().get(&id).cloned())
    }

    pub async fn list_observers(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<ObserverRow>> {
        let observers = self.observers.read();
        let mut result: Vec<ObserverRow> = observers
            .values()
            .filter(|o| {
                o.org_id == org_id
                    && if include_archived {
                        o.status != "deleted"
                    } else {
                        o.status != "archived" && o.status != "deleted"
                    }
            })
            .cloned()
            .collect();
        result.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        Ok(result)
    }

    pub async fn list_active_observers(&self, org_id: i64) -> Result<Vec<ObserverRow>> {
        let observers = self.observers.read();
        Ok(observers
            .values()
            .filter(|o| o.org_id == org_id && o.status == "active")
            .cloned()
            .collect())
    }

    pub async fn count_active_observers(&self, org_id: i64) -> Result<i64> {
        let observers = self.observers.read();
        Ok(observers
            .values()
            .filter(|o| o.org_id == org_id && matches!(o.status.as_str(), "active" | "paused"))
            .count() as i64)
    }

    pub async fn update_observer(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateObserverRow,
    ) -> Result<Option<ObserverRow>> {
        let mut observers = self.observers.write();
        let Some(observer) = observers.get_mut(&id) else {
            return Ok(None);
        };
        if observer.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            observer.name = name;
        }
        if let Some(description) = input.description {
            observer.description = Some(description);
        }
        if let Some(match_config) = input.match_config {
            observer.match_config = match_config;
        }
        if let Some(sampling_rate) = input.sampling_rate {
            observer.sampling_rate = sampling_rate;
        }
        if let Some(scorers) = input.scorers {
            observer.scorers = scorers;
        }
        if let Some(status) = input.status {
            observer.status = status;
        }
        observer.updated_at = Self::now();
        Ok(Some(observer.clone()))
    }

    pub async fn delete_observer(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut observers = self.observers.write();
        let Some(observer) = observers.get_mut(&id) else {
            return Ok(false);
        };
        if observer.org_id != org_id || !matches!(observer.status.as_str(), "active" | "paused") {
            return Ok(false);
        }
        observer.status = "archived".to_string();
        observer.archived_at = Some(Self::now());
        observer.updated_at = Self::now();
        Ok(true)
    }

    // ============================================
    // Trace scores (queue + results)
    // ============================================

    pub async fn enqueue_trace_scores(
        &self,
        org_id: i64,
        inputs: &[CreateTraceScoreRow],
    ) -> Result<u64> {
        let now = Self::now();
        let mut scores = self.trace_scores.write();
        let mut inserted = 0u64;
        for input in inputs {
            let duplicate = scores.values().any(|s| {
                s.observer_id == input.observer_id
                    && s.scorer_key == input.scorer_key
                    && s.session_id == input.session_id
                    && s.turn_id == input.turn_id
            });
            if duplicate {
                continue;
            }
            let id = Uuid::now_v7();
            scores.insert(
                id,
                TraceScoreRow {
                    id,
                    org_id,
                    public_id: input.public_id.clone(),
                    observer_id: input.observer_id,
                    scorer_key: input.scorer_key.clone(),
                    session_id: input.session_id,
                    turn_id: input.turn_id.clone(),
                    agent_id: input.agent_id,
                    agent_version_id: input.agent_version_id,
                    harness_id: input.harness_id,
                    status: "pending".to_string(),
                    attempts: 0,
                    pass: None,
                    value: None,
                    label: None,
                    reason: None,
                    judge_input_tokens: None,
                    judge_output_tokens: None,
                    judge_cost_usd: None,
                    error_message: None,
                    created_at: now,
                    updated_at: now,
                },
            );
            inserted += 1;
        }
        Ok(inserted)
    }

    pub async fn claim_trace_scores(
        &self,
        limit: i64,
        stale_after_seconds: i64,
        max_attempts: i32,
    ) -> Result<Vec<TraceScoreRow>> {
        let now = Self::now();
        let stale_cutoff = now - chrono::Duration::seconds(stale_after_seconds);
        let mut scores = self.trace_scores.write();

        for score in scores.values_mut() {
            if score.status == "scoring"
                && score.updated_at < stale_cutoff
                && score.attempts >= max_attempts
            {
                score.status = "errored".to_string();
                score.error_message = Some("scoring attempts exhausted".to_string());
                score.updated_at = now;
            }
        }

        let mut claimable: Vec<(chrono::DateTime<chrono::Utc>, Uuid)> = scores
            .values()
            .filter(|s| {
                s.attempts < max_attempts
                    && (s.status == "pending"
                        || (s.status == "scoring" && s.updated_at < stale_cutoff))
            })
            .map(|s| (s.created_at, s.id))
            .collect();
        claimable.sort();
        let claimable: Vec<Uuid> = claimable
            .into_iter()
            .take(limit.max(0) as usize)
            .map(|(_, id)| id)
            .collect();

        let mut claimed = Vec::with_capacity(claimable.len());
        for id in claimable {
            if let Some(score) = scores.get_mut(&id) {
                score.status = "scoring".to_string();
                score.attempts += 1;
                score.updated_at = now;
                claimed.push(score.clone());
            }
        }
        Ok(claimed)
    }

    pub async fn complete_trace_score(
        &self,
        id: Uuid,
        input: CompleteTraceScoreRow,
    ) -> Result<Option<TraceScoreRow>> {
        let mut scores = self.trace_scores.write();
        let Some(score) = scores.get_mut(&id) else {
            return Ok(None);
        };
        score.status = input.status;
        score.pass = input.pass;
        score.value = input.value;
        score.label = input.label;
        score.reason = input.reason;
        score.judge_input_tokens = input.judge_input_tokens;
        score.judge_output_tokens = input.judge_output_tokens;
        score.judge_cost_usd = input.judge_cost_usd;
        score.error_message = input.error_message;
        score.updated_at = Self::now();
        Ok(Some(score.clone()))
    }

    pub async fn list_trace_scores(
        &self,
        org_id: i64,
        params: ListTraceScoresParams,
    ) -> Result<Vec<TraceScoreRow>> {
        let scores = self.trace_scores.read();
        let mut result: Vec<TraceScoreRow> = scores
            .values()
            .filter(|s| s.org_id == org_id && s.observer_id == params.observer_id)
            .filter(|s| params.session_id.is_none_or(|sid| s.session_id == sid))
            .cloned()
            .collect();
        result.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        Ok(result
            .into_iter()
            .skip(params.offset.max(0) as usize)
            .take(params.limit.max(0) as usize)
            .collect())
    }
}
