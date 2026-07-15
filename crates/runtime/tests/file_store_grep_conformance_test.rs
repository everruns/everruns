use std::sync::Arc;

use everruns_core::traits::SessionFileSystem;
use everruns_core::typed_id::SessionId;
use everruns_runtime::{InMemorySessionFileStore, RealDiskFileStore};
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
}

#[tokio::test]
async fn in_memory_store_honors_grep_regex_contract() {
    assert_regex_contract(Arc::new(InMemorySessionFileStore::new())).await;
}

#[tokio::test]
async fn real_disk_store_honors_grep_regex_contract() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    assert_regex_contract(store).await;
}
