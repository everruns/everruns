//! [`Scorer`]s grade a [`Transcript`]. They compose freely: an [`Eval`] holds a
//! `Vec<Box<dyn Scorer>>` and every scorer runs against every transcript. This
//! unifies bashkit's string-keyed checks and everruns' `Scorer` enum into one
//! open, code-first vocabulary — deterministic, model-graded, or arbitrary.
//!
//! [`Eval`]: crate::eval::Eval

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;

use crate::{Sample, Score, Transcript};

#[async_trait]
pub trait Scorer: Send + Sync {
    fn name(&self) -> String;
    async fn score(&self, sample: &Sample, transcript: &Transcript) -> Score;
}

// ----- deterministic scorers ------------------------------------------------

/// Wraps a plain closure as a scorer. The escape hatch for one-off checks —
/// the bar for adding bespoke logic is a closure, not a new enum variant.
pub struct FnScorer {
    name: String,
    f: Box<dyn Fn(&Sample, &Transcript) -> Score + Send + Sync>,
}

#[async_trait]
impl Scorer for FnScorer {
    fn name(&self) -> String {
        self.name.clone()
    }
    async fn score(&self, sample: &Sample, transcript: &Transcript) -> Score {
        (self.f)(sample, transcript)
    }
}

/// Build a scorer from a closure: `scorer("nonempty", |_, t| ...)`.
pub fn scorer(
    name: impl Into<String>,
    f: impl Fn(&Sample, &Transcript) -> Score + Send + Sync + 'static,
) -> Box<dyn Scorer> {
    Box::new(FnScorer {
        name: name.into(),
        f: Box::new(f),
    })
}

/// Passes if the final response contains `needle`.
pub fn contains(needle: impl Into<String>) -> Box<dyn Scorer> {
    let needle = needle.into();
    scorer(format!("contains({needle:?})"), move |_, t| {
        if t.final_response.contains(&needle) {
            Score::pass("contains", format!("found {needle:?}"))
        } else {
            Score::fail("contains", format!("missing {needle:?}"))
        }
    })
}

/// Passes if the final response does NOT contain `needle`.
pub fn not_contains(needle: impl Into<String>) -> Box<dyn Scorer> {
    let needle = needle.into();
    scorer(format!("not_contains({needle:?})"), move |_, t| {
        if t.final_response.contains(&needle) {
            Score::fail("not_contains", format!("unexpectedly found {needle:?}"))
        } else {
            Score::pass("not_contains", format!("absent {needle:?}"))
        }
    })
}

/// Passes if the final response matches `pattern`.
pub fn regex(pattern: impl Into<String>) -> Box<dyn Scorer> {
    let pattern = pattern.into();
    let compiled = regex::Regex::new(&pattern);
    scorer(format!("regex({pattern:?})"), move |_, t| match &compiled {
        Ok(re) if re.is_match(&t.final_response) => {
            Score::pass("regex", format!("matched {pattern:?}"))
        }
        Ok(_) => Score::fail("regex", format!("no match for {pattern:?}")),
        Err(e) => Score::fail("regex", format!("bad pattern: {e}")),
    })
}

/// Passes if a tool named `tool` was invoked at least once.
pub fn tool_called(tool: impl Into<String>) -> Box<dyn Scorer> {
    let tool = tool.into();
    scorer(format!("tool_called({tool})"), move |_, t| {
        if t.tool_calls.iter().any(|n| n == &tool) {
            Score::pass("tool_called", format!("{tool} was called"))
        } else {
            Score::fail("tool_called", format!("{tool} never called"))
        }
    })
}

/// Passes if the run used no more than `max` tool calls.
pub fn tool_calls_within(max: usize) -> Box<dyn Scorer> {
    scorer(format!("tool_calls_within({max})"), move |_, t| {
        if t.tool_calls_count <= max {
            Score::pass(
                "tool_calls_within",
                format!("{} <= {max}", t.tool_calls_count),
            )
        } else {
            Score::fail(
                "tool_calls_within",
                format!("{} > {max}", t.tool_calls_count),
            )
        }
    })
}

/// Passes if the run took no more than `max` reasoning iterations.
pub fn turns_within(max: usize) -> Box<dyn Scorer> {
    scorer(format!("turns_within({max})"), move |_, t| {
        if t.iterations <= max {
            Score::pass("turns_within", format!("{} <= {max}", t.iterations))
        } else {
            Score::fail("turns_within", format!("{} > {max}", t.iterations))
        }
    })
}

/// Passes if the subject completed without an error.
pub fn succeeded() -> Box<dyn Scorer> {
    scorer("succeeded", move |_, t| match &t.error {
        None => Score::pass("succeeded", "no error"),
        Some(e) => Score::fail("succeeded", format!("error: {e}")),
    })
}

// ----- model-graded scorer --------------------------------------------------

/// An async judge: given a rubric and the transcript, return a [`Score`].
/// Backed by a separate model — the framework keeps the judge independent of
/// the model under test (the standard guidance), but does not enforce it.
pub type JudgeFn =
    Box<dyn Fn(String, Transcript) -> Pin<Box<dyn Future<Output = Score> + Send>> + Send + Sync>;

pub struct ModelGraded {
    rubric: String,
    judge: JudgeFn,
}

#[async_trait]
impl Scorer for ModelGraded {
    fn name(&self) -> String {
        format!("model_graded({:?})", self.rubric)
    }
    async fn score(&self, _sample: &Sample, transcript: &Transcript) -> Score {
        (self.judge)(self.rubric.clone(), transcript.clone()).await
    }
}

/// Grade a transcript against a natural-language `rubric` using a `judge`
/// (typically a cheaper model). Demonstrates that LLM-as-judge is just another
/// [`Scorer`], not a special case in the engine.
pub fn model_graded(rubric: impl Into<String>, judge: JudgeFn) -> Box<dyn Scorer> {
    Box::new(ModelGraded {
        rubric: rubric.into(),
        judge,
    })
}
