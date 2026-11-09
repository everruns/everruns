use super::*;
use crate::host::LocalCodeHost;
use everruns_contracts::typed_id::SessionId;
use std::sync::Arc;

#[test]
fn eager_repo_map_schema_stays_compact_and_bounded() {
    let schema = compact_scan_schema();
    let properties = schema["properties"].as_object().expect("properties");
    let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(
        names,
        ["language", "limit", "max_file_bytes", "path", "query"]
    );
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["properties"]["limit"]["maximum"], json!(MAX_LIMIT));
    assert_eq!(
        schema["properties"]["max_file_bytes"]["maximum"],
        json!(MAX_FILE_BYTES)
    );
    let bytes = serde_json::to_vec(&schema).expect("serialize schema").len();
    assert_eq!(bytes, 261, "eager repo_map schema cost changed");
}

#[test]
fn deferred_repo_symbols_keeps_its_descriptive_reveal_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = host(dir.path());
    let map_schema = RepoMapTool {
        host: workspace.clone(),
    }
    .parameters_schema();
    let symbols_schema = RepoSymbolsTool { host: workspace }.parameters_schema();

    assert!(
        symbols_schema["properties"]["query"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("ranked by relevance"))
    );
    assert!(
        serde_json::to_vec(&symbols_schema)
            .expect("serialize symbols schema")
            .len()
            > serde_json::to_vec(&map_schema)
                .expect("serialize map schema")
                .len()
    );
}

#[test]
fn eager_repo_map_description_explains_optional_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = RepoMapTool {
        host: host(dir.path()),
    };
    let description = tool.description();

    assert!(description.contains("Prefer a queried map narrowed"));
    assert!(description.contains("truncates alphabetically"));
    assert!(description.contains("defaults to 50"));
}

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
fn collects_rust_symbols_with_parent_context() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        &dir.path().join("src/lib.rs"),
        r#"
pub struct Widget {
id: String,
}

impl Widget {
pub fn new(id: String) -> Self {
    Self { id }
}

fn id(&self) -> &str {
    &self.id
}
}

trait Named {
fn name(&self) -> &str;
}
"#,
    );

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.scanned_files, 1);
    assert!(report.symbols.iter().any(|symbol| symbol.language == "rust"
        && symbol.kind == "struct"
        && symbol.name == "Widget"));
    let new_fn = report
        .symbols
        .iter()
        .find(|symbol| symbol.kind == "function" && symbol.name == "new")
        .expect("new function");
    assert_eq!(new_fn.parent.as_deref(), Some("impl Widget"));
    assert_eq!(new_fn.visibility.as_deref(), Some("pub"));
    assert!(
        report
            .symbols
            .iter()
            .any(|symbol| symbol.kind == "function_signature" && symbol.name == "name")
    );
}

#[test]
fn ranks_exact_name_match_above_incidental_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Files are scanned in path order, so `a_helpers.rs` comes before
    // `b_widget.rs`. Without relevance ranking the incidental signature
    // match would be returned first.
    write(
        &dir.path().join("a_helpers.rs"),
        "pub fn render_widget_list() {}\n",
    );
    write(&dir.path().join("b_widget.rs"), "pub fn widget() {}\n");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: Some("widget".to_string()),
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.symbols.len(), 2);
    assert_eq!(
        report.symbols[0].name, "widget",
        "exact name match should rank first, got {:?}",
        report.symbols
    );
}

#[test]
fn ranks_symbols_matching_more_query_terms_higher() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("a.rs"), "pub fn render() {}\n");
    write(&dir.path().join("b.rs"), "pub fn render_widget() {}\n");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: Some("render widget".to_string()),
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(
        report.symbols[0].name, "render_widget",
        "symbol matching both terms should rank first, got {:?}",
        report.symbols
    );
    assert!(
        report.symbols.iter().any(|symbol| symbol.name == "render"),
        "single-term match should still be returned"
    );
}

#[test]
fn tokenizes_query_lowercased_without_duplicates() {
    assert_eq!(
        tokenize_query(Some("Render RENDER  widget")),
        vec!["render".to_string(), "widget".to_string()]
    );
    assert!(tokenize_query(None).is_empty());
    assert!(tokenize_query(Some("   ")).is_empty());
}

#[test]
fn filters_by_query_and_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "pub fn keep_me() {}\n");
    write(&dir.path().join("tests/demo.rs"), "fn skip_me() {}\n");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: Some("src".to_string()),
            query: Some("keep".to_string()),
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.symbols.len(), 1);
    assert_eq!(report.symbols[0].path, "src/lib.rs");
    assert_eq!(report.symbols[0].name, "keep_me");
}

#[test]
fn collects_symbols_across_supported_languages() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        &dir.path().join("app.py"),
        "class Service:\n    def run(self):\n        pass\n",
    );
    write(
        &dir.path().join("widget.ts"),
        "export interface Widget {}\nexport function render() {}\n",
    );
    write(&dir.path().join("style.css"), ".button { color: red; }\n");
    write(
        &dir.path().join("index.html"),
        "<main><section></section></main>\n",
    );
    write(&dir.path().join("script.sh"), "deploy() { echo ok; }\n");
    write(
        &dir.path().join("schema.sql"),
        "CREATE TABLE users (id int);\n",
    );

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: None,
            limit: 100,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    for language in ["python", "typescript", "css", "html", "bash", "sql"] {
        assert!(
            report.languages.iter().any(|seen| seen == language),
            "expected {language} in {:?}",
            report.languages
        );
    }
    assert!(
        report
            .symbols
            .iter()
            .any(|symbol| symbol.language == "python"
                && symbol.kind == "class"
                && symbol.name == "Service")
    );
    assert!(
        report
            .symbols
            .iter()
            .any(|symbol| symbol.language == "typescript"
                && symbol.kind == "function"
                && symbol.name == "render")
    );
    assert!(
        report
            .symbols
            .iter()
            .any(|symbol| symbol.language == "css" && symbol.name.contains(".button"))
    );
    assert!(report.symbols.iter().any(|symbol| symbol.language == "bash"
        && symbol.kind == "function"
        && symbol.name == "deploy"));
}

#[test]
fn collects_symbols_from_newly_added_languages() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        &dir.path().join("Service.java"),
        "class Service {\n  void run() {}\n}\n",
    );
    write(
        &dir.path().join("math.c"),
        "int add(int a, int b) {\n  return a + b;\n}\n",
    );
    write(
        &dir.path().join("widget.cpp"),
        "class Widget {\npublic:\n  void render() {}\n};\n",
    );
    write(
        &dir.path().join("app.php"),
        "<?php\nclass App {\n  function boot() {}\n}\n",
    );
    write(
        &dir.path().join("service.rb"),
        "class Service\n  def run\n  end\nend\n",
    );
    write(
        &dir.path().join("Main.kt"),
        "class Greeter {\n  fun greet() {}\n}\n",
    );
    write(
        &dir.path().join("Main.scala"),
        "object Main {\n  def run(): Unit = {}\n}\n",
    );

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: None,
            limit: 200,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    let has = |language: &str, kind: &str, name: &str| {
        report
            .symbols
            .iter()
            .any(|symbol| symbol.language == language && symbol.kind == kind && symbol.name == name)
    };

    assert!(
        has("java", "class", "Service"),
        "java class: {:?}",
        report.symbols
    );
    assert!(
        has("java", "method", "run"),
        "java method: {:?}",
        report.symbols
    );
    assert!(
        has("c", "function", "add"),
        "c function: {:?}",
        report.symbols
    );
    assert!(
        has("cpp", "class", "Widget"),
        "cpp class: {:?}",
        report.symbols
    );
    assert!(
        has("php", "class", "App"),
        "php class: {:?}",
        report.symbols
    );
    assert!(
        has("ruby", "class", "Service"),
        "ruby class: {:?}",
        report.symbols
    );
    assert!(
        has("ruby", "method", "run"),
        "ruby method: {:?}",
        report.symbols
    );
    assert!(
        has("kotlin", "class", "Greeter"),
        "kotlin class: {:?}",
        report.symbols
    );
    assert!(
        has("kotlin", "function", "greet"),
        "kotlin fn: {:?}",
        report.symbols
    );
    assert!(
        has("scala", "object", "Main"),
        "scala object: {:?}",
        report.symbols
    );
}

#[test]
fn filters_by_language() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "pub fn rust_fn() {}\n");
    write(&dir.path().join("app.py"), "def py_fn():\n    pass\n");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: Some("python".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert_eq!(report.languages, vec!["python"]);
    assert!(report.symbols.iter().any(|symbol| symbol.name == "py_fn"));
    assert!(!report.symbols.iter().any(|symbol| symbol.name == "rust_fn"));
}

#[tokio::test]
async fn repo_symbols_tool_executes_multilanguage_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("app.py"), "def py_fn():\n    pass\n");

    let capability = RepoMapCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "repo_symbols")
        .expect("repo_symbols tool");
    let result = run_tool(
        tool.as_ref(),
        json!({
            "language": "python",
            "query": "py_fn",
            "limit": 10
        }),
    )
    .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };

    assert_eq!(value["languages"], json!(["python"]));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["symbols"][0]["language"], json!("python"));
    assert_eq!(value["symbols"][0]["name"], json!("py_fn"));
}

#[tokio::test]
async fn unqueried_repo_map_uses_compact_default_and_explains_truncation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = (0..80)
        .map(|index| format!("pub fn symbol_{index}() {{}}\n"))
        .collect::<String>();
    write(&dir.path().join("src/lib.rs"), &source);

    let capability = RepoMapCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "repo_map")
        .expect("repo_map tool");
    let result = run_tool(tool.as_ref(), json!({})).await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };

    assert_eq!(value["count"], json!(50));
    assert_eq!(value["truncated"], json!(true));
    assert_eq!(value["truncation"]["reason"], json!("symbol_limit"));
    assert!(
        value["truncation"]["suggestion"]
            .as_str()
            .is_some_and(
                |suggestion| suggestion.contains("Do not repeat") && suggestion.contains("query")
            )
    );
}

#[test]
fn queried_scans_keep_the_larger_default_limit() {
    assert_eq!(default_limit(Some("needle")), 200);
    assert_eq!(default_limit(None), 50);
}

#[cfg(unix)]
#[test]
fn skips_symlinked_directories_during_recursive_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    write(&dir.path().join("src/lib.rs"), "pub fn inside_fn() {}\n");
    write(
        &outside.path().join("outside.rs"),
        "pub fn outside_fn() {}\n",
    );
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked_outside"))
        .expect("symlink outside dir");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    assert!(
        report
            .symbols
            .iter()
            .any(|symbol| symbol.name == "inside_fn")
    );
    assert!(
        !report
            .symbols
            .iter()
            .any(|symbol| symbol.name == "outside_fn")
    );
}

#[test]
fn rejects_paths_outside_workspace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: Some("../outside".to_string()),
            query: None,
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect_err("outside path should fail");

    assert!(err.to_string().contains("inside the workspace"));
}

#[test]
fn accepts_host_workspace_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/lib.rs"), "pub fn inside_fn() {}\n");

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: Some(dir.path().display().to_string()),
            query: None,
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("host root should scan");

    assert!(report.symbols.iter().any(|s| s.name == "inside_fn"));
}

#[tokio::test]
async fn repo_map_tool_accepts_repository_relative_path_scope() {
    // Repository-relative paths resolve from the active repository root.
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("pkg/inside.rs"), "pub fn pkg_fn() {}\n");
    write(
        &dir.path().join("other/elsewhere.rs"),
        "pub fn other_fn() {}\n",
    );

    let capability = RepoMapCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "repo_map")
        .expect("repo_map tool");

    // Repository root scans the whole workspace.
    let ToolExecutionResult::Success(root) =
        run_tool(tool.as_ref(), json!({ "path": ".", "language": "rust" })).await
    else {
        panic!("expected success scanning repository root");
    };
    assert_eq!(root["count"], json!(2));

    // Repository-relative subpath scopes to just that directory.
    let ToolExecutionResult::Success(sub) =
        run_tool(tool.as_ref(), json!({ "path": "pkg", "language": "rust" })).await
    else {
        panic!("expected success scanning pkg");
    };
    assert_eq!(sub["count"], json!(1));
    assert_eq!(sub["files"][0]["symbols"][0]["name"], json!("pkg_fn"));
}

#[tokio::test]
async fn repo_map_scans_nested_repository_in_linked_worktree_layout() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A linked Git worktree identifies its repository with a .git file,
    // not a .git directory. The scanner must treat that as metadata while
    // still accepting a repository-relative nested path.
    write(
        &dir.path().join(".git"),
        "gitdir: /tmp/repo/.git/worktrees/fixture\n",
    );
    write(
        &dir.path().join("REPOS/nested/src/lib.rs"),
        "pub fn nested_owner() {}\n",
    );
    write(
        &dir.path().join("outside.rs"),
        "pub fn outside_scope() {}\n",
    );

    let capability = RepoMapCapability::new(host(dir.path()));
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "repo_map")
        .expect("repo_map tool");

    let path = "REPOS/nested";
    let ToolExecutionResult::Success(result) =
        run_tool(tool.as_ref(), json!({ "path": path, "language": "rust" })).await
    else {
        panic!("expected success scanning {path}");
    };
    assert_eq!(result["count"], json!(1), "scope {path}");
    assert_eq!(
        result["files"][0]["symbols"][0]["name"],
        json!("nested_owner"),
        "scope {path}"
    );
}

#[test]
fn accepts_host_absolute_subpath() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("pkg/inside.rs"), "pub fn pkg_fn() {}\n");
    write(
        &dir.path().join("other/elsewhere.rs"),
        "pub fn other_fn() {}\n",
    );

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: Some(dir.path().join("pkg").display().to_string()),
            query: None,
            language: Some("rust".to_string()),
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("host subpath should scan");

    assert!(report.symbols.iter().any(|s| s.name == "pkg_fn"));
    assert!(
        !report.symbols.iter().any(|s| s.name == "other_fn"),
        "scope should be limited to the requested host subpath"
    );
}

// Hidden files and directories (`.cache`, `.git`, `.env`, …) are noise for a
// symbol map and crowded out real source in the swebench eval run, where a
// vendored astropy checkout under `.cache/work/` dominated the map. The
// recursive walk skips every `.`-prefixed entry.
#[test]
fn recursive_scan_skips_hidden_files_and_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("src/real.rs"), "pub fn real_fn() {}\n");
    write(
        &dir.path().join(".cache/work/vendored.rs"),
        "pub fn vendored_fn() {}\n",
    );
    write(
        &dir.path().join(".git/hooks/hook.rs"),
        "pub fn hook_fn() {}\n",
    );
    write(
        &dir.path().join(".hidden.rs"),
        "pub fn hidden_file_fn() {}\n",
    );

    let report = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: None,
            query: None,
            language: Some("rust".to_string()),
            limit: 50,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect("scan");

    let names: Vec<&str> = report.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"real_fn"), "real source should be mapped");
    for hidden in ["vendored_fn", "hook_fn", "hidden_file_fn"] {
        assert!(
            !names.contains(&hidden),
            "hidden entry {hidden} should be skipped"
        );
    }
}

#[test]
fn absolute_host_path_does_not_bypass_containment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside");
    let err = collect_repo_symbols(
        dir.path(),
        SymbolScanOptions {
            path: Some(outside.path().display().to_string()),
            query: None,
            language: None,
            limit: 20,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        },
    )
    .expect_err("an absolute path outside the workspace should fail");

    assert!(err.to_string().contains("inside the workspace"));
}
