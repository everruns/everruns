// EVE-660 conformance: one mount-resolved namespace, one root, across a
// worktree switch.
//
// The agent's filesystem is `MountFs` over the workspace backend: `/workspace`
// is a mount + the default cwd, not a per-store prefix. This proves that
// `read_file`, `grep_files`, the on-disk files themselves, the bash cwd, and
// cwd-relative resolution all agree on the same root — and that repointing the
// host root (the worktree-switch scenario) moves every surface together.

use everruns_core::capabilities::BashTool;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::{MountFs, SessionFileSystem, SessionId, WorkspaceRootSet};
use everruns_runtime::{RealDiskFileStore, multi_root_file_system};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Run `pwd` through the bash tool and return the reported working directory.
async fn bash_pwd(ctx: &ToolContext) -> String {
    match BashTool::default()
        .execute_with_context(json!({ "commands": "pwd" }), ctx)
        .await
    {
        ToolExecutionResult::Success(output) => output["stdout"]
            .as_str()
            .expect("pwd stdout is a string")
            .trim()
            .to_string(),
        other => panic!("bash pwd did not succeed: {other:?}"),
    }
}

#[tokio::test]
async fn all_surfaces_agree_on_root_across_worktree_switch() {
    let session = SessionId::from_seed(660);

    // --- Worktree A ---
    let worktree_a = TempDir::new().unwrap();
    // The host-backed store, behind the mount resolver (as the runtime wires it).
    let backend = Arc::new(RealDiskFileStore::new(worktree_a.path()).unwrap());
    let store: Arc<dyn SessionFileSystem> = MountFs::wrap(backend.clone());
    let ctx = ToolContext::with_file_store(session, store.clone());

    let root_a = backend.root();

    // Model addresses files at /workspace; the resolver maps to the backend.
    store
        .write_file(session, "/workspace/marker.txt", "ALPHA", "text")
        .await
        .unwrap();

    // read_file agrees — both the /workspace view and the backend-native path.
    let read = store
        .read_file(session, "/workspace/marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content.as_deref(), Some("ALPHA"));
    assert_eq!(read.path, "/marker.txt");
    let direct = backend
        .read_file(session, "/marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.path, direct.path,
        "MountFs preserves primary result paths"
    );

    // cwd-relative resolution agrees: cwd defaults to /workspace.
    let relative = store
        .read_file(session, "marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(relative.content.as_deref(), Some("ALPHA"));

    // grep_files agrees.
    let hits = store.grep_files(session, "ALPHA", None).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/marker.txt");

    // The file is physically on disk under worktree A's root — what a host-path
    // tool (e.g. ast_grep) shelling out against `backend.root()` would scan.
    assert_eq!(
        std::fs::read_to_string(backend.root().join("marker.txt")).unwrap(),
        "ALPHA"
    );

    // Mount routing does not replace the real-disk store's host path identity.
    assert_eq!(store.display_root(), root_a.display().to_string());
    assert_eq!(
        store.display_path("/marker.txt"),
        root_a.join("marker.txt").display().to_string()
    );
    assert_eq!(
        store.resolve_path("marker.txt"),
        root_a.join("marker.txt").display().to_string()
    );
    assert_eq!(bash_pwd(&ctx).await, root_a.display().to_string());

    // --- Switch to Worktree B (the embedder moved worktrees) ---
    let worktree_b = TempDir::new().unwrap();
    backend.set_host_root(worktree_b.path()).unwrap();
    let root_b = backend.root();
    assert_ne!(root_a, root_b);

    // A file that exists only in B.
    store
        .write_file(session, "/marker.txt", "BETA", "text")
        .await
        .unwrap();

    // Every surface now resolves to worktree B — using the *same* context.
    let read_b = store
        .read_file(session, "/workspace/marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_b.content.as_deref(), Some("BETA"));

    let hits_b = store.grep_files(session, "BETA", None).await.unwrap();
    assert_eq!(hits_b.len(), 1);
    assert_eq!(hits_b[0].path, "/marker.txt");

    // The write landed on disk under the new root, not the old one.
    assert_eq!(
        std::fs::read_to_string(root_b.join("marker.txt")).unwrap(),
        "BETA"
    );
    assert_eq!(
        std::fs::read_to_string(root_a.join("marker.txt")).unwrap(),
        "ALPHA",
        "worktree A is untouched by writes after the switch"
    );

    // The visible namespace follows the backing store after the switch.
    assert_eq!(store.display_root(), root_b.display().to_string());
    assert_eq!(
        store.display_path("/marker.txt"),
        root_b.join("marker.txt").display().to_string()
    );
    assert_eq!(bash_pwd(&ctx).await, root_b.display().to_string());
}

#[tokio::test]
async fn multi_root_display_and_containment_contract() {
    let session = SessionId::from_seed(661);
    let primary = TempDir::new().unwrap();
    let secondary = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let roots = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), secondary.path().to_path_buf())],
    )
    .unwrap();
    let store = multi_root_file_system(&roots).unwrap();

    assert_eq!(
        store.display_root(),
        roots.primary_host_root().display().to_string()
    );
    assert_eq!(
        store.display_path("/src/lib.rs"),
        roots
            .primary_host_root()
            .join("src/lib.rs")
            .display()
            .to_string()
    );

    let primary_file = store
        .write_file(session, "src/lib.rs", "primary", "text")
        .await
        .unwrap();
    assert_eq!(primary_file.path, "/src/lib.rs");
    assert_eq!(
        std::fs::read_to_string(primary.path().join("src/lib.rs")).unwrap(),
        "primary"
    );

    let secondary_file = store
        .write_file(
            session,
            "/workspace/roots/backend/Cargo.toml",
            "secondary",
            "text",
        )
        .await
        .unwrap();
    assert_eq!(secondary_file.path, "/workspace/roots/backend/Cargo.toml");
    assert_eq!(
        store.display_path(&secondary_file.path),
        "/workspace/roots/backend/Cargo.toml"
    );
    assert_eq!(
        std::fs::read_to_string(secondary.path().join("Cargo.toml")).unwrap(),
        "secondary"
    );

    let traversal = store
        .read_file(session, "/workspace/roots/backend/../escape")
        .await
        .unwrap_err();
    assert!(traversal.to_string().contains("path traversal rejected"));

    let outside_path = outside.path().join("secret.txt");
    std::fs::write(&outside_path, "secret").unwrap();
    let containment = store
        .read_file(session, outside_path.to_str().unwrap())
        .await
        .unwrap();
    assert!(
        containment.is_none(),
        "an absolute path outside every registered root cannot expose that host file"
    );
}
