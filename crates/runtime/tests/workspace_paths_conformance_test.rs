// EVE-660 conformance: one mount-resolved namespace, one root, across a
// worktree switch.
//
// The agent's filesystem is `MountFs` over the workspace backend: `/workspace`
// is a mount + the default cwd, not a per-store prefix. This proves that
// `read_file`, `grep_files`, the on-disk files themselves, the bash cwd, and
// cwd-relative resolution all agree on the same root — and that repointing the
// host root (the worktree-switch scenario) moves every surface together while
// the model-facing namespace stays a stable `/workspace`.

use everruns_core::capabilities::BashTool;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::{MountFs, SessionFileSystem, SessionId};
use everruns_runtime::RealDiskFileStore;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Run `pwd` through the bash tool and return the reported working directory.
async fn bash_pwd(ctx: &ToolContext) -> String {
    match BashTool
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

    // The model-facing namespace is a stable /workspace, even on real disk.
    assert_eq!(store.display_root(), "/workspace");
    assert_eq!(store.display_path("/marker.txt"), "/workspace/marker.txt");
    assert_eq!(bash_pwd(&ctx).await, "/workspace");

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

    // The model-facing namespace is unchanged: still /workspace.
    assert_eq!(store.display_root(), "/workspace");
    assert_eq!(bash_pwd(&ctx).await, "/workspace");
}
