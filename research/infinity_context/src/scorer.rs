//! Scoring system for evaluation
//!
//! Supports multiple scorers per scenario, each producing a 0.0-1.0 score.

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ============================================================================
// Score Result
// ============================================================================

/// Result of a single scorer evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    /// Scorer name (for identification)
    pub name: String,

    /// Score value from 0.0 (completely wrong) to 1.0 (perfect)
    pub value: f64,

    /// Human-readable explanation of the score
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Score {
    pub fn new(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            value: value.clamp(0.0, 1.0),
            rationale: None,
        }
    }

    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    /// Returns true if score >= 0.5 (passing threshold)
    pub fn passed(&self) -> bool {
        self.value >= 0.5
    }
}

// ============================================================================
// Scorer Definition
// ============================================================================

/// A scorer definition that can evaluate responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Scorer {
    /// Check if response contains all specified strings (case-insensitive)
    Contains {
        name: String,
        values: Vec<String>,
        #[serde(default)]
        case_sensitive: bool,
    },

    /// Check if response matches regex pattern
    Pattern { name: String, pattern: String },

    /// Exact match (case-insensitive by default)
    Exact {
        name: String,
        expected: String,
        #[serde(default)]
        case_sensitive: bool,
    },

    /// LLM-as-judge evaluation
    LlmJudge {
        name: String,
        /// Criteria for evaluation (what makes a good answer)
        criteria: String,
        /// Optional: the expected/ideal answer for reference
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ideal: Option<String>,
    },
}

impl Scorer {
    /// Get the scorer name
    pub fn name(&self) -> &str {
        match self {
            Scorer::Contains { name, .. } => name,
            Scorer::Pattern { name, .. } => name,
            Scorer::Exact { name, .. } => name,
            Scorer::LlmJudge { name, .. } => name,
        }
    }

    /// Check if this scorer requires LLM calls
    pub fn requires_llm(&self) -> bool {
        matches!(self, Scorer::LlmJudge { .. })
    }

    /// Evaluate response with non-LLM scorers (returns None for LLM scorers)
    pub fn evaluate_sync(&self, response: &str) -> Option<Score> {
        match self {
            Scorer::Contains {
                name,
                values,
                case_sensitive,
            } => {
                let response_cmp = if *case_sensitive {
                    response.to_string()
                } else {
                    response.to_lowercase()
                };

                let matched: Vec<&String> = values
                    .iter()
                    .filter(|v| {
                        let v_cmp = if *case_sensitive {
                            v.to_string()
                        } else {
                            v.to_lowercase()
                        };
                        response_cmp.contains(&v_cmp)
                    })
                    .collect();

                let score = if values.is_empty() {
                    1.0
                } else {
                    matched.len() as f64 / values.len() as f64
                };

                let rationale = if matched.len() == values.len() {
                    format!("Found all {} expected values", values.len())
                } else {
                    format!(
                        "Found {}/{} values. Missing: {:?}",
                        matched.len(),
                        values.len(),
                        values
                            .iter()
                            .filter(|v| !matched.contains(v))
                            .collect::<Vec<_>>()
                    )
                };

                Some(Score::new(name, score).with_rationale(rationale))
            }

            Scorer::Pattern { name, pattern } => {
                let is_match = regex::Regex::new(pattern)
                    .map(|re| re.is_match(response))
                    .unwrap_or(false);

                let score = if is_match { 1.0 } else { 0.0 };
                let rationale = if is_match {
                    "Pattern matched".to_string()
                } else {
                    format!("Pattern '{}' not found in response", pattern)
                };

                Some(Score::new(name, score).with_rationale(rationale))
            }

            Scorer::Exact {
                name,
                expected,
                case_sensitive,
            } => {
                let matches = if *case_sensitive {
                    response.trim() == expected.trim()
                } else {
                    response.trim().to_lowercase() == expected.trim().to_lowercase()
                };

                let score = if matches { 1.0 } else { 0.0 };
                let rationale = if matches {
                    "Exact match".to_string()
                } else {
                    format!("Expected '{}', got '{}'", expected.trim(), response.trim())
                };

                Some(Score::new(name, score).with_rationale(rationale))
            }

            Scorer::LlmJudge { .. } => None, // Requires async LLM call
        }
    }
}

// ============================================================================
// LLM Judge Implementation
// ============================================================================

/// Configuration for LLM judge
#[derive(Debug, Clone)]
pub struct JudgeConfig {
    pub model: String,
    pub api_key: String,
}

/// Evaluate using LLM-as-judge
pub async fn evaluate_with_llm(
    scorer: &Scorer,
    task: &str,
    response: &str,
    config: &JudgeConfig,
) -> Result<Score> {
    let Scorer::LlmJudge {
        name,
        criteria,
        ideal,
    } = scorer
    else {
        anyhow::bail!("evaluate_with_llm called with non-LLM scorer");
    };

    let ideal_section = ideal
        .as_ref()
        .map(|i| format!("\n\nIdeal/Expected Answer:\n{}", i))
        .unwrap_or_default();

    let prompt = format!(
        r#"You are evaluating an AI assistant's response to a task.

Task given to the assistant:
{task}

Evaluation criteria:
{criteria}{ideal_section}

Assistant's response:
{response}

Score the response from 0.0 to 1.0 based on how well it meets the criteria:
- 1.0 = Perfectly meets criteria
- 0.7-0.9 = Mostly correct with minor issues
- 0.4-0.6 = Partially correct
- 0.1-0.3 = Mostly incorrect but shows some understanding
- 0.0 = Completely wrong or doesn't address the task

Respond in this exact format:
SCORE: <number between 0.0 and 1.0>
RATIONALE: <brief explanation>"#
    );

    // Call LLM
    let judge_response = call_judge_llm(&prompt, config).await?;

    // Parse response
    parse_judge_response(name, &judge_response)
}

/// Call the judge LLM
async fn call_judge_llm(prompt: &str, config: &JudgeConfig) -> Result<String> {
    let client = reqwest::Client::new();

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": config.model,
            "max_tokens": 256,
            "messages": [
                {"role": "user", "content": prompt}
            ]
        }))
        .send()
        .await?;

    let status = response.status();
    let body: serde_json::Value = response.json().await?;

    if !status.is_success() {
        anyhow::bail!(
            "Judge LLM error: {} - {:?}",
            status,
            body.get("error").unwrap_or(&body)
        );
    }

    // Extract text from response
    let text = body["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid judge response format"))?;

    Ok(text.to_string())
}

/// Parse the judge LLM response into a Score
fn parse_judge_response(name: &str, response: &str) -> Result<Score> {
    // Look for SCORE: line
    let score_value = response
        .lines()
        .find(|line| line.trim().to_uppercase().starts_with("SCORE:"))
        .and_then(|line| {
            line.split(':')
                .nth(1)
                .and_then(|s| s.trim().parse::<f64>().ok())
        })
        .unwrap_or(0.5); // Default to 0.5 if parsing fails

    // Look for RATIONALE: line
    let rationale = response
        .lines()
        .find(|line| line.trim().to_uppercase().starts_with("RATIONALE:"))
        .map(|line| {
            line.split(':')
                .skip(1)
                .collect::<Vec<_>>()
                .join(":")
                .trim()
                .to_string()
        })
        .unwrap_or_else(|| response.to_string());

    Ok(Score::new(name, score_value).with_rationale(rationale))
}

// ============================================================================
// Batch Evaluation
// ============================================================================

/// Evaluate all scorers for a response
pub async fn evaluate_all(
    scorers: &[Scorer],
    task: &str,
    response: &str,
    judge_config: Option<&JudgeConfig>,
) -> Vec<Score> {
    let mut scores = Vec::new();

    for scorer in scorers {
        let score = if let Some(sync_score) = scorer.evaluate_sync(response) {
            sync_score
        } else if let Some(config) = judge_config {
            // LLM scorer
            match evaluate_with_llm(scorer, task, response, config).await {
                Ok(s) => s,
                Err(e) => {
                    Score::new(scorer.name(), 0.0).with_rationale(format!("Judge error: {}", e))
                }
            }
        } else {
            // No judge config, skip LLM scorers
            Score::new(scorer.name(), 0.0).with_rationale("LLM judge not configured")
        };

        scores.push(score);
    }

    scores
}

/// Calculate aggregate score from multiple scores
pub fn aggregate_scores(scores: &[Score]) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    scores.iter().map(|s| s.value).sum::<f64>() / scores.len() as f64
}

// ============================================================================
// Helper constructors
// ============================================================================

impl Scorer {
    /// Create a contains scorer
    pub fn contains(name: impl Into<String>, values: Vec<String>) -> Self {
        Scorer::Contains {
            name: name.into(),
            values,
            case_sensitive: false,
        }
    }

    /// Create an LLM judge scorer
    pub fn llm_judge(name: impl Into<String>, criteria: impl Into<String>) -> Self {
        Scorer::LlmJudge {
            name: name.into(),
            criteria: criteria.into(),
            ideal: None,
        }
    }

    /// Create an LLM judge scorer with ideal answer
    pub fn llm_judge_with_ideal(
        name: impl Into<String>,
        criteria: impl Into<String>,
        ideal: impl Into<String>,
    ) -> Self {
        Scorer::LlmJudge {
            name: name.into(),
            criteria: criteria.into(),
            ideal: Some(ideal.into()),
        }
    }
}

// ============================================================================
// Health Check
// ============================================================================

/// Result of a health check
#[derive(Debug)]
pub struct HealthCheckResult {
    pub llm_judge_configured: bool,
    pub llm_judge_working: bool,
    pub model: Option<String>,
    pub error: Option<String>,
}

/// Check if LLM judge is properly configured and working
pub async fn check_llm_judge_health() -> HealthCheckResult {
    // Check if API key is set
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            return HealthCheckResult {
                llm_judge_configured: false,
                llm_judge_working: false,
                model: None,
                error: Some("ANTHROPIC_API_KEY environment variable not set".to_string()),
            };
        }
    };

    let model = "claude-3-5-haiku-20241022".to_string();
    let config = JudgeConfig {
        model: model.clone(),
        api_key,
    };

    // Try a simple judge call
    let test_scorer = Scorer::llm_judge("health_check", "Response should say hello");
    let result = evaluate_with_llm(&test_scorer, "Say hello", "Hello there!", &config).await;

    match result {
        Ok(score) => HealthCheckResult {
            llm_judge_configured: true,
            llm_judge_working: true,
            model: Some(model),
            error: Some(format!(
                "Test score: {:.2} - {}",
                score.value,
                score.rationale.unwrap_or_default()
            )),
        },
        Err(e) => HealthCheckResult {
            llm_judge_configured: true,
            llm_judge_working: false,
            model: Some(model),
            error: Some(format!("LLM judge call failed: {}", e)),
        },
    }
}

/// Check if a specific LLM provider API key is configured
pub fn check_provider_api_key(provider: &str) -> (bool, Option<String>) {
    let key_name = match provider.to_lowercase().as_str() {
        "anthropic" | "claude" => "ANTHROPIC_API_KEY",
        "openai" | "gpt" => "OPENAI_API_KEY",
        _ => return (false, Some(format!("Unknown provider: {}", provider))),
    };

    match std::env::var(key_name) {
        Ok(key) if !key.is_empty() => (true, None),
        Ok(_) => (false, Some(format!("{} is set but empty", key_name))),
        Err(_) => (false, Some(format!("{} not set", key_name))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_scorer_all_match() {
        let scorer = Scorer::contains("test", vec!["hello".to_string(), "world".to_string()]);
        let score = scorer.evaluate_sync("Hello World!").unwrap();
        assert_eq!(score.value, 1.0);
        assert!(score.passed());
    }

    #[test]
    fn test_contains_scorer_partial_match() {
        let scorer = Scorer::contains(
            "test",
            vec!["hello".to_string(), "world".to_string(), "foo".to_string()],
        );
        let score = scorer.evaluate_sync("Hello World!").unwrap();
        assert!((score.value - 0.666).abs() < 0.01);
        assert!(score.passed());
    }

    #[test]
    fn test_contains_scorer_no_match() {
        let scorer = Scorer::contains("test", vec!["foo".to_string(), "bar".to_string()]);
        let score = scorer.evaluate_sync("Hello World!").unwrap();
        assert_eq!(score.value, 0.0);
        assert!(!score.passed());
    }

    #[test]
    fn test_pattern_scorer() {
        let scorer = Scorer::Pattern {
            name: "test".to_string(),
            pattern: r"\d{4}-\d{2}-\d{2}".to_string(),
        };
        let score = scorer.evaluate_sync("Date: 2024-01-15").unwrap();
        assert_eq!(score.value, 1.0);
    }

    #[test]
    fn test_exact_scorer() {
        let scorer = Scorer::Exact {
            name: "test".to_string(),
            expected: "PostgreSQL".to_string(),
            case_sensitive: false,
        };
        let score = scorer.evaluate_sync("postgresql").unwrap();
        assert_eq!(score.value, 1.0);
    }

    #[test]
    fn test_parse_judge_response() {
        let response = "SCORE: 0.85\nRATIONALE: The response correctly identifies the database.";
        let score = parse_judge_response("test", response).unwrap();
        assert_eq!(score.value, 0.85);
        assert!(score.rationale.unwrap().contains("correctly identifies"));
    }

    #[test]
    fn test_aggregate_scores() {
        let scores = vec![
            Score::new("a", 1.0),
            Score::new("b", 0.5),
            Score::new("c", 0.0),
        ];
        let avg = aggregate_scores(&scores);
        assert_eq!(avg, 0.5);
    }
}
