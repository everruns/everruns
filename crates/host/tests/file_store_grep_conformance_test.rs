use std::sync::Arc;

use everruns_core::session_files::SessionFileSystem;
use everruns_core::{GREP_MAX_RETURN_BYTES, GrepOptions};
use everruns_host::{InMemorySessionFileStore, RealDiskFileStore};
use everruns_provider::typed_id::SessionId;
use tempfile::TempDir;

async fn assert_regex_contract(store: Arc<dyn SessionFileSystem>) {
    let session = SessionId::from_seed(776);

    let error = store
        .grep_files(session, "[invalid", None)
        .await
        .expect_err("invalid regex must fail before scanning");
    assert!(
        error.to_string().contains("Invalid regex pattern"),
        "unexpected error: {error}"
    );

    for (path, content) in [
        (
            "/src/app.log",
            "Error: code 42\nall good\ntimeout after 120s",
        ),
        ("/src/worker.log", "request failed with code 7"),
        ("/docs/readme.md", "Error handling guide 99"),
    ] {
        store
            .write_file(session, path, content, "text")
            .await
            .unwrap();
    }

    let mut alternatives: Vec<_> = store
        .grep_files(session, "Error|failed|timeout", Some("src/**/*.log"))
        .await
        .unwrap()
        .into_iter()
        .map(|hit| (hit.path, hit.line_number))
        .collect();
    alternatives.sort();
    assert_eq!(
        alternatives,
        vec![
            ("/src/app.log".to_string(), 1),
            ("/src/app.log".to_string(), 3),
            ("/src/worker.log".to_string(), 1),
        ]
    );

    let mut repeated_digits: Vec<_> = store
        .grep_files(session, r"\d{2,}", None)
        .await
        .unwrap()
        .into_iter()
        .map(|hit| (hit.path, hit.line_number))
        .collect();
    repeated_digits.sort();
    assert_eq!(
        repeated_digits,
        vec![
            ("/docs/readme.md".to_string(), 1),
            ("/src/app.log".to_string(), 1),
            ("/src/app.log".to_string(), 3),
        ]
    );

    let literal = store
        .grep_files(session, "all good", Some("src/app.log"))
        .await
        .unwrap();
    assert_eq!(literal.len(), 1);
    assert_eq!(literal[0].path, "/src/app.log");
    assert_eq!(literal[0].line_number, 2);

    let contextual = store
        .grep_files_with_options(
            session,
            "Error|timeout",
            &GrepOptions {
                path_pattern: Some("src/app.log".to_string()),
                before_context: 1,
                after_context: 1,
                offset: 0,
                limit: 2,
                max_bytes: GREP_MAX_RETURN_BYTES,
            },
        )
        .await
        .unwrap();
    assert_eq!(contextual.total_matches, 2);
    assert_eq!(contextual.returned_matches, 2);
    assert_eq!(contextual.blocks.len(), 1);
    let block = &contextual.blocks[0];
    assert_eq!(block.start_line, 1);
    assert_eq!(block.end_line, 3);
    assert_eq!(block.match_line_numbers, vec![1, 3]);
    assert_eq!(
        block
            .lines
            .iter()
            .map(|line| (line.line_number, line.is_match))
            .collect::<Vec<_>>(),
        vec![(1, true), (2, false), (3, true)]
    );

    let paged = store
        .grep_files_with_options(
            session,
            "Error|failed|timeout",
            &GrepOptions {
                path_pattern: None,
                before_context: 1,
                after_context: 1,
                offset: 1,
                limit: 1,
                max_bytes: GREP_MAX_RETURN_BYTES,
            },
        )
        .await
        .unwrap();
    assert_eq!(paged.returned_matches, 1);
    assert_eq!(paged.next_offset, Some(2));
    assert_eq!(
        paged
            .blocks
            .iter()
            .flat_map(|block| &block.match_line_numbers)
            .copied()
            .collect::<Vec<_>>(),
        vec![1]
    );
}

async fn assert_grep_limits(store: Arc<dyn SessionFileSystem>) {
    let session = SessionId::from_seed(777);
    let error = store
        .grep_files(session, &"a".repeat(1_001), None)
        .await
        .expect_err("oversized regex must fail before scanning");
    assert!(error.to_string().contains("Regex pattern too long"));

    let error = store
        .grep_files(session, "needle", Some(&"a".repeat(1_001)))
        .await
        .expect_err("oversized path pattern must fail before scanning");
    assert!(error.to_string().contains("Path pattern too long"));

    store
        .write_file(session, "/large.txt", &"needle".repeat(100_000), "text")
        .await
        .unwrap();
    assert!(
        store
            .grep_files(session, "needle", None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn in_memory_store_honors_grep_regex_contract() {
    let store = Arc::new(InMemorySessionFileStore::new());
    assert_regex_contract(store.clone()).await;
    assert_grep_limits(store).await;
}

#[tokio::test]
async fn real_disk_store_honors_grep_regex_contract() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    assert_regex_contract(store.clone()).await;
    assert_grep_limits(store).await;
}

#[tokio::test]
async fn mounted_grep_preserves_alias_segment_boundaries_for_real_backends() {
    let root = TempDir::new().unwrap();
    let backends: Vec<Arc<dyn SessionFileSystem>> = vec![
        Arc::new(InMemorySessionFileStore::new()),
        Arc::new(RealDiskFileStore::new(root.path()).unwrap()),
    ];
    for backend in backends {
        let store = everruns_core::MountFs::new(backend);
        let session = SessionId::from_seed(778);
        for (path, content) in [
            ("/workspacefoo/target.txt", "before\nneedle target\nafter"),
            ("/foo/decoy.txt", "needle decoy"),
        ] {
            store
                .write_file(session, path, content, "text")
                .await
                .unwrap();
        }
        for (filter, expected) in [
            ("/workspacefoo", "/workspacefoo/target.txt"),
            ("/workspacefoo/*.txt", "/workspacefoo/target.txt"),
            ("/workspace/foo/*.txt", "/foo/decoy.txt"),
        ] {
            let flat = store
                .grep_files(session, "needle", Some(filter))
                .await
                .unwrap();
            assert_eq!(
                flat.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(),
                [expected]
            );
            let contextual = store
                .grep_files_with_options(
                    session,
                    "needle",
                    &GrepOptions {
                        path_pattern: Some(filter.into()),
                        before_context: 1,
                        after_context: 1,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(contextual.total_matches, 1);
            assert_eq!(contextual.returned_matches, 1);
            assert_eq!(
                contextual
                    .blocks
                    .iter()
                    .map(|block| block.path.as_str())
                    .collect::<Vec<_>>(),
                [expected]
            );
            if expected == "/workspacefoo/target.txt" {
                assert_eq!(
                    contextual.blocks[0]
                        .lines
                        .iter()
                        .map(|line| (line.line.as_str(), line.is_match))
                        .collect::<Vec<_>>(),
                    [("before", false), ("needle target", true), ("after", false)]
                );
            }
        }
    }
}
