use super::*;

#[test]
fn a_tail_keeps_the_end_and_says_what_it_dropped() {
    let kept = tail("abcdefghij", 4);
    assert!(kept.ends_with("ghij"), "{kept}");
    assert!(kept.contains("6 earlier characters omitted"), "{kept}");
    assert_eq!(tail("abc", 10), "abc");
}

#[test]
fn a_bounded_tail_stops_growing() {
    let mut output = BoundedTail::new(50);
    for _ in 0..200 {
        output.push("0123456789");
    }
    assert!(output.as_str().chars().count() <= 100);
    assert!(output.as_str().ends_with("0123456789"));
}

#[test]
fn history_and_events_are_bounded_to_their_configured_limits() {
    let config = Config {
        history_limit: 2,
        event_limit: 3,
        ..Config::default()
    };
    let workers: Vec<WorkerRecord> = (1..=5)
        .map(|n| WorkerRecord::new(format!("worker-{n}"), WorkerKind::Coding, n, 100))
        .collect();
    let events: VecDeque<String> = (1..=10).map(|n| format!("event-{n}")).collect();

    let observation = build(Evidence {
        job: "job",
        run_id: "run",
        status: "running",
        iteration: 4,
        workers: &workers,
        verification: &[],
        events: &events,
        previous_assessment: None,
        previous_intervention: None,
        failures: &[],
        elapsed: Duration::from_secs(1),
        git: GitEvidence::default(),
        tests: None,
        config: &config,
    });

    assert_eq!(observation.worker_history.len(), 2);
    assert_eq!(observation.worker_history[1].worker_id, "worker-5");
    // The most recent events, in the order they happened.
    assert_eq!(
        observation.recent_events,
        vec!["event-8", "event-9", "event-10"]
    );
    // Every worker is still counted, even when only some are described.
    assert_eq!(observation.attempts, 5);
}

#[test]
fn only_running_workers_are_active() {
    let mut workers = vec![
        WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100),
        WorkerRecord::new("worker-2", WorkerKind::Verifier, 2, 100),
    ];
    workers[0].status = WorkerStatus::Completed;
    workers[0].finished = Some(Instant::now());

    let observation = build(Evidence {
        job: "job",
        run_id: "run",
        status: "running",
        iteration: 2,
        workers: &workers,
        verification: &[],
        events: &VecDeque::new(),
        previous_assessment: None,
        previous_intervention: None,
        failures: &[],
        elapsed: Duration::from_secs(1),
        git: GitEvidence::default(),
        tests: None,
        config: &Config::default(),
    });

    assert_eq!(observation.active_workers.len(), 1);
    assert_eq!(observation.active_workers[0].worker_id, "worker-2");
    assert_eq!(observation.worker_history.len(), 2);
}

/// Run the fake runtime, absorbing the one failure the harness can cause.
///
/// These tests write an executable and then run it. Between the write's
/// `open` and its `close`, any sibling test that spawns a process — `git` in
/// the fixture and the evidence helpers, a stand-in worker — forks and its
/// child inherits the writable descriptor until it execs. `execve` answers
/// `ETXTBSY` while such a descriptor exists, so an unlucky interleaving
/// fails the spawn with "Text file busy".
///
/// That is a property of writing and exec'ing a file inside one
/// multi-threaded process, not of the code under test, and it cannot be
/// asserted away: the production spawn is `docker`, a binary nobody wrote a
/// moment ago. So retry that one error here, and let every other outcome —
/// including a spawn failure for any other reason — through to the
/// assertions unchanged.
#[cfg(unix)]
async fn run_tests_absorbing_exec_race(
    repository: &Path,
    command: &str,
    config: &Config,
    runtime: &str,
) -> TestRun {
    for _ in 0..20 {
        let run = run_tests_with_runtime(repository, command, config, runtime, "test-image").await;
        if !run.output_tail.contains("Text file busy") {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the fake runtime stayed unexecutable: ETXTBSY did not clear");
}

#[cfg(unix)]
#[tokio::test]
async fn a_test_run_reports_what_the_command_did() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::default();
    let runtime = fake_container_runtime(root.path());

    let green = run_tests_absorbing_exec_race(
        root.path(),
        "echo '2 passed, 0 failed'",
        &config,
        runtime.to_str().unwrap(),
    )
    .await;
    assert!(green.passed);
    assert_eq!(green.exit_code, Some(0));
    assert!(green.output_tail.contains("2 passed"));

    let red = run_tests_absorbing_exec_race(
        root.path(),
        "echo boom >&2; exit 3",
        &config,
        runtime.to_str().unwrap(),
    )
    .await;
    assert!(!red.passed);
    assert_eq!(red.exit_code, Some(3));
    // Both streams are evidence; a failure usually explains itself on stderr.
    assert!(red.output_tail.contains("boom"));

    let args = std::fs::read_to_string(root.path().join("runtime-args")).unwrap();
    assert!(args.contains("--network none"));
    assert!(args.contains("readonly"));
    assert!(args.contains("--cap-drop ALL"));
    assert!(args.contains("--security-opt no-new-privileges"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_test_command_that_never_returns_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let config = Config {
        test_timeout: Duration::from_millis(200),
        ..Config::default()
    };
    let runtime = fake_container_runtime(root.path());
    let run =
        run_tests_absorbing_exec_race(root.path(), "sleep 30", &config, runtime.to_str().unwrap())
            .await;
    assert!(!run.passed);
    assert_eq!(run.exit_code, None);
    assert!(run.output_tail.contains("exceeded"), "{}", run.output_tail);
}

/// Linux refuses exec with a writable handle; macOS allows it.
/// ETXTBSY is reachable on purpose, so prove both halves of the harness's
/// contract rather than asserting the race away: the spawn really does fail
/// that way while a write handle is open, and the wrapper really does
/// recover once it closes.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_held_write_handle_makes_the_runtime_unexecutable_until_it_closes() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::default();
    let runtime = fake_container_runtime(root.path());

    // Exactly what a sibling test's fork leaves behind for a moment.
    let held = std::fs::OpenOptions::new()
        .write(true)
        .open(&runtime)
        .expect("open the runtime for writing");
    let direct = run_tests_with_runtime(
        root.path(),
        "echo hi",
        &config,
        runtime.to_str().unwrap(),
        "test-image",
    )
    .await;
    assert!(
        direct.output_tail.contains("Text file busy"),
        "expected ETXTBSY while a write handle is open, got {}",
        direct.output_tail
    );

    let closing = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        drop(held);
    });
    let absorbed = run_tests_absorbing_exec_race(
        root.path(),
        "echo '2 passed, 0 failed'",
        &config,
        runtime.to_str().unwrap(),
    )
    .await;
    closing.await.expect("closer task");
    assert!(absorbed.passed, "{}", absorbed.output_tail);
}

#[cfg(unix)]
fn fake_container_runtime(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let runtime = root.join("fake-docker");
    // Emulates the slice of `docker run` this code depends on, including
    // its refusal to start when `--cidfile` names a path that exists.
    std::fs::write(
        &runtime,
        r#"#!/bin/bash
args="$*"
printf '%s' "$args" > "$(dirname "$0")/runtime-args"
previous=""
for argument in "$@"; do
  if [ "$previous" = "--cidfile" ]; then
if [ -e "$argument" ]; then
  echo "Container ID file found, make sure the other container isn't running or delete $argument" >&2
  exit 125
fi
printf 'fake-container-id' > "$argument"
  fi
  previous="$argument"
done
case "$args" in
  *'sleep 30'*) sleep 30 ;;
  *'exit 3'*) echo boom >&2; exit 3 ;;
  *) echo '2 passed, 0 failed' ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&runtime).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&runtime, permissions).unwrap();
    runtime
}

#[test]
fn an_observation_serializes_to_classifier_state() {
    let state = serde_json::to_value(crate::test_support::observation()).unwrap();
    assert!(state.get("original_job").is_some());
    assert!(state.get("git_diff").is_some());
    assert_eq!(state["active_workers"][0]["worker_kind"], "coding");
}
