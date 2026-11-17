#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn bash(script: &str) -> String {
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(script)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_scan_lists_the_workspace_and_its_digest_matches_the_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join("a.md"), "a").unwrap();
    std::fs::write(root.join("a.b"), "ab").unwrap();
    std::fs::write(root.join("src/deep/b.rs"), "fn b() {}").unwrap();
    std::fs::write(root.join("node_modules/x/i.js"), "skip").unwrap();
    std::fs::write(root.join(".git/HEAD"), "skip").unwrap();
    let root = root.to_str().unwrap();
    let listing = listing_path(root);
    let script = scan_script(root, "0");

    let scan = parse_scan(&bash(&script)).unwrap();

    assert!(scan.present);
    assert_eq!(scan.count, 3);
    let changed = scan.changed.unwrap();
    let paths: BTreeSet<String> = changed.keys().cloned().collect();
    assert_eq!(
        paths,
        BTreeSet::from(["/a.b".into(), "/a.md".into(), "/src/deep/b.rs".into()])
    );
    assert_eq!(
        digest(&paths),
        scan.digest,
        "the shell and Rust digests agree"
    );
    assert_eq!(changed["/src/deep/b.rs"].size, 9);

    // Nothing changed since the scan: the next one lists no file.
    let again = scan_script(root, &scan.now);
    let next = parse_scan(&bash(&again)).unwrap();
    assert_eq!(next.changed.unwrap().len(), 0);
    assert_eq!(next.digest, scan.digest);

    // A change after it is listed, and only it.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(format!("{root}/a.md"), "changed").unwrap();
    let after = parse_scan(&bash(&again)).unwrap();
    assert_eq!(
        after.changed.unwrap().into_keys().collect::<Vec<_>>(),
        ["/a.md"]
    );
    assert_eq!(
        parse_listing(&std::fs::read_to_string(&listing).unwrap()).len(),
        3
    );
}

#[test]
fn a_missing_workspace_scans_as_absent() {
    let script = scan_script("/nonexistent/everruns-fs-test", "0");
    let scan = parse_scan(&bash(&script)).unwrap();
    assert!(!scan.present);
}

#[test]
fn skipped_paths_are_tool_caches_and_checkouts() {
    assert!(skipped("/node_modules/x.js"));
    assert!(skipped("/app/.git/HEAD"));
    assert!(skipped("/target/debug/x"));
    assert!(!skipped("/src/target.rs"));
    assert!(!skipped("/notes.md"));
}

#[test]
fn an_entry_matches_content_inline_or_by_blob_digest() {
    let entry = |content: Content| -> Entry {
        let mut entry: Entry = serde_json::from_value(json!({})).unwrap();
        entry.content = content;
        entry
    };
    let inline = entry(Content::Inline { data: "hi".into() });
    assert!(same(&inline, "hi", "text"));
    assert!(!same(&inline, "hi", "base64"));
    assert!(!same(&inline, "ho", "text"));
    let blob = entry(Content::Blob {
        sha256: hex::encode(sha2::Sha256::digest(b"large")),
    });
    assert!(same(&blob, "large", "text"));
    assert!(!same(&blob, "larger", "text"));
}

#[test]
fn sandbox_times_compare_as_numbers() {
    assert!(newer("1731830000.200000000", "1731830000.1"));
    assert!(!newer("1731830000.1", "1731830000.100000000"));
    assert!(newer("1", "0"));
}

#[test]
fn only_a_session_sandbox_entry_is_taken() {
    let entry = json!({ "ref": "session_sandbox", "config": { "provider": "daytona" } });
    assert_eq!(capability_entry(&entry), Some(entry));
    assert_eq!(capability_entry(&json!({ "ref": "bashkit_shell" })), None);
}

#[test]
fn the_providers_are_linked() {
    for provider in ["daytona", "e2b", "boxd", "modal", "kubernetes"] {
        assert!(
            everruns_capabilities::session_sandbox::create_session_sandbox_provider(provider)
                .is_some(),
            "{provider} is registered"
        );
    }
}
