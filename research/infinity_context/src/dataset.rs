//! Dataset module for JSONL-based eval datasets
//!
//! Format compatible with HuggingFace datasets. Each line is a JSON object:
//! ```json
//! {"name": "needle_basic_0", "type": "needle_in_haystack", "messages": [...], "task": "...", "expected": {...}}
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::types::{ExpectedResult, Message, PlantedInfo, Scenario, ScenarioType};

// ============================================================================
// Dataset Record (input)
// ============================================================================

/// A single record in the evaluation dataset (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    /// Unique identifier for this scenario
    pub name: String,

    /// Scenario type for categorization
    #[serde(rename = "type")]
    pub scenario_type: ScenarioType,

    /// Human-readable description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The conversation history
    pub messages: Vec<Message>,

    /// The task/question to answer
    pub task: String,

    /// Expected answer or evaluation criteria
    pub expected: ExpectedResult,

    /// Metadata about planted information (for debugging)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planted_info: Vec<PlantedInfo>,
}

impl From<Scenario> for DatasetRecord {
    fn from(s: Scenario) -> Self {
        Self {
            name: s.name,
            scenario_type: s.scenario_type,
            description: Some(s.description),
            messages: s.messages,
            task: s.task,
            expected: s.expected,
            planted_info: s.planted_info,
        }
    }
}

impl From<DatasetRecord> for Scenario {
    fn from(r: DatasetRecord) -> Self {
        Self {
            name: r.name,
            description: r.description.unwrap_or_default(),
            scenario_type: r.scenario_type,
            messages: r.messages,
            task: r.task,
            expected: r.expected,
            planted_info: r.planted_info,
        }
    }
}

// ============================================================================
// Eval Result Record (output)
// ============================================================================

/// A single evaluation result record (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResultRecord {
    /// Scenario name
    pub scenario: String,

    /// Scenario type
    pub scenario_type: ScenarioType,

    /// Capability/strategy name
    pub capability: String,

    /// Whether evaluation passed
    pub success: bool,

    /// Model response
    pub response: String,

    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Evaluation metrics
    pub metrics: ResultMetrics,
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

    /// Messages included in context
    pub messages_in_context: usize,

    /// Messages excluded from context
    pub messages_excluded: usize,
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
            messages_in_context: m.messages_in_context,
            messages_excluded: m.messages_excluded,
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
pub fn write_results(path: &Path, metadata: &RunMetadata, results: &[EvalResultRecord]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_dataset_roundtrip() {
        let records = vec![DatasetRecord {
            name: "test_scenario".to_string(),
            scenario_type: ScenarioType::NeedleInHaystack,
            description: Some("Test description".to_string()),
            messages: vec![Message::user("Hello"), Message::assistant("Hi there")],
            task: "What was said?".to_string(),
            expected: ExpectedResult::Contains {
                contains: vec!["Hello".to_string()],
            },
            planted_info: vec![],
        }];

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        write_dataset(&path, &records).unwrap();
        let loaded = read_dataset(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test_scenario");
    }
}
