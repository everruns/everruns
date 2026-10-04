//! The disposable shipping-rate repository and fixed acceptance checks.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// The job the factory is started on.
pub const JOB: &str = "Replace the flat shipping rate in shipkit with weight tiers: 700 cents up \
                       to 1000 g, 1200 up to 5000 g, 2400 up to 20000 g, and 4800 above that, \
                       with the existing zone surcharge still added on top. Cover the tier \
                       boundaries with tests, update the README to describe the tiers, and run the suite.";

/// The command that runs the fixture's suite.
pub const TESTS: &str = "bash tests/run.sh";

/// Fixed checks remain outside the repository the coding worker can edit.
pub const ACCEPTANCE: &str = include_str!("resources/acceptance.sh");

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
        // THREAT[TM-FS-019]: Never follow attacker-prepared fixture paths outside `root`.
        reject_symlinks(root, &target)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        reject_symlinks(root, &target)?;

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(target)?.write_all(contents.as_bytes())?;
    }
    commit_baseline(root);
    Ok(())
}

/// Reject an existing symlink in the fixture root or destination path.
fn reject_symlinks(root: &Path, target: &Path) -> io::Result<()> {
    let relative = target.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fixture path escapes its root: {}", target.display()),
        )
    })?;
    let mut current = root.to_owned();
    reject_symlink(&current)?;

    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid fixture path: {}", target.display()),
            ));
        };
        current.push(segment);
        reject_symlink(&current)?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "fixture paths must not contain symlinks: {}",
                path.display()
            ),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Best-effort baseline commit. Without Git the run still works; the supervisor
/// simply has no diff to look at, which is one evidence stream short rather
/// than a failure.
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

/// Whether the working copy differs from the fixture at all.
pub fn changed(root: &Path) -> bool {
    FILES.iter().any(|(path, original)| {
        fs::read_to_string(root.join(path)).is_ok_and(|current| current != *original)
    }) || fs::read_dir(root.join("tests"))
        .map(|entries| entries.count() > 1)
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../tests/unit/fixture.rs"]
mod tests;
