//! The sample repository this example hands to the agent.
//!
//! The fixture is embedded in the binary and materialized into a fresh
//! directory on every run, so `resources/sample-repo/` stays
//! pristine and each run starts from the same known state.

use std::fs;
use std::io;
use std::path::Path;

pub const TARGET_VERSION: &str = "0.2.0";

/// Repository-relative path and contents of every fixture file.
pub const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("resources/sample-repo/Cargo.toml"),
    ),
    ("README.md", include_str!("resources/sample-repo/README.md")),
    (
        "CHANGELOG.md",
        include_str!("resources/sample-repo/CHANGELOG.md"),
    ),
    (
        "crates/core/Cargo.toml",
        include_str!("resources/sample-repo/crates/core/Cargo.toml"),
    ),
    (
        "crates/core/src/lib.rs",
        include_str!("resources/sample-repo/crates/core/src/lib.rs"),
    ),
    (
        "crates/cli/Cargo.toml",
        include_str!("resources/sample-repo/crates/cli/Cargo.toml"),
    ),
    (
        "crates/cli/src/main.rs",
        include_str!("resources/sample-repo/crates/cli/src/main.rs"),
    ),
    (
        "changelog.d/0001-retry-backoff.md",
        include_str!("resources/sample-repo/changelog.d/0001-retry-backoff.md"),
    ),
    (
        "changelog.d/0002-redirect-timeout.md",
        include_str!("resources/sample-repo/changelog.d/0002-redirect-timeout.md"),
    ),
    (
        "changelog.d/0003-cli-json.md",
        include_str!("resources/sample-repo/changelog.d/0003-cli-json.md"),
    ),
];

/// Write a fresh working copy of the sample repository under `root`.
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

/// Re-read the working copy and verify the release independently of the agent.
pub fn verify(root: &Path, release_date: &str) -> Vec<Check> {
    let mut checks = Vec::new();
    for manifest in ["crates/core/Cargo.toml", "crates/cli/Cargo.toml"] {
        let text = fs::read_to_string(root.join(manifest)).unwrap_or_default();
        checks.push(Check::new(
            text.contains(&format!("version = \"{TARGET_VERSION}\"")) && !text.contains("0.1.0"),
            format!("{manifest} pins {TARGET_VERSION} only"),
        ));
    }

    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
    checks.push(Check::new(
        changelog.contains(&format!("## {TARGET_VERSION} - {release_date}")),
        format!("CHANGELOG.md opens a '{TARGET_VERSION} - {release_date}' section"),
    ));
    checks.push(Check::new(
        changelog.contains("## 0.1.0"),
        "CHANGELOG.md keeps the 0.1.0 history",
    ));
    for (fragment, marker) in [
        ("0001-retry-backoff.md", "backoff"),
        ("0002-redirect-timeout.md", "redirect"),
        ("0003-cli-json.md", "--json"),
    ] {
        checks.push(Check::new(
            changelog.contains(marker),
            format!("CHANGELOG.md carries {fragment} (mentions '{marker}')"),
        ));
    }

    let leftovers = fs::read_dir(root.join("changelog.d"))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(usize::MAX);
    checks.push(Check::new(leftovers == 0, "changelog.d/ is empty"));
    checks
}

pub struct Check {
    pub passed: bool,
    pub label: String,
}

impl Check {
    fn new(passed: bool, label: impl Into<String>) -> Self {
        Self {
            passed,
            label: label.into(),
        }
    }
}
