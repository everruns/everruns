use std::sync::Arc;

use everruns_contracts::runtime::mount_fs::MountFs;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::InMemorySessionFileStore;
use everruns_core::session_files::SessionFileSystem;
use serde_json::{Value, json};

fn with(capabilities: Value) -> Arc<Value> {
    Arc::new(json!({
        "name": "docs",
        "instructions": "Answer.",
        "model": { "provider": "llmsim", "model": "echo" },
        "capabilities": capabilities,
    }))
}

fn mounted(definition: &Arc<Value>) -> MountFs {
    let backends = super::attach(everruns_core::host::HostBackends::in_memory(), definition);
    let mut fs = MountFs::new(Arc::new(InMemorySessionFileStore::new()));
    for (point, backend, root) in backends.file_mounts {
        fs = fs.with_mount(point, backend, root);
    }
    fs
}

#[tokio::test]
async fn the_platform_capabilitys_docs_are_read_only_under_the_workspace() {
    let definition = with(json!([{ "ref": "platform" }]));
    let fs = mounted(&definition);
    let session = SessionId::new();

    let listed = fs.list_directory(session, "/workspace/docs").await.unwrap();
    assert!(!listed.is_empty(), "the docs are mounted");
    assert!(
        listed
            .iter()
            .all(|e| e.path.starts_with("/workspace/docs/")),
        "{:?}",
        listed.iter().map(|e| &e.path).collect::<Vec<_>>()
    );
    // The same files under both spellings.
    let file = listed
        .iter()
        .find(|e| !e.is_directory)
        .or_else(|| listed.first())
        .unwrap();
    let mut path = file.path.clone();
    if file.is_directory {
        let inner = fs.list_directory(session, &path).await.unwrap();
        path = inner.iter().find(|e| !e.is_directory).unwrap().path.clone();
    }
    let read = fs.read_file(session, &path).await.unwrap().unwrap();
    assert!(read.is_readonly);
    assert!(!read.content.unwrap_or_default().is_empty());
    let short = path.trim_start_matches("/workspace");
    assert!(fs.read_file(session, short).await.unwrap().is_some());

    // Read-only: no write, no delete; the rest of the workspace is the
    // session's own.
    let refused = fs
        .write_file(session, &path, "changed", "text")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("read-only"), "{refused}");
    assert!(fs.delete_file(session, &path, false).await.is_err());
    fs.write_file(session, "/workspace/notes.md", "mine", "text")
        .await
        .unwrap();

    // Found by a search of the workspace.
    let hits = fs
        .grep_files(session, "Everruns", Some("/workspace/docs/**"))
        .await
        .unwrap();
    assert!(!hits.is_empty(), "grep finds the docs");

    // Resolved once per shared definition.
    assert!(Arc::ptr_eq(
        &super::trees(&definition),
        &super::trees(&definition)
    ));
}

#[tokio::test]
async fn an_agent_without_mounting_capabilities_gets_nothing() {
    let definition = with(json!([{ "ref": "current_time" }]));
    let backends = super::attach(everruns_core::host::HostBackends::in_memory(), &definition);
    assert!(backends.file_mounts.is_empty());
    assert!(super::starter_files(&definition).is_empty());
}

#[test]
fn inline_capability_files_are_starter_files() {
    let definition = with(json!([{ "ref": "data_knowledge" }]));
    let files = super::starter_files(&definition);
    let index = files
        .iter()
        .find(|f| f.path == "/knowledge/index.md")
        .unwrap_or_else(|| panic!("{files:?}"));
    assert!(index.is_readonly);
    assert!(files.iter().any(|f| f.path == "/knowledge/tables/index.md"));
    // No virtual tree among them: those are mounted, not copied.
    assert!(
        super::starter_files(&with(json!([{ "ref": "platform" }])))
            .iter()
            .all(|f| !f.path.starts_with("/docs"))
    );
}
