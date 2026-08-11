// EVE-750: end-to-end conformance for model-visible filesystem path identity.
//
// Exercises tool results, system prompts, persistence hooks, and the recursive
// path scanner across in-memory and mounted real-disk backends.

#![cfg(feature = "filesystem")]

use everruns_builtins::{DistillOutputHook, PersistOutputHook, ToolSearchCapability};
use everruns_core::atoms::PostToolExecHook;
use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};
use everruns_core::path_identity::{
    PathIdentityExpectations, assert_model_visible_value, assert_no_forbidden_prefixes,
    assert_system_prompt, assert_tool_result_paths_conform, collect_absolute_paths,
};
use everruns_core::session_path::WORKSPACE_PREFIX;
use everruns_core::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResult,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::{Capability, MountFs, SessionFileSystem, SessionId, WorkspaceRootSet};
use everruns_host::{
    InMemorySessionFileStore, RealDiskFileStore, multi_root_file_system,
    runtime_capability_registry,
};
use everruns_integrations_filesystem::{
    DeleteFileTool, EditFileTool, FileSystemCapability, GrepFilesTool, ListDirectoryTool,
    ReadFileTool, SESSION_FILE_SYSTEM_CAPABILITY_ID, StatFileTool, WriteFileTool,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;

fn production_context(session: SessionId, store: Arc<dyn SessionFileSystem>) -> ToolContext {
    ToolContext::with_file_store(session, MountFs::wrap(store))
}

fn production_context_direct(session: SessionId, store: Arc<dyn SessionFileSystem>) -> ToolContext {
    ToolContext::with_file_store(session, store)
}

fn expect_success(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        ToolExecutionResult::SuccessWithImages { result, .. } => result,
        other => panic!("expected success, got {other:?}"),
    }
}

async fn seed_target_file(store: &dyn SessionFileSystem, session: SessionId, rel: &str) {
    store
        .write_file(session, rel, "target body", "text")
        .await
        .unwrap();
}

async fn read_hash(ctx: &ToolContext, path: &str) -> String {
    let result = ReadFileTool
        .execute_with_context(json!({ "path": path }), ctx)
        .await;
    expect_success(result)["content_hash"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn assert_read_paths(
    ctx: &ToolContext,
    store: &dyn SessionFileSystem,
    input: &str,
    expected_display: &str,
) {
    let result = ReadFileTool
        .execute_with_context(json!({ "path": input }), ctx)
        .await;
    let value = expect_success(result);
    assert_eq!(value["path"], expected_display);
    assert_tool_result_paths_conform(store, "read_file", &value);
}

async fn run_read_input_matrix(
    store: Arc<dyn SessionFileSystem>,
    expectations: &PathIdentityExpectations,
    rel: &str,
    ctx: ToolContext,
) {
    let session = SessionId::from_seed(7501);
    seed_target_file(store.as_ref(), session, rel).await;

    let expected = store.display_path(&format!("/{rel}"));
    let legacy = format!("{WORKSPACE_PREFIX}/{rel}");

    assert_read_paths(&ctx, store.as_ref(), rel, &expected).await;
    assert_read_paths(&ctx, store.as_ref(), &legacy, &expected).await;
    assert_read_paths(&ctx, store.as_ref(), &expected, &expected).await;

    let err = ReadFileTool
        .execute_with_context(json!({ "path": "/definitely/missing/file.txt" }), &ctx)
        .await;
    match err {
        ToolExecutionResult::ToolError(message) => {
            assert_no_forbidden_prefixes(&json!(message), expectations, "read_file error");
            if expectations.expected_root != WORKSPACE_PREFIX {
                assert!(
                    !message.contains(WORKSPACE_PREFIX),
                    "error must not leak `/workspace`: {message}"
                );
            }
        }
        other => panic!("expected tool error, got {other:?}"),
    }
}

async fn run_write_list_grep_stat_delete_matrix(
    store: Arc<dyn SessionFileSystem>,
    expectations: &PathIdentityExpectations,
    ctx: ToolContext,
) {
    let root = expectations.expected_root.clone();
    let rel = "matrix/out.txt";

    let write = WriteFileTool
        .execute_with_context(
            json!({ "path": format!("{WORKSPACE_PREFIX}/{rel}"), "content": "matrix" }),
            &ctx,
        )
        .await;
    let write_value = expect_success(write);
    let expected = store.display_path(&format!("/{rel}"));
    assert_eq!(write_value["path"], expected);
    assert_tool_result_paths_conform(store.as_ref(), "write_file", &write_value);

    let list = ListDirectoryTool
        .execute_with_context(json!({}), &ctx)
        .await;
    let list_value = expect_success(list);
    assert_eq!(list_value["path"], root);
    assert_tool_result_paths_conform(store.as_ref(), "list_directory", &list_value);

    let grep = GrepFilesTool
        .execute_with_context(json!({ "pattern": "matrix" }), &ctx)
        .await;
    let grep_value = expect_success(grep);
    assert!(
        grep_value["matches"].as_array().unwrap().iter().all(|m| {
            let path = m["path"].as_str().unwrap();
            path.starts_with(&format!("{root}/")) || path == root
        }),
        "grep matches must use active root: {grep_value}"
    );
    assert_tool_result_paths_conform(store.as_ref(), "grep_files", &grep_value);

    let stat = StatFileTool
        .execute_with_context(json!({ "path": rel }), &ctx)
        .await;
    let stat_value = expect_success(stat);
    assert_eq!(stat_value["path"], expected);
    assert_tool_result_paths_conform(store.as_ref(), "stat_file", &stat_value);

    let hash = read_hash(&ctx, &expected).await;
    let edit = EditFileTool
        .execute_with_context(
            json!({
                "path": format!("{WORKSPACE_PREFIX}/{rel}"),
                "edits": [{"old_text": "matrix", "new_text": "edited"}],
                "expected_hash": hash
            }),
            &ctx,
        )
        .await;
    let edit_value = expect_success(edit);
    assert_eq!(edit_value["path"], expected);
    assert_tool_result_paths_conform(store.as_ref(), "edit_file", &edit_value);

    let delete = DeleteFileTool
        .execute_with_context(json!({ "path": rel }), &ctx)
        .await;
    let delete_value = expect_success(delete);
    assert_eq!(delete_value["path"], expected);
    assert_tool_result_paths_conform(store.as_ref(), "delete_file", &delete_value);
}

async fn assert_system_prompt_for_store(store: Arc<dyn SessionFileSystem>) {
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let cap = FileSystemCapability;
    let ctx = SystemPromptContext {
        session_id: SessionId::from_seed(7503),
        locale: None,
        file_store: Some(store),
        model: None,
    };
    let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
    assert_system_prompt(&prompt, &expectations);
    assert_no_forbidden_prefixes(&json!(prompt), &expectations, "system prompt");
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
        id: "call_identity".to_string(),
        name: "test_output".to_string(),
        arguments: json!({}),
    }
}

async fn assert_persistence_identity(store: Arc<dyn SessionFileSystem>, wrap_mount: bool) {
    let session = SessionId::from_seed(7504);
    let ctx = if wrap_mount {
        production_context(session, store.clone())
    } else {
        production_context_direct(session, store.clone())
    };
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let root = store.display_root();
    let stdout_path = format!("{root}/outputs/call_identity.stdout");

    let mut persist_result = ToolResult {
        tool_call_id: "call_identity".to_string(),
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
            &mut persist_result,
            &ctx,
        )
        .await;
    let persist_value = persist_result.result.as_ref().unwrap();
    assert_model_visible_value(persist_value, store.as_ref(), "persist_output");
    assert_eq!(persist_value["full_output"], json!(stdout_path));

    let rows: Vec<_> = (0..2000)
        .map(|i| json!({"id": i, "name": format!("row-{i}")}))
        .collect();
    let mut distill_result = ToolResult {
        tool_call_id: "call_identity".to_string(),
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
            &mut distill_result,
            &ctx,
        )
        .await;
    let distill_value = distill_result.result.as_ref().unwrap();
    assert_model_visible_value(distill_value, store.as_ref(), "distill_output");
    assert_no_forbidden_prefixes(distill_value, &expectations, "distill_output");
}

async fn assert_file_tool_schemas(store: Arc<dyn SessionFileSystem>) {
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let ctx = SystemPromptContext {
        session_id: SessionId::from_seed(7507),
        locale: None,
        file_store: Some(store),
        model: None,
    };
    let registry = runtime_capability_registry();
    let collected = collect_capabilities(
        &[SESSION_FILE_SYSTEM_CAPABILITY_ID.to_string()],
        &registry,
        &ctx,
    )
    .await;
    let mut tools = collected.tool_definitions;
    for hook in &collected.tool_definition_hooks {
        tools = hook.transform(tools);
    }
    for def in &tools {
        assert_no_forbidden_prefixes(def.parameters(), &expectations, def.name());
    }
}

#[tokio::test]
async fn filesystem_root_rewrite_precedes_deferred_schema_capture() {
    let root = TempDir::new().unwrap();
    let store: Arc<dyn SessionFileSystem> = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let ctx = SystemPromptContext {
        session_id: SessionId::from_seed(7508),
        locale: None,
        file_store: Some(store),
        model: None,
    };

    let filesystem_hooks =
        FileSystemCapability.tool_definition_hooks_with_context(&ctx, &json!({}));
    assert_eq!(filesystem_hooks.len(), 1);
    let read_file = ToolDefinition::Builtin(BuiltinTool {
        name: "read_file".to_string(),
        display_name: None,
        description: "Read file".to_string(),
        parameters: ReadFileTool.parameters_schema(),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::Automatic,
        hints: ToolHints::default(),
        full_parameters: None,
    });

    let eager = filesystem_hooks[0].transform(vec![read_file.clone()]);
    assert_no_forbidden_prefixes(
        eager[0].full_parameters(),
        &expectations,
        "eager read_file schema",
    );

    let tool_search = ToolSearchCapability::with_threshold(1);
    let tool_search_hooks =
        tool_search.tool_definition_hooks_with_context(&ctx, &json!({ "threshold": 1 }));
    let deferred = tool_search_hooks[0].transform(filesystem_hooks[0].transform(vec![
        read_file,
        ToolDefinition::Builtin(BuiltinTool {
            name: "other_tool".to_string(),
            display_name: None,
            description: "Other".to_string(),
            parameters: json!({ "type": "object" }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Automatic,
            hints: ToolHints::default(),
            full_parameters: None,
        }),
    ]));
    let read = deferred
        .iter()
        .find(|tool| tool.name() == "read_file")
        .expect("read_file present");
    assert_no_forbidden_prefixes(
        read.full_parameters(),
        &expectations,
        "deferred read_file schema",
    );
}

async fn run_backend_suite(store: Arc<dyn SessionFileSystem>, wrap_mount: bool) {
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let session_read = SessionId::from_seed(7501);
    let session_write = SessionId::from_seed(7502);
    let ctx_for = |session| {
        if wrap_mount {
            production_context(session, store.clone())
        } else {
            production_context_direct(session, store.clone())
        }
    };

    run_read_input_matrix(
        store.clone(),
        &expectations,
        "crates/server",
        ctx_for(session_read),
    )
    .await;
    run_write_list_grep_stat_delete_matrix(store.clone(), &expectations, ctx_for(session_write))
        .await;
    assert_system_prompt_for_store(store.clone()).await;
    assert_persistence_identity(store.clone(), wrap_mount).await;
    assert_file_tool_schemas(store).await;
}

#[tokio::test]
async fn in_memory_vfs_path_identity_conformance() {
    let store: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    assert_eq!(store.display_root(), WORKSPACE_PREFIX);
    run_backend_suite(store, true).await;
}

#[tokio::test]
async fn real_disk_path_identity_conformance() {
    let root = TempDir::new().unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let backend: Arc<dyn SessionFileSystem> =
        Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    assert_eq!(backend.display_root(), canonical_root.display().to_string());
    assert_eq!(
        backend.display_path("/src/lib.rs"),
        canonical_root.join("src/lib.rs").display().to_string()
    );

    let mounted = MountFs::wrap(backend);
    assert_eq!(mounted.display_root(), WORKSPACE_PREFIX);
    run_backend_suite(mounted, false).await;
}

#[tokio::test]
async fn multi_root_secondary_mount_path_identity() {
    let session = SessionId::from_seed(7505);
    let primary = TempDir::new().unwrap();
    let secondary = TempDir::new().unwrap();
    let roots = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), secondary.path().to_path_buf())],
    )
    .unwrap();
    let store = multi_root_file_system(&roots).unwrap();
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let ctx = production_context_direct(session, Arc::new(store.clone()));

    let secondary_path = "/workspace/roots/backend/run.log";
    WriteFileTool
        .execute_with_context(json!({ "path": secondary_path, "content": "log" }), &ctx)
        .await;

    let read = ReadFileTool
        .execute_with_context(json!({ "path": secondary_path }), &ctx)
        .await;
    let read_value = expect_success(read);
    assert_eq!(read_value["path"], secondary_path);
    assert_tool_result_paths_conform(store.as_ref(), "read_file", &read_value);
    assert_no_forbidden_prefixes(&read_value, &expectations, "secondary read");
}

#[tokio::test]
#[should_panic(expected = "forbidden path prefix `/workspace`")]
async fn recursive_scan_catches_unlisted_path_field() {
    let expectations = PathIdentityExpectations::host_backed("/repo");
    let value = json!({
        "future_surface": {
            "nested_pointer": "/workspace/secret/leak.txt"
        }
    });
    let mut paths = Vec::new();
    collect_absolute_paths(&value, "", &mut paths);
    assert_eq!(paths.len(), 1);
    assert_no_forbidden_prefixes(&value, &expectations, "future field");
}

#[tokio::test]
async fn collected_capability_prompt_uses_store_root() {
    let root = TempDir::new().unwrap();
    let store: Arc<dyn SessionFileSystem> = Arc::new(RealDiskFileStore::new(root.path()).unwrap());
    let expectations = PathIdentityExpectations::for_store(store.as_ref());
    let ctx = SystemPromptContext {
        session_id: SessionId::from_seed(7506),
        locale: None,
        file_store: Some(store),
        model: None,
    };
    let registry = runtime_capability_registry();
    let collected = collect_capabilities(
        &[SESSION_FILE_SYSTEM_CAPABILITY_ID.to_string()],
        &registry,
        &ctx,
    )
    .await;
    let prompt = collected.system_prompt_prefix().expect("system prompt");
    assert_system_prompt(&prompt, &expectations);
    assert_no_forbidden_prefixes(&json!(prompt), &expectations, "collected prompt");

    let mut tools = collected.tool_definitions;
    for hook in &collected.tool_definition_hooks {
        tools = hook.transform(tools);
    }
    for def in &tools {
        let name = def.name();
        if name.starts_with("read_")
            || name.starts_with("write_")
            || name.starts_with("edit_")
            || name.starts_with("list_")
            || name.starts_with("grep_")
            || name.starts_with("delete_")
            || name.starts_with("stat_")
        {
            assert_no_forbidden_prefixes(def.parameters(), &expectations, name);
        }
    }
}
