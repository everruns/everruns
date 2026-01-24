//! Metrics computation and aggregation

use crate::types::{ScenarioResult, StrategyResults};

/// Aggregate results for a single strategy across all scenarios
pub fn aggregate_strategy_results(
    strategy_name: &str,
    results: Vec<ScenarioResult>,
) -> StrategyResults {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed()).count();
    let failed = total - passed;
    let context_exceeded = results
        .iter()
        .filter(|r| r.metrics.context_exceeded)
        .count();

    let total_score: f64 = results.iter().map(|r| r.score).sum();
    let total_tokens: usize = results.iter().map(|r| r.metrics.total_tokens).sum();
    let total_latency: u64 = results.iter().map(|r| r.metrics.latency_ms).sum();
    let total_history_queries: usize = results.iter().map(|r| r.metrics.history_queries).sum();

    StrategyResults {
        strategy_name: strategy_name.to_string(),
        total_scenarios: total,
        passed,
        failed,
        context_exceeded,
        avg_score: if total > 0 {
            total_score / total as f64
        } else {
            0.0
        },
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
