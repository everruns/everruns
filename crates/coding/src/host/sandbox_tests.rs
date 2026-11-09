use super::*;
use crate::{AstEditCapability, AstGrepCapability, RepoMapCapability};
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::tools::ToolExecutionResult;
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use tokio::io::AsyncReadExt;

/// A sandbox that is a directory on this machine: every command runs with
/// `sh`, files are read and written directly, and checkpoints are counted.
struct ShellSandbox {
    root: PathBuf,
    checkpoints: AtomicUsize,
}

#[async_trait]
impl SandboxWorkspace for ShellSandbox {
    async fn exec_with_secret_file(
        &self,
        _context: &ToolContext,
        _command: &str,
        _cwd: &str,
        _secret: &str,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        Err(ToolExecutionResult::tool_error(
            "not used by code navigation",
        ))
    }

    async fn provider_root(&self, _context: &ToolContext) -> Result<String, ToolExecutionResult> {
        Ok(self.root.to_string_lossy().into_owned())
    }

    async fn exec_raw(
        &self,
        _context: &ToolContext,
        command: &str,
        cwd: Option<&str>,
        _timeout_ms: Option<u64>,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd.map(PathBuf::from).unwrap_or_else(|| self.root.clone()))
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|err| ToolExecutionResult::tool_error(err.to_string()))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(SessionSandboxExecResponse {
            exit_code: output.status.code().unwrap_or(-1),
            total_lines: stdout.lines().count(),
            stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
            truncated: false,
            raw_output: None,
            hint: None,
        })
    }

    async fn read_file_raw(
        &self,
        _context: &ToolContext,
        path: &str,
    ) -> Result<Vec<u8>, ToolExecutionResult> {
        std::fs::read(path).map_err(|err| ToolExecutionResult::tool_error(err.to_string()))
    }

    async fn write_file_raw(
        &self,
        _context: &ToolContext,
        path: &str,
        content: &[u8],
    ) -> Result<(), ToolExecutionResult> {
        std::fs::write(path, content)
            .map_err(|err| ToolExecutionResult::tool_error(err.to_string()))
    }

    async fn checkpoint(&self, _context: &ToolContext) -> Result<(), ToolExecutionResult> {
        self.checkpoints.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    sandbox: Arc<ShellSandbox>,
    context: ToolContext,
}

/// A workspace holding two repositories, `app` and `lib`.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (path, content) in [
        ("app/.git/HEAD", "ref: refs/heads/main\n"),
        (
            "app/src/main.rs",
            "fn main() { helper(); }\nfn helper() {}\n",
        ),
        ("app/node_modules/dep/index.js", "function vendored() {}\n"),
        ("lib/.git/HEAD", "ref: refs/heads/main\n"),
        ("lib/src/lib.rs", "fn shared() {}\n"),
        ("lib/web/app.ts", "console.log(value);\n"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }
    let sandbox = Arc::new(ShellSandbox {
        root: root.clone(),
        checkpoints: AtomicUsize::new(0),
    });
    let context = ToolContext::new(SessionId::new()).with_extension(Arc::new(SandboxWorkspaceExt(
        sandbox.clone() as Arc<dyn SandboxWorkspace>,
    )));
    Fixture {
        _dir: dir,
        root,
        sandbox,
        context,
    }
}

async fn call(
    capability: &dyn Capability,
    tool: &str,
    context: &ToolContext,
    arguments: Value,
) -> ToolExecutionResult {
    let tools = capability.tools();
    let tool = tools
        .iter()
        .find(|candidate| candidate.name() == tool)
        .unwrap();
    tool.execute_with_context(arguments, context).await
}

fn success(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn ast_grep_searches_every_repository_and_skips_vendored_code() {
    let fixture = fixture();
    let capability = AstGrepCapability::new(Arc::new(SandboxCodeHost));

    let value = success(
        call(
            &capability,
            "ast_grep",
            &fixture.context,
            json!({ "pattern": "fn $NAME() {}", "language": "rust" }),
        )
        .await,
    );
    let paths: Vec<&str> = value["matches"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|found| found["path"].as_str())
        .collect();
    assert_eq!(paths, vec!["app/src/main.rs", "lib/src/lib.rs"], "{value}");
    assert_eq!(value["workspace_root"], "/workspace");

    // Vendored JavaScript is never fetched, whatever the language filter.
    let value = success(
        call(
            &capability,
            "ast_grep",
            &fixture.context,
            json!({ "pattern": "function $NAME() {}", "language": "javascript" }),
        )
        .await,
    );
    assert_eq!(value["count"], 0, "{value}");
}

#[tokio::test]
async fn paths_are_scoped_by_the_models_workspace_name() {
    let fixture = fixture();
    let capability = AstGrepCapability::new(Arc::new(SandboxCodeHost));

    for path in ["/workspace/lib", "lib", "lib/src/lib.rs"] {
        let value = success(
            call(
                &capability,
                "ast_grep",
                &fixture.context,
                json!({ "pattern": "fn $NAME() {}", "language": "rust", "path": path }),
            )
            .await,
        );
        assert_eq!(value["count"], 1, "{path}: {value}");
        assert_eq!(value["matches"][0]["path"], "lib/src/lib.rs");
    }

    for (path, expected) in [
        ("missing", "path not found"),
        ("/etc", "inside the workspace"),
        ("../outside", "inside the workspace"),
    ] {
        let result = call(
            &capability,
            "ast_grep",
            &fixture.context,
            json!({ "pattern": "fn $NAME() {}", "path": path }),
        )
        .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected a tool error for {path}, got {result:?}");
        };
        assert!(message.contains(expected), "{path}: {message}");
    }
}

#[tokio::test]
async fn ast_edit_previews_then_writes_back_and_checkpoints_once() {
    let fixture = fixture();
    let capability = AstEditCapability::new(Arc::new(SandboxCodeHost));
    let file = fixture.root.join("lib/web/app.ts");
    let arguments = |dry_run: bool| {
        json!({
            "pattern": "console.log($$$ARGS)",
            "replacement": "logger.info($$$ARGS)",
            "language": "typescript",
            "dry_run": dry_run,
        })
    };

    let preview = success(call(&capability, "ast_edit", &fixture.context, arguments(true)).await);
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["count"], 1);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "console.log(value);\n"
    );
    assert_eq!(fixture.sandbox.checkpoints.load(Ordering::SeqCst), 0);

    let applied = success(call(&capability, "ast_edit", &fixture.context, arguments(false)).await);
    assert_eq!(applied["applied"], true, "{applied}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "logger.info(value);\n"
    );
    assert_eq!(fixture.sandbox.checkpoints.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn repo_map_reads_symbols_from_the_sandbox() {
    let fixture = fixture();
    let capability = RepoMapCapability::new(Arc::new(SandboxCodeHost));
    let value = success(
        call(
            &capability,
            "repo_symbols",
            &fixture.context,
            json!({ "query": "shared helper" }),
        )
        .await,
    );
    let names: Vec<&str> = value["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|symbol| symbol["name"].as_str())
        .collect();
    assert!(
        names.contains(&"shared") && names.contains(&"helper"),
        "{value}"
    );
}

#[tokio::test]
async fn a_session_without_a_sandbox_gets_a_clear_error() {
    let capability = RepoMapCapability::new(Arc::new(SandboxCodeHost));
    let result = call(
        &capability,
        "repo_map",
        &ToolContext::new(SessionId::new()),
        json!({}),
    )
    .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("expected a tool error, got {result:?}");
    };
    assert!(message.contains("needs the session's sandbox"), "{message}");
}

#[tokio::test]
async fn project_root_is_the_repository_a_file_belongs_to() {
    let fixture = fixture();
    let host = SandboxCodeHost;
    let root = fixture.root.to_string_lossy().into_owned();

    let file = host
        .resolve_file(&fixture.context, "/workspace/app/src/main.rs")
        .await
        .unwrap();
    assert_eq!(file, format!("{root}/app/src/main.rs"));
    assert_eq!(
        host.project_root(&fixture.context, &file).await.unwrap(),
        format!("{root}/app")
    );
    // A file outside any repository belongs to the workspace itself.
    std::fs::write(fixture.root.join("notes.rs"), "").unwrap();
    assert_eq!(
        host.project_root(&fixture.context, &format!("{root}/notes.rs"))
            .await
            .unwrap(),
        root
    );
}

#[tokio::test]
async fn read_texts_answers_in_order_and_marks_missing_files() {
    let fixture = fixture();
    let root = fixture.root.to_string_lossy().into_owned();
    let wanted = |names: &[&str]| -> Vec<String> {
        names.iter().map(|name| format!("{root}/{name}")).collect()
    };
    // Up to three files are read one by one; more travel as one archive.
    for names in [
        vec!["lib/src/lib.rs", "missing.rs"],
        vec![
            "lib/src/lib.rs",
            "missing.rs",
            "app/src/main.rs",
            "lib/web/app.ts",
            "app/.git/HEAD",
        ],
    ] {
        let texts = SandboxCodeHost
            .read_texts(&fixture.context, &wanted(&names))
            .await
            .unwrap();
        assert_eq!(texts.len(), names.len());
        assert_eq!(texts[0].as_deref(), Some("fn shared() {}\n"));
        assert_eq!(texts[1], None);
        if names.len() > 2 {
            assert_eq!(texts[3].as_deref(), Some("console.log(value);\n"));
        }
    }
}

#[tokio::test]
async fn apply_runs_file_operations_then_writes_and_checkpoints() {
    let fixture = fixture();
    let root = fixture.root.to_string_lossy().into_owned();
    let path = |name: &str| format!("{root}/{name}");

    SandboxCodeHost
        .apply(
            &fixture.context,
            &root,
            &[
                FileOp::Rename {
                    from: path("lib/src/lib.rs"),
                    to: path("lib/src/core/mod.rs"),
                },
                FileOp::Create {
                    path: path("lib/src/new.rs"),
                    overwrite: false,
                    ignore_if_exists: false,
                },
                FileOp::Delete {
                    path: path("lib/web/app.ts"),
                    recursive: false,
                },
                FileOp::Write {
                    path: path("lib/src/core/mod.rs"),
                    content: "pub fn renamed() {}\n".to_string(),
                },
                FileOp::Write {
                    path: path("lib/src/new.rs"),
                    content: "pub mod core;\n".to_string(),
                },
            ],
        )
        .await
        .unwrap();

    assert!(!fixture.root.join("lib/src/lib.rs").exists());
    assert!(!fixture.root.join("lib/web/app.ts").exists());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("lib/src/core/mod.rs")).unwrap(),
        "pub fn renamed() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("lib/src/new.rs")).unwrap(),
        "pub mod core;\n"
    );
    assert_eq!(fixture.sandbox.checkpoints.load(Ordering::SeqCst), 1);

    // Nothing outside the workspace or inside a protected directory.
    for target in ["/etc/hosts".to_string(), path("app/.git/config")] {
        let refused = SandboxCodeHost
            .apply(
                &fixture.context,
                &root,
                &[FileOp::Write {
                    path: target.clone(),
                    content: String::new(),
                }],
            )
            .await;
        assert!(refused.is_err(), "{target}");
    }
}

/// `cat` stands in for a language server: whatever the client writes comes
/// back, which proves bytes cross the relay intact in both directions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_carries_bytes_to_a_detached_process_and_back() {
    let fixture = fixture();
    let root = fixture.root.to_string_lossy().into_owned();
    let key = format!("test|{root}|cat");
    let mut io = SandboxCodeHost
        .launch(
            &fixture.context,
            &key,
            &ServerCommand {
                command: "cat".to_string(),
                args: Vec::new(),
            },
            &root,
        )
        .await
        .unwrap();
    let pump = io.pump.take().unwrap();

    // One write that rides inside the command, one large enough to be staged
    // as a file first.
    for size in [1_000usize, INLINE_WRITE_BYTES + 20_000] {
        let sent: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
        io.writer.write_all(&sent).await.unwrap();
        let mut received = vec![0u8; size];
        let read = async {
            io.reader.read_exact(&mut received).await.unwrap();
        };
        let ticks = async {
            loop {
                pump.tick(&fixture.context).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        tokio::select! {
            () = read => {}
            () = ticks => {}
            () = tokio::time::sleep(std::time::Duration::from_secs(30)) => panic!("relay timed out at {size} bytes"),
        }
        assert_eq!(received, sent, "{size} bytes");
    }

    // Stop the detached processes; the next tick reports the server gone.
    let dir = format!("/tmp/.everruns-lsp/{}", relay_name(&key));
    fixture
        .sandbox
        .exec_raw(
            &fixture.context,
            &format!("kill $(cat {dir}/pid) $(cat {dir}/holder); sleep 0.2"),
            None,
            None,
        )
        .await
        .unwrap();
    let mut gone = false;
    for _ in 0..50 {
        if pump.tick(&fixture.context).await.is_err() {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(gone, "a stopped server must be reported");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_missing_language_server_is_named() {
    let fixture = fixture();
    let root = fixture.root.to_string_lossy().into_owned();
    let Err(error) = SandboxCodeHost
        .launch(
            &fixture.context,
            "test|missing",
            &ServerCommand {
                command: "definitely-not-a-language-server".to_string(),
                args: Vec::new(),
            },
            &root,
        )
        .await
    else {
        panic!("launch should fail");
    };
    assert!(error.to_string().contains("is not installed"), "{error}");
}
