//! Report generation for evaluation results

use crate::runner::{ApproachResults, EvaluationResults};

/// Generate a markdown report from evaluation results
pub fn generate(results: &EvaluationResults) -> String {
    let mut report = String::new();

    // Header
    report.push_str(&format!(
        "# Infinity Context Evaluation Report\n\n\
        **Generated:** {}\n\
        **Model:** {}\n\n",
        results.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
        results.model,
    ));

    // Summary table
    report.push_str("## Summary\n\n");
    report.push_str("| Approach | Avg Score | Passed | Avg Tokens | Avg Latency | Context Exceeded | History Queries |\n");
    report.push_str("|----------|-----------|--------|------------|-------------|------------------|----------------|\n");

    for approach in &results.approach_results {
        report.push_str(&format!(
            "| {} | {:.1}% | {}/{} | {:.0} | {:.0}ms | {} | {:.1} |\n",
            approach.approach_name,
            approach.accuracy(),
            approach.passed,
            approach.total_records,
            approach.avg_tokens,
            approach.avg_latency_ms,
            approach.context_exceeded,
            approach.avg_history_queries
        ));
    }

    report.push('\n');

    // Detailed results per approach
    report.push_str("## Detailed Results\n\n");

    for approach in &results.approach_results {
        report.push_str(&format!("### {}\n\n", approach.approach_name));
        report.push_str(&format!(
            "- **Total records:** {}\n\
            - **Avg score:** {:.1}%\n\
            - **Passed (≥50%):** {}\n\
            - **Failed:** {}\n\
            - **Context exceeded:** {}\n\n",
            approach.total_records,
            approach.accuracy(),
            approach.passed,
            approach.failed,
            approach.context_exceeded
        ));

        // Per-record results table
        report.push_str("| Record | Score | Tokens | Latency | Queries | Error |\n");
        report.push_str("|--------|-------|--------|---------|---------|-------|\n");

        for result in &approach.record_results {
            let status = if result.score >= 0.8 {
                format!("✅ {:.0}%", result.score * 100.0)
            } else if result.score >= 0.5 {
                format!("⚠️ {:.0}%", result.score * 100.0)
            } else if result.metrics.context_exceeded {
                "⊘ Context".to_string()
            } else {
                format!("❌ {:.0}%", result.score * 100.0)
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
                result.record_id,
                status,
                result.metrics.total_tokens,
                result.metrics.latency_ms,
                result.metrics.history_queries,
                error
            ));
        }

        report.push('\n');
    }

    // Analysis section
    report.push_str("## Analysis\n\n");
    report.push_str(&generate_analysis(&results.approach_results));

    report
}

/// Generate analysis text comparing approaches
fn generate_analysis(approaches: &[ApproachResults]) -> String {
    let mut analysis = String::new();

    // Find baseline and infinity approaches for comparison
    let baseline = approaches.iter().find(|s| s.approach_name == "baseline");
    let naive = approaches.iter().find(|s| s.approach_name == "naive_trim");
    let infinity = approaches
        .iter()
        .find(|s| s.approach_name == "infinity_context");

    if let (Some(baseline), Some(infinity)) = (baseline, infinity) {
        let score_improvement = infinity.avg_score - baseline.avg_score;

        if score_improvement > 0.0 {
            analysis.push_str(&format!(
                "**Infinity context shows {:.1}% score improvement** over baseline.\n\n",
                score_improvement * 100.0
            ));
        }

        if baseline.context_exceeded > 0 {
            analysis.push_str(&format!(
                "- Baseline failed on {} record(s) due to context limits\n",
                baseline.context_exceeded
            ));
        }

        if infinity.context_exceeded == 0 && baseline.context_exceeded > 0 {
            analysis.push_str("- Infinity context handled all records without context overflow\n");
        }
    }

    if let (Some(naive), Some(infinity)) = (naive, infinity) {
        let score_improvement = infinity.avg_score - naive.avg_score;

        if score_improvement > 0.0 {
            analysis.push_str(&format!(
                "- Infinity context shows {:.1}% score improvement over naive trimming\n",
                score_improvement * 100.0
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
                "- Average history queries per record: {:.1}\n",
                infinity.avg_history_queries
            ));
        }
    }

    if analysis.is_empty() {
        analysis.push_str("Insufficient data for comparative analysis.\n");
    }

    analysis.push_str("\n### Key Findings\n\n");

    // Generate key findings based on results
    let best_approach = approaches
        .iter()
        .max_by(|a, b| a.avg_score.partial_cmp(&b.avg_score).unwrap());

    if let Some(best) = best_approach {
        analysis.push_str(&format!(
            "1. **Best performing approach:** {} ({:.1}% avg score)\n",
            best.approach_name,
            best.accuracy()
        ));
    }

    let most_efficient = approaches
        .iter()
        .filter(|s| s.avg_score > 0.5)
        .min_by(|a, b| a.avg_tokens.partial_cmp(&b.avg_tokens).unwrap());

    if let Some(efficient) = most_efficient {
        analysis.push_str(&format!(
            "2. **Most token-efficient (>50% score):** {} ({:.0} avg tokens)\n",
            efficient.approach_name, efficient.avg_tokens
        ));
    }

    let fastest = approaches
        .iter()
        .filter(|s| s.avg_score > 0.5)
        .min_by(|a, b| a.avg_latency_ms.partial_cmp(&b.avg_latency_ms).unwrap());

    if let Some(fast) = fastest {
        analysis.push_str(&format!(
            "3. **Fastest (>50% score):** {} ({:.0}ms avg latency)\n",
            fast.approach_name, fast.avg_latency_ms
        ));
    }

    analysis
}
