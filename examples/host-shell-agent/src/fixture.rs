//! The crate this example hands to the agent, and the independent verdict on it.
//!
//! The fixture is embedded in the binary and materialized into a fresh
//! directory on every run, so `resources/sample-crate/` stays pristine and each
//! run starts from the same red suite.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

/// Crate-relative path and contents of every fixture file.
pub const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("resources/sample-crate/Cargo.toml"),
    ),
    (
        "src/lib.rs",
        include_str!("resources/sample-crate/src/lib.rs"),
    ),
];

/// Write a fresh working copy of the sample crate under `root`.
pub fn materialize(root: &Path) -> io::Result<()> {
    for (path, contents) in FILES {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, contents)?;
    }
    Ok(())
}

/// One thing that is either true of the working copy or not.
pub struct Check {
    pub passed: bool,
    pub label: String,
    /// Output worth showing when it failed.
    pub detail: String,
}

impl Check {
    fn new(passed: bool, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            passed,
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// Re-run the suite from the host and check the agent fixed the code, not the test.
///
/// This is the success condition, not the agent's own report. A model that says
/// it fixed the bug, or that deleted the assertions instead, fails here.
pub fn verify(root: &Path) -> Vec<Check> {
    let mut checks = Vec::new();

    let source = fs::read_to_string(root.join("src/lib.rs")).unwrap_or_default();
    for assertion in ["assert_eq!(chunk_count(10, 3), 4);", "chunks.concat(),"] {
        checks.push(Check::new(
            source.contains(assertion),
            format!("the test still asserts `{assertion}`"),
            "the agent changed the test rather than the code".to_string(),
        ));
    }
    checks.push(Check::new(
        !source.contains("#[ignore]"),
        "no test was ignored",
        "an ignored test is one nothing runs".to_string(),
    ));

    let output = Command::new("cargo").arg("test").current_dir(root).output();
    let (passed, detail) = match output {
        Ok(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => (false, format!("cargo could not be run: {error}")),
    };
    checks.push(Check::new(
        passed,
        "`cargo test` passes from the host",
        detail,
    ));

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_starts_red_and_verify_says_so() {
        let workspace = tempfile::tempdir().expect("workspace");
        materialize(workspace.path()).expect("materialized");

        let checks = verify(workspace.path());
        // The assertions are intact before the run; only the suite is red.
        let (suite, intact): (Vec<_>, Vec<_>) = checks
            .iter()
            .partition(|check| check.label.contains("cargo test"));
        assert!(intact.iter().all(|check| check.passed));
        assert!(
            suite.iter().all(|check| !check.passed),
            "the fixture must hand the agent a failing suite"
        );
    }
}
