//! The sample repository this example hands to the agent.
//!
//! The fixture is embedded in the binary and materialized into a fresh
//! directory on every run, so the checked-in copy under `sample-repo/` stays
//! pristine and each run starts from the same known state.

use std::fs;
use std::io;
use std::path::Path;

/// Repository-relative path and contents of every fixture file.
pub const FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../sample-repo/Cargo.toml")),
    ("README.md", include_str!("../sample-repo/README.md")),
    ("CHANGELOG.md", include_str!("../sample-repo/CHANGELOG.md")),
    (
        "crates/core/Cargo.toml",
        include_str!("../sample-repo/crates/core/Cargo.toml"),
    ),
    (
        "crates/core/src/lib.rs",
        include_str!("../sample-repo/crates/core/src/lib.rs"),
    ),
    (
        "crates/cli/Cargo.toml",
        include_str!("../sample-repo/crates/cli/Cargo.toml"),
    ),
    (
        "crates/cli/src/main.rs",
        include_str!("../sample-repo/crates/cli/src/main.rs"),
    ),
    (
        "changelog.d/0001-retry-backoff.md",
        include_str!("../sample-repo/changelog.d/0001-retry-backoff.md"),
    ),
    (
        "changelog.d/0002-redirect-timeout.md",
        include_str!("../sample-repo/changelog.d/0002-redirect-timeout.md"),
    ),
    (
        "changelog.d/0003-cli-json.md",
        include_str!("../sample-repo/changelog.d/0003-cli-json.md"),
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
