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
pub const JOB: &str = "Replace the flat shipping rate in shipkit with weight-based tiers, \
                       and make sure the tier boundaries are covered by tests.";

/// The fixture, as repository-relative paths and contents.
pub const FILES: [(&str, &str); 3] = [
    ("README.md", include_str!("resources/sample-repo/README.md")),
    (
        "src/rates.py",
        include_str!("resources/sample-repo/src/rates.py"),
    ),
    (
        "tests/test_rates.py",
        include_str!("resources/sample-repo/tests/test_rates.py"),
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
fn commit_baseline(root: &Path) {
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
    let rates = fs::read_to_string(root.join("src/rates.py")).unwrap_or_default();
    let lowercase_rates = rates.to_lowercase();
    let tests = read_tests(root);
    let lowercase_tests = tests.to_lowercase();

    vec![
        Check {
            label: "src/rates.py no longer prices every parcel the same",
            passed: !rates.contains("return FLAT_RATE_CENTS + ZONE_SURCHARGE_CENTS[destination]"),
        },
        Check {
            label: "src/rates.py names weight tiers",
            passed: ["tier", "bracket", "band"]
                .iter()
                .any(|word| lowercase_rates.contains(word)),
        },
        Check {
            label: "the tests mention a tier or a boundary",
            passed: lowercase_tests.contains("tier") || lowercase_tests.contains("boundar"),
        },
        Check {
            label: "the existing zone-surcharge test still stands",
            passed: tests.contains("test_zone_surcharge_is_added"),
        },
    ]
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
    fn a_tiered_rewrite_satisfies_the_checks() {
        let root = tempfile::tempdir().unwrap();
        materialize(root.path()).unwrap();
        fs::write(
            root.path().join("src/rates.py"),
            "TIERS = [(1.0, 800), (5.0, 1400)]\n\
             def quote(weight_kg, destination):\n\
             \x20   return tier_for(weight_kg) + surcharge(destination)\n\
             def tier_for(weight_kg):\n\
             \x20   return next(c for limit, c in TIERS if weight_kg <= limit)\n",
        )
        .unwrap();
        fs::write(
            root.path().join("tests/test_tiers.py"),
            "def test_tier_boundary_is_inclusive():\n    assert True\n",
        )
        .unwrap();

        assert!(changed(root.path()));
        let checks = verify(root.path());
        assert!(checks.iter().all(|check| check.passed));
    }
}
