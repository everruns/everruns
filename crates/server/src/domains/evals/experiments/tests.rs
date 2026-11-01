use super::*;
use chrono::Utc;

fn summary(pass_rate: f64, cases: u32, every: u32) -> RunSummary {
    let mut s = empty_summary();
    s.pass_rate = pass_rate;
    s.cases = cases;
    s.cases_passed_every_trial = every;
    s
}

fn variant(label: &str, status: EvalRunStatus, summary: Option<RunSummary>) -> ExperimentVariant {
    ExperimentVariant {
        label: label.into(),
        run_id: String::new(),
        target: None,
        model_override: None,
        status,
        summary,
        cost_per_case_usd: None,
    }
}

fn experiment(variants: Vec<ExperimentVariant>, verdict: Option<Verdict>) -> EvalExperiment {
    let status = status_of(&variants);
    EvalExperiment {
        id: String::new(),
        eval_id: String::new(),
        name: "x".into(),
        dataset_version: 1,
        trials: 3,
        max_cost_usd: None,
        baseline: "base".into(),
        status,
        comparison: verdict.map(|verdict| ExperimentComparison {
            baseline: "base".into(),
            verdicts: vec![VariantVerdict {
                label: "cand".into(),
                paired_cases: 10,
                pass_rate_delta_points: None,
                interval_low_points: None,
                interval_high_points: None,
                fixed: 0,
                broke: 0,
                verdict,
            }],
            cases: vec![],
        }),
        variants,
        created_at: Utc::now(),
    }
}

#[test]
fn the_gate_waits_for_every_run_and_judges_the_last_variant_by_default() {
    let running = experiment(
        vec![
            variant("base", EvalRunStatus::Completed, Some(summary(0.8, 10, 7))),
            variant("cand", EvalRunStatus::Running, None),
        ],
        None,
    );
    assert!(matches!(
        gate(&running, &GateRule::default()),
        Err(GateRefusal::Running)
    ));

    let done = experiment(
        vec![
            variant("base", EvalRunStatus::Completed, Some(summary(0.8, 10, 7))),
            variant("cand", EvalRunStatus::Completed, Some(summary(0.93, 10, 9))),
        ],
        Some(Verdict::Improvement),
    );
    let passed = gate(
        &done,
        &GateRule {
            min_pass_rate: Some(0.9),
            min_passed_every_trial: Some(0.9),
            no_regression: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(passed.variant, "cand");
    assert_eq!(passed.verdict, Some(Verdict::Improvement));
    assert!((passed.passed_every_trial - 0.9).abs() < 1e-9);
}

#[test]
fn the_gate_names_every_bar_the_variant_missed() {
    let done = experiment(
        vec![
            variant("base", EvalRunStatus::Completed, Some(summary(0.9, 10, 9))),
            variant("cand", EvalRunStatus::Completed, Some(summary(0.7, 10, 5))),
        ],
        Some(Verdict::Regression),
    );
    let Err(GateRefusal::Failed(reasons)) = gate(
        &done,
        &GateRule {
            min_pass_rate: Some(0.9),
            min_passed_every_trial: Some(0.8),
            no_regression: true,
            ..Default::default()
        },
    ) else {
        panic!("expected the gate to fail");
    };
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert!(reasons[0].contains("70%"), "{reasons:?}");
    assert!(reasons[2].contains("regression"), "{reasons:?}");

    // The baseline itself can be judged, and a bar above 1 is refused.
    assert!(
        gate(
            &done,
            &GateRule {
                variant: Some("base".into()),
                min_pass_rate: Some(0.9),
                ..Default::default()
            }
        )
        .is_ok()
    );
    assert!(matches!(
        gate(
            &done,
            &GateRule {
                min_pass_rate: Some(90.0),
                ..Default::default()
            }
        ),
        Err(GateRefusal::Invalid(_))
    ));
    assert!(matches!(
        gate(
            &done,
            &GateRule {
                variant: Some("nope".into()),
                ..Default::default()
            }
        ),
        Err(GateRefusal::Invalid(_))
    ));
}

#[test]
fn a_failed_or_cancelled_variant_settles_the_experiment() {
    let failed = experiment(
        vec![
            variant("base", EvalRunStatus::Completed, None),
            variant("cand", EvalRunStatus::Failed, None),
        ],
        None,
    );
    assert_eq!(failed.status, ExperimentStatus::Failed);
    let Err(GateRefusal::Failed(reasons)) = gate(&failed, &GateRule::default()) else {
        panic!("a failed run never passes the gate");
    };
    assert!(reasons[0].contains("failed"), "{reasons:?}");
}

#[test]
fn variants_are_validated() {
    let spec = |label: &str| ExperimentVariantSpec {
        label: label.into(),
        target: None,
        model_override: None,
    };
    let req = |variants: Vec<ExperimentVariantSpec>| CreateEvalExperimentRequest {
        variants,
        ..Default::default()
    };
    assert!(validate(&req(vec![])).is_err());
    assert!(validate(&req((0..5).map(|i| spec(&format!("v{i}"))).collect())).is_err());
    assert!(validate(&req(vec![spec("a"), spec(" a ")])).is_err());
    assert!(validate(&req(vec![spec(" ")])).is_err());
    assert_eq!(
        validate(&req(vec![spec("a"), spec("b")])).unwrap(),
        DEFAULT_TRIALS
    );
}
