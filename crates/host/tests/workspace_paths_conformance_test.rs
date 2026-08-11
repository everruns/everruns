// EVE-660 conformance: one mount-resolved namespace, one root, across a
// worktree switch.
//
// The agent's filesystem is `MountFs` over the workspace backend: `/workspace`
// is a mount + the default cwd, not a per-store prefix. This proves that
// `read_file`, `grep_files`, the on-disk files themselves, the bash cwd, and
// cwd-relative resolution all agree on the same root — and that repointing the
// host root (the worktree-switch scenario) moves every surface together.

use everruns_builtins::{DistillOutputHook, PersistOutputHook};
use everruns_core::atoms::PostToolExecHook;
use everruns_core::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResult,
};
#[cfg(any(feature = "bashkit", feature = "filesystem"))]
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::{MountFs, SessionFileSystem, SessionId, WorkspaceRootSet};
use everruns_host::{InMemorySessionFileStore, RealDiskFileStore, multi_root_file_system};
#[cfg(feature = "bashkit")]
use everruns_integrations_bashkit::BashTool;
#[cfg(feature = "filesystem")]
use everruns_integrations_filesystem::WriteFileTool;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Run `pwd` through the bash tool and return the reported working directory.
#[cfg(feature = "bashkit")]
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

fn output_tool_def(persist_output: bool) -> ToolDefinition {
    let hints = if persist_output {
        ToolHints::default().with_persist_output(true)
    } else {
        ToolHints::default()
    };
    ToolDefinition::Builtin(BuiltinTool {
        name: "test_output".to_string(),
        display_name: None,
        description: "test output".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints,
        full_parameters: None,
    })
}

fn output_tool_call() -> ToolCall {
    ToolCall {
        id: "call_paths".to_string(),
        name: "test_output".to_string(),
        arguments: json!({}),
    }
}

async fn assert_output_pointers_follow_store(store: Arc<dyn SessionFileSystem>) {
    let session = SessionId::from_seed(662);
    let context = ToolContext::with_file_store(session, store.clone());
    let root = store.display_root();
    let stdout_path = format!("{root}/outputs/call_paths.stdout");
    let stderr_path = format!("{root}/outputs/call_paths.stderr");
    let mut result = ToolResult {
        tool_call_id: "call_paths".to_string(),
        result: Some(json!({
            "stdout": "stdout body",
            "stderr": "stderr body",
            "exit_code": 0,
        })),
        images: None,
        error: None,
        connection_required: None,
        raw_output: Some("stdout body\n--- stderr ---\nstderr body".to_string()),
    };

    PersistOutputHook
        .after_exec(
            &output_tool_call(),
            &output_tool_def(true),
            &mut result,
            &context,
        )
        .await;

    let value = result.result.as_ref().unwrap();
    assert_eq!(value["full_output"], json!(stdout_path));
    assert_eq!(value["output_files"], json!([stdout_path, stderr_path]));
    for pointer in value["output_files"].as_array().unwrap() {
        assert!(
            store
                .read_file(session, pointer.as_str().unwrap())
                .await
                .unwrap()
                .is_some(),
            "model-visible output pointer must be readable by file tools"
        );
    }
    assert!(
        value["stdout"]
            .as_str()
            .unwrap()
            .contains(&store.display_path("/outputs/call_paths.stdout"))
    );
}

async fn assert_distillation_pointer_follows_store(store: Arc<dyn SessionFileSystem>) {
    let session = SessionId::from_seed(663);
    let context = ToolContext::with_file_store(session, store.clone());
    let rows: Vec<_> = (0..2000)
        .map(|i| json!({"id": i, "name": format!("row-{i}")}))
        .collect();
    let mut result = ToolResult {
        tool_call_id: "call_paths".to_string(),
        result: Some(json!({"rows": rows})),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    };

    DistillOutputHook
        .after_exec(
            &output_tool_call(),
            &output_tool_def(false),
            &mut result,
            &context,
        )
        .await;

    let expected = store.display_path("/outputs/call_paths.stdout");
    let value = result.result.as_ref().unwrap();
    assert_eq!(value["full_output"], json!(expected));
    assert_eq!(value["output_files"], json!([expected]));
    assert!(value["distill_note"].as_str().unwrap().contains(&expected));
    assert!(store.read_file(session, &expected).await.unwrap().is_some());
}

#[tokio::test]
async fn persisted_output_paths_follow_store_display_identity() {
    let direct_root = TempDir::new().unwrap();
    let direct: Arc<dyn SessionFileSystem> =
        Arc::new(RealDiskFileStore::new(direct_root.path()).unwrap());
    assert_output_pointers_follow_store(direct.clone()).await;
    assert_distillation_pointer_follows_store(direct).await;

    let mounted_root = TempDir::new().unwrap();
    let mounted = MountFs::wrap(Arc::new(
        RealDiskFileStore::new(mounted_root.path()).unwrap(),
    ));
    assert_output_pointers_follow_store(mounted.clone()).await;
    assert_distillation_pointer_follows_store(mounted).await;

    let memory: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    assert_eq!(memory.display_root(), "/workspace");
    assert_output_pointers_follow_store(memory.clone()).await;
    assert_distillation_pointer_follows_store(memory).await;
}

#[cfg(feature = "bashkit")]
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

    // MountFs keeps agent-visible presentation in the stable /workspace
    // namespace; host paths remain available through the real-disk backend for
    // host-side integrations.
    assert_eq!(store.display_root(), "/workspace");
    assert_eq!(store.display_path("/marker.txt"), "/workspace/marker.txt");
    assert_eq!(store.resolve_path("marker.txt"), "/workspace/marker.txt");
    assert_eq!(backend.display_root(), root_a.display().to_string());
    assert_eq!(
        backend.display_path("/marker.txt"),
        root_a.join("marker.txt").display().to_string()
    );
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

    // The host-backed contents follow the worktree switch, while the
    // agent-visible namespace remains stable and host-agnostic.
    assert_eq!(store.display_root(), "/workspace");
    assert_eq!(store.display_path("/marker.txt"), "/workspace/marker.txt");
    assert_eq!(backend.display_root(), root_b.display().to_string());
    assert_eq!(bash_pwd(&ctx).await, "/workspace");
}

#[test]
fn backend_native_display_keeps_literal_workspace_segment() {
    let root = TempDir::new().unwrap();
    let canonical_root = std::fs::canonicalize(root.path()).unwrap();
    let backend = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    let store = MountFs::new(backend).with_backend_display();

    assert_eq!(
        store.display_path("/workspace/collide.txt"),
        canonical_root
            .join("workspace/collide.txt")
            .display()
            .to_string()
    );
    assert_eq!(
        store.resolve_path("workspace/collide.txt"),
        canonical_root
            .join("workspace/collide.txt")
            .display()
            .to_string()
    );
    assert_eq!(
        store.resolve_path("/workspace/collide.txt"),
        canonical_root.join("collide.txt").display().to_string()
    );
}

#[cfg(feature = "filesystem")]
#[tokio::test]
async fn direct_real_disk_write_reports_workspace_alias_target() {
    let root = TempDir::new().unwrap();
    let store: Arc<dyn SessionFileSystem> = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    let context = ToolContext::with_file_store(SessionId::from_seed(661), store);

    let result = WriteFileTool
        .execute_with_context(
            json!({ "path": "/workspace/matrix/out.txt", "content": "hello" }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("write_file did not succeed: {result:?}");
    };

    let reported = output["path"].as_str().expect("path is a string");
    let expected = std::fs::canonicalize(root.path().join("matrix/out.txt")).unwrap();
    assert_eq!(
        reported,
        expected.to_str().unwrap(),
        "real-disk output uses the canonical host path on platforms where the temp root is a symlink"
    );
    assert!(std::path::Path::new(reported).is_file());
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

    assert_eq!(store.display_root(), "/workspace");
    assert_eq!(store.display_path("/src/lib.rs"), "/workspace/src/lib.rs");

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
