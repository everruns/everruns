//! Reporting. The prototype renders a per-case summary, a model×eval pass-rate
//! matrix, and a JSON record. The spec's `report.html` (a self-contained,
//! no-server transcript viewer) and the historical `/benches` aggregation
//! consume this same JSON.

use crate::protocol::RunResult;
use crate::runner::RunReport;

/// Print a per-case list, a matrix table, and totals to stdout.
pub fn print_summary(report: &RunReport) {
    println!("\n── cases ──");
    for outcome in &report.outcomes {
        let mark = if outcome.passed { "PASS" } else { "FAIL" };
        println!(
            "  [{mark}] {}  ({:.0}%)",
            outcome.key(),
            outcome.aggregate * 100.0
        );
        for score in &outcome.scores {
            let m = if score.pass { "✓" } else { "✗" };
            println!("         {m} {} — {}", score.scorer, score.reason);
        }
    }

    print_matrix(report);

    for skipped in &report.skipped {
        println!("  [SKIP] {skipped}");
    }

    println!(
        "\n{} passed / {} total ({} failed, {} skipped)",
        report.passed(),
        report.total(),
        report.failed(),
        report.skipped.len(),
    );
}

/// A compact pass-rate grid: evals down the side, models across the top.
fn print_matrix(report: &RunReport) {
    let mut evals: Vec<String> = Vec::new();
    let mut models: Vec<String> = Vec::new();
    for o in &report.outcomes {
        if !evals.contains(&o.eval) {
            evals.push(o.eval.clone());
        }
        if !models.contains(&o.model) {
            models.push(o.model.clone());
        }
    }
    if evals.is_empty() || models.is_empty() {
        return;
    }

    println!("\n── matrix (passed/total) ──");
    let label_w = evals.iter().map(|e| e.len()).max().unwrap_or(4).max(4);
    let col_w = models.iter().map(|m| m.len()).max().unwrap_or(6).max(7);

    print!("  {:label_w$}", "eval", label_w = label_w);
    for m in &models {
        print!("  {:>col_w$}", m, col_w = col_w);
    }
    println!();

    for eval in &evals {
        print!("  {:label_w$}", eval, label_w = label_w);
        for model in &models {
            let cells: Vec<_> = report
                .outcomes
                .iter()
                .filter(|o| &o.eval == eval && &o.model == model)
                .collect();
            let passed = cells.iter().filter(|o| o.passed).count();
            let cell = format!("{passed}/{}", cells.len());
            print!("  {:>col_w$}", cell, col_w = col_w);
        }
        println!();
    }
}

// ----- protocol-result reporting (host side) --------------------------------

/// Print a per-case list, a model×eval matrix, and totals for protocol results
/// collected by the host (including any restored from a checkpoint).
pub fn print_results(results: &[RunResult]) {
    println!("\n── cases ──");
    for r in results {
        if r.skipped {
            println!("  [SKIP] {}/{}@{}", r.eval, r.sample, r.model);
            continue;
        }
        let mark = if r.passed { "PASS" } else { "FAIL" };
        println!(
            "  [{mark}] {}/{}@{}  ({:.0}%)",
            r.eval,
            r.sample,
            r.model,
            r.aggregate * 100.0
        );
        for s in &r.scores {
            let m = if s.pass { "✓" } else { "✗" };
            println!("         {m} {} — {}", s.scorer, s.reason);
        }
    }

    print_results_matrix(results);

    let ran: Vec<_> = results.iter().filter(|r| !r.skipped).collect();
    let passed = ran.iter().filter(|r| r.passed).count();
    let skipped = results.len() - ran.len();
    println!(
        "\n{} passed / {} ran ({} failed, {} skipped)",
        passed,
        ran.len(),
        ran.len() - passed,
        skipped,
    );
}

fn print_results_matrix(results: &[RunResult]) {
    let mut evals: Vec<String> = Vec::new();
    let mut models: Vec<String> = Vec::new();
    for r in results {
        if !evals.contains(&r.eval) {
            evals.push(r.eval.clone());
        }
        if !models.contains(&r.model) {
            models.push(r.model.clone());
        }
    }
    if evals.is_empty() || models.is_empty() {
        return;
    }

    println!("\n── matrix (passed/ran) ──");
    let label_w = evals.iter().map(|e| e.len()).max().unwrap_or(4).max(4);
    let col_w = models.iter().map(|m| m.len()).max().unwrap_or(6).max(7);

    print!("  {:label_w$}", "eval", label_w = label_w);
    for m in &models {
        print!("  {:>col_w$}", m, col_w = col_w);
    }
    println!();

    for eval in &evals {
        print!("  {:label_w$}", eval, label_w = label_w);
        for model in &models {
            let cells: Vec<_> = results
                .iter()
                .filter(|r| &r.eval == eval && &r.model == model && !r.skipped)
                .collect();
            let passed = cells.iter().filter(|r| r.passed).count();
            let cell = if cells.is_empty() {
                "—".to_string()
            } else {
                format!("{passed}/{}", cells.len())
            };
            print!("  {:>col_w$}", cell, col_w = col_w);
        }
        println!();
    }
}

/// Canonical JSON record over protocol results (host side).
pub fn results_json(results: &[RunResult]) -> serde_json::Value {
    let ran = results.iter().filter(|r| !r.skipped).count();
    let passed = results.iter().filter(|r| !r.skipped && r.passed).count();
    serde_json::json!({
        "summary": {
            "ran": ran,
            "passed": passed,
            "failed": ran - passed,
            "skipped": results.len() - ran,
        },
        "cases": results,
    })
}

/// Structured JSON record (scores + lightweight transcript fields; raw event
/// streams are omitted to keep the artifact small). This is the canonical
/// machine-readable output the HTML viewer and trend aggregation read.
pub fn to_json(report: &RunReport) -> serde_json::Value {
    let cases: Vec<serde_json::Value> = report
        .outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "eval": o.eval,
                "sample_id": o.sample_id,
                "model": o.model,
                "passed": o.passed,
                "aggregate": o.aggregate,
                "scores": o.scores,
                "transcript": {
                    "final_response": o.transcript.final_response,
                    "iterations": o.transcript.iterations,
                    "tool_calls_count": o.transcript.tool_calls_count,
                    "tool_calls": o.transcript.tool_calls,
                    "usage": o.transcript.usage,
                    "error": o.transcript.error,
                },
            })
        })
        .collect();

    serde_json::json!({
        "summary": {
            "total": report.total(),
            "passed": report.passed(),
            "failed": report.failed(),
            "skipped": report.skipped.len(),
        },
        "skipped": report.skipped,
        "cases": cases,
    })
}
