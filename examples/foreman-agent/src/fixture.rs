//! The repository the factory works on.
//!
//! A disposable copy, materialized into a scratch directory and committed once
//! so the supervisor's `git diff` has a baseline to describe. Never point this
//! at a repository you care about: the worker may change anything inside it.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

/// The job the factory is started on.
///
/// Free-form, the way a ticket is, but complete: the rate schedule is stated
/// rather than left to the worker. An underspecified job is a real thing for a
/// supervisor to find — `needs_human` rises and the run escalates, correctly —
/// which makes it a bad default for an example about the rest of the loop.
pub const JOB: &str = "Replace the flat shipping rate in shipkit with weight tiers: 700 cents up \
                       to 1000 g, 1200 up to 5000 g, 2400 up to 20000 g, and 4800 above that, \
                       with the existing zone surcharge still added on top. Cover the tier \
                       boundaries with tests, and run the suite.";

/// The command that runs the fixture's suite.
///
/// Shell, deliberately. It runs inside the Bashkit sandbox, on the host, and
/// inside an external agent's own shell alike — so the worker can check its
/// work and the supervisor can check it independently, neither needing a
/// toolchain that happens to be installed.
pub const TESTS: &str = "bash tests/run.sh";

/// The fixture, as repository-relative paths and contents.
pub const FILES: [(&str, &str); 3] = [
    ("README.md", include_str!("resources/sample-repo/README.md")),
    (
        "lib/rates.sh",
        include_str!("resources/sample-repo/lib/rates.sh"),
    ),
    (
        "tests/run.sh",
        include_str!("resources/sample-repo/tests/run.sh"),
    ),
];

/// Write the fixture into `root` and commit it, so later changes show as a diff.
pub fn materialize(root: &Path) -> io::Result<()> {
    for (path, contents) in FILES {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, contents)?;
    }
    commit_baseline(root);
    Ok(())
}

/// Best-effort baseline commit. Without Git the run still works; the supervisor
/// simply has no diff to look at, which is one evidence stream short rather
/// than a failure.
///
/// Skipped entirely inside an existing work tree. The example is documented as
/// destructive to whatever directory it is pointed at, but overwriting files
/// somebody can recover with `git checkout` is a different thing from writing a
/// commit into their history, so this one does not happen by accident.
fn commit_baseline(root: &Path) {
    if inside_work_tree(root) {
        return;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .is_ok()
    };
    if !git(&["init", "--quiet"]) {
        return;
    }
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=shipkit",
        "-c",
        "user.email=shipkit@example.com",
        "commit",
        "--quiet",
        "-m",
        "Flat shipping rates",
    ]);
}

/// Whether `root` already belongs to a Git work tree.
fn inside_work_tree(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        })
}

/// One host-side observation about the repository after the run.
pub struct Check {
    /// What was looked for.
    pub label: &'static str,
    /// Whether it is there.
    pub passed: bool,
}

/// Read the working copy and report what the run actually left behind.
///
/// These are evidence, not the success condition: the factory's own decision is
/// what the example is about, and grading a live model's code is not.
pub fn verify(root: &Path) -> Vec<Check> {
    let rates = fs::read_to_string(root.join("lib/rates.sh")).unwrap_or_default();
    let lowercase_rates = rates.to_lowercase();
    let tests = read_tests(root);
    let lowercase_tests = tests.to_lowercase();

    vec![
        Check {
            label: "lib/rates.sh no longer prices every parcel the same",
            passed: !rates.contains("echo $((FLAT_RATE_CENTS + surcharge))"),
        },
        Check {
            label: "lib/rates.sh names weight tiers",
            passed: ["tier", "bracket", "band"]
                .iter()
                .any(|word| lowercase_rates.contains(word)),
        },
        Check {
            label: "the tests cover a tier or a boundary",
            passed: lowercase_tests.contains("tier") || lowercase_tests.contains("boundar"),
        },
        Check {
            label: "the existing zone-surcharge test still stands",
            passed: tests.contains("zone surcharge is added"),
        },
    ]
}

/// Run the fixture's suite the way anyone would, and say whether it passed.
///
/// The example's own last word on a run: the supervisor's reading of the tests
/// is evidence, and this is the fact.
pub fn tests_pass(root: &Path) -> bool {
    Command::new("bash")
        .arg("tests/run.sh")
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Whether the working copy differs from the fixture at all.
pub fn changed(root: &Path) -> bool {
    FILES.iter().any(|(path, original)| {
        fs::read_to_string(root.join(path)).is_ok_and(|current| current != *original)
    }) || fs::read_dir(root.join("tests"))
        .map(|entries| entries.count() > 1)
        .unwrap_or(false)
}

fn read_tests(root: &Path) -> String {
    let Ok(entries) = fs::read_dir(root.join("tests")) else {
        return String::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_starts_before_the_job_is_done() {
        let root = tempfile::tempdir().unwrap();
        materialize(root.path()).unwrap();

        assert!(!changed(root.path()));
        let checks = verify(root.path());
        // Only the pre-existing test is there at the start; everything the job
        // asks for is still missing.
        for check in &checks {
            assert_eq!(
                check.passed,
                check.label.contains("still stands"),
                "unexpected starting state: {}",
                check.label
            );
        }
    }

    #[test]
    fn an_existing_work_tree_keeps_its_own_history() {
        let outer = tempfile::tempdir().unwrap();
        materialize(outer.path()).unwrap();
        assert!(inside_work_tree(outer.path()));
        let head = |root: &Path| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        };
        let before = head(outer.path());
        assert!(before.is_some(), "the fixture commits its own baseline");

        // Materializing again, as if somebody pointed the example at a
        // repository they own: files are rewritten, history is not.
        materialize(outer.path()).unwrap();
        assert_eq!(head(outer.path()), before);
    }
}
