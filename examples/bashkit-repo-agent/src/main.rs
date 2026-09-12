//! A release-prep agent that edits a real repository through the sandboxed
//! Bashkit shell.
//!
//! The example materializes the bundled `sample-repo/` fixture into a throwaway
//! working copy, mounts that directory as the agent's `/workspace`, and gives
//! the agent exactly one capability: [`BashkitShell`]. Every step the agent
//! takes — reading the tree, bumping crate versions, folding changelog
//! fragments — is a bash script interpreted in-process by Bashkit against the
//! session filesystem. When the turn ends, the host re-reads the directory and
//! checks the release actually landed.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-bashkit-repo-agent
//! OPENAI_API_KEY=... cargo run -p everruns-bashkit-repo-agent -- /tmp/release-run
//! ```

// Terminal presentation is shared by every example; this file is the agent.
use everruns_example_demo::shell as demo;

mod sample_repo;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use everruns::{Agent, BashkitShell, Engine, OpenAI, WorkspacePolicy};

const MODEL: &str = "gpt-5.6-terra";
const TARGET_VERSION: &str = "0.2.0";

fn release_request(release_date: &str) -> String {
    format!(
        "Cut release {TARGET_VERSION} of the repository in /workspace, dated {release_date}.\n\
         1. Read README.md and CHANGELOG.md to learn the release process.\n\
         2. Set the package version to {TARGET_VERSION} in every crate manifest under crates/, \
            including path dependencies that pin a version.\n\
         3. Fold every fragment in changelog.d/ into a new '## {TARGET_VERSION} - {release_date}' \
            section directly under the '# Changelog' title, one bullet per fragment, keeping the \
            existing 0.1.0 section below it.\n\
         4. Delete the fragment files, leaving changelog.d/ in place.\n\
         5. Print the final CHANGELOG.md and the version line of each manifest, then report what \
            you changed."
    )
}

fn build_agent(provider: OpenAI, workspace: &Path) -> Result<Agent, everruns::BuildError> {
    Agent::builder()
        .name("bashkit-repo-agent")
        .instructions(include_str!("../instructions.md"))
        .provider(provider)
        .model(MODEL)
        // One real host directory becomes the session's /workspace. The
        // read/write policy is an explicit opt-in; the default is read-only.
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(BashkitShell::new())
        .build()
}

/// One post-run assertion about the working copy on disk.
struct Check {
    passed: bool,
    label: String,
}

fn check(passed: bool, label: impl Into<String>) -> Check {
    Check {
        passed,
        label: label.into(),
    }
}

/// Re-read the working copy and confirm the release the agent reported.
fn verify(root: &Path, release_date: &str) -> Vec<Check> {
    let mut checks = Vec::new();
    for manifest in ["crates/core/Cargo.toml", "crates/cli/Cargo.toml"] {
        let text = fs::read_to_string(root.join(manifest)).unwrap_or_default();
        checks.push(check(
            text.contains(&format!("version = \"{TARGET_VERSION}\"")) && !text.contains("0.1.0"),
            format!("{manifest} pins {TARGET_VERSION} only"),
        ));
    }

    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
    checks.push(check(
        changelog.contains(&format!("## {TARGET_VERSION} - {release_date}")),
        format!("CHANGELOG.md opens a '{TARGET_VERSION} - {release_date}' section"),
    ));
    checks.push(check(
        changelog.contains("## 0.1.0"),
        "CHANGELOG.md keeps the 0.1.0 history",
    ));
    for (fragment, marker) in [
        ("0001-retry-backoff.md", "backoff"),
        ("0002-redirect-timeout.md", "redirect"),
        ("0003-cli-json.md", "--json"),
    ] {
        checks.push(check(
            changelog.contains(marker),
            format!("CHANGELOG.md carries {fragment} (mentions '{marker}')"),
        ));
    }

    let leftovers = fs::read_dir(root.join("changelog.d"))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(usize::MAX);
    checks.push(check(leftovers == 0, "changelog.d/ is empty"));
    checks
}

/// Today's UTC date as `YYYY-MM-DD`, so the example never bakes in a stale one.
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requested = std::env::args_os().nth(1).map(PathBuf::from);
    let temporary = match requested {
        Some(_) => None,
        None => Some(tempfile::tempdir()?),
    };
    let root = match (&requested, &temporary) {
        (Some(path), _) => path.clone(),
        (None, Some(directory)) => directory.path().join("fetchkit"),
        _ => unreachable!("one of the two is always set"),
    };
    fs::create_dir_all(&root)?;
    sample_repo::materialize(&root)?;

    let release_date = today_utc();
    demo::banner("everruns · bashkit repo agent");
    demo::field("model", MODEL);
    demo::field(
        "capability",
        "bashkit_shell — sandboxed Bash over /workspace",
    );
    demo::field("workspace", &root.display().to_string());
    demo::field("release", &format!("{TARGET_VERSION} ({release_date})"));

    // `OpenAI::from_env` reads OPENAI_API_KEY; the turn below is a real
    // provider call, not a simulation.
    let agent = build_agent(OpenAI::from_env()?, &root)?;
    let session = Engine::new().create(agent);
    demo::run(&session, &release_request(&release_date)).await?;

    let checks = verify(&root, &release_date);
    demo::section("RELEASE CHECKS ON DISK");
    for item in &checks {
        demo::check(item.passed, &item.label);
    }
    demo::section("CHANGELOG.md AFTER THE RUN");
    demo::body(&fs::read_to_string(root.join("CHANGELOG.md"))?, demo::DIM);

    let failed = checks.iter().filter(|item| !item.passed).count();
    if failed > 0 {
        return Err(format!("{failed} release check(s) failed").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use everruns::OpenAI;

    use super::{build_agent, sample_repo, today_utc, verify};

    #[test]
    fn builds_without_contacting_the_provider() {
        let workspace = tempfile::tempdir().expect("temp dir");
        assert!(build_agent(OpenAI::new("test-key"), workspace.path()).is_ok());
    }

    #[test]
    fn fixture_starts_before_the_release() {
        let workspace = tempfile::tempdir().expect("temp dir");
        sample_repo::materialize(workspace.path()).expect("materialize");
        let checks = verify(workspace.path(), &today_utc());
        // The fixture already carries the 0.1.0 history; every other check
        // describes work only the agent's run can do.
        for check in &checks {
            let expected = check.label.contains("0.1.0 history");
            assert_eq!(
                check.passed, expected,
                "unexpected pre-run state for check: {}",
                check.label
            );
        }
    }

    #[test]
    fn today_is_an_iso_date() {
        let today = today_utc();
        assert_eq!(today.len(), 10, "unexpected date: {today}");
        assert!(today.starts_with("20"), "unexpected date: {today}");
    }
}
