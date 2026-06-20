//! [`Eval`]: one evaluation = a dataset + a subject + scorers + a model matrix.
//!
//! This mirrors Inspect AI's `Task`, but the matrix (which models to run) is a
//! first-class axis. The code-first builder is the primary authoring surface;
//! `Dataset::jsonl` is the secondary, config-style on-ramp.
//!
//! The polish target (see `SPEC.md`) is an `#[eval]` attribute that registers a
//! function returning [`Eval`] and runs it under a `libtest-mimic` harness, so
//! evals get `cargo test`-style discovery, filtering, and parallelism for free.
//! The prototype uses an explicit [`Suite`] instead, which is the same model
//! without the macro magic.

use std::sync::Arc;

use crate::scorer::Scorer;
use crate::subject::{ModelSpec, Subject};
use crate::{Dataset, Sample};

/// A dataset row, in eval-authoring terms.
pub type Case = Sample;

/// A single evaluation, ready to run across its model matrix.
pub struct Eval {
    pub name: String,
    pub dataset: Dataset,
    pub subject: Arc<dyn Subject>,
    pub scorers: Vec<Box<dyn Scorer>>,
    pub models: Vec<ModelSpec>,
    pub max_turns: usize,
}

impl Eval {
    /// Starts the builder. Returns an [`EvalBuilder`] (not `Self`) so the fluent
    /// API can require a subject before producing an [`Eval`].
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: impl Into<String>) -> EvalBuilder {
        EvalBuilder {
            name: name.into(),
            dataset: Dataset::default(),
            subject: None,
            scorers: Vec::new(),
            models: Vec::new(),
            max_turns: 12,
        }
    }
}

pub struct EvalBuilder {
    name: String,
    dataset: Dataset,
    subject: Option<Arc<dyn Subject>>,
    scorers: Vec<Box<dyn Scorer>>,
    models: Vec<ModelSpec>,
    max_turns: usize,
}

impl EvalBuilder {
    /// Provide the dataset wholesale (e.g. `Dataset::jsonl(...)`).
    pub fn dataset(mut self, dataset: impl Into<Dataset>) -> Self {
        self.dataset = dataset.into();
        self
    }

    /// Inline one case in code — no dataset file needed for small evals.
    pub fn case(mut self, id: impl Into<String>, prompt: impl Into<String>) -> Self {
        self.dataset.samples.push(Sample::new(id, prompt));
        self
    }

    /// Inline a fully-built [`Sample`] (with tags, target, seeded files).
    pub fn sample(mut self, sample: Sample) -> Self {
        self.dataset.samples.push(sample);
        self
    }

    /// The thing under evaluation.
    pub fn subject(mut self, subject: impl Subject + 'static) -> Self {
        self.subject = Some(Arc::new(subject));
        self
    }

    /// Add a scorer. Every scorer runs against every sample × model cell.
    pub fn scorer(mut self, scorer: Box<dyn Scorer>) -> Self {
        self.scorers.push(scorer);
        self
    }

    /// Add one matrix cell (a model). Omit entirely to default to `sim`.
    pub fn model(mut self, model: ModelSpec) -> Self {
        self.models.push(model);
        self
    }

    /// Replace the matrix with `models`.
    pub fn models(mut self, models: impl IntoIterator<Item = ModelSpec>) -> Self {
        self.models = models.into_iter().collect();
        self
    }

    pub fn max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn build(self) -> Eval {
        Eval {
            name: self.name,
            dataset: self.dataset,
            subject: self.subject.expect("eval requires a subject"),
            scorers: self.scorers,
            models: if self.models.is_empty() {
                vec![ModelSpec::sim()]
            } else {
                self.models
            },
            max_turns: self.max_turns,
        }
    }
}
