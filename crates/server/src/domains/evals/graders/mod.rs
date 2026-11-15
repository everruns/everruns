// Graders: the one scoring entry point shared by eval runs and observers
// (knowledge/evaluation/evals.md, knowledge/evaluation/online-evals.md).
//
// Decision: eval cases and observer scorers grade with the same `Scorer`
// vocabulary through `Graders::grade`. Before this, the eval runner and the
// observer worker each carried their own copy of the model-backed scorers, so
// a scorer added to one silently did nothing in the other.
//
// Decision: three kinds of grader live here.
// - Rules (contains, regex, tool use, token and turn bounds, JSON schema) are
//   pure functions of the trace.
// - `judge` asks the org's own chat model to grade against a rubric and
//   returns a 0..1 value with reasoning.
// - `decision` asks a decision model one yes/no question about the answer and
//   uses the probability of yes as the value. Decision models are the
//   default choice for checks that have a yes/no shape: they are cheaper,
//   return a typed answer with no parsing, and report whether the
//   probability is calibrated.
//
// Decision: which decision model answers follows the org's `system_decisions`
// setting, the same rule the Slack relevance check and tool label
// suggestions follow. An org that selects its own model is graded only by it,
// never by the deployment's service as a fallback (THREAT[TM-LLM-037]).
//
// Decision: a grader that cannot run reports why instead of guessing.
// `GradeError::Unavailable` is a lasting condition (no model configured, no
// session files to read): eval runs record it as a failing score with the
// reason, observers mark the score skipped. `GradeError::Failed` is a
// provider or parsing failure: observers retry it.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::connection_services::{DecisionModelBinding, SystemDecisionModel};
#[cfg(test)]
use everruns_core::message::{TextAnnotation, VerificationStatus};
use everruns_core::{DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionsService};
use serde_json::json;
use uuid::Uuid;

use crate::domains::evals::multistep::{LlmUserSimulator, UserSimulator};
use crate::domains::evals::record::{Score, Scorer};
use crate::domains::observers::judge::{JudgeClient, LlmJudgeClient, TurnEvidence};
use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;

/// Request metadata naming this caller, for attribution.
const DECISION_PURPOSE: &str = "evals.grader";

pub use everruns_core::evaluation::GradeInput;

/// Model usage of one grade, for accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl GraderUsage {
    pub fn add(&mut self, other: &GraderUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost_usd = match (self.cost_usd, other.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
    }
}

/// One grade: the score plus what grading it cost.
#[derive(Debug, Clone)]
pub struct Graded {
    pub score: Score,
    /// Short category from a judge, when it gave one.
    pub label: Option<String>,
    /// `None` for rules, which use no model.
    pub usage: Option<GraderUsage>,
}

impl From<Score> for Graded {
    fn from(score: Score) -> Self {
        Self {
            score,
            label: None,
            usage: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GradeError {
    /// The grader cannot run here, and retrying will not change that.
    #[error("{0}")]
    Unavailable(String),
    /// The grader failed in a way a retry may fix.
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}

impl GradeError {
    /// The failing score an eval case records in place of a grade.
    pub fn into_score(self) -> Score {
        Score {
            pass: false,
            value: 0.0,
            reason: match self {
                GradeError::Unavailable(reason) => reason,
                GradeError::Failed(error) => format!("grader error: {error}"),
            },
        }
    }
}

/// Finds the decision model that answers for an org.
#[async_trait]
pub trait DecisionResolver: Send + Sync {
    async fn for_org(&self, org_id: i64) -> Result<Arc<dyn DecisionsService>, GradeError>;
}

/// Shared scoring services. Cheap to clone.
#[derive(Clone)]
pub struct Graders {
    db: Arc<StorageBackend>,
    judge: Option<Arc<dyn JudgeClient>>,
    decisions: Option<Arc<dyn DecisionResolver>>,
    /// Plays the user in multistep eval cases, on the same org models the
    /// judge uses. Lives here so every holder of graders can simulate.
    simulator: Option<Arc<dyn UserSimulator>>,
    /// Writes findings up (detection layer 3), on the same org models.
    inspector: Option<Arc<dyn crate::domains::health_issues::inspector::InspectorModel>>,
    /// Proposes eval cases (the Eval Builder), on the same org models.
    proposer: Option<Arc<dyn crate::domains::evals::builder::CaseProposer>>,
}

impl Graders {
    /// Rules only: model-backed graders report themselves unavailable.
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            judge: None,
            decisions: None,
            simulator: None,
            inspector: None,
            proposer: None,
        }
    }

    /// The production graders: the org's chat models judge, and decision
    /// questions go to the decision model the org's settings select.
    pub fn for_host(
        db: Arc<StorageBackend>,
        host: &everruns_core::host::HostComposition,
        driver_registry: Arc<crate::kernel_imports::contracts::driver_registry::DriverRegistry>,
        provider_resolver: Arc<ProviderResolverService>,
    ) -> Self {
        let client = Arc::new(LlmJudgeClient::new(
            db.clone(),
            driver_registry,
            provider_resolver.clone(),
        ));
        Self::new(db.clone())
            .with_judge(client.clone())
            .with_simulator(Arc::new(LlmUserSimulator::new(client.clone())))
            .with_inspector(Arc::new(
                crate::domains::health_issues::inspector::LlmInspector::new(client.clone()),
            ))
            .with_proposer(Arc::new(
                crate::domains::evals::builder::LlmCaseProposer::new(client),
            ))
            .with_decisions(Arc::new(HostDecisions {
                db,
                deployment: host.decisions(),
                provider_resolver,
                egress: host.egress_service(),
            }))
    }

    pub fn with_judge(mut self, judge: Arc<dyn JudgeClient>) -> Self {
        self.judge = Some(judge);
        self
    }

    pub fn with_simulator(mut self, simulator: Arc<dyn UserSimulator>) -> Self {
        self.simulator = Some(simulator);
        self
    }

    /// The simulated-user player, `None` without a model stack.
    pub fn simulator(&self) -> Option<&Arc<dyn UserSimulator>> {
        self.simulator.as_ref()
    }

    pub fn with_inspector(
        mut self,
        inspector: Arc<dyn crate::domains::health_issues::inspector::InspectorModel>,
    ) -> Self {
        self.inspector = Some(inspector);
        self
    }

    /// The finding writer, `None` without a model stack.
    pub fn inspector(
        &self,
    ) -> Option<Arc<dyn crate::domains::health_issues::inspector::InspectorModel>> {
        self.inspector.clone()
    }

    pub fn with_proposer(
        mut self,
        proposer: Arc<dyn crate::domains::evals::builder::CaseProposer>,
    ) -> Self {
        self.proposer = Some(proposer);
        self
    }

    /// The Eval Builder's model, `None` without a model stack.
    pub fn proposer(&self) -> Option<Arc<dyn crate::domains::evals::builder::CaseProposer>> {
        self.proposer.clone()
    }

    pub fn with_decisions(mut self, decisions: Arc<dyn DecisionResolver>) -> Self {
        self.decisions = Some(decisions);
        self
    }

    /// Who answers decision questions for an org, when the host has one.
    pub fn decisions(&self) -> Option<Arc<dyn DecisionResolver>> {
        self.decisions.clone()
    }

    pub fn judge(&self) -> Option<&Arc<dyn JudgeClient>> {
        self.judge.as_ref()
    }

    /// Grade `input` with `scorer` for `org_id`.
    pub async fn grade(
        &self,
        org_id: i64,
        scorer: &Scorer,
        input: &GradeInput<'_>,
    ) -> Result<Graded, GradeError> {
        if let Some(score) = score_rule(scorer, input) {
            return Ok(score.into());
        }
        match scorer {
            Scorer::FileContains { path, text, .. } => {
                let session_id = input.session_id.ok_or_else(|| {
                    GradeError::Unavailable("file_contains needs session files".to_string())
                })?;
                Ok(self.file_contains(session_id, path, text).await.into())
            }
            Scorer::CitationFaithful {
                min_citations,
                pass_threshold,
                ..
            } => Ok(
                score_citation_faithful(input.annotations, *min_citations, *pass_threshold).into(),
            ),
            Scorer::CitationJudged {
                rubric,
                model_id,
                pass_threshold,
                ..
            } => {
                if input.annotations.is_empty() {
                    return Ok(Score {
                        pass: true,
                        value: 1.0,
                        reason: "no citations to judge".to_string(),
                    }
                    .into());
                }
                let evidence = TurnEvidence {
                    input_message: String::new(),
                    final_answer: build_citation_pairs(input.final_answer, input.annotations),
                    tool_names: Vec::new(),
                };
                let rubric = rubric.as_deref().unwrap_or(CITATION_JUDGE_RUBRIC);
                self.run_judge(org_id, *model_id, rubric, &evidence, *pass_threshold)
                    .await
            }
            Scorer::Judge {
                rubric,
                model_id,
                pass_threshold,
                ..
            } => {
                // A multistep case shows the judge the whole conversation in
                // place of the last user message.
                let input_message = if input.transcript.is_empty() {
                    input.input_message.to_string()
                } else {
                    format!("Full conversation:\n\n{}", input.transcript)
                };
                let evidence = TurnEvidence {
                    input_message,
                    final_answer: input.final_answer.to_string(),
                    tool_names: input.tool_calls.to_vec(),
                };
                self.run_judge(org_id, *model_id, rubric, &evidence, *pass_threshold)
                    .await
            }
            Scorer::Decision {
                question,
                yes,
                no,
                pass_threshold,
                ..
            } => {
                self.run_decision(org_id, question, yes, no, *pass_threshold, input)
                    .await
            }
            // score_rule answered every other variant.
            _ => Err(GradeError::Unavailable(format!(
                "scorer {} is not supported here",
                scorer.kind()
            ))),
        }
    }

    async fn file_contains(&self, session_id: Uuid, path: &str, text: &str) -> Score {
        let file = self
            .db
            .get_session_file(session_id, path)
            .await
            .ok()
            .flatten();
        let content = file
            .as_ref()
            .and_then(|file| file.content.as_deref())
            .and_then(|bytes| std::str::from_utf8(bytes).ok());
        everruns_core::evaluation::score_file_contains(path, text, content)
    }

    async fn run_judge(
        &self,
        org_id: i64,
        model_id: Option<everruns_contracts::typed_id::ModelId>,
        rubric: &str,
        evidence: &TurnEvidence,
        pass_threshold: f64,
    ) -> Result<Graded, GradeError> {
        let judge = self.judge.as_ref().ok_or_else(|| {
            GradeError::Unavailable("judge unavailable (no model provider)".to_string())
        })?;
        let result = judge
            .judge(org_id, model_id, rubric, evidence)
            .await?
            .ok_or_else(|| {
                GradeError::Unavailable("no judge model configured for org".to_string())
            })?;
        Ok(Graded {
            score: Score {
                pass: result.value >= pass_threshold,
                value: result.value,
                reason: result.reasoning,
            },
            label: result.label,
            usage: Some(GraderUsage {
                input_tokens: result.input_tokens.unwrap_or_default(),
                output_tokens: result.output_tokens.unwrap_or_default(),
                cost_usd: result.cost_usd,
            }),
        })
    }

    async fn run_decision(
        &self,
        org_id: i64,
        question: &str,
        yes: &Option<String>,
        no: &Option<String>,
        pass_threshold: f64,
        input: &GradeInput<'_>,
    ) -> Result<Graded, GradeError> {
        let resolver = self.decisions.as_ref().ok_or_else(|| {
            GradeError::Unavailable("decision model unavailable on this deployment".to_string())
        })?;
        let service = resolver.for_org(org_id).await?;
        let request = decision_request(question, yes, no, input);
        let outcome = service
            .evaluate(request)
            .await
            .map_err(|error| GradeError::Failed(anyhow::anyhow!("{error}")))?;
        decision_grade(&outcome, pass_threshold)
    }
}

/// The decision request for one `decision` scorer: the answer is the state,
/// the scorer's question the only question.
fn decision_request(
    question: &str,
    yes: &Option<String>,
    no: &Option<String>,
    input: &GradeInput<'_>,
) -> DecisionRequest {
    let mut state = json!({
        "user_message": truncate(input.input_message, 4000),
        "agent_answer": truncate(input.final_answer, 8000),
        "tools_used": input.tool_calls,
    });
    if !input.transcript.is_empty() {
        state["conversation"] = json!(truncate(input.transcript, 12000));
    }
    DecisionRequest::new(state)
        .ask(
            "grade",
            DecisionQuestion::Noul {
                instructions: question.to_string(),
                yes: yes.clone(),
                no: no.clone(),
            },
        )
        .with_metadata("purpose", DECISION_PURPOSE)
}

fn decision_grade(outcome: &DecisionOutcome, pass_threshold: f64) -> Result<Graded, GradeError> {
    let probability = outcome
        .get("grade")
        .and_then(|answer| answer.probability_yes())
        .ok_or_else(|| {
            GradeError::Failed(anyhow::anyhow!("decision model returned no yes/no answer"))
        })?;
    let how = if outcome.is_calibrated() {
        "calibrated"
    } else {
        "label only, not calibrated"
    };
    Ok(Graded {
        score: Score {
            pass: probability >= pass_threshold,
            value: probability,
            reason: format!(
                "{} answered yes with probability {probability:.2} ({how}; pass at {pass_threshold:.2})",
                outcome.model
            ),
        },
        label: None,
        usage: Some(GraderUsage {
            input_tokens: outcome.usage.input_tokens,
            output_tokens: outcome.usage.output_tokens,
            cost_usd: outcome.attribution.as_ref().and_then(|a| a.cost_usd),
        }),
    })
}

/// Decision models as the host provides them: the org's own default when its
/// settings select one, otherwise the deployment's service.
struct HostDecisions {
    db: Arc<StorageBackend>,
    deployment: Arc<dyn DecisionsService>,
    provider_resolver: Arc<ProviderResolverService>,
    egress: Arc<dyn everruns_core::EgressService>,
}

#[async_trait]
impl DecisionResolver for HostDecisions {
    async fn for_org(&self, org_id: i64) -> Result<Arc<dyn DecisionsService>, GradeError> {
        let org_selected = self
            .db
            .get_organization_settings(org_id)
            .await?
            .is_some_and(|settings| {
                crate::storage::SystemDecisions::from_db(&settings.system_decisions)
                    == crate::storage::SystemDecisions::Organization
            });
        if !org_selected {
            if !self.deployment.is_configured() {
                return Err(GradeError::Unavailable(
                    "decision scorers need a decision service, which this deployment does not \
                     configure"
                        .to_string(),
                ));
            }
            return Ok(self.deployment.clone());
        }
        // No session is billed here: personal providers fail closed and usage
        // goes to the structured log (THREAT[TM-LLM-049]).
        match self
            .provider_resolver
            .resolve_system_decision_model(org_id, None)
            .await
            .map_err(|error| {
                GradeError::Unavailable(format!(
                    "this organization's decision model is not usable: {error}"
                ))
            })? {
            SystemDecisionModel::Organization(binding) => Ok(Arc::new(OrgDecisionModel {
                binding,
                egress: self.egress.clone(),
            })),
            SystemDecisionModel::Deployment => Err(GradeError::Unavailable(
                "the organization no longer selects its own decision model".to_string(),
            )),
        }
    }
}

/// The org's decision default behind the deployment service's interface.
struct OrgDecisionModel {
    binding: DecisionModelBinding,
    egress: Arc<dyn everruns_core::EgressService>,
}

#[async_trait]
impl DecisionsService for OrgDecisionModel {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        everruns_integrations::typesafe::evaluate_unmetered(
            &self.binding,
            request,
            self.egress.as_ref(),
        )
        .await
        .map_err(everruns_contracts::error::AgentLoopError::tool)
    }
}

// ============================================
// Rules
// ============================================

use everruns_core::evaluation::score_citation_faithful;
pub use everruns_core::evaluation::score_rule;
// ============================================
// Citations
// ============================================

use everruns_core::evaluation::{CITATION_JUDGE_RUBRIC, build_citation_pairs};

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}

#[cfg(test)]
mod tests;
