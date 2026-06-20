//! The eval protocol: newline-delimited JSON over stdio, MCP-style.
//!
//! Two processes talk:
//! * the **server** (your eval program) — defines evals in Rust, owns runtime
//!   construction and scoring, and knows nothing about selection, aggregation,
//!   checkpoints, or rendering. It just answers requests. See [`crate::server`].
//! * the **host** (the `mira` CLI) — compiles + spawns the server, enumerates
//!   evals, plans the run (selection × matrix), drives execution, then
//!   aggregates / saves / checkpoints / visualizes. See [`crate::host`].
//!
//! Provider API keys live only in the server's environment and never cross the
//! wire — the host addresses models by *label*.
//!
//! Framing: one JSON object per line. A line with `id` is a [`Response`];
//! a line with `method` but no `id` is a [`Notification`]. Requests flow
//! host→server; responses and notifications flow server→host.

use serde::{Deserialize, Serialize};

use crate::{Score, Usage};

pub const PROTOCOL_VERSION: &str = "0.1";

/// host → server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// server → host, correlated to a [`Request`] by `id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: u64, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(RpcError {
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub message: String,
}

/// server → host, fire-and-forget progress (no `id`). Carries live events
/// (a turn started, a tool was called, tokens spent) so the host can render
/// progress and, later, stream into the transcript viewer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

// ----- method payloads ------------------------------------------------------

/// `initialize` result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub server: String,
    pub evals: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleInfo {
    pub id: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub label: String,
    /// False when a real provider's API key is absent in the server env.
    pub available: bool,
}

/// One eval, as advertised by `list`. Enough for the host to plan the full
/// `samples × models` grid and apply selection without running anything.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalInfo {
    pub name: String,
    pub samples: Vec<SampleInfo>,
    pub scorers: Vec<String>,
    pub models: Vec<ModelInfo>,
    pub max_turns: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListResult {
    pub evals: Vec<EvalInfo>,
}

/// `run` params: address one matrix cell by `(eval, sample, model label)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunParams {
    pub eval: String,
    pub sample: String,
    pub model: String,
}

/// Lightweight transcript carried in results and checkpoints.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TranscriptSummary {
    pub final_response: String,
    pub iterations: usize,
    pub tool_calls_count: usize,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `run` result for one cell. Also the unit persisted in checkpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub eval: String,
    pub sample: String,
    pub model: String,
    pub passed: bool,
    pub aggregate: f64,
    pub scores: Vec<Score>,
    pub transcript: TranscriptSummary,
    /// True when the cell was not executed (e.g. model unavailable).
    #[serde(default)]
    pub skipped: bool,
}

impl RunResult {
    /// Stable cell identity: `eval/sample@model`. Used for selection, dedupe,
    /// and checkpoint resume.
    pub fn key(&self) -> String {
        format!("{}/{}@{}", self.eval, self.sample, self.model)
    }
}
