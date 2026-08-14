// Observer service — CRUD over observers and read access to trace scores.
//
// Decision: Observers reuse agent-management permissions; scores read also
// requires session-management because scores expose production content.
// Decision: The service is stateless (db only); scoring happens in the
// background worker (worker.rs), triggered by the listener (listener.rs).

use crate::api::observers::{CreateObserverRequest, UpdateObserverRequest};
use crate::errors::BadRequestError;
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateObserverRow, ListTraceScoresParams, ObserverRow, TraceScoreRow, UpdateObserverRow,
};
use anyhow::Result;
use everruns_core::{Caller, Permission, Policy, Rule};
use everruns_platform::observer::*;
use everruns_provider::typed_id::{ObserverId, SessionId};
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
        match &scorer.method {
            ScorerMethod::Rule { rule } => {
                // file_contains needs the session filesystem, which is not part
                // of the observable trace contract. Reject it for observers.
                if matches!(rule, everruns_platform::eval::Scorer::FileContains { .. }) {
                    anyhow::bail!(BadRequestError::new(
                        "file_contains scorer is not supported by observers"
                    ));
                }
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
        turn_id: row.turn_id,
        agent_id: row
            .agent_id
            .map(everruns_provider::typed_id::AgentId::from_uuid),
        agent_version_id: row
            .agent_version_id
            .map(everruns_provider::typed_id::AgentVersionId::from_uuid),
        harness_id: row
            .harness_id
            .map(everruns_provider::typed_id::HarnessId::from_uuid),
        status: TraceScoreStatus::from(row.status.as_str()),
        pass: row.pass,
        value: row.value,
        label: row.label,
        reason: row.reason,
        judge_input_tokens: row.judge_input_tokens.map(|t| t as u64),
        judge_output_tokens: row.judge_output_tokens.map(|t| t as u64),
        judge_cost_usd: row.judge_cost_usd,
        error_message: row.error_message,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::observers::CreateObserverRequest;
    use crate::storage::models::CreateTraceScoreRow;
    use everruns_platform::observer::{ObserverScope, ObserverScorerConfig, ScorerMethod};
    use everruns_provider::typed_id::TraceScoreId;

    fn contains_scorer(key: impl Into<String>, text: impl Into<String>) -> ObserverScorerConfig {
        ObserverScorerConfig {
            key: key.into(),
            scope: ObserverScope::Turn,
            method: ScorerMethod::Rule {
                rule: everruns_platform::eval::Scorer::Contains {
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
        let db = Arc::new(StorageBackend::in_memory());
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
                session_id: Uuid::now_v7(),
                turn_id: "turn_x".into(),
                agent_id: None,
                agent_version_id: None,
                harness_id: None,
            }],
        )
        .await
        .unwrap();

        let scores = svc
            .list_scores(&caller, &observer.public_id.to_string(), None, 10, 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scores.len(), 1);
        // The score's public id and observer id must match what was stored —
        // not values derived from the internal row UUID.
        assert_eq!(scores[0].public_id, score_public);
        assert_eq!(scores[0].observer_id, observer.public_id);
    }

    use crate::storage::models::{CreateModelRow, CreateProviderRow};
    use everruns_platform::observer::LlmJudgeConfig;
    use everruns_provider::typed_id::ModelId;

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
        }
    }

    #[tokio::test]
    async fn create_accepts_judge_with_accessible_model() {
        let db = Arc::new(StorageBackend::in_memory());
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
        let db = Arc::new(StorageBackend::in_memory());
        let svc = ObserverService::new(db.clone());
        let caller = Caller::internal(1);

        svc.create(&caller, create_req(vec![judge_scorer("q", None)]))
            .await
            .expect("judge without a model should be accepted");
    }

    #[tokio::test]
    async fn create_rejects_judge_with_unknown_model() {
        let db = Arc::new(StorageBackend::in_memory());
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
        let db = Arc::new(StorageBackend::in_memory());
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
        let db = Arc::new(StorageBackend::in_memory());
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
        let db = Arc::new(StorageBackend::in_memory());
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
                },
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available to this organization"), "{err}");
    }
}
