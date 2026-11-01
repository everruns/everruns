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

/// The trace slice a grader sees.
#[derive(Debug, Clone, Copy, Default)]
pub struct GradeInput<'a> {
    /// The user message being answered (the last one in the slice).
    pub input_message: &'a str,
    /// What the agent answered.
    pub final_answer: &'a str,
    /// Citation annotations on the final answer.
    pub annotations: &'a [TextAnnotation],
    /// Names of the tools called, in order.
    pub tool_calls: &'a [String],
    /// Argument JSON of each call in `tool_calls`, same order. Shorter (or
    /// empty) where arguments are not part of the trace; a call with no
    /// recorded arguments never matches `arguments_contain`.
    pub tool_arguments: &'a [String],
    /// The whole conversation as "User: ... / Agent: ..." text, for multistep
    /// cases. Empty for a single exchange; judges and decisions then see only
    /// `input_message` and `final_answer`.
    pub transcript: &'a str,
    /// Turns (eval case) or iterations (observer turn).
    pub turns: u32,
    /// Input plus output tokens the agent spent.
    pub tokens: u64,
    /// Session whose files `file_contains` reads. `None` where session files
    /// are not part of the trace (observers).
    pub session_id: Option<Uuid>,
}

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
}

impl Graders {
    /// Rules only: model-backed graders report themselves unavailable.
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            judge: None,
            decisions: None,
            simulator: None,
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
            .with_simulator(Arc::new(LlmUserSimulator::new(client)))
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
        let pass = match self.db.get_session_file(session_id, path).await {
            Ok(Some(file)) => file
                .content
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .is_some_and(|content| content.contains(text)),
            _ => false,
        };
        Score {
            pass,
            value: if pass { 1.0 } else { 0.0 },
            reason: if pass {
                format!("File '{path}' contains '{text}'")
            } else {
                format!("File '{path}' does not contain '{text}' or not found")
            },
        }
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

fn binary(pass: bool, reason: String) -> Score {
    Score {
        pass,
        value: if pass { 1.0 } else { 0.0 },
        reason,
    }
}

/// Grade the pure rules. `None` for scorers that need more than the trace.
pub fn score_rule(scorer: &Scorer, input: &GradeInput<'_>) -> Option<Score> {
    let answer = input.final_answer;
    let tool_calls = input.tool_calls;
    let score = match scorer {
        Scorer::Contains { text, .. } => {
            let pass = answer.contains(text.as_str());
            binary(
                pass,
                if pass {
                    format!("Output contains '{text}'")
                } else {
                    format!("Output does not contain '{text}'")
                },
            )
        }
        Scorer::NotContains { text, .. } => {
            let pass = !answer.contains(text.as_str());
            binary(
                pass,
                if pass {
                    format!("Output does not contain '{text}'")
                } else {
                    format!("Output contains '{text}'")
                },
            )
        }
        Scorer::Regex { pattern, .. } => match regex::Regex::new(pattern) {
            Ok(re) => {
                let pass = re.is_match(answer);
                binary(
                    pass,
                    if pass {
                        format!("Output matches pattern '{pattern}'")
                    } else {
                        format!("Output does not match pattern '{pattern}'")
                    },
                )
            }
            Err(error) => binary(false, format!("Invalid pattern '{pattern}': {error}")),
        },
        Scorer::ToolCalled {
            tool,
            min,
            arguments_contain,
            ..
        } => {
            let count = tool_calls
                .iter()
                .enumerate()
                .filter(|(_, t)| *t == tool)
                .filter(|(index, _)| match arguments_contain {
                    None => true,
                    Some(text) => input
                        .tool_arguments
                        .get(*index)
                        .is_some_and(|args| args.contains(text.as_str())),
                })
                .count() as u32;
            let with = arguments_contain
                .as_ref()
                .map(|text| format!(" with arguments containing '{text}'"))
                .unwrap_or_default();
            binary(
                count >= *min,
                format!("Tool '{tool}' called{with} {count} times (min: {min})"),
            )
        }
        Scorer::ToolNotCalled { tool, .. } => {
            let pass = !tool_calls.iter().any(|t| t == tool);
            binary(
                pass,
                if pass {
                    format!("Tool '{tool}' was not called")
                } else {
                    format!("Tool '{tool}' was called")
                },
            )
        }
        Scorer::ToolCallCount { min, max, .. } => {
            let count = tool_calls.len() as u32;
            let pass = min.is_none_or(|m| count >= m) && max.is_none_or(|m| count <= m);
            let bound = |v: &Option<u32>| v.map(|v| v.to_string()).unwrap_or("-".into());
            binary(
                pass,
                format!(
                    "Total tool calls: {count} (min: {}, max: {})",
                    bound(min),
                    bound(max)
                ),
            )
        }
        Scorer::TurnsWithin { max, .. } => binary(
            input.turns <= *max,
            format!("Turns: {} (max: {max})", input.turns),
        ),
        Scorer::TokensWithin { max, .. } => binary(
            input.tokens <= *max,
            format!("Tokens: {} (max: {max})", input.tokens),
        ),
        Scorer::JsonSchema { schema, .. } => score_json_schema(answer, schema),
        Scorer::FileContains { .. }
        | Scorer::CitationFaithful { .. }
        | Scorer::CitationJudged { .. }
        | Scorer::Judge { .. }
        | Scorer::Decision { .. } => return None,
    };
    Some(score)
}

/// The answer must parse as JSON (optionally inside a ```json fence) and
/// validate against `schema`.
fn score_json_schema(answer: &str, schema: &serde_json::Value) -> Score {
    let trimmed = answer.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return binary(false, "Output is not valid JSON".to_string());
    };
    let validator = match jsonschema::validator_for(schema) {
        Ok(validator) => validator,
        Err(error) => return binary(false, format!("Scorer schema is invalid: {error}")),
    };
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .take(3)
        .map(|error| format!("{} at '{}'", error, error.instance_path()))
        .collect();
    if errors.is_empty() {
        binary(true, "Output matches the JSON schema".to_string())
    } else {
        binary(
            false,
            format!(
                "Output does not match the JSON schema: {}",
                errors.join("; ")
            ),
        )
    }
}

// ============================================
// Citations
// ============================================

/// Grade citation faithfulness from the answer's annotations: coverage (enough
/// citations) plus faithfulness (fraction verified `entailed`). Verdicts are
/// stamped by the `citation_verification` capability; without it, citations
/// are unverified and score 0, and the reason says so.
fn score_citation_faithful(
    annotations: &[TextAnnotation],
    min_citations: u32,
    pass_threshold: f64,
) -> Score {
    let total = annotations.len();
    if total < min_citations as usize {
        return binary(false, format!("{total} citations (min {min_citations})"));
    }
    if total == 0 {
        // min_citations is 0 here, so coverage is satisfied vacuously.
        return binary(true, "no citations required or present".to_string());
    }
    let with_verdict = annotations.iter().filter(|a| a.verified.is_some()).count();
    let entailed = annotations
        .iter()
        .filter(|a| {
            a.verified
                .as_ref()
                .is_some_and(|v| v.status == VerificationStatus::Entailed)
        })
        .count();
    let fraction = entailed as f64 / total as f64;
    let reason = if with_verdict == 0 {
        format!(
            "{entailed}/{total} citations entailed, none verified (enable citation_verification)"
        )
    } else {
        format!(
            "{entailed}/{total} citations entailed ({with_verdict} verified); \
             faithfulness {fraction:.2} (threshold {pass_threshold:.2})"
        )
    };
    Score {
        pass: fraction >= pass_threshold,
        value: fraction,
        reason,
    }
}

const CITATION_JUDGE_RUBRIC: &str = "You are judging citation faithfulness. Each numbered item is a CLAIM the agent \
     made and the SOURCE it cited. Decide whether each SOURCE actually supports its \
     CLAIM. `value` is the fraction of items whose source supports the claim \
     (1.0 = all supported, 0.0 = none). Penalize claims whose source is unrelated \
     or contradicts them.";

/// Render each annotation as a numbered CLAIM/SOURCE pair for the judge. The
/// claim is the answer span the citation is attached to; the source is the
/// cited snippet (and uri).
fn build_citation_pairs(text: &str, annotations: &[TextAnnotation]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for (i, ann) in annotations.iter().enumerate() {
        let claim: String = chars
            .get(ann.start..ann.end.min(chars.len()))
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let source = ann.source.snippet.as_deref().unwrap_or("(no snippet)");
        out.push_str(&format!(
            "[{}] CLAIM: {}\n    SOURCE ({}): {}\n",
            i + 1,
            claim.trim(),
            ann.source.uri,
            source.trim(),
        ));
    }
    out
}

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
