#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Run from the repository checkout; see README.md for credentials and scenarios.
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use everruns::Engine;
use everruns_example_demo::shell as demo;

mod agent;
mod fixture;

const DEFAULT_TASK: &str =
    "Cut release 0.2.0 for today's date. Follow the repository release process.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    let (_temporary, workspace) = match options.workspace {
        Some(path) => (None, path),
        None => {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("fetchkit");
            (Some(dir), path)
        }
    };
    fs::create_dir_all(&workspace)?;
    fixture::materialize(&workspace)?;

    let task = if options.interactive {
        read_task()?
    } else {
        DEFAULT_TASK.to_owned()
    };
    let release_date = today_utc();
    let request = release_request(&task, &release_date);
    // OPENAI_API_KEY, declared by the OpenAI driver itself.
    let agent = agent::build(everruns::OpenAI::from_env()?, &workspace)?;
    let engine = Engine::new();
    let session = engine.create(agent);

    demo::banner("everruns · bashkit repo agent");
    demo::field("model", agent::MODEL);
    demo::field("capability", "bashkit_shell — sandboxed Bash");
    demo::field("workspace", "/workspace — disposable read/write mount");
    demo::run(&session, &request).await?;

    verify_release(&workspace, &release_date)?;
    println!("\nCompleted: release verified on disk");
    Ok(())
}

fn release_request(task: &str, release_date: &str) -> String {
    format!(
        "{task}\n\nAcceptance criteria:\n\
         - Use release version {} and date {release_date}.\n\
         - Read README.md and CHANGELOG.md before editing.\n\
         - Update every crate package version and pinned path-dependency version.\n\
         - Fold every changelog.d fragment into a new dated section under '# Changelog'.\n\
         - Preserve the 0.1.0 history and leave changelog.d empty.\n\
         - Verify the result with shell commands before reporting it.",
        fixture::TARGET_VERSION
    )
}

fn read_task() -> io::Result<String> {
    print!("Task: ");
    io::stdout().flush()?;

    let mut task = String::new();
    io::stdin().read_line(&mut task)?;
    validate_task(&task)
}

fn validate_task(task: &str) -> io::Result<String> {
    let task = task.trim();
    if task.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "task cannot be empty",
        ));
    }
    Ok(task.to_owned())
}

fn verify_release(workspace: &Path, release_date: &str) -> Result<(), Box<dyn std::error::Error>> {
    let checks = fixture::verify(workspace, release_date);
    demo::section("RELEASE CHECKS ON DISK");
    for check in &checks {
        demo::check(check.passed, &check.label);
    }
    demo::section("CHANGELOG.md AFTER THE RUN");
    demo::body(
        &fs::read_to_string(workspace.join("CHANGELOG.md"))?,
        demo::DIM,
    );

    let failures = checks.iter().filter(|check| !check.passed).count();
    if failures > 0 {
        return Err(format!("{failures} release check(s) failed").into());
    }
    Ok(())
}

/// Today's UTC date as `YYYY-MM-DD`, so the fixture never bakes in a stale date.
fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or_default() as i64;
    // Days since the epoch to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

struct Options {
    interactive: bool,
    workspace: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut interactive = false;
        let mut workspace = None;
        for argument in arguments {
            if argument == "--interactive" {
                interactive = true;
            } else if workspace.is_none() {
                workspace = Some(PathBuf::from(argument));
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: bashkit-repo-agent [--interactive] [workspace]",
                ));
            }
        }
        Ok(Self {
            interactive,
            workspace,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interactive_workspace_in_either_order() {
        for arguments in [
            vec!["--interactive", "/tmp/release"],
            vec!["/tmp/release", "--interactive"],
        ] {
            let options = Options::parse(arguments.into_iter().map(OsString::from)).unwrap();
            assert!(options.interactive);
            assert_eq!(options.workspace, Some(PathBuf::from("/tmp/release")));
        }
    }

    #[test]
    fn interactive_task_is_trimmed_and_required() {
        assert_eq!(
            validate_task("  Cut the release.\n").unwrap(),
            "Cut the release."
        );
        assert_eq!(
            validate_task(" \n").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn fixture_starts_before_the_release() {
        let workspace = tempfile::tempdir().unwrap();
        fixture::materialize(workspace.path()).unwrap();
        let checks = fixture::verify(workspace.path(), &today_utc());
        for check in &checks {
            assert_eq!(
                check.passed,
                check.label.contains("0.1.0 history"),
                "unexpected pre-run state for check: {}",
                check.label
            );
        }
    }
}
