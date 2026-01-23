//! Report generation for evaluation results

use crate::runner::EvalConfig;
use crate::types::{EvaluationResults, StrategyResults};

/// Generate a markdown report from evaluation results
pub fn generate(results: &EvaluationResults, config: &EvalConfig) -> String {
    let mut report = String::new();

    // Header
    report.push_str(&format!(
        "# Infinity Context Evaluation Report\n\n\
        **Generated:** {}\n\
        **Model:** {}\n\
        **Context Window:** {} tokens\n\
        **Budget:** {:.0}%\n\n",
        results.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
        results.model,
        config.context_window,
        config.budget_percent * 100.0
    ));

    // Summary table
    report.push_str("## Summary\n\n");
    report.push_str("| Strategy | Accuracy | Avg Tokens | Avg Latency | Context Exceeded | History Queries |\n");
    report.push_str("|----------|----------|------------|-------------|------------------|----------------|\n");

    for strategy in &results.strategy_results {
        report.push_str(&format!(
            "| {} | {:.1}% | {:.0} | {:.0}ms | {} | {:.1} |\n",
            strategy.strategy_name,
            strategy.accuracy(),
            strategy.avg_tokens,
            strategy.avg_latency_ms,
            strategy.context_exceeded,
            strategy.avg_history_queries
        ));
    }

    report.push_str("\n");

    // Detailed results per strategy
    report.push_str("## Detailed Results\n\n");

    for strategy in &results.strategy_results {
        report.push_str(&format!("### {}\n\n", strategy.strategy_name));
        report.push_str(&format!(
            "- **Total scenarios:** {}\n\
            - **Successful:** {} ({:.1}%)\n\
            - **Failed:** {}\n\
            - **Context exceeded:** {}\n\n",
            strategy.total_scenarios,
            strategy.successful,
            strategy.accuracy(),
            strategy.failed,
            strategy.context_exceeded
        ));

        // Per-scenario results table
        report.push_str("| Scenario | Status | Tokens | Latency | Queries | Error |\n");
        report.push_str("|----------|--------|--------|---------|---------|-------|\n");

        for result in &strategy.scenario_results {
            let status = if result.success {
                "Pass"
            } else if result.metrics.context_exceeded {
                "Context Exceeded"
            } else {
                "Fail"
            };

            let error = result
                .error
                .as_ref()
                .map(|e| {
                    if e.len() > 50 {
                        format!("{}...", &e[..50])
                    } else {
                        e.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            report.push_str(&format!(
                "| {} | {} | {} | {}ms | {} | {} |\n",
                result.scenario_name,
                status,
                result.metrics.total_tokens,
                result.metrics.latency_ms,
                result.metrics.history_queries,
                error
            ));
        }

        report.push_str("\n");
    }

    // Analysis section
    report.push_str("## Analysis\n\n");
    report.push_str(&generate_analysis(&results.strategy_results));

    report
}

/// Generate analysis text comparing strategies
fn generate_analysis(strategies: &[StrategyResults]) -> String {
    let mut analysis = String::new();

    // Find baseline and infinity strategies for comparison
    let baseline = strategies.iter().find(|s| s.strategy_name == "baseline");
    let naive = strategies.iter().find(|s| s.strategy_name == "naive_trim");
    let infinity = strategies
        .iter()
        .find(|s| s.strategy_name == "infinity_context");

    if let (Some(baseline), Some(infinity)) = (baseline, infinity) {
        let accuracy_improvement = infinity.accuracy() - baseline.accuracy();

        if accuracy_improvement > 0.0 {
            analysis.push_str(&format!(
                "**Infinity context shows {:.1}% accuracy improvement** over baseline.\n\n",
                accuracy_improvement
            ));
        }

        if baseline.context_exceeded > 0 {
            analysis.push_str(&format!(
                "- Baseline failed on {} scenario(s) due to context limits\n",
                baseline.context_exceeded
            ));
        }

        if infinity.context_exceeded == 0 && baseline.context_exceeded > 0 {
            analysis.push_str("- Infinity context handled all scenarios without context overflow\n");
        }
    }

    if let (Some(naive), Some(infinity)) = (naive, infinity) {
        let accuracy_improvement = infinity.accuracy() - naive.accuracy();

        if accuracy_improvement > 0.0 {
            analysis.push_str(&format!(
                "- Infinity context shows {:.1}% accuracy improvement over naive trimming\n",
                accuracy_improvement
            ));
        }

        let token_overhead = if naive.avg_tokens > 0.0 {
            ((infinity.avg_tokens - naive.avg_tokens) / naive.avg_tokens) * 100.0
        } else {
            0.0
        };

        if token_overhead > 0.0 {
            analysis.push_str(&format!(
                "- Token overhead for infinity context: {:.1}% (due to history queries)\n",
                token_overhead
            ));
        }

        if infinity.avg_history_queries > 0.0 {
            analysis.push_str(&format!(
                "- Average history queries per scenario: {:.1}\n",
                infinity.avg_history_queries
            ));
        }
    }

    if analysis.is_empty() {
        analysis.push_str("Insufficient data for comparative analysis.\n");
    }

    analysis.push_str("\n### Key Findings\n\n");

    // Generate key findings based on results
    let best_strategy = strategies
        .iter()
        .max_by(|a, b| a.accuracy().partial_cmp(&b.accuracy()).unwrap());

    if let Some(best) = best_strategy {
        analysis.push_str(&format!(
            "1. **Best performing strategy:** {} ({:.1}% accuracy)\n",
            best.strategy_name,
            best.accuracy()
        ));
    }

    let most_efficient = strategies
        .iter()
        .filter(|s| s.accuracy() > 50.0)
        .min_by(|a, b| a.avg_tokens.partial_cmp(&b.avg_tokens).unwrap());

    if let Some(efficient) = most_efficient {
        analysis.push_str(&format!(
            "2. **Most token-efficient (>50% accuracy):** {} ({:.0} avg tokens)\n",
            efficient.strategy_name, efficient.avg_tokens
        ));
    }

    let fastest = strategies
        .iter()
        .filter(|s| s.accuracy() > 50.0)
        .min_by(|a, b| a.avg_latency_ms.partial_cmp(&b.avg_latency_ms).unwrap());

    if let Some(fast) = fastest {
        analysis.push_str(&format!(
            "3. **Fastest (>50% accuracy):** {} ({:.0}ms avg latency)\n",
            fast.strategy_name, fast.avg_latency_ms
        ));
    }

    analysis
}

