// Run summaries computed from stored rows.
//
// Decision: the summary is always rebuilt from the result rows, never from
// in-memory tallies of the task that happened to finish last. Case work runs
// as independent durable tasks that may finish on different replicas or be
// retried, so the database is the only complete picture.
//
// Decision: trials are repetitions of one logical case. `total`/`passed`/
// `pass_rate` count every repetition (per-trial success rate, pass@1), while
// `cases_passed_every_trial` (pass^k) and `cases_passed_any_trial` (pass@k)
// group them back by case. pass^k is the number an agent that must work every
// time is judged by.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domains::evals::record::{CaseResultStatus, RunSummary, Score, Scorer};
use crate::storage::{EvalCaseResultRow, EvalCaseRow};

/// Build the summary of an internal run from its case and result rows.
pub fn build_run_summary(
    case_rows: &[EvalCaseRow],
    result_rows: &[EvalCaseResultRow],
    trials: u32,
) -> RunSummary {
    let total = result_rows.len() as u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errored = 0u32;
    let mut total_score = 0.0f64;
    let mut total_turns = 0.0f64;
    let mut total_latency = 0u64;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;
    // Per case: (trials seen, trials passed).
    let mut by_case: BTreeMap<Uuid, (u32, u32)> = BTreeMap::new();

    for result in result_rows {
        let status = CaseResultStatus::from(result.status.as_str());
        let entry = by_case.entry(result.eval_case_id).or_default();
        entry.0 += 1;
        match status {
            CaseResultStatus::Passed => {
                passed += 1;
                entry.1 += 1;
            }
            CaseResultStatus::Failed => failed += 1,
            CaseResultStatus::Errored | CaseResultStatus::Timeout => errored += 1,
            CaseResultStatus::Pending | CaseResultStatus::Running | CaseResultStatus::Skipped => {}
        }

        if matches!(status, CaseResultStatus::Passed | CaseResultStatus::Failed) {
            total_score += case_result_avg_score(
                result,
                case_rows.iter().find(|case| case.id == result.eval_case_id),
            );
            total_turns += result.turns.unwrap_or_default() as f64;
            total_latency += result.latency_ms.unwrap_or_default() as u64;
            total_input_tokens += result.input_tokens.unwrap_or_default() as u64;
            total_output_tokens += result.output_tokens.unwrap_or_default() as u64;
        }
    }

    let ratio = |n: f64| if total > 0 { n / total as f64 } else { 0.0 };
    RunSummary {
        total,
        passed,
        failed,
        errored,
        pass_rate: ratio(passed as f64),
        avg_score: ratio(total_score),
        avg_turns: ratio(total_turns),
        avg_latency_ms: if total > 0 {
            total_latency / total as u64
        } else {
            0
        },
        total_input_tokens,
        total_output_tokens,
        trials: trials.max(1),
        cases: by_case.len() as u32,
        cases_passed_every_trial: by_case
            .values()
            .filter(|(seen, ok)| *seen > 0 && ok == seen)
            .count() as u32,
        cases_passed_any_trial: by_case.values().filter(|(_, ok)| *ok > 0).count() as u32,
        total_cost_usd: Some(result_rows.iter().map(result_cost_usd).sum()),
    }
}

/// What one result spent: its session's model calls (`cost_usd`) plus its
/// grading (`grader_usage.cost_usd`), both recorded in result metadata.
pub fn result_cost_usd(result: &EvalCaseResultRow) -> f64 {
    let Some(meta) = result.metadata.as_ref() else {
        return 0.0;
    };
    let agent = meta.get("cost_usd").and_then(|v| v.as_f64());
    let grading = meta
        .pointer("/grader_usage/cost_usd")
        .and_then(|v| v.as_f64());
    agent.unwrap_or(0.0) + grading.unwrap_or(0.0)
}

/// Weighted score of one result, joining its scores positionally to the
/// case's scorers. Falls back to the plain mean when they do not line up.
pub fn case_result_avg_score(result: &EvalCaseResultRow, case_row: Option<&EvalCaseRow>) -> f64 {
    let Some(scores) = result
        .scores
        .clone()
        .and_then(|value| serde_json::from_value::<Vec<Score>>(value).ok())
    else {
        return 0.0;
    };
    if scores.is_empty() {
        return 0.0;
    }

    let scorers = case_row
        .and_then(|case| serde_json::from_value::<Vec<Scorer>>(case.scorers.clone()).ok())
        .unwrap_or_default();
    weighted_score(&scores, &scorers)
}

/// Weighted mean of `scores` by their positional `scorers`' weights.
pub fn weighted_score(scores: &[Score], scorers: &[Scorer]) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    if scorers.len() == scores.len() {
        let total_weight: f64 = scorers.iter().map(Scorer::weight).sum();
        if total_weight > 0.0 {
            let weighted_sum: f64 = scores
                .iter()
                .zip(scorers)
                .map(|(score, scorer)| score.value * scorer.weight())
                .sum();
            return weighted_sum / total_weight;
        }
    }
    scores.iter().map(|score| score.value).sum::<f64>() / scores.len() as f64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Utc;

    /// A result row for tests: `status` with a matching score.
    pub(crate) fn result_row(case: Uuid, trial: i32, status: &str) -> EvalCaseResultRow {
        EvalCaseResultRow {
            id: Uuid::now_v7(),
            eval_run_id: Uuid::nil(),
            eval_case_id: case,
            public_id: String::new(),
            session_id: None,
            target: None,
            target_snapshot: None,
            status: status.to_string(),
            scores: Some(
                serde_json::json!([{ "pass": status == "passed", "value": if status == "passed" { 1.0 } else { 0.0 }, "reason": "" }]),
            ),
            metadata: None,
            turns: Some(2),
            latency_ms: Some(1000),
            input_tokens: Some(10),
            output_tokens: Some(5),
            error_message: None,
            artifacts: None,
            trial,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn trials_group_back_into_pass_every_and_pass_any() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let rows = vec![
            result_row(a, 0, "passed"),
            result_row(a, 1, "passed"),
            result_row(a, 2, "passed"),
            result_row(b, 0, "passed"),
            result_row(b, 1, "failed"),
            result_row(b, 2, "passed"),
            result_row(c, 0, "failed"),
            result_row(c, 1, "errored"),
            result_row(c, 2, "failed"),
        ];
        let summary = build_run_summary(&[], &rows, 3);
        assert_eq!(summary.total, 9);
        assert_eq!(summary.passed, 5);
        assert_eq!(summary.failed, 3);
        assert_eq!(summary.errored, 1);
        assert_eq!(summary.trials, 3);
        assert_eq!(summary.cases, 3);
        assert_eq!(summary.cases_passed_every_trial, 1);
        assert_eq!(summary.cases_passed_any_trial, 2);
        assert!((summary.pass_rate - 5.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn single_trial_run_matches_plain_counts() {
        let a = Uuid::now_v7();
        let summary = build_run_summary(&[], &[result_row(a, 0, "passed")], 1);
        assert_eq!(summary.cases_passed_every_trial, 1);
        assert_eq!(summary.cases_passed_any_trial, 1);
        assert_eq!(summary.pass_rate, 1.0);
        assert_eq!(summary.avg_score, 1.0);
    }

    #[test]
    fn skipped_trial_breaks_pass_every_trial() {
        let a = Uuid::now_v7();
        let rows = vec![result_row(a, 0, "passed"), result_row(a, 1, "skipped")];
        let summary = build_run_summary(&[], &rows, 2);
        assert_eq!(summary.cases_passed_every_trial, 0);
        assert_eq!(summary.cases_passed_any_trial, 1);
    }

    #[test]
    fn run_cost_adds_agent_and_grading_spend_of_every_result() {
        let a = Uuid::now_v7();
        let mut scored = result_row(a, 0, "passed");
        scored.metadata = Some(serde_json::json!({
            "cost_usd": 0.25,
            "grader_usage": {"input_tokens": 10, "output_tokens": 2, "cost_usd": 0.05},
        }));
        // A case stopped by a budget still spent what its agent spent.
        let mut stopped = result_row(a, 1, "skipped");
        stopped.metadata = Some(serde_json::json!({ "cost_usd": 0.5 }));
        let untouched = result_row(a, 2, "skipped");
        let summary = build_run_summary(&[], &[scored, stopped, untouched], 3);
        let cost = summary.total_cost_usd.expect("internal runs report cost");
        assert!((cost - 0.8).abs() < 1e-9, "{cost}");
    }
}
