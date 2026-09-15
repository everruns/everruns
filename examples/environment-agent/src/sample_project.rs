//! The sample project this example hands to the agent.
//!
//! The fixture is embedded in the binary and materialized into a fresh
//! directory on every run, so the checked-in copy under `resources/project/`
//! stays pristine and each run starts from the same known state.

use std::fs;
use std::io;
use std::path::Path;

/// Project-relative path and contents of every fixture file.
pub const FILES: &[(&str, &str)] = &[
    ("README.md", include_str!("resources/project/README.md")),
    (
        "src/client.rs",
        include_str!("resources/project/src/client.rs"),
    ),
    (
        "src/retry.rs",
        include_str!("resources/project/src/retry.rs"),
    ),
    (
        "src/cache.rs",
        include_str!("resources/project/src/cache.rs"),
    ),
];

/// How many `TODO` markers the fixture actually carries, computed from the
/// embedded copy rather than written down, so the expected answer cannot drift
/// away from the files the agent is given.
pub fn todo_total() -> usize {
    FILES
        .iter()
        .map(|(_, contents)| contents.matches("TODO").count())
        .sum()
}

/// Write a fresh working copy of the sample project under `root`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_carries_markers_in_two_of_its_three_modules() {
        assert_eq!(todo_total(), 4);
        let counts: Vec<usize> = FILES
            .iter()
            .map(|(_, contents)| contents.matches("TODO").count())
            .collect();
        assert_eq!(counts, vec![0, 3, 1, 0]);
    }
}
