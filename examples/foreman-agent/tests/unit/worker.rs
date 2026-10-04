use super::*;

#[test]
fn codex_runs_foremans_own_command_line() {
    let argv = ExternalAgent::codex().command(Path::new("/tmp/repo"), "Add rate limiting.");
    assert_eq!(
        argv,
        vec![
            "codex",
            "exec",
            "--cd",
            "/tmp/repo",
            "--color",
            "never",
            "--json",
            "--sandbox",
            "workspace-write",
            "Add rate limiting.",
        ]
    );
}

#[test]
fn yolop_runs_its_one_shot_print_interface() {
    let argv = ExternalAgent::yolop().command(Path::new("/tmp/repo"), "Fix it.");
    assert_eq!(argv, vec!["yolop", "-C", "/tmp/repo", "-p", "Fix it."]);
}

#[test]
fn a_mission_stays_one_argument_however_it_is_written() {
    // No shell sits between the template and the process, so a mission
    // carrying quotes, newlines, or a semicolon is still one argv entry.
    let mission = "Fix \"it\"; then\nrun the tests";
    let argv = ExternalAgent::yolop().command(Path::new("/tmp/repo"), mission);
    assert_eq!(argv.len(), 5);
    assert_eq!(argv[4], mission);
}

#[test]
fn an_operator_template_needs_somewhere_to_put_the_mission() {
    let agent = ExternalAgent::from_template("mycli --repo {repo} --task {mission}").unwrap();
    assert_eq!(agent.label, "mycli");
    assert_eq!(
        agent.command(Path::new("/repo"), "go"),
        vec!["mycli", "--repo", "/repo", "--task", "go"]
    );
    assert!(matches!(
        ExternalAgent::from_template("mycli --repo {repo}"),
        Err(TemplateError::NoMission)
    ));
    assert!(matches!(
        ExternalAgent::from_template("   "),
        Err(TemplateError::Empty)
    ));
}

#[test]
fn a_jsonl_line_names_its_own_step() {
    assert_eq!(
        json_type(r#"{"type":"tool_use","name":"shell"}"#).as_deref(),
        Some("tool_use")
    );
    assert_eq!(json_type("not json at all"), None);
    assert_eq!(json_type(r#"{"no":"type"}"#), None);
}

#[test]
fn a_tool_call_is_summarized_by_its_first_real_line() {
    assert_eq!(
        first_line("\n\n  cat src/rates.py\nls tests\n"),
        "cat src/rates.py"
    );
    assert_eq!(first_line(""), "");
    let clipped = first_line(&"x".repeat(200));
    assert_eq!(clipped.chars().count(), 80);
    assert!(clipped.ends_with('…'));
}

#[tokio::test]
async fn every_external_worker_policy_excludes_unlisted_host_secrets() {
    // Cargo supplies this variable to the test process, making it a stable
    // sentinel without mutating the process environment in parallel tests.
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    let custom = ExternalAgent::from_template("worker {mission}").unwrap();
    for agent in [ExternalAgent::codex(), ExternalAgent::yolop(), custom] {
        let arguments = vec![
            "-c".to_owned(),
            "printf '%s' \"${CARGO_MANIFEST_DIR-unset}\"".to_owned(),
        ];
        let output = sanitized_command(
            "sh",
            &arguments,
            Path::new("."),
            agent.credential_environment(),
        )
        .output()
        .await
        .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "unset",
            "{}",
            agent.label
        );
        assert!(!agent.credential_environment().contains(&"TYPESAFE_API_KEY"));
    }
}
