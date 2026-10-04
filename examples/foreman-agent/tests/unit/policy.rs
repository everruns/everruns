use super::*;

#[test]
fn a_new_job_dispatches_coding_without_a_classifier_request() {
    let decision = decide(
        &Floor::default(),
        &Assessment::default(),
        &Config::default(),
    );
    assert_eq!(decision.action, Action::StartWorker);
    assert_eq!(decision.reason, "dispatching the coding role");
}

fn working() -> Floor {
    Floor {
        iteration: 1,
        active_worker: Some("worker-1".into()),
        workers_started: 1,
        ..Floor::default()
    }
}

fn idle() -> Floor {
    Floor {
        iteration: 1,
        workers_started: 1,
        ..Floor::default()
    }
}

fn complete() -> Assessment {
    Assessment {
        implementation_complete: 0.95,
        tests_sufficient: 0.9,
        requirements_satisfied: 0.92,
        needs_verification: 0.1,
        ready_to_finish: 0.93,
        meaningful_progress: 0.9,
        ..Assessment::default()
    }
}

#[test]
fn a_person_is_needed_before_anything_else() {
    let assessment = Assessment {
        needs_human: 0.81,
        // Every other signal says finish; the human threshold still wins.
        ..complete()
    };
    assert_eq!(
        decide(&idle(), &assessment, &Config::default()).action,
        Action::Escalate
    );
}

#[test]
fn the_iteration_ceiling_escalates_even_on_healthy_evidence() {
    let floor = Floor {
        iteration: 20,
        ..working()
    };
    let decision = decide(&floor, &complete(), &Config::default());
    assert_eq!(decision.action, Action::Escalate);
    assert_eq!(decision.reason, "maximum supervisory iterations reached");
}

#[test]
fn drift_outranks_stuckness_and_names_the_worker() {
    let assessment = Assessment {
        work_off_track: 0.9,
        worker_stuck: 0.95,
        ..Assessment::default()
    };
    let decision = decide(&working(), &assessment, &Config::default());
    assert_eq!(decision.action, Action::StopWorker);
    assert_eq!(decision.reason, "active worker appears off track");
    assert_eq!(decision.worker.as_deref(), Some("worker-1"));
}

#[test]
fn a_stuck_worker_is_stopped() {
    let assessment = Assessment {
        worker_stuck: 0.85,
        ..Assessment::default()
    };
    assert_eq!(
        decide(&working(), &assessment, &Config::default()).action,
        Action::StopWorker
    );
}

#[test]
fn an_idle_floor_cannot_be_stopped() {
    let assessment = Assessment {
        worker_stuck: 0.99,
        work_off_track: 0.99,
        ..Assessment::default()
    };
    assert_eq!(
        decide(&idle(), &assessment, &Config::default()).action,
        Action::StartWorker
    );
}

#[test]
fn a_stop_is_followed_by_one_retry_then_escalation() {
    let mut floor = Floor {
        last_action: Some(Action::StopWorker),
        ..idle()
    };
    assert_eq!(
        decide(&floor, &Assessment::default(), &Config::default()).action,
        Action::RetryWorker
    );

    floor.retries = 1;
    let decision = decide(&floor, &Assessment::default(), &Config::default());
    assert_eq!(decision.action, Action::Escalate);
    assert_eq!(decision.reason, "worker retry limit reached");
}

#[test]
fn verification_is_started_once_and_only_once() {
    let assessment = Assessment {
        implementation_complete: 0.9,
        needs_verification: 0.8,
        ..Assessment::default()
    };
    assert_eq!(
        decide(&idle(), &assessment, &Config::default()).action,
        Action::StartVerifier
    );

    let floor = Floor {
        verification_started: true,
        ..idle()
    };
    assert_eq!(
        decide(&floor, &assessment, &Config::default()).action,
        Action::Escalate
    );
}

#[test]
fn verification_waits_for_implementation() {
    let assessment = Assessment {
        implementation_complete: 0.4,
        needs_verification: 0.9,
        ..Assessment::default()
    };
    assert_eq!(
        decide(&idle(), &assessment, &Config::default()).action,
        Action::StartWorker
    );
}

#[test]
fn finishing_needs_completion_and_a_resolved_verification() {
    let config = Config::default();
    assert_eq!(
        decide(&idle(), &complete(), &config).action,
        Action::StartVerifier
    );

    // Same completion evidence, but verification is now wanted and has not
    // happened: the job is not finishable yet.
    let wants_verification = Assessment {
        needs_verification: 0.9,
        ..complete()
    };
    assert_eq!(
        decide(&idle(), &wants_verification, &config).action,
        Action::StartVerifier
    );

    let verified = Floor {
        verification_started: true,
        verification_completed: true,
        workers_started: 2,
        ..idle()
    };
    assert_eq!(
        decide(&verified, &wants_verification, &config).action,
        Action::Finish
    );
}

#[test]
fn weak_tests_hold_a_finish_back() {
    let assessment = Assessment {
        tests_sufficient: 0.3,
        ..complete()
    };
    assert_eq!(
        decide(&idle(), &assessment, &Config::default()).action,
        Action::StartWorker
    );
}

#[test]
fn the_worker_budget_escalates_rather_than_starting_a_fourth() {
    let floor = Floor {
        workers_started: 3,
        ..idle()
    };
    let decision = decide(&floor, &Assessment::default(), &Config::default());
    assert_eq!(decision.action, Action::Escalate);
    assert_eq!(decision.reason, "worker limit reached before completion");
}

#[test]
fn an_unremarkable_floor_lets_the_worker_work() {
    let assessment = Assessment {
        meaningful_progress: 0.9,
        implementation_complete: 0.4,
        ..Assessment::default()
    };
    assert_eq!(
        decide(&working(), &assessment, &Config::default()).action,
        Action::Continue
    );
}

#[test]
fn environment_overrides_land_on_the_defaults_they_name() {
    // Read through the same helpers the override path uses; setting process
    // environment in a threaded test runner is not safe.
    assert_eq!(count("FOREMAN_UNSET_LIMIT_FOR_TEST"), None);
    assert_eq!(seconds("FOREMAN_UNSET_SECONDS_FOR_TEST"), None);
    let config = Config::default();
    assert_eq!(config.max_workers, 3);
    assert_eq!(config.periodic_assessment, Duration::from_secs(30));
}

#[test]
fn finishing_always_requires_independent_verification() {
    assert_eq!(
        decide(&idle(), &complete(), &Config::default()).action,
        Action::StartVerifier
    );
}

#[test]
fn a_known_failing_suite_blocks_finish_despite_high_scores() {
    let floor = Floor {
        verification_started: true,
        verification_completed: true,
        tests_passed: Some(false),
        ..idle()
    };
    assert_ne!(
        decide(&floor, &complete(), &Config::default()).action,
        Action::Finish
    );
}

#[test]
fn a_repaired_repository_needs_another_verification_pass() {
    let floor = Floor {
        workers_started: 3,
        verification_started: false,
        verification_completed: false,
        ..idle()
    };
    let config = Config {
        max_workers: 4,
        ..Config::default()
    };
    assert_eq!(
        decide(&floor, &complete(), &config).action,
        Action::StartVerifier
    );
}
