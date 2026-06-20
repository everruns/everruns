//! [`Runner`]: expands `evals × models × samples` into cases, applies selective
//! filtering, runs each case through its subject + scorers, and collects a
//! [`RunReport`].
//!
//! Selection mirrors `cargo test`: a free-text `filter` is a substring match on
//! the case key `eval/sample@model`, and `--tag` narrows by sample tag. In the
//! macro-based design this filtering is delegated to the test harness; here it
//! is explicit so the prototype is self-contained.

use crate::eval::Eval;
use crate::subject::ModelSpec;
use crate::{RunCx, Sample, Score, Transcript};

/// Run a single matrix cell: one sample, one model. Shared by the in-process
/// [`Runner`] and the protocol [`server`](crate::server) so both paths score
/// identically.
pub async fn run_cell(eval: &Eval, sample: &Sample, model: &ModelSpec) -> CaseOutcome {
    let cx = RunCx {
        model: model.clone(),
        max_turns: eval.max_turns,
    };
    let transcript = eval.subject.run(sample, &cx).await;

    let mut scores = Vec::with_capacity(eval.scorers.len());
    for scorer in &eval.scorers {
        scores.push(scorer.score(sample, &transcript).await);
    }

    let passed = !scores.is_empty() && scores.iter().all(|s| s.pass);
    let aggregate = if scores.is_empty() {
        0.0
    } else {
        scores.iter().map(|s| s.value).sum::<f64>() / scores.len() as f64
    };

    CaseOutcome {
        eval: eval.name.clone(),
        sample_id: sample.id.clone(),
        model: model.label.clone(),
        scores,
        passed,
        aggregate,
        transcript,
    }
}

/// The result of one matrix cell: one sample, one model.
#[derive(Clone, Debug)]
pub struct CaseOutcome {
    pub eval: String,
    pub sample_id: String,
    pub model: String,
    pub scores: Vec<Score>,
    pub passed: bool,
    pub aggregate: f64,
    pub transcript: Transcript,
}

impl CaseOutcome {
    pub fn key(&self) -> String {
        format!("{}/{}@{}", self.eval, self.sample_id, self.model)
    }
}

/// Aggregate result of a run.
#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub outcomes: Vec<CaseOutcome>,
    pub skipped: Vec<String>,
}

impl RunReport {
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }
    pub fn passed(&self) -> usize {
        self.outcomes.iter().filter(|o| o.passed).count()
    }
    pub fn failed(&self) -> usize {
        self.total() - self.passed()
    }
    pub fn all_passed(&self) -> bool {
        self.failed() == 0
    }
}

/// Runs a suite of [`Eval`]s with optional selection.
#[derive(Default)]
pub struct Runner {
    evals: Vec<Eval>,
    filter: Option<String>,
    tag: Option<String>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, eval: Eval) -> Self {
        self.evals.push(eval);
        self
    }

    /// Substring filter on the case key `eval/sample@model` (like `cargo test PAT`).
    pub fn filter(mut self, filter: Option<String>) -> Self {
        self.filter = filter;
        self
    }

    /// Only run samples carrying this tag.
    pub fn tag(mut self, tag: Option<String>) -> Self {
        self.tag = tag;
        self
    }

    pub async fn run(&self) -> RunReport {
        let mut report = RunReport::default();

        for eval in &self.evals {
            for model in &eval.models {
                for sample in &eval.dataset.samples {
                    let key = format!("{}/{}@{}", eval.name, sample.id, model.label);

                    if let Some(f) = &self.filter
                        && !key.contains(f.as_str())
                    {
                        continue;
                    }
                    if let Some(tag) = &self.tag
                        && !sample.tags.iter().any(|t| t == tag)
                    {
                        continue;
                    }
                    if model.missing_key() {
                        report.skipped.push(format!("{key} (no API key)"));
                        continue;
                    }

                    report.outcomes.push(run_cell(eval, sample, model).await);
                }
            }
        }

        report
    }
}
