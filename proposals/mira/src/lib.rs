//! Mira — a Rust-first, code-first evaluation framework for agents and tools.
//!
//! This is a **prototype** that demonstrates the design proposed in `SPEC.md`.
//!
//! ## Authoring model
//! Three composable pieces, mirroring Inspect AI's `dataset + solver + scorer`
//! but Rust-native:
//!
//! * [`Subject`] — the thing under evaluation (a runtime session, a `Tool`, or
//!   an external CLI). One adapter per "shape"; this is what unifies the three
//!   existing harnesses (yolop / bashkit / everruns).
//! * [`Dataset`] of [`Sample`]s — the inputs and optional targets.
//! * [`Scorer`] — grades a [`Transcript`] into a [`Score`]. Deterministic,
//!   model-graded, or arbitrary closures, freely composed.
//!
//! An [`Eval`](eval::Eval) bundles those, plus a **matrix** of models to run
//! against.
//!
//! ## Execution model: two processes, one protocol
//! Evals are defined in *your* program (the **server**); a generic CLI (the
//! **host**, `src/bin/mira.rs`) drives them over newline-delimited JSON on
//! stdio, MCP-style (see [`protocol`]):
//!
//! * Your program calls [`serve`] with its evals and does nothing else — the
//!   host owns selection, the matrix, aggregation, checkpoints, and rendering.
//! * The [`Host`] compiles/spawns the server, enumerates evals, plans the run,
//!   and executes each cell. Provider API keys live only in the server's
//!   environment and never cross the wire — models are addressed by *label*.
//!
//! [`Runner`](runner::Runner) is the same core run loop exposed for in-process
//! use (and reused by [`serve`]).
//!
//! See `src/bin/demo_evals.rs` (a server) and `src/bin/mira.rs` (the host) for
//! an end-to-end run against the real `everruns-runtime` using the offline
//! `llmsim` provider (no API key needed).

// Boxed `Fn -> Pin<Box<dyn Future>>` aliases (subject factory, judge) are the
// idiomatic way to express async callbacks behind trait objects here.
#![allow(clippy::type_complexity)]

pub mod eval;
pub mod host;
pub mod protocol;
pub mod report;
pub mod runner;
pub mod scorer;
pub mod server;
pub mod subject;

use serde::{Deserialize, Serialize};

pub use eval::{Case, Eval};
pub use host::Host;
pub use runner::{CaseOutcome, RunReport, Runner};
pub use scorer::Scorer;
pub use server::serve;
pub use subject::{ModelSpec, RuntimeSubject, Subject};

/// One dataset row: an input conversation plus optional target/metadata.
///
/// Datasets are deliberately language-agnostic JSON so the same files drive
/// Rust, CLI, or polyglot subjects. Small evals skip the file entirely and
/// inline samples in Rust via [`Eval::case`](eval::Eval::case).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sample {
    pub id: String,
    /// Sequence of user turns to send. Most samples have exactly one.
    pub input: Vec<String>,
    /// Optional reference answer / expected value for target-based scorers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<serde_json::Value>,
    /// Files to pre-seed into the subject's workspace before the run.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub files: std::collections::BTreeMap<String, String>,
    /// Free-form tags for selective evaluation (`--tag smoke`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Sample {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            input: vec![prompt.into()],
            ..Default::default()
        }
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn target(mut self, target: impl Into<serde_json::Value>) -> Self {
        self.target = Some(target.into());
        self
    }
}

/// A dataset is just a sequence of [`Sample`]s. Loaders are convenience
/// constructors; the engine only cares about the resulting `Vec<Sample>`.
#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub samples: Vec<Sample>,
}

impl Dataset {
    pub fn new(samples: Vec<Sample>) -> Self {
        Self { samples }
    }

    /// Load a JSONL dataset (one [`Sample`] object per line). This is the
    /// secondary, config-style on-ramp; code-first evals usually inline cases.
    pub fn jsonl(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut samples = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let sample: Sample = serde_json::from_str(line).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("line {}: {e}", i + 1),
                )
            })?;
            samples.push(sample);
        }
        Ok(Self { samples })
    }
}

impl From<Vec<Sample>> for Dataset {
    fn from(samples: Vec<Sample>) -> Self {
        Self { samples }
    }
}

/// Token / cost accounting, summed across all turns of a run.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// Normalized result of running a [`Subject`] on one [`Sample`].
///
/// The prototype captures the reliable, provider-agnostic signal: the final
/// response text, turn/tool counts, token usage, and the raw serialized event
/// stream (everruns' canonical JSONL `Event`s) so structural scorers can search
/// it without depending on internal struct shapes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Transcript {
    pub final_response: String,
    pub iterations: usize,
    pub tool_calls_count: usize,
    pub usage: Usage,
    /// Best-effort list of tool names invoked, extracted from the event stream.
    pub tool_calls: Vec<String>,
    /// Raw serialized events (the everruns `Event` JSONL transcript).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<serde_json::Value>,
    /// Set when the subject failed to complete the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outcome of a single [`Scorer`] on a [`Transcript`]. `value` is in `0.0..=1.0`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Score {
    pub scorer: String,
    pub value: f64,
    pub pass: bool,
    pub reason: String,
}

impl Score {
    pub fn pass(scorer: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            scorer: scorer.into(),
            value: 1.0,
            pass: true,
            reason: reason.into(),
        }
    }

    pub fn fail(scorer: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            scorer: scorer.into(),
            value: 0.0,
            pass: false,
            reason: reason.into(),
        }
    }
}

/// Per-run context handed to a [`Subject`]: which model to use this matrix cell,
/// and run limits. Extend with cost caps, seeds, etc.
#[derive(Clone, Debug)]
pub struct RunCx {
    pub model: ModelSpec,
    pub max_turns: usize,
}
