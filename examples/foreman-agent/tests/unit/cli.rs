use super::*;

fn run(arguments: &[&str]) -> Run {
    match Cli::try_parse_from(arguments).unwrap().command {
        Command::Run(run) => run,
        other => panic!("expected a run, got {other:?}"),
    }
}

#[test]
fn foremans_own_invocation_parses() {
    // The line from Foreman's README, unchanged.
    let run = run(&[
        "foreman",
        "run",
        "--repo",
        "./my-project",
        "--job",
        "Add rate limiting to the API and make sure it is properly tested.",
    ]);
    assert_eq!(run.repo, PathBuf::from("./my-project"));
    assert!(run.job.starts_with("Add rate limiting"));
    // An Everruns session is the default crew; Foreman's Codex worker is
    // one flag away.
    assert_eq!(run.worker, WorkerChoice::Session);
    assert!(run.external().unwrap().is_none());
}

#[test]
fn a_test_command_is_optional_and_passed_through() {
    assert_eq!(
        run(&["foreman", "run", "--repo", ".", "--job", "j"]).tests,
        None
    );
    assert_eq!(
        run(&[
            "foreman",
            "run",
            "--repo",
            ".",
            "--job",
            "j",
            "--tests",
            "cargo test"
        ])
        .tests
        .as_deref(),
        Some("cargo test")
    );
}

#[test]
fn a_named_worker_selects_its_preset() {
    let codex = run(&[
        "foreman", "run", "--repo", ".", "--job", "j", "--worker", "codex",
    ])
    .external()
    .unwrap()
    .unwrap();
    assert_eq!(codex.label, "codex");

    let yolop = run(&[
        "foreman", "run", "--repo", ".", "--job", "j", "--worker", "yolop",
    ])
    .external()
    .unwrap()
    .unwrap();
    assert_eq!(yolop.label, "yolop");
}

#[test]
fn a_command_template_outranks_a_named_worker() {
    let run = run(&[
        "foreman",
        "run",
        "--repo",
        ".",
        "--job",
        "j",
        "--worker",
        "codex",
        "--worker-command",
        "mycli --cd {repo} --task {mission}",
    ]);
    let agent = run.external().unwrap().unwrap();
    assert_eq!(agent.label, "mycli");
    assert_eq!(
        agent.command(std::path::Path::new("/r"), "go"),
        vec!["mycli", "--cd", "/r", "--task", "go"]
    );
}

#[test]
fn a_template_with_nowhere_to_put_the_mission_is_refused() {
    let run = run(&[
        "foreman",
        "run",
        "--repo",
        ".",
        "--job",
        "j",
        "--worker-command",
        "mycli --cd {repo}",
    ]);
    assert!(run.external().is_err());
}

#[test]
fn a_run_without_a_job_is_not_a_run() {
    assert!(Cli::try_parse_from(["foreman", "run", "--repo", "."]).is_err());
}

#[test]
fn a_demo_needs_nothing_and_takes_the_same_workers() {
    let Command::Demo(demo) = Cli::try_parse_from(["foreman", "demo"]).unwrap().command else {
        panic!("expected a demo");
    };
    assert_eq!(demo.worker, WorkerChoice::Session);
    assert!(!demo.interactive);
    assert!(demo.external().unwrap().is_none());

    let Command::Demo(demo) = Cli::try_parse_from(["foreman", "demo", "--worker", "codex"])
        .unwrap()
        .command
    else {
        panic!("expected a demo");
    };
    assert_eq!(demo.external().unwrap().unwrap().label, "codex");
}

#[test]
fn an_interactive_demo_accepts_a_job_from_the_terminal() {
    let Command::Demo(demo) = Cli::try_parse_from(["foreman", "demo", "--interactive"])
        .unwrap()
        .command
    else {
        panic!("expected a demo");
    };
    assert!(demo.interactive);
}

#[test]
fn a_relative_repository_is_resolved_once_for_cwd_and_argv() {
    let scratch = tempfile::tempdir_in(".").unwrap();
    let root = repository(scratch.path()).unwrap();
    assert!(root.is_absolute());
    let agent = ExternalAgent::codex();
    let argv = agent.command(&root, "Check.");
    assert_eq!(argv[3], root.display().to_string());
    assert_eq!(root, scratch.path().canonicalize().unwrap());
}
