// Observer service — CRUD over observers and read access to trace scores.
//
// Decision: Observers reuse agent-management permissions; scores read also
// requires session-management because scores expose production content.
// Decision: The service is stateless (db only); scoring happens in the
// background worker (worker.rs), triggered by the listener (listener.rs).

use crate::domains::observers::matching::{SessionFacts, Slice, plan_scores};
use crate::domains::observers::record::*;
use crate::domains::observers::types::{
    BackfillObserverRequest, CreateObserverRequest, ReviewScoreRequest, UpdateObserverRequest,
};
use crate::errors::BadRequestError;
use crate::storage::StorageBackend;
use crate::storage::{
    CreateObserverRow, ListTraceScoresParams, ObserverRow, ScoreGroup, ScoreSummaryRow,
    TraceScoreRow, UpdateObserverRow,
};
use anyhow::Result;
use everruns_contracts::typed_id::{ObserverId, SessionId};
use everruns_core::{Caller, Permission, Policy, Rule};
use std::sync::Arc;
use uuid::Uuid;

/// Policy: View observers and their scores (read-only). Scores expose
/// production content, so viewing requires session-management permission.
pub const OBSERVER_VIEW: Policy = Policy {
    id: "observer.view",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSessionsManage),
    ],
};

/// Policy: Manage observers (create, update, delete).
pub const OBSERVER_MANAGE: Policy = Policy {
    id: "observer.manage",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSessionsManage),
    ],
};

/// Default cap on active+paused observers per org. Bounds judge fan-out.
const DEFAULT_MAX_OBSERVERS_PER_ORG: i64 = 50;
/// Bound per-turn enqueue fan-out and per-score worker config scans.
const MAX_SCORERS_PER_OBSERVER: usize = 32;
const MAX_SCORER_KEY_BYTES: usize = 128;
const MAX_SCORER_RULE_BYTES: usize = 8 * 1024;
const MAX_SCORER_TOOLS: usize = 64;
const MAX_REVIEW_NOTE_CHARS: usize = 2_000;
const BACKFILL_DEFAULT_HOURS: u32 = 24;
const BACKFILL_MAX_HOURS: u32 = 720;
const BACKFILL_DEFAULT_LIMIT: u32 = 500;
const BACKFILL_MAX_LIMIT: u32 = 5_000;
const SUMMARY_DEFAULT_DAYS: u32 = 7;
const SUMMARY_MAX_DAYS: u32 = 90;

fn max_observers_per_org() -> i64 {
    std::env::var("OBSERVER_MAX_PER_ORG")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_OBSERVERS_PER_ORG)
}

pub struct ObserverService {
    db: Arc<StorageBackend>,
}

/// Validate observer configuration before persisting.
fn validate(sampling_rate: f64, scorers: &[ObserverScorerConfig]) -> Result<()> {
    if !(0.0..=1.0).contains(&sampling_rate) {
        anyhow::bail!(BadRequestError::new(
            "sampling_rate must be between 0.0 and 1.0"
        ));
    }
    if scorers.is_empty() {
        anyhow::bail!(BadRequestError::new(
            "observer must define at least one scorer"
        ));
    }
    if scorers.len() > MAX_SCORERS_PER_OBSERVER {
        anyhow::bail!(BadRequestError::new(format!(
            "observer must define at most {} scorers",
            MAX_SCORERS_PER_OBSERVER
        )));
    }
    let mut keys = std::collections::HashSet::new();
    for scorer in scorers {
        if scorer.key.trim().is_empty() {
            anyhow::bail!(BadRequestError::new("scorer key must not be empty"));
        }
        if scorer.key.len() > MAX_SCORER_KEY_BYTES {
            anyhow::bail!(BadRequestError::new(format!(
                "scorer key must be at most {} bytes",
                MAX_SCORER_KEY_BYTES
            )));
        }
        let rule_bytes = serde_json::to_vec(&scorer.method)?.len();
        if rule_bytes > MAX_SCORER_RULE_BYTES {
            anyhow::bail!(BadRequestError::new(format!(
                "scorer rule must be at most {} bytes",
                MAX_SCORER_RULE_BYTES
            )));
        }
        if !keys.insert(&scorer.key) {
            anyhow::bail!(BadRequestError::new(format!(
                "duplicate scorer key '{}'",
                scorer.key
            )));
        }
        if let Some(tools) = &scorer.tools {
            if scorer.scope != ObserverScope::Tool && !tools.is_empty() {
                anyhow::bail!(BadRequestError::new(format!(
                    "scorer '{}': tools applies to tool scope only",
                    scorer.key
                )));
            }
            if tools.len() > MAX_SCORER_TOOLS
                || tools.iter().any(|t| t.trim().is_empty() || t.len() > 128)
            {
                anyhow::bail!(BadRequestError::new(format!(
                    "scorer '{}': tools must be at most {MAX_SCORER_TOOLS} non-empty names",
                    scorer.key
                )));
            }
        }
        match &scorer.method {
            ScorerMethod::Rule { rule } => {
                // file_contains needs the session filesystem, which is not part
                // of the observable trace contract. Reject it for observers.
                if matches!(
                    rule,
                    crate::domains::evals::record::Scorer::FileContains { .. }
                ) {
                    anyhow::bail!(BadRequestError::new(
                        "file_contains scorer is not supported by observers"
                    ));
                }
                rule.validate()
                    .map_err(|reason| anyhow::Error::new(BadRequestError::new(reason)))?;
            }
            ScorerMethod::LlmJudge(judge) => {
                if judge.rubric.trim().is_empty() {
                    anyhow::bail!(BadRequestError::new("llm_judge rubric must not be empty"));
                }
                if !(0.0..=1.0).contains(&judge.pass_threshold) {
                    anyhow::bail!(BadRequestError::new(
                        "llm_judge pass_threshold must be between 0.0 and 1.0"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Validate a pass-rate alert.
fn validate_alert(alert: &ObserverAlert) -> Result<()> {
    if !(0.0..=1.0).contains(&alert.min_pass_rate) {
        anyhow::bail!(BadRequestError::new(
            "alert min_pass_rate must be between 0.0 and 1.0"
        ));
    }
    if !(1..=168).contains(&alert.window_hours) {
        anyhow::bail!(BadRequestError::new(
            "alert window_hours must be between 1 and 168"
        ));
    }
    if alert.min_scored == 0 {
        anyhow::bail!(BadRequestError::new("alert min_scored must be at least 1"));
    }
    Ok(())
}

fn pass_rate(scored: u64, passed: u64) -> Option<f64> {
    (scored > 0).then(|| passed as f64 / scored as f64)
}

fn quality_group(row: ScoreSummaryRow) -> Option<QualityGroup> {
    let scored = row.scored.max(0) as u64;
    let passed = row.passed.max(0) as u64;
    Some(QualityGroup {
        key: row.key?,
        scored,
        passed,
        pass_rate: pass_rate(scored, passed),
        sessions: row.sessions.max(0) as u64,
        trend: Vec::new(),
    })
}

impl ObserverService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// Verify every `llm_judge` scorer that names a model references one the
    /// org can actually use. A judge with no `model_id` falls back to the org
    /// default at scoring time, so only explicit ids are checked here.
    ///
    /// This mirrors runtime resolution (`get_model` is org-scoped and
    /// enabled-only, same as `ProviderResolverService::resolve_model`): without
    /// it a saved observer would point at an inaccessible model and silently
    /// `skip` every score at scoring time instead of failing fast on save.
    async fn validate_model_access(
        &self,
        org_id: i64,
        scorers: &[ObserverScorerConfig],
    ) -> Result<()> {
        for scorer in scorers {
            if let ScorerMethod::LlmJudge(judge) = &scorer.method
                && let Some(model_id) = judge.model_id
                && self.db.get_model(org_id, model_id.uuid()).await?.is_none()
            {
                anyhow::bail!(BadRequestError::new(format!(
                    "scorer '{}' references a model that is not available to this organization",
                    scorer.key
                )));
            }
        }
        Ok(())
    }

    pub async fn create(&self, caller: &Caller, req: CreateObserverRequest) -> Result<Observer> {
        let sampling_rate = req.sampling_rate.unwrap_or(0.1);
        validate(sampling_rate, &req.scorers)?;
        if let Some(alert) = &req.alert {
            validate_alert(alert)?;
        }
        self.validate_model_access(caller.org_id, &req.scorers)
            .await?;

        let active = self.db.count_active_observers(caller.org_id).await?;
        if active >= max_observers_per_org() {
            anyhow::bail!(BadRequestError::new(format!(
                "observer limit reached ({} max per org)",
                max_observers_per_org()
            )));
        }

        let internal_uuid = Uuid::now_v7();
        let public_id = ObserverId::from_uuid(internal_uuid);

        let input = CreateObserverRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            match_config: serde_json::to_value(req.match_config.unwrap_or_default())?,
            sampling_rate,
            scorers: serde_json::to_value(&req.scorers)?,
            alert: req.alert.map(serde_json::to_value).transpose()?,
        };

        let row = self.db.create_observer(caller.org_id, input).await?;
        row_to_observer(row)
    }

    pub async fn get_by_public_id(
        &self,
        caller: &Caller,
        public_id: &str,
    ) -> Result<Option<Observer>> {
        let row = self
            .db
            .get_observer_by_public_id(caller.org_id, public_id)
            .await?;
        match row {
            Some(row) if row.status != "deleted" => Ok(Some(row_to_observer(row)?)),
            _ => Ok(None),
        }
    }

    pub async fn list(&self, caller: &Caller, include_archived: bool) -> Result<Vec<Observer>> {
        let rows = self
            .db
            .list_observers(caller.org_id, include_archived)
            .await?;
        rows.into_iter().map(row_to_observer).collect()
    }

    pub async fn update(
        &self,
        caller: &Caller,
        public_id: &str,
        req: UpdateObserverRequest,
    ) -> Result<Option<Observer>> {
        let existing = self
            .db
            .get_observer_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing.filter(|o| o.status != "deleted") else {
            return Ok(None);
        };

        // Validate the effective configuration after applying the patch.
        let effective_rate = req.sampling_rate.unwrap_or(existing.sampling_rate);
        let effective_scorers: Vec<ObserverScorerConfig> = match &req.scorers {
            Some(scorers) => scorers.clone(),
            None => serde_json::from_value(existing.scorers.clone())?,
        };
        validate(effective_rate, &effective_scorers)?;
        if let Some(Some(alert)) = &req.alert {
            validate_alert(alert)?;
        }
        // Only re-check models when the scorers are actually being changed; an
        // unrelated patch (e.g. pause) must not fail because a previously valid
        // model was later disabled.
        if req.scorers.is_some() {
            self.validate_model_access(caller.org_id, &effective_scorers)
                .await?;
        }

        let status = match req.status {
            Some(status) => {
                let s = status.to_string();
                if !matches!(s.as_str(), "active" | "paused" | "archived") {
                    anyhow::bail!(BadRequestError::new("invalid observer status"));
                }
                Some(s)
            }
            None => None,
        };

        let input = UpdateObserverRow {
            name: req.name,
            description: req.description,
            match_config: req.match_config.map(serde_json::to_value).transpose()?,
            sampling_rate: req.sampling_rate,
            scorers: req.scorers.map(serde_json::to_value).transpose()?,
            status,
            alert: req
                .alert
                .map(|alert| alert.map(serde_json::to_value).transpose())
                .transpose()?,
        };

        let row = self
            .db
            .update_observer(caller.org_id, existing.id, input)
            .await?;
        row.map(row_to_observer).transpose()
    }

    pub async fn delete(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        let existing = self
            .db
            .get_observer_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        self.db.delete_observer(caller.org_id, existing.id).await
    }

    pub async fn list_scores(
        &self,
        caller: &Caller,
        observer_public_id: &str,
        session_id: Option<SessionId>,
        review_queue: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Option<Vec<TraceScore>>> {
        let observer = self
            .db
            .get_observer_by_public_id(caller.org_id, observer_public_id)
            .await?;
        let Some(observer) = observer.filter(|o| o.status != "deleted") else {
            return Ok(None);
        };

        let rows = self
            .db
            .list_trace_scores(
                caller.org_id,
                ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: session_id.map(|s| s.uuid()),
                    review_queue,
                    limit: limit.clamp(1, 500),
                    offset: offset.max(0),
                },
            )
            .await?;
        // The observer's public id is the externally-visible identifier; the
        // trace_scores row only carries the internal observer UUID.
        let observer_public: ObserverId = observer.public_id.parse()?;
        rows.into_iter()
            .map(|row| row_to_trace_score(row, observer_public))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    async fn live_observer(&self, caller: &Caller, public_id: &str) -> Result<Option<ObserverRow>> {
        Ok(self
            .db
            .get_observer_by_public_id(caller.org_id, public_id)
            .await?
            .filter(|o| o.status != "deleted"))
    }

    /// Record a person's verdict on one completed score. `None` when the
    /// observer or a completed score with that id is not found.
    pub async fn review_score(
        &self,
        caller: &Caller,
        observer_public_id: &str,
        score_public_id: &str,
        req: ReviewScoreRequest,
    ) -> Result<Option<TraceScore>> {
        let note = req
            .note
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        if note
            .as_ref()
            .is_some_and(|n| n.chars().count() > MAX_REVIEW_NOTE_CHARS)
        {
            anyhow::bail!(BadRequestError::new(format!(
                "review note must be at most {MAX_REVIEW_NOTE_CHARS} characters"
            )));
        }
        let Some(observer) = self.live_observer(caller, observer_public_id).await? else {
            return Ok(None);
        };
        let review = ScoreReview {
            agrees: req.agrees,
            note,
            reviewed_by: caller.user_id.map(|u| u.to_string()),
            reviewed_at: Some(chrono::Utc::now()),
        };
        let row = self
            .db
            .review_trace_score(
                caller.org_id,
                observer.id,
                score_public_id,
                &serde_json::to_value(&review)?,
            )
            .await?;
        let observer_public: ObserverId = observer.public_id.parse()?;
        row.map(|row| row_to_trace_score(row, observer_public))
            .transpose()
    }

    /// Queue scores for past traffic: the turn ends, tool results and session
    /// idles of the last `since_hours`, matched and sampled exactly as live
    /// traffic is. Slices already scored are not queued twice.
    pub async fn backfill(
        &self,
        caller: &Caller,
        observer_public_id: &str,
        req: BackfillObserverRequest,
    ) -> Result<Option<BackfillResult>> {
        let hours = req.since_hours.unwrap_or(BACKFILL_DEFAULT_HOURS);
        if !(1..=BACKFILL_MAX_HOURS).contains(&hours) {
            anyhow::bail!(BadRequestError::new(format!(
                "since_hours must be between 1 and {BACKFILL_MAX_HOURS}"
            )));
        }
        let limit = req.limit.unwrap_or(BACKFILL_DEFAULT_LIMIT);
        if !(1..=BACKFILL_MAX_LIMIT).contains(&limit) {
            anyhow::bail!(BadRequestError::new(format!(
                "limit must be between 1 and {BACKFILL_MAX_LIMIT}"
            )));
        }
        let Some(observer) = self.live_observer(caller, observer_public_id).await? else {
            return Ok(None);
        };
        if observer.status != "active" {
            anyhow::bail!(BadRequestError::new("only an active observer can backfill"));
        }
        let now = chrono::Utc::now();
        let since = now - chrono::Duration::hours(hours as i64);
        let events = self
            .db
            .observer_backfill_events(caller.org_id, since, limit as i64)
            .await?;
        let sample = req.sample.unwrap_or(true);
        let observers = [observer];
        let mut planned = Vec::new();
        let mut sessions_idled = std::collections::HashSet::new();
        for event in &events {
            let Some(slice) = Slice::from_event(&event.event_type, &event.data) else {
                continue;
            };
            // One session score per session, however often it idled.
            if slice == Slice::Session && !sessions_idled.insert(event.session_id) {
                continue;
            }
            let tags = event.tags.clone().unwrap_or_default();
            let facts = SessionFacts {
                session_id: event.session_id,
                agent_id: event.agent_id,
                harness_id: event.harness_id,
                tags: &tags,
            };
            let mut rows = plan_scores(&observers, &facts, &slice, sample, now);
            // Past sessions have settled already.
            for row in &mut rows {
                row.not_before = None;
            }
            planned.extend(rows);
        }
        let enqueued = if planned.is_empty() {
            0
        } else {
            self.db
                .enqueue_trace_scores(caller.org_id, &planned)
                .await?
        };
        Ok(Some(BackfillResult {
            events_read: events.len() as u64,
            enqueued: enqueued as u64,
        }))
    }

    /// Live quality over the last `days`: totals, and per agent, observer
    /// and day.
    pub async fn summary(
        &self,
        caller: &Caller,
        days: Option<u32>,
        agent_id: Option<Uuid>,
        observer_public_id: Option<&str>,
    ) -> Result<Option<QualitySummary>> {
        let days = days.unwrap_or(SUMMARY_DEFAULT_DAYS);
        if !(1..=SUMMARY_MAX_DAYS).contains(&days) {
            anyhow::bail!(BadRequestError::new(format!(
                "days must be between 1 and {SUMMARY_MAX_DAYS}"
            )));
        }
        let observer_id = match observer_public_id {
            Some(public_id) => match self.live_observer(caller, public_id).await? {
                Some(observer) => Some(observer.id),
                None => return Ok(None),
            },
            None => None,
        };
        let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
        let read = |group| {
            self.db
                .summarize_trace_scores(caller.org_id, since, group, observer_id, agent_id)
        };
        let total = read(ScoreGroup::Total).await?.into_iter().next();
        let groups = |rows: Vec<ScoreSummaryRow>| -> Vec<QualityGroup> {
            rows.into_iter().filter_map(quality_group).collect()
        };
        let mut by_agent = groups(read(ScoreGroup::Agent).await?);
        // Daily rates per agent, keyed `<agent uuid>|<day>`; days sort as text.
        let mut agent_days = groups(read(ScoreGroup::AgentDay).await?);
        agent_days.sort_by(|a, b| a.key.cmp(&b.key));
        for group in &mut by_agent {
            let prefix = format!("{}|", group.key);
            group.trend = agent_days
                .iter()
                .filter(|day| day.key.starts_with(&prefix))
                .filter_map(|day| day.pass_rate)
                .collect();
        }
        // The row carries the internal agent UUID; the API speaks public ids.
        for group in &mut by_agent {
            if let Ok(uuid) = group.key.parse::<Uuid>() {
                group.key = everruns_contracts::typed_id::AgentId::from_uuid(uuid).to_string();
            }
        }
        let mut by_observer = groups(read(ScoreGroup::Observer).await?);
        by_observer.sort_by_key(|g| std::cmp::Reverse(g.scored));
        by_agent.sort_by_key(|g| std::cmp::Reverse(g.scored));
        let mut by_day = groups(read(ScoreGroup::Day).await?);
        by_day.sort_by(|a, b| a.key.cmp(&b.key));
        let (scored, passed, sessions) = total
            .map(|t| {
                (
                    t.scored.max(0) as u64,
                    t.passed.max(0) as u64,
                    t.sessions.max(0) as u64,
                )
            })
            .unwrap_or_default();
        Ok(Some(QualitySummary {
            since,
            days,
            scored,
            passed,
            pass_rate: pass_rate(scored, passed),
            sessions,
            by_agent,
            by_observer,
            by_day,
        }))
    }
}

pub fn row_to_observer(row: ObserverRow) -> Result<Observer> {
    Ok(Observer {
        public_id: row.public_id.parse()?,
        internal_id: row.id,
        org_id: row.org_id,
        name: row.name,
        description: row.description,
        match_config: serde_json::from_value(row.match_config)?,
        sampling_rate: row.sampling_rate,
        scorers: serde_json::from_value(row.scorers)?,
        alert: row.alert.map(serde_json::from_value).transpose()?,
        status: ObserverStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
    })
}

/// Map a stored trace-score row to the API type. `observer_id` is the
/// observer's public id (the row only carries the internal observer UUID),
/// and the score's own public id comes from the stored `public_id` column —
/// neither is derivable from the row's internal UUID.
pub fn row_to_trace_score(row: TraceScoreRow, observer_id: ObserverId) -> Result<TraceScore> {
    Ok(TraceScore {
        public_id: row.public_id.parse()?,
        internal_id: row.id,
        org_id: row.org_id,
        observer_id,
        scorer_key: row.scorer_key,
        session_id: SessionId::from_uuid(row.session_id),
        scope: ObserverScope::from(row.scope.as_str()),
        turn_id: row.turn_id,
        agent_id: row
            .agent_id
            .map(everruns_contracts::typed_id::AgentId::from_uuid),
        harness_id: row
            .harness_id
            .map(everruns_contracts::typed_id::HarnessId::from_uuid),
        status: TraceScoreStatus::from(row.status.as_str()),
        pass: row.pass,
        value: row.value,
        label: row.label,
        reason: row.reason,
        judge_input_tokens: row.judge_input_tokens.map(|t| t as u64),
        judge_output_tokens: row.judge_output_tokens.map(|t| t as u64),
        judge_cost_usd: row.judge_cost_usd,
        error_message: row.error_message,
        review: row.review.and_then(|r| serde_json::from_value(r).ok()),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::observers::record::{ObserverScope, ObserverScorerConfig, ScorerMethod};
    use crate::domains::observers::types::CreateObserverRequest;
    use crate::storage::CreateTraceScoreRow;
    use everruns_contracts::typed_id::TraceScoreId;

    fn contains_scorer(key: impl Into<String>, text: impl Into<String>) -> ObserverScorerConfig {
        ObserverScorerConfig {
            key: key.into(),
            scope: ObserverScope::Turn,
            tools: None,
            method: ScorerMethod::Rule {
                rule: crate::domains::evals::record::Scorer::Contains {
                    text: text.into(),
                    weight: 1.0,
                },
            },
        }
    }

    #[test]
    fn validate_rejects_too_many_scorers() {
        let scorers = (0..=MAX_SCORERS_PER_OBSERVER)
            .map(|i| contains_scorer(format!("k{i}"), "x"))
            .collect::<Vec<_>>();

        let err = validate(1.0, &scorers).unwrap_err().to_string();

        assert!(err.contains("at most"));
        assert!(err.contains(&MAX_SCORERS_PER_OBSERVER.to_string()));
    }

    #[test]
    fn validate_rejects_oversized_scorer_key() {
        let key = "k".repeat(MAX_SCORER_KEY_BYTES + 1);

        let err = validate(1.0, &[contains_scorer(key, "x")])
            .unwrap_err()
            .to_string();

        assert!(err.contains("scorer key must be at most"));
    }

    #[test]
    fn validate_rejects_oversized_scorer_rule() {
        let text = "x".repeat(MAX_SCORER_RULE_BYTES + 1);

        let err = validate(1.0, &[contains_scorer("k", text)])
            .unwrap_err()
            .to_string();

        assert!(err.contains("scorer rule must be at most"));
    }

    // The trace_scores row's internal UUID is DB-generated and independent of
    // the externally-visible public ids, so the API must echo the stored
    // public ids — not ones fabricated from the internal UUID.
    #[tokio::test]
    async fn list_scores_returns_stored_public_ids() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);

        let observer = svc
            .create(
                &caller,
                CreateObserverRequest {
                    name: "o".into(),
                    description: None,
                    match_config: None,
                    sampling_rate: Some(1.0),
                    scorers: vec![contains_scorer("k", "x")],
                    alert: None,
                },
            )
            .await
            .unwrap();

        let score_public = TraceScoreId::from_uuid(Uuid::now_v7());
        db.enqueue_trace_scores(
            1,
            &[CreateTraceScoreRow {
                public_id: score_public.to_string(),
                observer_id: observer.internal_id,
                scorer_key: "k".into(),
                session_id: db.create_test_session().await.uuid(),
                turn_id: "turn_x".into(),
                agent_id: None,
                harness_id: None,
                scope: "turn".to_string(),
                not_before: None,
            }],
        )
        .await
        .unwrap();

        let scores = svc
            .list_scores(&caller, &observer.public_id.to_string(), None, false, 10, 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scores.len(), 1);
        // The score's public id and observer id must match what was stored —
        // not values derived from the internal row UUID.
        assert_eq!(scores[0].public_id, score_public);
        assert_eq!(scores[0].observer_id, observer.public_id);
    }

    use crate::domains::observers::record::LlmJudgeConfig;
    use crate::storage::{CreateModelRow, CreateProviderRow};
    use everruns_contracts::typed_id::ModelId;

    /// Create a model in `org_id` and return its id. `enabled` controls whether
    /// it is usable (disabled models are invisible to `get_model`).
    async fn create_model(db: &StorageBackend, org_id: i64, enabled: bool) -> ModelId {
        let provider = db
            .create_provider(
                org_id,
                CreateProviderRow {
                    name: "p".into(),
                    provider_type: "openai".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        db.create_model(
            org_id,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "gpt-test".into(),
                display_name: "GPT Test".into(),
                capabilities: vec![],
                is_favorite: false,
                enabled,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    fn judge_scorer(key: &str, model_id: Option<ModelId>) -> ObserverScorerConfig {
        ObserverScorerConfig {
            key: key.into(),
            scope: ObserverScope::Turn,
            tools: None,
            method: ScorerMethod::LlmJudge(LlmJudgeConfig {
                rubric: "score the answer".into(),
                model_id,
                pass_threshold: 0.5,
            }),
        }
    }

    fn create_req(scorers: Vec<ObserverScorerConfig>) -> CreateObserverRequest {
        CreateObserverRequest {
            name: "o".into(),
            description: None,
            match_config: None,
            sampling_rate: Some(1.0),
            scorers,
            alert: None,
        }
    }

    #[tokio::test]
    async fn create_accepts_judge_with_accessible_model() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);
        let model = create_model(&db, 1, true).await;

        svc.create(&caller, create_req(vec![judge_scorer("q", Some(model))]))
            .await
            .expect("accessible model should be accepted");
    }

    #[tokio::test]
    async fn create_accepts_judge_without_model() {
        // No explicit model means "use the org default" at scoring time, so it
        // must be accepted even when the org has no model configured here.
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);

        svc.create(&caller, create_req(vec![judge_scorer("q", None)]))
            .await
            .expect("judge without a model should be accepted");
    }

    #[tokio::test]
    async fn create_rejects_judge_with_unknown_model() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);

        let err = svc
            .create(
                &caller,
                create_req(vec![judge_scorer("q", Some(ModelId::new()))]),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available to this organization"), "{err}");
    }

    #[tokio::test]
    async fn create_rejects_judge_with_disabled_model() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);
        let model = create_model(&db, 1, false).await;

        let err = svc
            .create(&caller, create_req(vec![judge_scorer("q", Some(model))]))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available to this organization"), "{err}");
    }

    #[tokio::test]
    async fn create_rejects_model_from_another_org() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        // Model lives in org 2; caller is org 1 and must not be able to use it.
        let other_model = create_model(&db, 2, true).await;
        let caller = Caller::internal(1);

        let err = svc
            .create(
                &caller,
                create_req(vec![judge_scorer("q", Some(other_model))]),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available to this organization"), "{err}");
    }

    #[tokio::test]
    async fn update_rechecks_model_access_when_scorers_change() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);
        let observer = svc
            .create(&caller, create_req(vec![contains_scorer("k", "x")]))
            .await
            .unwrap();

        let err = svc
            .update(
                &caller,
                &observer.public_id.to_string(),
                UpdateObserverRequest {
                    name: None,
                    description: None,
                    match_config: None,
                    sampling_rate: None,
                    scorers: Some(vec![judge_scorer("q", Some(ModelId::new()))]),
                    status: None,
                    alert: None,
                },
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available to this organization"), "{err}");
    }

    async fn observer_on_all_traffic(
        svc: &ObserverService,
        caller: &Caller,
        scorers: Vec<ObserverScorerConfig>,
    ) -> Observer {
        svc.create(
            caller,
            CreateObserverRequest {
                name: "live".into(),
                description: None,
                match_config: None,
                sampling_rate: Some(1.0),
                scorers,
                alert: None,
            },
        )
        .await
        .unwrap()
    }

    async fn seed(
        db: &StorageBackend,
        session: SessionId,
        event_type: &str,
        data: serde_json::Value,
    ) {
        db.create_event(crate::storage::CreateEventRow {
            session_id: session,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    fn scoped(key: &str, scope: ObserverScope) -> ObserverScorerConfig {
        ObserverScorerConfig {
            scope,
            ..contains_scorer(key, "x")
        }
    }

    #[tokio::test]
    async fn backfill_queues_past_slices_once_per_scope() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);
        let observer = observer_on_all_traffic(
            &svc,
            &caller,
            vec![
                scoped("answer", ObserverScope::Turn),
                scoped("convo", ObserverScope::Session),
                scoped("call", ObserverScope::Tool),
            ],
        )
        .await;
        let session = db.create_test_session().await;
        seed(
            &db,
            session,
            "turn.completed",
            serde_json::json!({"turn_id": "turn_1"}),
        )
        .await;
        seed(
            &db,
            session,
            "tool.completed",
            serde_json::json!({"tool_call_id": "call_1", "tool_name": "bash"}),
        )
        .await;
        // Two idles, one conversation score.
        seed(&db, session, "session.idled", serde_json::json!({})).await;
        seed(&db, session, "session.idled", serde_json::json!({})).await;

        let public = observer.public_id.to_string();
        let result = svc
            .backfill(&caller, &public, BackfillObserverRequest::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.events_read, 4);
        assert_eq!(result.enqueued, 3);
        let scores = svc
            .list_scores(&caller, &public, Some(session), false, 10, 0)
            .await
            .unwrap()
            .unwrap();
        let mut keys: Vec<_> = scores
            .iter()
            .map(|s| (s.scorer_key.clone(), s.scope.as_str(), s.turn_id.clone()))
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                ("answer".into(), "turn", "turn_1".into()),
                ("call".into(), "tool", "call_1".into()),
                ("convo".into(), "session", "session".into()),
            ]
        );

        // Turns and calls already queued are not queued again.
        let again = svc
            .backfill(&caller, &public, BackfillObserverRequest::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            again.enqueued, 1,
            "only the conversation score is refreshed"
        );

        let err = svc
            .backfill(
                &caller,
                &public,
                BackfillObserverRequest {
                    since_hours: Some(721),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("since_hours"), "{err}");
    }

    #[tokio::test]
    async fn summary_and_review_read_completed_scores() {
        let db = Arc::new(StorageBackend::test_database());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);
        let agent = db.create_test_agent(1, Uuid::now_v7()).await;
        let observer =
            observer_on_all_traffic(&svc, &caller, vec![contains_scorer("k", "x")]).await;
        let session = db.create_test_session().await;
        let rows: Vec<_> = (0..4)
            .map(|i| CreateTraceScoreRow {
                public_id: TraceScoreId::from_uuid(Uuid::now_v7()).to_string(),
                observer_id: observer.internal_id,
                scorer_key: "k".into(),
                session_id: session.uuid(),
                turn_id: format!("turn_{i}"),
                agent_id: Some(agent),
                harness_id: None,
                scope: "turn".into(),
                not_before: None,
            })
            .collect();
        db.enqueue_trace_scores(1, &rows).await.unwrap();
        let public = observer.public_id.to_string();
        let stored = svc
            .list_scores(&caller, &public, None, false, 10, 0)
            .await
            .unwrap()
            .unwrap();
        // Three pass, one fails.
        for (i, score) in stored.iter().enumerate() {
            db.complete_trace_score(
                score.internal_id,
                crate::storage::CompleteTraceScoreRow {
                    status: "completed".into(),
                    pass: Some(i != 0),
                    value: Some(if i == 0 { 0.0 } else { 1.0 }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let failed = stored[0].public_id.to_string();

        let summary = svc
            .summary(&caller, None, None, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (summary.scored, summary.passed, summary.sessions),
            (4, 3, 1)
        );
        assert_eq!(summary.pass_rate, Some(0.75));
        let agent_public = everruns_contracts::typed_id::AgentId::from_uuid(agent).to_string();
        assert_eq!(summary.by_agent.len(), 1);
        assert_eq!(summary.by_agent[0].key, agent_public);
        assert_eq!(summary.by_agent[0].trend, vec![0.75]);
        assert_eq!(summary.by_observer[0].key, public);
        assert_eq!(summary.by_day.len(), 1);
        let filtered = svc
            .summary(&caller, Some(7), Some(Uuid::now_v7()), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(filtered.scored, 0);
        assert_eq!(filtered.pass_rate, None);
        assert!(svc.summary(&caller, Some(91), None, None).await.is_err());

        // The review queue leads with the failure.
        let queue = svc
            .list_scores(&caller, &public, None, true, 10, 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queue.len(), 4);
        assert_eq!(queue[0].public_id.to_string(), failed);

        let reviewed = svc
            .review_score(
                &caller,
                &public,
                &failed,
                ReviewScoreRequest {
                    agrees: false,
                    note: Some("  It did answer.  ".into()),
                },
            )
            .await
            .unwrap()
            .unwrap();
        let review = reviewed.review.unwrap();
        assert!(!review.agrees);
        assert_eq!(review.note.as_deref(), Some("It did answer."));
        assert!(review.reviewed_at.is_some());
        let queue = svc
            .list_scores(&caller, &public, None, true, 10, 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queue.len(), 3, "a reviewed score leaves the queue");

        let missing = svc
            .review_score(
                &caller,
                &public,
                "score_00000000000000000000000000000000",
                ReviewScoreRequest {
                    agrees: true,
                    note: None,
                },
            )
            .await
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn validate_rejects_tools_outside_tool_scope_and_bad_alerts() {
        let mut scorer = contains_scorer("k", "x");
        scorer.tools = Some(vec!["bash".into()]);
        let err = validate(1.0, &[scorer.clone()]).unwrap_err().to_string();
        assert!(err.contains("tool scope only"), "{err}");
        scorer.scope = ObserverScope::Tool;
        assert!(validate(1.0, &[scorer]).is_ok());

        let alert = |min_pass_rate, window_hours, min_scored| ObserverAlert {
            min_pass_rate,
            window_hours,
            min_scored,
        };
        assert!(validate_alert(&alert(0.8, 24, 20)).is_ok());
        assert!(validate_alert(&alert(1.2, 24, 20)).is_err());
        assert!(validate_alert(&alert(0.8, 0, 20)).is_err());
        assert!(validate_alert(&alert(0.8, 169, 20)).is_err());
        assert!(validate_alert(&alert(0.8, 24, 0)).is_err());
    }
}
