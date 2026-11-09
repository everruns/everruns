use super::manager::default_server_specs;
use super::*;
use crate::host::LocalCodeHost;
use everruns_contracts::typed_id::SessionId;
use manager::path_to_uri;
use std::fs;
use std::path::PathBuf;
use tokio::io::duplex;

fn host(path: &Path) -> Arc<dyn CodeHost> {
    Arc::new(LocalCodeHost::new(path))
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn range(start_line: u64, start_char: u64, end_line: u64, end_char: u64) -> Value {
    json!({
        "start": { "line": start_line, "character": start_char },
        "end": { "line": end_line, "character": end_char },
    })
}

/// In-process fake language server covering every request the tools send.
/// Speaks utf-8 positions so ASCII test fixtures assert 1:1. The first
/// `references` request returns an empty result to exercise the warmup
/// retry path — tools must transparently ask again and get the real set.
async fn fake_server(transport: tokio::io::DuplexStream, root: PathBuf) {
    let (read_half, mut write_half) = tokio::io::split(transport);
    let mut reader = tokio::io::BufReader::new(read_half);
    let uri = |name: &str| path_to_uri(&root.join(name));
    let mut references_requests = 0u32;
    let diagnostic = json!({
        "range": range(0, 3, 0, 6),
        "severity": 1,
        "message": "mock error",
        "source": "fake",
        "code": "E999",
    });
    while let Ok(Some(message)) = client::read_message(&mut reader).await {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "capabilities": {
                    "positionEncoding": "utf-8",
                    "diagnosticProvider": {},
                    "renameProvider": true,
                }
            }),
            "textDocument/didOpen" => {
                let doc_uri = message["params"]["textDocument"]["uri"].clone();
                let notification = json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": doc_uri, "diagnostics": [diagnostic.clone()] },
                });
                client::write_message(&mut write_half, &notification)
                    .await
                    .expect("publish");
                continue;
            }
            "initialized" | "textDocument/didChange" => continue,
            "textDocument/diagnostic" => {
                json!({ "kind": "full", "items": [diagnostic.clone()] })
            }
            "textDocument/definition" => json!([{
                "uri": uri("target.rs"),
                "range": range(0, 3, 0, 12),
            }]),
            "textDocument/references" => {
                references_requests += 1;
                if references_requests == 1 {
                    // Warmup artifact: empty-but-successful first answer.
                    json!([])
                } else {
                    json!([
                        { "uri": uri("main.rs"), "range": range(0, 3, 0, 6) },
                        { "uri": uri("main.rs"), "range": range(1, 14, 1, 17) },
                    ])
                }
            }
            "textDocument/hover" => json!({
                "contents": { "kind": "markdown", "value": "```rust\nfn old()\n```" }
            }),
            "textDocument/rename" => json!({
                "changes": {
                    uri("main.rs"): [
                        { "range": range(0, 3, 0, 6), "newText": "renamed" },
                        { "range": range(1, 14, 1, 17), "newText": "renamed" },
                    ],
                    uri("other.rs"): [
                        { "range": range(0, 20, 0, 23), "newText": "renamed" },
                    ],
                }
            }),
            "textDocument/documentSymbol" => json!([{
                "name": "S",
                "kind": 23,
                "range": range(0, 0, 2, 1),
                "selectionRange": range(0, 7, 0, 8),
                "children": [{
                    "name": "m",
                    "kind": 6,
                    "range": range(1, 4, 1, 20),
                    "selectionRange": range(1, 7, 1, 8),
                }],
            }]),
            "workspace/symbol" => json!([{
                "name": "target_fn",
                "kind": 12,
                "location": { "uri": uri("target.rs"), "range": range(0, 3, 0, 12) },
                "containerName": "target",
            }]),
            "textDocument/codeAction" => json!([
                {
                    "title": "Insert comment",
                    "kind": "quickfix",
                    "isPreferred": true,
                    "edit": {
                        "changes": {
                            uri("main.rs"): [
                                { "range": range(0, 0, 0, 0), "newText": "// fixed\n" }
                            ]
                        }
                    }
                },
                { "title": "Run command", "kind": "refactor", "command": { "command": "x" } }
            ]),
            other => {
                if !id.is_null() {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("fake: {other}") },
                    });
                    client::write_message(&mut write_half, &response)
                        .await
                        .expect("error response");
                }
                continue;
            }
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        client::write_message(&mut write_half, &response)
            .await
            .expect("response");
    }
}

/// The tools of one workspace, wired to the in-process fake server.
struct TestTools {
    host: Arc<dyn CodeHost>,
    workspaces: Arc<Workspaces>,
}

fn test_manager(root: &Path) -> TestTools {
    let canonical = root.canonicalize().expect("canonicalize test root");
    let host = host(&canonical);
    let scope = host.scope_key(&ToolContext::new(SessionId::new()));
    let manager = LspManager::new(
        host.clone(),
        scope.clone(),
        default_server_specs(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
    .with_transport_factory(Box::new(move |_spec| {
        let (client_transport, server_transport) = duplex(1 << 20);
        tokio::spawn(fake_server(server_transport, canonical.clone()));
        let (read_half, write_half) = tokio::io::split(client_transport);
        Ok((Box::new(read_half) as _, Box::new(write_half) as _))
    }));
    let workspaces = Arc::new(Workspaces::default());
    workspaces.0.lock().expect("workspaces lock").insert(
        scope,
        ScopeServers {
            config: Value::Null,
            manager: Arc::new(manager),
            last_used: Instant::now(),
        },
    );
    TestTools { host, workspaces }
}

fn tool(tools: &TestTools, action: LspAction) -> LspTool {
    LspTool {
        host: tools.host.clone(),
        workspaces: tools.workspaces.clone(),
        config: Value::Null,
        action,
    }
}

fn seed_workspace(root: &Path) {
    write(
        &root.join("main.rs"),
        "fn old() {}\nfn caller() { old(); }\n",
    );
    write(&root.join("other.rs"), "fn other() { crate::old(); }\n");
    write(&root.join("target.rs"), "fn target_fn() {}\n");
}

async fn run_tool(tools: &TestTools, action: LspAction, arguments: Value) -> Value {
    let result = tool(tools, action)
        .execute_with_context(arguments, &ToolContext::new(SessionId::new()))
        .await;
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn definition_returns_workspace_relative_location_with_preview() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Definition,
        json!({ "path": "main.rs", "line": 2, "column": 15 }),
    )
    .await;
    assert_eq!(value["count"], 1);
    let location = &value["locations"][0];
    assert_eq!(location["path"], "target.rs");
    assert_eq!(location["line"], 1);
    assert_eq!(location["column"], 4);
    assert_eq!(location["end_column"], 13);
    assert_eq!(location["preview"], "fn target_fn() {}");
    assert!(location.get("external").is_none());
}

#[tokio::test]
async fn references_lists_all_sites() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::References,
        json!({ "path": "main.rs", "line": 1, "column": 4 }),
    )
    .await;
    assert_eq!(value["count"], 2);
    assert_eq!(value["locations"][0]["path"], "main.rs");
    assert_eq!(value["locations"][1]["line"], 2);
    assert_eq!(value["truncated"], false);
}

#[tokio::test]
async fn hover_flattens_markup_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Hover,
        json!({ "path": "main.rs", "line": 1, "column": 4 }),
    )
    .await;
    assert_eq!(value["contents"], "```rust\nfn old()\n```");
}

#[tokio::test]
async fn diagnostics_merges_pull_and_publish_without_duplicates() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Diagnostics,
        json!({ "path": "main.rs" }),
    )
    .await;
    assert_eq!(
        value["count"], 1,
        "pull+publish overlap must dedup: {value}"
    );
    let diagnostic = &value["diagnostics"][0];
    assert_eq!(diagnostic["severity"], "error");
    assert_eq!(diagnostic["message"], "mock error");
    assert_eq!(diagnostic["line"], 1);
    assert_eq!(diagnostic["column"], 4);
    assert_eq!(diagnostic["code"], "E999");
    assert_eq!(diagnostic["source"], "fake");
}

#[tokio::test]
async fn rename_applies_workspace_edit_across_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Rename,
        json!({ "path": "main.rs", "line": 1, "column": 4, "new_name": "renamed" }),
    )
    .await;
    assert_eq!(value["applied"], true);
    assert_eq!(value["total_edits"], 3);
    let root = dir.path().canonicalize().expect("canonicalize");
    assert_eq!(
        fs::read_to_string(root.join("main.rs")).expect("read"),
        "fn renamed() {}\nfn caller() { renamed(); }\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("other.rs")).expect("read"),
        "fn other() { crate::renamed(); }\n"
    );
}

#[tokio::test]
async fn rename_dry_run_previews_without_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Rename,
        json!({
            "path": "main.rs", "line": 1, "column": 4,
            "new_name": "renamed", "dry_run": true
        }),
    )
    .await;
    assert_eq!(value["applied"], false);
    assert_eq!(value["files"].as_array().map(Vec::len), Some(2));
    let root = dir.path().canonicalize().expect("canonicalize");
    assert_eq!(
        fs::read_to_string(root.join("main.rs")).expect("read"),
        "fn old() {}\nfn caller() { old(); }\n",
        "dry_run must not touch the file"
    );
}

#[tokio::test]
async fn symbols_flattens_document_outline_with_containers() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(&manager, LspAction::Symbols, json!({ "path": "main.rs" })).await;
    assert_eq!(value["count"], 2);
    assert_eq!(value["symbols"][0]["name"], "S");
    assert_eq!(value["symbols"][0]["kind"], "Struct");
    assert_eq!(value["symbols"][1]["name"], "m");
    assert_eq!(value["symbols"][1]["kind"], "Method");
    assert_eq!(value["symbols"][1]["container"], "S");
}

#[tokio::test]
async fn workspace_symbol_search_uses_hint_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let value = run_tool(
        &manager,
        LspAction::Symbols,
        json!({ "query": "target", "hint_path": "main.rs" }),
    )
    .await;
    assert_eq!(value["count"], 1);
    assert_eq!(value["symbols"][0]["name"], "target_fn");
    assert_eq!(value["symbols"][0]["kind"], "Function");
    assert_eq!(value["symbols"][0]["path"], "target.rs");
    assert_eq!(value["symbols"][0]["container"], "target");
}

#[tokio::test]
async fn code_actions_lists_and_applies_by_title() {
    let dir = tempfile::tempdir().expect("tempdir");
    seed_workspace(dir.path());
    let manager = test_manager(dir.path());

    let listed = run_tool(
        &manager,
        LspAction::CodeActions,
        json!({ "path": "main.rs", "line": 1, "column": 4 }),
    )
    .await;
    assert_eq!(listed["count"], 2);
    assert_eq!(listed["actions"][0]["title"], "Insert comment");
    assert_eq!(listed["actions"][0]["has_edit"], true);
    assert_eq!(listed["actions"][1]["has_command"], true);

    let applied = run_tool(
        &manager,
        LspAction::CodeActions,
        json!({ "path": "main.rs", "line": 1, "column": 4, "apply": "Insert comment" }),
    )
    .await;
    assert_eq!(applied["applied"], "Insert comment");
    let root = dir.path().canonicalize().expect("canonicalize");
    assert!(
        fs::read_to_string(root.join("main.rs"))
            .expect("read")
            .starts_with("// fixed\n"),
        "the quickfix edit should be applied"
    );
}

#[tokio::test]
async fn unsupported_extension_yields_helpful_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("notes.zig"), "const x = 1;\n");
    let manager = test_manager(dir.path());

    let result = tool(&manager, LspAction::Hover)
        .execute_with_context(
            json!({ "path": "notes.zig", "line": 1, "column": 1 }),
            &ToolContext::new(SessionId::new()),
        )
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("expected tool error, got {result:?}");
    };
    assert!(
        message.contains("no language server configured"),
        "{message}"
    );
}

#[test]
fn resolve_specs_merges_overrides() {
    let settings = parse_settings(&json!({
        "servers": {
            "rust": { "command": "my-ra", "args": ["--custom"] },
            "python": { "enabled": false },
            "zig": { "command": "zls", "extensions": [".zig"] },
        }
    }))
    .expect("parse");
    let specs = resolve_specs(&settings).expect("resolve");
    let rust = specs.iter().find(|spec| spec.key == "rust").expect("rust");
    assert_eq!(rust.command, "my-ra");
    assert_eq!(rust.args, vec!["--custom"]);
    assert!(!specs.iter().any(|spec| spec.key == "python"));
    let zig = specs.iter().find(|spec| spec.key == "zig").expect("zig");
    assert_eq!(zig.extensions, vec!["zig"]);
}

#[test]
fn resolve_specs_rejects_new_server_without_command() {
    let settings =
        parse_settings(&json!({ "servers": { "zig": { "args": ["x"] } } })).expect("parse");
    let err = resolve_specs(&settings).expect_err("missing command");
    assert!(err.contains("requires a `command`"), "{err}");
}

#[test]
fn capability_validates_config_and_exposes_seven_tools() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capability = LspCapability::new(host(dir.path()));

    assert!(
        capability
            .validate_config(&json!({ "bogus": true }))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({ "request_timeout_ms": 30000 }))
            .is_ok()
    );

    let tools = capability.tools();
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "lsp_code_actions",
            "lsp_definition",
            "lsp_diagnostics",
            "lsp_hover",
            "lsp_references",
            "lsp_rename",
            "lsp_symbols",
        ]
    );
    for tool in &tools {
        assert_eq!(tool.parameters_schema()["type"], "object");
    }
}
