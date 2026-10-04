use super::*;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn the_fixture_starts_before_the_job_is_done() {
    let root = tempfile::tempdir().unwrap();
    materialize(root.path()).unwrap();

    assert!(!changed(root.path()));
    assert!(crate::test_support::tests_pass(root.path()));
    assert!(!crate::test_support::acceptance_pass(root.path()));
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

#[cfg(unix)]
#[test]
fn materializing_rejects_a_symlinked_root() {
    let outer = tempfile::tempdir().unwrap();
    let outside = outer.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let root = outer.path().join("workspace");
    symlink(&outside, &root).unwrap();

    assert!(materialize(&root).is_err());
    assert!(!outside.join("README.md").exists());
}

#[cfg(unix)]
#[test]
fn materializing_rejects_a_symlinked_parent() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("lib")).unwrap();

    assert!(materialize(root.path()).is_err());
    assert!(!outside.path().join("rates.sh").exists());
}

#[cfg(unix)]
#[test]
fn materializing_rejects_a_symlinked_file() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "keep me").unwrap();
    symlink(outside.path(), root.path().join("README.md")).unwrap();

    assert!(materialize(root.path()).is_err());
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "keep me");
}
