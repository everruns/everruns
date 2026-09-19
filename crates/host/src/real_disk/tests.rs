//! Tests for the real-disk session filesystem.
//!
//! Split out of `real_disk.rs` so the module stays readable; the guard in
//! `scripts/lib/check-file-size.sh` is what made the overdue split happen.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use tempfile::TempDir;

fn make_store() -> (RealDiskFileStore, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let store = RealDiskFileStore::new(dir.path()).expect("store");
    (store, dir)
}

fn sid() -> SessionId {
    SessionId::new()
}

#[tokio::test]
async fn multi_root_reads_writes_lists_and_greps() {
    let primary = TempDir::new().unwrap();
    let backend = TempDir::new().unwrap();
    let root_set = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), backend.path().to_path_buf())],
    )
    .unwrap();
    let store = multi_root_file_system(&root_set).unwrap();
    let session = sid();

    let primary_file = store
        .write_file(session, "/workspace/README.md", "needle primary", "text")
        .await
        .unwrap();
    assert_eq!(primary_file.path, "/README.md");
    assert_eq!(
        std::fs::read_to_string(primary.path().join("README.md")).unwrap(),
        "needle primary"
    );

    let backend_file = store
        .write_file(
            session,
            "/workspace/roots/backend/Cargo.toml",
            "needle backend",
            "text",
        )
        .await
        .unwrap();
    assert_eq!(backend_file.path, "/workspace/roots/backend/Cargo.toml");
    assert_eq!(
        std::fs::read_to_string(backend.path().join("Cargo.toml")).unwrap(),
        "needle backend"
    );

    let listed = store
        .list_directory(session, "/workspace/roots/backend")
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, "/workspace/roots/backend/Cargo.toml");

    let matches = store.grep_files(session, "needle", None).await.unwrap();
    let paths: Vec<_> = matches.into_iter().map(|m| m.path).collect();
    assert_eq!(
        paths,
        vec![
            "/README.md".to_string(),
            "/workspace/roots/backend/Cargo.toml".to_string()
        ]
    );
}

#[tokio::test]
async fn multi_root_escape_attempts_fail() {
    let primary = TempDir::new().unwrap();
    let backend = TempDir::new().unwrap();
    let root_set = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), backend.path().to_path_buf())],
    )
    .unwrap();
    let store = multi_root_file_system(&root_set).unwrap();

    let err = store
        .write_file(
            sid(),
            "/workspace/roots/backend/../../outside.txt",
            "nope",
            "text",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("path traversal rejected"));
}

#[tokio::test]
async fn multi_root_blocklist_applies_to_every_root() {
    let primary = TempDir::new().unwrap();
    let backend = TempDir::new().unwrap();
    let root_set = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), backend.path().to_path_buf())],
    )
    .unwrap();
    let inner = multi_root_file_system(&root_set).unwrap();
    let store: Arc<dyn SessionFileSystem> = Arc::new(crate::WriteBlocklistFileStore::new(inner));

    let primary_err = store
        .write_file(sid(), "/workspace/target/out.txt", "nope", "text")
        .await
        .unwrap_err();
    assert!(primary_err.to_string().contains("write blocklist rejected"));

    let backend_err = store
        .write_file(
            sid(),
            "/workspace/roots/backend/node_modules/pkg.js",
            "nope",
            "text",
        )
        .await
        .unwrap_err();
    assert!(backend_err.to_string().contains("write blocklist rejected"));
}

#[tokio::test]
async fn factory_context_root_set_repoints_only_primary() {
    let configured = TempDir::new().unwrap();
    let primary = TempDir::new().unwrap();
    let backend = TempDir::new().unwrap();
    let root_set = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), backend.path().to_path_buf())],
    )
    .unwrap();
    let factory = RealDiskSessionFileSystemFactory::new(configured.path());
    let store = factory
        .create_session_file_system(
            SessionFileSystemFactoryContext::new().with_workspace_roots(Arc::new(root_set)),
        )
        .await
        .unwrap();

    store
        .write_file(sid(), "/workspace/primary.txt", "primary", "text")
        .await
        .unwrap();
    store
        .write_file(
            sid(),
            "/workspace/roots/backend/backend.txt",
            "backend",
            "text",
        )
        .await
        .unwrap();

    assert!(!configured.path().join("primary.txt").exists());
    assert_eq!(
        std::fs::read_to_string(primary.path().join("primary.txt")).unwrap(),
        "primary"
    );
    assert_eq!(
        std::fs::read_to_string(backend.path().join("backend.txt")).unwrap(),
        "backend"
    );
}

#[tokio::test]
async fn round_trip_text_file() {
    let (store, _dir) = make_store();
    let session = sid();
    let written = store
        .write_file(session, "/notes.md", "# hello", "text")
        .await
        .expect("write");
    assert_eq!(written.path, "/notes.md");
    assert_eq!(written.encoding, "text");

    let read = store
        .read_file(session, "/notes.md")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.content.as_deref(), Some("# hello"));
    assert_eq!(read.encoding, "text");
    assert_eq!(read.size_bytes, 7);
    assert!(!read.is_directory);
}

#[tokio::test]
async fn round_trip_binary_file() {
    let (store, _dir) = make_store();
    let session = sid();
    let bytes = [0x89u8, b'P', b'N', b'G', 0, 1, 2, 3];
    let (encoded, encoding) = SessionFile::encode_content(&bytes);
    assert_eq!(encoding, "base64");

    store
        .write_file(session, "/img.bin", &encoded, &encoding)
        .await
        .expect("write");

    let read = store
        .read_file(session, "/img.bin")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.encoding, "base64");
    let decoded = SessionFile::decode_content(read.content.as_deref().unwrap(), &read.encoding)
        .expect("decode");
    assert_eq!(decoded, bytes);
}

#[tokio::test]
async fn workspace_prefix_normalized() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .write_file(session, "/workspace/sub/dir/file.txt", "hi", "text")
        .await
        .expect("write");

    let via_canonical = store
        .read_file(session, "/sub/dir/file.txt")
        .await
        .expect("read")
        .expect("present");
    let via_workspace = store
        .read_file(session, "/workspace/sub/dir/file.txt")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(via_canonical.content, via_workspace.content);
    assert_eq!(via_canonical.path, "/sub/dir/file.txt");
}

#[tokio::test]
async fn real_disk_display_paths_use_host_root() {
    let (store, dir) = make_store();
    let root = std::fs::canonicalize(dir.path()).expect("canonical tempdir");

    assert_eq!(store.display_root(), root.display().to_string());
    assert_eq!(
        store.display_path("/sub/dir/file.txt"),
        root.join("sub/dir/file.txt").display().to_string()
    );
}

#[tokio::test]
async fn host_absolute_paths_under_root_are_workspace_aliases() {
    let (store, _dir) = make_store();
    let session = sid();
    let host_path = store.display_path("/sub/dir/file.txt");

    store
        .write_file(session, &host_path, "hi", "text")
        .await
        .expect("write via host path");

    let via_workspace = store
        .read_file(session, "/workspace/sub/dir/file.txt")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(via_workspace.content.as_deref(), Some("hi"));
    assert_eq!(via_workspace.path, "/sub/dir/file.txt");
}

#[tokio::test]
async fn host_absolute_aliases_allow_current_dir_segments() {
    let (store, _dir) = make_store();
    let session = sid();
    let host_path = Path::new(&store.display_root())
        .join("./sub/dir/file.txt")
        .display()
        .to_string();

    store
        .write_file(session, &host_path, "hi", "text")
        .await
        .expect("write via host path");

    let via_workspace = store
        .read_file(session, "/workspace/sub/dir/file.txt")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(via_workspace.content.as_deref(), Some("hi"));
    assert_eq!(via_workspace.path, "/sub/dir/file.txt");
}

#[tokio::test]
async fn grep_path_pattern_accepts_host_absolute_path_alias() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .write_file(session, "/src/lib.rs", "needle", "text")
        .await
        .expect("write src");
    store
        .write_file(session, "/docs/readme.md", "needle", "text")
        .await
        .expect("write docs");
    let host_filter = store.display_path("/src");

    let matches = store
        .grep_files(session, "needle", Some(&host_filter))
        .await
        .expect("grep");

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, "/src/lib.rs");
}

#[tokio::test]
async fn grep_path_pattern_supports_globs() {
    let (store, _dir) = make_store();
    let session = sid();
    for path in [
        "/src/lib.rs",
        "/src/nested/mod.rs",
        "/docs/readme.md",
        "/docs/nested/guide.md",
        "/notes.txt",
        "/nested/notes.txt",
    ] {
        store
            .write_file(session, path, "needle", "text")
            .await
            .expect("write fixture");
    }

    let cases = [
        ("src/**/*.rs", vec!["/src/lib.rs", "/src/nested/mod.rs"]),
        (
            "**/*",
            vec![
                "/docs/nested/guide.md",
                "/docs/readme.md",
                "/nested/notes.txt",
                "/notes.txt",
                "/src/lib.rs",
                "/src/nested/mod.rs",
            ],
        ),
        ("docs/*", vec!["/docs/readme.md"]),
        ("*.txt", vec!["/nested/notes.txt", "/notes.txt"]),
        (
            "/workspace/src/**/*.rs",
            vec!["/src/lib.rs", "/src/nested/mod.rs"],
        ),
    ];

    for (path_pattern, expected) in cases {
        let mut paths: Vec<_> = store
            .grep_files(session, "needle", Some(path_pattern))
            .await
            .expect("grep")
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        paths.sort();
        assert_eq!(paths, expected, "path_pattern={path_pattern}");
    }

    let host_pattern = Path::new(&store.display_root())
        .join("src/**/*.rs")
        .display()
        .to_string();
    let mut paths: Vec<_> = store
        .grep_files(session, "needle", Some(&host_pattern))
        .await
        .expect("host-absolute glob")
        .into_iter()
        .map(|hit| hit.path)
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["/src/lib.rs", "/src/nested/mod.rs"]);
}

#[tokio::test]
async fn path_traversal_rejected() {
    let (store, _dir) = make_store();
    let session = sid();
    let err = store
        .read_file(session, "/../outside.txt")
        .await
        .expect_err("must reject traversal");
    let msg = format!("{err}");
    assert!(msg.contains("traversal"), "got: {msg}");

    let err = store
        .write_file(session, "/foo/../../etc/passwd", "x", "text")
        .await
        .expect_err("must reject traversal");
    let msg = format!("{err}");
    assert!(msg.contains("traversal"), "got: {msg}");
}

#[cfg(unix)]
#[tokio::test]
async fn read_file_rejects_symlink_to_outside_workspace() {
    let (store, dir) = make_store();
    let outside = TempDir::new().expect("outside tempdir");
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("docs/secret")).unwrap();

    let err = store
        .read_file(sid(), "/docs/secret/secret.txt")
        .await
        .expect_err("symlink read must be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("symlink"), "got: {msg}");
}

#[cfg(unix)]
#[tokio::test]
async fn list_directory_rejects_symlink_to_outside_workspace() {
    let (store, dir) = make_store();
    let outside = TempDir::new().expect("outside tempdir");
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("secret_dir")).unwrap();

    let err = store
        .list_directory(sid(), "/secret_dir")
        .await
        .expect_err("symlink list must be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("symlink"), "got: {msg}");
}

#[cfg(unix)]
#[tokio::test]
async fn write_file_rejects_symlink_parent() {
    let (store, dir) = make_store();
    let outside = TempDir::new().expect("outside tempdir");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("outlink")).unwrap();

    let err = store
        .write_file(sid(), "/outlink/owned.txt", "owned", "text")
        .await
        .expect_err("symlink write must be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("symlink"), "got: {msg}");
    assert!(!outside.path().join("owned.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn list_directory_skips_symlink_children() {
    let (store, dir) = make_store();
    let outside = TempDir::new().expect("outside tempdir");
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        dir.path().join("link.txt"),
    )
    .unwrap();
    store
        .write_file(sid(), "/safe.txt", "safe", "text")
        .await
        .unwrap();

    let entries = store.list_directory(sid(), "/").await.unwrap();
    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert!(paths.contains(&"/safe.txt"));
    assert!(!paths.contains(&"/link.txt"));
}

#[tokio::test]
async fn list_directory_returns_children() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .write_file(session, "/a.txt", "1", "text")
        .await
        .unwrap();
    store
        .write_file(session, "/sub/b.txt", "2", "text")
        .await
        .unwrap();
    store
        .write_file(session, "/sub/c.txt", "3", "text")
        .await
        .unwrap();

    let root = store.list_directory(session, "/").await.unwrap();
    let paths: Vec<&str> = root.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"/a.txt"));
    assert!(paths.contains(&"/sub"));

    let sub = store.list_directory(session, "/sub").await.unwrap();
    let sub_paths: Vec<&str> = sub.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(sub_paths, vec!["/sub/b.txt", "/sub/c.txt"]);
}

#[tokio::test]
async fn grep_finds_matches_and_respects_ignore_files() {
    let (store, dir) = make_store();
    let session = sid();
    // The `ignore` crate honors `.ignore` files unconditionally; it
    // honors `.gitignore` only inside a real git repo, which we don't
    // need for this test. Both files are walked by `WalkBuilder`.
    std::fs::write(dir.path().join(".ignore"), "ignored.txt\n").unwrap();
    store
        .write_file(
            session,
            "/src.rs",
            "fn needle() {}\nfn other() {}\n",
            "text",
        )
        .await
        .unwrap();
    store
        .write_file(session, "/ignored.txt", "needle\n", "text")
        .await
        .unwrap();

    let hits = store.grep_files(session, "needle", None).await.unwrap();
    let hit_paths: Vec<&str> = hits.iter().map(|m| m.path.as_str()).collect();
    assert!(hit_paths.contains(&"/src.rs"));
    assert!(!hit_paths.contains(&"/ignored.txt"));

    let filtered = store
        .grep_files(session, "needle", Some(".rs"))
        .await
        .unwrap();
    assert!(filtered.iter().all(|m| m.path.ends_with(".rs")));
}

#[tokio::test]
async fn cas_rejects_stale_writes() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .write_file(session, "/foo.txt", "v1", "text")
        .await
        .unwrap();

    // Stale CAS — expects v0 content.
    let stale = store
        .write_file_if_content_matches(session, "/foo.txt", "v0", "text", "v2", "text")
        .await
        .unwrap();
    assert!(stale.is_none(), "stale CAS should not update");

    let read = store.read_file(session, "/foo.txt").await.unwrap().unwrap();
    assert_eq!(read.content.as_deref(), Some("v1"));

    // Matching CAS — updates.
    let updated = store
        .write_file_if_content_matches(session, "/foo.txt", "v1", "text", "v2", "text")
        .await
        .unwrap();
    assert!(updated.is_some(), "matching CAS should update");
    let read = store.read_file(session, "/foo.txt").await.unwrap().unwrap();
    assert_eq!(read.content.as_deref(), Some("v2"));
}

#[tokio::test]
async fn cas_is_serialized_across_stores_for_the_same_shared_root() {
    let directory = TempDir::new().unwrap();
    let first = RealDiskFileStore::new(directory.path()).unwrap();
    let second = RealDiskFileStore::new(directory.path()).unwrap();
    let session = sid();
    first
        .write_file(session, "/shared.txt", "v1", "text")
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let write = |store: RealDiskFileStore, content: &'static str| {
        let barrier = barrier.clone();
        async move {
            barrier.wait().await;
            store
                .write_file_if_content_matches(
                    session,
                    "/shared.txt",
                    "v1",
                    "text",
                    content,
                    "text",
                )
                .await
                .unwrap()
                .is_some()
        }
    };
    let first_write = tokio::spawn(write(first, "first"));
    let second_write = tokio::spawn(write(second, "second"));
    barrier.wait().await;
    let (first_won, second_won) = tokio::join!(first_write, second_write);
    assert_ne!(first_won.unwrap(), second_won.unwrap());
}

#[tokio::test]
async fn delete_non_recursive_fails_on_nonempty_dir() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .write_file(session, "/d/x.txt", "x", "text")
        .await
        .unwrap();

    let removed = store.delete_file(session, "/d", false).await.unwrap();
    assert!(!removed, "non-recursive delete must refuse non-empty dir");

    let removed = store.delete_file(session, "/d", true).await.unwrap();
    assert!(removed);
    let after = store.read_file(session, "/d/x.txt").await.unwrap();
    assert!(after.is_none());
}

#[tokio::test]
async fn seed_initial_file_persists() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .seed_initial_file(
            session,
            &InitialFile {
                path: "/workspace/AGENTS.md".to_string(),
                content: "# Project rules".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
        )
        .await
        .unwrap();

    let read = store
        .read_file(session, "/AGENTS.md")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content.as_deref(), Some("# Project rules"));
}

#[tokio::test]
async fn root_directory_resolves() {
    let (store, _dir) = make_store();
    let session = sid();
    let stat = store.stat_file(session, "/").await.unwrap().unwrap();
    assert!(stat.is_directory);
    assert_eq!(stat.path, "/");
}

#[tokio::test]
async fn rejects_missing_root() {
    let missing = std::env::temp_dir().join("everruns-nonexistent-xyz-12345");
    let _ = std::fs::remove_dir_all(&missing);
    let err = RealDiskFileStore::new(&missing).expect_err("must reject missing root");
    let msg = format!("{err}");
    assert!(msg.contains("does not exist"), "got: {msg}");
}

#[tokio::test]
async fn delete_root_returns_explicit_error() {
    let (store, _dir) = make_store();
    let session = sid();
    let err = store
        .delete_file(session, "/", true)
        .await
        .expect_err("root delete must be an explicit error, not Ok(false)");
    assert!(format!("{err}").contains("workspace root"));
}

#[tokio::test]
async fn seeded_readonly_file_rejects_writes() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .seed_initial_file(
            session,
            &InitialFile {
                path: "/locked.txt".to_string(),
                content: "starter".to_string(),
                encoding: "text".to_string(),
                is_readonly: true,
            },
        )
        .await
        .unwrap();

    let read = store
        .read_file(session, "/locked.txt")
        .await
        .unwrap()
        .unwrap();
    assert!(read.is_readonly);

    let err = store
        .write_file(session, "/locked.txt", "changed", "text")
        .await
        .expect_err("readonly write must fail");
    assert!(format!("{err}").contains("read-only"));

    let err = store
        .delete_file(session, "/locked.txt", false)
        .await
        .expect_err("readonly delete must fail");
    assert!(format!("{err}").contains("read-only"));
}

#[tokio::test]
async fn reseeding_clears_readonly() {
    let (store, _dir) = make_store();
    let session = sid();
    store
        .seed_initial_file(
            session,
            &InitialFile {
                path: "/foo.txt".to_string(),
                content: "v1".to_string(),
                encoding: "text".to_string(),
                is_readonly: true,
            },
        )
        .await
        .unwrap();
    // Re-seed without readonly: subsequent writes must succeed.
    store
        .seed_initial_file(
            session,
            &InitialFile {
                path: "/foo.txt".to_string(),
                content: "v2".to_string(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
        )
        .await
        .unwrap();
    store
        .write_file(session, "/foo.txt", "v3", "text")
        .await
        .unwrap();
}
