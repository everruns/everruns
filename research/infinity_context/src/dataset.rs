//! Dataset module for JSONL-based eval datasets
//!
//! Format compatible with HuggingFace datasets. Each line is a JSON object:
//! ```json
//! {"id": "needle_basic_0", "task": "...", "input": {"events": [...]}, "expectations": [...]}
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::scorer::Score;

// ============================================================================
// Dataset Record (input)
// ============================================================================

/// A single record in the evaluation dataset (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    /// Unique identifier for this scenario
    pub id: String,

    /// Human-readable description
    pub description: String,

    /// The task/question to answer
    pub task: String,

    /// Input data for the scenario
    pub input: Input,

    /// Expectations for LLM judge to evaluate
    pub expectations: Vec<String>,

    /// Metadata for debugging and categorization
    #[serde(default)]
    pub meta: Meta,
}

/// Input data containing events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    /// Events that make up the conversation history
    pub events: Vec<Event>,
}

/// An event in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// User sent a message
    UserMessage { content: String },

    /// Assistant sent a message
    AssistantMessage { content: String },

    /// Tool was called and returned a result
    ToolResult {
        tool_name: String,
        tool_call_id: String,
        result: String,
    },

    /// System message
    SystemMessage { content: String },
}

/// Metadata for the scenario
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    /// Scenario type for categorization
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_type: Option<String>,

    /// Position of planted information (for needle scenarios)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planted_at: Option<usize>,

    /// Key information that was planted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planted_key: Option<String>,

    /// Value that was planted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planted_value: Option<String>,

    /// Additional planted info for multi-hop scenarios
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planted_info: Vec<PlantedInfo>,
}

/// Information planted in the conversation for retrieval tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantedInfo {
    /// Position in event list (0-indexed)
    pub event_index: usize,
    /// The key information to find
    pub key: String,
    /// The value/content to retrieve
    pub value: String,
}

// ============================================================================
// Eval Result Record (output)
// ============================================================================

/// A single evaluation result record (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResultRecord {
    /// Scenario id
    pub id: String,

    /// Capability/strategy name
    pub capability: String,

    /// Aggregate score (0.0-1.0, average of all expectation scores)
    pub score: f64,

    /// Individual expectation results
    pub scores: Vec<Score>,

    /// Model response
    pub response: String,

    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Evaluation metrics
    pub metrics: ResultMetrics,
}

impl EvalResultRecord {
    /// Returns true if aggregate score >= 0.5
    pub fn passed(&self) -> bool {
        self.score >= 0.5
    }
}

/// Metrics for a single evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultMetrics {
    /// Total tokens used
    pub total_tokens: usize,

    /// Input tokens
    pub input_tokens: usize,

    /// Output tokens
    pub output_tokens: usize,

    /// Number of LLM calls
    pub llm_calls: usize,

    /// Number of history queries (for infinity strategy)
    pub history_queries: usize,

    /// Latency in milliseconds
    pub latency_ms: u64,

    /// Whether context limit was exceeded
    pub context_exceeded: bool,

    /// Events included in context
    pub events_in_context: usize,

    /// Events excluded from context
    pub events_excluded: usize,
}

impl From<crate::types::EvalMetrics> for ResultMetrics {
    fn from(m: crate::types::EvalMetrics) -> Self {
        Self {
            total_tokens: m.total_tokens,
            input_tokens: m.input_tokens,
            output_tokens: m.output_tokens,
            llm_calls: m.llm_calls,
            history_queries: m.history_queries,
            latency_ms: m.latency_ms,
            context_exceeded: m.context_exceeded,
            events_in_context: m.messages_in_context,
            events_excluded: m.messages_excluded,
        }
    }
}

// ============================================================================
// Run Metadata
// ============================================================================

/// Metadata for an evaluation run (saved as first line or separate file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    /// Timestamp of the run
    pub timestamp: DateTime<Utc>,

    /// Model used for evaluation
    pub model: String,

    /// Context window size
    pub context_window: usize,

    /// Budget percentage
    pub budget_percent: f64,

    /// Dataset file used
    pub dataset: String,

    /// Number of scenarios
    pub scenario_count: usize,

    /// Number of capabilities evaluated
    pub capability_count: usize,

    /// Capabilities evaluated
    pub capabilities: Vec<String>,
}

// ============================================================================
// JSONL Read/Write
// ============================================================================

/// Write dataset records to JSONL file
pub fn write_dataset(path: &Path, records: &[DatasetRecord]) -> Result<()> {
    let file = std::fs::File::create(path).context("Failed to create dataset file")?;
    let mut writer = std::io::BufWriter::new(file);

    for record in records {
        let json = serde_json::to_string(record).context("Failed to serialize record")?;
        writeln!(writer, "{}", json)?;
    }

    writer.flush()?;
    Ok(())
}

/// Read dataset records from JSONL file
pub fn read_dataset(path: &Path) -> Result<Vec<DatasetRecord>> {
    let file = std::fs::File::open(path).context("Failed to open dataset file")?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line.context("Failed to read line")?;
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }

        let record: DatasetRecord = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse line {}: {}", line_num + 1, line))?;
        records.push(record);
    }

    Ok(records)
}

/// Write evaluation results to JSONL file
pub fn write_results(
    path: &Path,
    metadata: &RunMetadata,
    results: &[EvalResultRecord],
) -> Result<()> {
    let file = std::fs::File::create(path).context("Failed to create results file")?;
    let mut writer = std::io::BufWriter::new(file);

    // First line is metadata (prefixed with _meta for easy filtering)
    let meta_json = serde_json::json!({
        "_meta": metadata
    });
    writeln!(writer, "{}", serde_json::to_string(&meta_json)?)?;

    // Remaining lines are results
    for result in results {
        let json = serde_json::to_string(result).context("Failed to serialize result")?;
        writeln!(writer, "{}", json)?;
    }

    writer.flush()?;
    Ok(())
}

/// Read evaluation results from JSONL file
pub fn read_results(path: &Path) -> Result<(RunMetadata, Vec<EvalResultRecord>)> {
    let file = std::fs::File::open(path).context("Failed to open results file")?;
    let reader = BufReader::new(file);

    let mut metadata: Option<RunMetadata> = None;
    let mut results = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line.context("Failed to read line")?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // Check for metadata line
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse line {}", line_num + 1))?;

        if value.get("_meta").is_some() {
            metadata = Some(serde_json::from_value(value["_meta"].clone())?);
        } else {
            let result: EvalResultRecord = serde_json::from_value(value)?;
            results.push(result);
        }
    }

    let metadata = metadata.ok_or_else(|| anyhow::anyhow!("No metadata found in results file"))?;
    Ok((metadata, results))
}

// ============================================================================
// Conversion helpers
// ============================================================================

impl Event {
    pub fn user(content: impl Into<String>) -> Self {
        Event::UserMessage {
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Event::AssistantMessage {
            content: content.into(),
        }
    }

    pub fn tool_result(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Event::ToolResult {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            result: result.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Event::SystemMessage {
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_dataset_roundtrip() {
        let records = vec![DatasetRecord {
            id: "test_scenario".to_string(),
            description: "Test description".to_string(),
            task: "What was said?".to_string(),
            input: Input {
                events: vec![Event::user("Hello"), Event::assistant("Hi there")],
            },
            expectations: vec!["Response mentions Hello".to_string()],
            meta: Meta::default(),
        }];

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        write_dataset(&path, &records).unwrap();
        let loaded = read_dataset(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test_scenario");
        assert_eq!(loaded[0].input.events.len(), 2);
    }
}
