use clap::{CommandFactory, Parser};
use everruns_coding_cli::{Cli, SessionMode};

#[test]
fn long_help_teaches_head_resume_and_shared_flows() {
    let help = Cli::command().render_long_help().to_string();
    for expected in [
        "--head <NAME>",
        "--base <REVISION>",
        "--shared",
        "--shared-head <SESSION_ID>",
        "--resume <SESSION_ID>",
        "--read-only",
        "survive process exit",
    ] {
        assert!(help.contains(expected), "missing {expected:?} in:\n{help}");
    }
}

#[test]
fn head_modes_are_mutually_exclusive_and_safe_by_default() {
    let default = Cli::try_parse_from(["ercode", "--offline"]).expect("default args");
    assert_eq!(
        default.session_mode(),
        SessionMode::NewHead {
            name: "ercode".into(),
            base: None,
            shared: false,
        }
    );

    assert!(
        Cli::try_parse_from([
            "ercode",
            "--head",
            "one",
            "--resume",
            "session_00000000000000000000000000000001",
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["ercode", "--shared"]).is_err());
    assert!(Cli::try_parse_from(["ercode", "--base", "main"]).is_err());
    assert!(
        Cli::try_parse_from([
            "ercode",
            "--cwd",
            "/tmp/other",
            "--resume",
            "session_00000000000000000000000000000001",
        ])
        .is_err()
    );
}
