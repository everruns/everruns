use super::*;
use crate::host::LocalCodeHost;
use everruns_contracts::typed_id::SessionId;
use std::sync::Arc;

fn host(path: &Path) -> Arc<dyn CodeHost> {
    Arc::new(LocalCodeHost::new(path))
}

async fn run_tool(tool: &dyn Tool, arguments: Value) -> ToolExecutionResult {
    tool.execute_with_context(arguments, &ToolContext::new(SessionId::new()))
        .await
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

#[test]
fn finds_rust_function_pattern() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        &dir.path().join("src/lib.rs"),
        "fn keep() {}\nfn skip(arg: usize) { let _ = arg; }\n",
    );

    let report = ast_grep(
        dir.path(),
        AstGrepOptions {
            pattern: "fn $NAME() {}".to_string(),
            path: None,
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.matches.len(), 1);
    let mat = &report.matches[0];
    assert_eq!(mat.path, "src/lib.rs");
    assert_eq!(mat.language, "rust");
    assert_eq!(mat.kind, "function_item");
    assert!(
        mat.captures
            .iter()
            .any(|capture| capture.name == "NAME" && capture.text == "keep")
    );
}

#[test]
fn filters_by_path_and_language() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("app.py"), "print(value)\n");
    write(&dir.path().join("web/app.ts"), "console.log(value);\n");

    let report = ast_grep(
        dir.path(),
        AstGrepOptions {
            pattern: "console.log($A)".to_string(),
            path: Some("web".to_string()),
            language: Some("typescript".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.languages, vec!["typescript"]);
    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].path, "web/app.ts");
    assert!(
        report.matches[0]
            .captures
            .iter()
            .any(|capture| capture.name == "A" && capture.text == "value")
    );
}

#[tokio::test]
async fn ast_grep_tool_executes_structural_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "fn target() {}\n");

    let capability = AstGrepCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "ast_grep")
        .expect("ast_grep tool");
    let result = run_tool(
        tool.as_ref(),
        json!({
            "pattern": "fn $NAME() {}",
            "language": "rust",
            "limit": 10
        }),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };

    assert_eq!(value["count"], json!(1));
    assert_eq!(value["matches"][0]["path"], json!("src/lib.rs"));
    assert_eq!(value["matches"][0]["captures"][0]["name"], json!("NAME"));
    assert_eq!(value["matches"][0]["captures"][0]["text"], json!("target"));
}

#[tokio::test]
async fn ast_grep_tool_accepts_repository_relative_path_scope() {
    // Like repo_map, ast_grep resolves repository-relative paths through
    // resolve_host_scope.
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "fn target() {}\n");
    write(&dir.path().join("other/lib.rs"), "fn other() {}\n");

    let capability = AstGrepCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "ast_grep")
        .expect("ast_grep tool");
    let result = run_tool(
        tool.as_ref(),
        json!({
            "pattern": "fn $NAME() {}",
            "language": "rust",
            "path": "src",
            "limit": 10
        }),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };

    assert_eq!(value["count"], json!(1));
    assert_eq!(value["matches"][0]["path"], json!("src/lib.rs"));
}

#[test]
fn accepts_host_absolute_subpath() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "fn target() {}\n");

    let report = ast_grep(
        dir.path(),
        AstGrepOptions {
            pattern: "fn $NAME() {}".to_string(),
            path: Some(dir.path().join("src").display().to_string()),
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("host subpath should scan");

    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].path, "src/lib.rs");
}

#[test]
fn rejects_paths_outside_workspace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = ast_grep(
        dir.path(),
        AstGrepOptions {
            pattern: "fn $NAME() {}".to_string(),
            path: Some("../outside".to_string()),
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect_err("outside path should fail");

    assert!(err.to_string().contains("inside the workspace"));
}

#[test]
fn rejects_oversized_numeric_limits_before_casting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = ast_grep_from_arguments(
        dir.path(),
        &json!({
            "pattern": "fn $NAME() {}",
            "limit": u64::MAX,
        }),
    )
    .expect_err("oversized limit should fail");

    assert!(err.to_string().contains("between 1 and"));
}

#[cfg(unix)]
#[test]
fn skips_symlinked_directories_during_recursive_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    write(&dir.path().join("src/lib.rs"), "fn inside_fn() {}\n");
    write(&outside.path().join("outside.rs"), "fn outside_fn() {}\n");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked_outside"))
        .expect("symlink outside dir");

    let report = ast_grep(
        dir.path(),
        AstGrepOptions {
            pattern: "fn $NAME() {}".to_string(),
            path: None,
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert!(
        report
            .matches
            .iter()
            .any(|mat| mat.text == "fn inside_fn() {}")
    );
    assert!(
        !report
            .matches
            .iter()
            .any(|mat| mat.text == "fn outside_fn() {}")
    );
}

#[test]
fn ast_edit_dry_run_previews_without_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.ts");
    write(&path, "console.log(value);\n");

    let report = ast_edit(
        dir.path(),
        AstEditOptions {
            pattern: "console.log($$$ARGS)".to_string(),
            replacement: "logger.info($$$ARGS)".to_string(),
            path: None,
            language: Some("typescript".to_string()),
            dry_run: true,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("preview rewrite");

    assert_eq!(report.count, 1);
    assert!(report.dry_run);
    assert!(!report.applied);
    assert!(report.diff.contains("console.log"));
    assert!(report.diff.contains("logger.info"));
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "console.log(value);\n"
    );
}

#[test]
fn ast_edit_applies_rewrite_when_dry_run_false() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.ts");
    write(&path, "console.log(value);\n");

    let report = ast_edit(
        dir.path(),
        AstEditOptions {
            pattern: "console.log($$$ARGS)".to_string(),
            replacement: "logger.info($$$ARGS)".to_string(),
            path: None,
            language: Some("typescript".to_string()),
            dry_run: false,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("apply rewrite");

    assert_eq!(report.count, 1);
    assert!(!report.dry_run);
    assert!(report.applied);
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "logger.info(value);\n"
    );
}

#[tokio::test]
async fn ast_edit_tool_defaults_to_preview_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("app.ts"), "console.log(value);\n");

    let capability = AstEditCapability::new(host(dir.path()));
    let tool = capability
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "ast_edit")
        .expect("ast_edit tool");
    let result = run_tool(
        tool.as_ref(),
        json!({
            "pattern": "console.log($$$ARGS)",
            "replacement": "logger.info($$$ARGS)",
            "language": "typescript"
        }),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };

    assert_eq!(value["dry_run"], json!(true));
    assert_eq!(value["applied"], json!(false));
    assert_eq!(value["count"], json!(1));
    assert!(value["diff"].as_str().unwrap_or("").contains("logger.info"));
}

#[test]
fn capability_exposes_ast_grep_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capability = AstGrepCapability::new(host(dir.path()));
    let names: Vec<String> = capability
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["ast_grep"]);
}

#[test]
fn capability_exposes_ast_edit_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capability = AstEditCapability::new(host(dir.path()));
    let names: Vec<String> = capability
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["ast_edit"]);
}
