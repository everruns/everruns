// Paired comparison of an experiment's variants.
//
// Decision: every variant ran the same cases, so variants are compared case by
// case (paired), not by their overall rates alone. A case's value is the share
// of its trials that passed. The verdict comes from a paired bootstrap over
// cases: resample the cases with replacement, take the mean difference from
// the baseline, and read the 95% interval. With twenty cases most differences
// sit inside the noise, and the verdict says so instead of calling a winner.
//
// Decision: the bootstrap is seeded from the experiment id, so the same
// results always give the same interval.

use super::types::{CaseCell, CaseComparison, ExperimentComparison, VariantVerdict, Verdict};
use crate::domains::evals::record::CaseResultStatus;
use crate::domains::evals::summary::result_cost_usd;
use crate::storage::{EvalCaseResultRow, EvalCaseRow};
use everruns_contracts::typed_id::EvalCaseId;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Resamples drawn for the interval.
const RESAMPLES: usize = 2000;
/// Fewer paired cases than this give no verdict.
const MIN_PAIRED_CASES: usize = 3;

/// One variant's results, in the experiment's variant order.
pub struct VariantResults<'a> {
    pub label: &'a str,
    pub results: &'a [EvalCaseResultRow],
}

#[derive(Default, Clone, Copy)]
struct Tally {
    trials: u32,
    passed: u32,
    cost_usd: f64,
}

impl Tally {
    fn rate(&self) -> Option<f64> {
        (self.trials > 0).then(|| self.passed as f64 / self.trials as f64)
    }
    fn all_passed(&self) -> bool {
        self.trials > 0 && self.passed == self.trials
    }
}

fn tally(results: &[EvalCaseResultRow]) -> BTreeMap<Uuid, Tally> {
    let mut by_case: BTreeMap<Uuid, Tally> = BTreeMap::new();
    for result in results {
        let status = CaseResultStatus::from(result.status.as_str());
        let counted = matches!(
            status,
            CaseResultStatus::Passed
                | CaseResultStatus::Failed
                | CaseResultStatus::Errored
                | CaseResultStatus::Timeout
        );
        let entry = by_case.entry(result.eval_case_id).or_default();
        entry.cost_usd += result_cost_usd(result);
        if counted {
            entry.trials += 1;
            if status == CaseResultStatus::Passed {
                entry.passed += 1;
            }
        }
    }
    by_case
}

/// Compare `variants` (the first is the baseline) over `cases`.
pub fn compare(
    seed: Uuid,
    cases: &[EvalCaseRow],
    variants: &[VariantResults<'_>],
) -> ExperimentComparison {
    let tallies: Vec<BTreeMap<Uuid, Tally>> = variants.iter().map(|v| tally(v.results)).collect();
    let baseline = variants
        .first()
        .map(|v| v.label.to_string())
        .unwrap_or_default();

    let mut rows: Vec<(bool, i32, CaseComparison)> = Vec::new();
    for case in cases {
        // Only cases some variant ran (a run snapshots its cases).
        if !tallies.iter().any(|t| t.contains_key(&case.id)) {
            continue;
        }
        let cells: Vec<CaseCell> = variants
            .iter()
            .zip(&tallies)
            .map(|(variant, t)| {
                let tally = t.get(&case.id).copied().unwrap_or_default();
                CaseCell {
                    label: variant.label.to_string(),
                    passed: tally.passed,
                    trials: tally.trials,
                    cost_usd: tally.cost_usd,
                }
            })
            .collect();
        let base_rate = tallies
            .first()
            .and_then(|t| t.get(&case.id))
            .and_then(Tally::rate);
        let changed = tallies.iter().skip(1).any(|t| {
            let rate = t.get(&case.id).and_then(Tally::rate);
            rate.is_some() && base_rate.is_some() && rate != base_rate
        });
        rows.push((
            changed,
            case.position,
            CaseComparison {
                case_id: EvalCaseId::from_uuid(case.id).to_string(),
                case_name: case.name.clone(),
                changed,
                cells,
            },
        ));
    }
    // Changed cases first, then in the dataset's order.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let verdicts = variants
        .iter()
        .zip(&tallies)
        .skip(1)
        .map(|(variant, t)| verdict(seed, variant.label, &tallies[0], t))
        .collect();

    ExperimentComparison {
        baseline,
        verdicts,
        cases: rows.into_iter().map(|(_, _, row)| row).collect(),
    }
}

fn verdict(
    seed: Uuid,
    label: &str,
    base: &BTreeMap<Uuid, Tally>,
    other: &BTreeMap<Uuid, Tally>,
) -> VariantVerdict {
    let mut diffs = Vec::new();
    let (mut fixed, mut broke) = (0u32, 0u32);
    for (case, b) in base {
        let Some(o) = other.get(case) else { continue };
        let (Some(br), Some(or)) = (b.rate(), o.rate()) else {
            continue;
        };
        diffs.push(or - br);
        if o.all_passed() && !b.all_passed() {
            fixed += 1;
        }
        if b.all_passed() && !o.all_passed() {
            broke += 1;
        }
    }
    let paired_cases = diffs.len() as u32;
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    if diffs.len() < MIN_PAIRED_CASES {
        return VariantVerdict {
            label: label.to_string(),
            paired_cases,
            pass_rate_delta_points: (!diffs.is_empty()).then(|| mean(&diffs) * 100.0),
            interval_low_points: None,
            interval_high_points: None,
            fixed,
            broke,
            verdict: Verdict::NotEnoughData,
        };
    }
    let (low, high) = bootstrap_interval(seed, &diffs);
    let verdict = if low > 0.0 {
        Verdict::Improvement
    } else if high < 0.0 {
        Verdict::Regression
    } else {
        Verdict::WithinNoise
    };
    VariantVerdict {
        label: label.to_string(),
        paired_cases,
        pass_rate_delta_points: Some(mean(&diffs) * 100.0),
        interval_low_points: Some(low * 100.0),
        interval_high_points: Some(high * 100.0),
        fixed,
        broke,
        verdict,
    }
}

/// 95% interval of the mean of `diffs` by resampling them with replacement.
fn bootstrap_interval(seed: Uuid, diffs: &[f64]) -> (f64, f64) {
    // xorshift64*: enough randomness for resampling, and reproducible.
    let mut state = (seed.as_u128() as u64) | 1;
    let mut next = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let n = diffs.len();
    let mut means: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            (0..n)
                .map(|_| diffs[(next() % n as u64) as usize])
                .sum::<f64>()
                / n as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    let at = |q: f64| means[((RESAMPLES as f64 - 1.0) * q).round() as usize];
    (at(0.025), at(0.975))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn case(position: i32, name: &str) -> EvalCaseRow {
        EvalCaseRow {
            id: Uuid::now_v7(),
            eval_id: Uuid::nil(),
            public_id: String::new(),
            name: name.into(),
            description: None,
            target: None,
            tags: vec![],
            conversation: serde_json::json!([]),
            post: None,
            artifacts: None,
            scorers: serde_json::json!([]),
            max_turns: None,
            timeout_seconds: None,
            position,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn results(cases: &[EvalCaseRow], passed: &[u32], trials: u32) -> Vec<EvalCaseResultRow> {
        let mut out = Vec::new();
        for (case, &ok) in cases.iter().zip(passed) {
            for trial in 0..trials {
                let status = if trial < ok { "passed" } else { "failed" };
                out.push(crate::domains::evals::summary::tests::result_row(
                    case.id,
                    trial as i32,
                    status,
                ));
            }
        }
        out
    }

    #[test]
    fn a_consistent_gain_is_an_improvement_and_changed_cases_come_first() {
        let cases: Vec<_> = (0..12).map(|i| case(i, &format!("case {i}"))).collect();
        let base = results(&cases, &[1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3], 3);
        let cand = results(&cases, &[3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2], 3);
        let cmp = compare(
            Uuid::nil(),
            &cases,
            &[
                VariantResults {
                    label: "v14",
                    results: &base,
                },
                VariantResults {
                    label: "v15",
                    results: &cand,
                },
            ],
        );
        assert_eq!(cmp.baseline, "v14");
        let v = &cmp.verdicts[0];
        assert_eq!(v.verdict, Verdict::Improvement, "{v:?}");
        assert_eq!((v.fixed, v.broke), (6, 1));
        assert!(v.interval_low_points.unwrap() > 0.0);
        assert!(cmp.cases[..7].iter().all(|c| c.changed));
        assert!(!cmp.cases[7].changed);
        assert_eq!(cmp.cases[0].cells[1].passed, 3);
    }

    #[test]
    fn mixed_results_are_within_noise_and_few_cases_give_no_verdict() {
        let cases: Vec<_> = (0..10).map(|i| case(i, "c")).collect();
        let base = results(&cases, &[3, 0, 3, 0, 3, 0, 3, 0, 3, 0], 3);
        let cand = results(&cases, &[0, 3, 0, 3, 0, 3, 0, 3, 3, 0], 3);
        let cmp = compare(
            Uuid::from_u128(7),
            &cases,
            &[
                VariantResults {
                    label: "a",
                    results: &base,
                },
                VariantResults {
                    label: "b",
                    results: &cand,
                },
            ],
        );
        assert_eq!(cmp.verdicts[0].verdict, Verdict::WithinNoise);

        let few = &cases[..2];
        let cmp = compare(
            Uuid::nil(),
            few,
            &[
                VariantResults {
                    label: "a",
                    results: &results(few, &[0, 0], 1),
                },
                VariantResults {
                    label: "b",
                    results: &results(few, &[1, 1], 1),
                },
            ],
        );
        assert_eq!(cmp.verdicts[0].verdict, Verdict::NotEnoughData);
        assert_eq!(cmp.verdicts[0].pass_rate_delta_points, Some(100.0));
    }
}
