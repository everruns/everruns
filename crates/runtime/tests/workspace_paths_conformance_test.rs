// EVE-660 conformance: every workspace surface shares one namespace and one
// root, including across a worktree switch.
//
// Acceptance criterion: `read_file`, `grep_files`, a host-path scanner
// capability, and the bash cwd must all agree on the same root before and after
// `set_host_root` repoints the workspace (the worktree-switch scenario).

use everruns_core::capabilities::BashTool;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::{SessionFileSystem, SessionId};
use everruns_runtime::RealDiskFileStore;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Read a path the way a host-path capability (e.g. `repo_map`, `ast_grep`)
/// would: resolve through the shared `WorkspacePaths` to a host path, then scan
/// the real file. No local `/workspace` stripping.
fn host_scanner_read(ctx: &ToolContext, input: &str) -> std::io::Result<String> {
    let paths = ctx.workspace_paths();
    let rel = paths
        .parse_input(input)
        .expect("scanner: parse workspace path");
    let host = paths.to_host(&rel).expect("scanner: map to host");
    std::fs::read_to_string(host)
}

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
    let store = Arc::new(RealDiskFileStore::new(worktree_a.path()).unwrap());
    let ctx = ToolContext::with_file_store(session, store.clone());

    let root_a = store.root();

    // The model addresses files with the workspace alias; the store maps it.
    store
        .write_file(session, "/workspace/marker.txt", "ALPHA", "text")
        .await
        .unwrap();

    // read_file agrees.
    let read = store
        .read_file(session, "/workspace/marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content.as_deref(), Some("ALPHA"));
    assert_eq!(read.path, "/marker.txt");

    // grep_files agrees.
    let hits = store.grep_files(session, "ALPHA", None).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/marker.txt");

    // Host-path scanner agrees — and reads from worktree A on disk.
    assert_eq!(host_scanner_read(&ctx, "/marker.txt").unwrap(), "ALPHA");
    assert_eq!(
        ctx.workspace_paths()
            .to_host(&ctx.workspace_paths().parse_input("/").unwrap())
            .unwrap(),
        root_a
    );

    // Bash cwd agrees: pwd reports worktree A's root.
    assert_eq!(bash_pwd(&ctx).await, root_a.display().to_string());

    // --- Switch to Worktree B (e.g. the embedder moved worktrees) ---
    let worktree_b = TempDir::new().unwrap();
    store.set_host_root(worktree_b.path()).unwrap();
    let root_b = store.root();
    assert_ne!(root_a, root_b);

    // A file that exists only in B.
    store
        .write_file(session, "/marker.txt", "BETA", "text")
        .await
        .unwrap();

    // Every surface now agrees on worktree B — using the *same* context.
    let read_b = store
        .read_file(session, "/workspace/marker.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_b.content.as_deref(), Some("BETA"));

    let hits_b = store.grep_files(session, "BETA", None).await.unwrap();
    assert_eq!(hits_b.len(), 1);
    assert_eq!(hits_b[0].path, "/marker.txt");

    assert_eq!(host_scanner_read(&ctx, "/marker.txt").unwrap(), "BETA");

    // The old root no longer holds the file the scanner now resolves.
    let host_b = ctx
        .workspace_paths()
        .to_host(&ctx.workspace_paths().parse_input("/marker.txt").unwrap())
        .unwrap();
    assert!(host_b.starts_with(&root_b));
    assert!(!host_b.starts_with(&root_a));

    // Bash cwd followed the switch too.
    assert_eq!(bash_pwd(&ctx).await, root_b.display().to_string());
}
