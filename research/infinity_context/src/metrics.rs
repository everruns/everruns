//! Metrics computation and aggregation

use crate::types::{ExpectedResult, ScenarioResult, StrategyResults};

/// Check if a response matches the expected result
pub fn check_success(response: &str, expected: &ExpectedResult) -> bool {
    match expected {
        ExpectedResult::Exact(expected_str) => {
            response.trim().to_lowercase() == expected_str.trim().to_lowercase()
        }
        ExpectedResult::Contains { contains } => {
            let response_lower = response.to_lowercase();
            contains
                .iter()
                .all(|s| response_lower.contains(&s.to_lowercase()))
        }
        ExpectedResult::Pattern { pattern } => {
            regex::Regex::new(pattern)
                .map(|re| re.is_match(response))
                .unwrap_or(false)
        }
        ExpectedResult::LlmJudge { criteria: _ } => {
            // Would need actual LLM call - for now, assume success if response is non-empty
            // TODO: Implement LLM-as-judge evaluation
            !response.trim().is_empty()
        }
    }
}

/// Aggregate results for a single strategy across all scenarios
pub fn aggregate_strategy_results(
    strategy_name: &str,
    results: Vec<ScenarioResult>,
) -> StrategyResults {
    let total = results.len();
    let successful = results.iter().filter(|r| r.success).count();
    let failed = total - successful;
    let context_exceeded = results.iter().filter(|r| r.metrics.context_exceeded).count();

    let total_tokens: usize = results.iter().map(|r| r.metrics.total_tokens).sum();
    let total_latency: u64 = results.iter().map(|r| r.metrics.latency_ms).sum();
    let total_history_queries: usize = results.iter().map(|r| r.metrics.history_queries).sum();

    StrategyResults {
        strategy_name: strategy_name.to_string(),
        total_scenarios: total,
        successful,
        failed,
        context_exceeded,
        avg_tokens: if total > 0 {
            total_tokens as f64 / total as f64
        } else {
            0.0
        },
        avg_latency_ms: if total > 0 {
            total_latency as f64 / total as f64
        } else {
            0.0
        },
        avg_history_queries: if total > 0 {
            total_history_queries as f64 / total as f64
        } else {
            0.0
        },
        scenario_results: results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let expected = ExpectedResult::Exact("hello world".to_string());
        assert!(check_success("Hello World", &expected));
        assert!(check_success("  hello world  ", &expected));
        assert!(!check_success("hello", &expected));
    }

    #[test]
    fn test_contains_match() {
        let expected = ExpectedResult::Contains {
            contains: vec!["api".to_string(), "key".to_string()],
        };
        assert!(check_success("The API key is abc123", &expected));
        assert!(check_success("API KEY", &expected));
        assert!(!check_success("The token is abc123", &expected));
    }

    #[test]
    fn test_pattern_match() {
        let expected = ExpectedResult::Pattern {
            pattern: r"sk-[a-z0-9]+".to_string(),
        };
        assert!(check_success("The key is sk-abc123", &expected));
        assert!(!check_success("The key is pk-abc123", &expected));
    }
}
