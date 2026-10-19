use std::sync::Arc;

use everruns_core::host::{InMemorySessionFileStore, InMemorySessionStorageStore};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::tools::Tool;

use super::super::environment_tools::EnvironmentWriteFileTool;
use super::*;

fn config() -> Value {
    json!({"provider": "core-test-session-sandbox"})
}

/// The fake provider keys files by sandbox, and every test's sandbox shares
/// one id, so each test writes under its own directory.
fn unique_dir() -> String {
    format!("/workspace/t-{}", uuid::Uuid::now_v7())
}

fn sandbox_context() -> (ToolContext, Arc<InMemorySessionFileStore>) {
    let files = Arc::new(InMemorySessionFileStore::new());
    let context = ToolContext::with_stores(
        everruns_contracts::typed_id::SessionId::new(),
        files.clone(),
        Arc::new(InMemorySessionStorageStore::new()),
    );
    (context, files)
}

async fn write_sandbox(context: &ToolContext, path: &str, content: &str) {
    let result = EnvironmentWriteFileTool::new(config())
        .execute_with_context(json!({"path": path, "content": content}), context)
        .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
}

async fn read(context: &ToolContext, path: &str) -> ToolExecutionResult {
    EnvironmentReadFileTool::new(config())
        .execute_with_context(json!({"path": path}), context)
        .await
}

#[tokio::test]
async fn read_many_files_reads_the_sandbox_in_order_with_per_path_errors() {
    let (context, _) = sandbox_context();
    let dir = unique_dir();
    write_sandbox(&context, &format!("{dir}/a.txt"), "alpha").await;
    write_sandbox(&context, &format!("{dir}/b.txt"), "beta").await;

    let result = EnvironmentReadManyFilesTool::new(config())
        .execute_with_context(
            json!({"paths": [
                format!("{dir}/a.txt"),
                format!("{dir}/missing.txt"),
                format!("{dir}/b.txt"),
            ]}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("read_many_files failed: {result:?}");
    };
    assert_eq!(value["count"], 3);
    let results = value["results"].as_array().unwrap();
    assert_eq!(results[0]["content"], "1|alpha");
    assert_eq!(results[0]["path"], format!("{dir}/a.txt"));
    assert!(
        results[1]["error"]
            .as_str()
            .unwrap()
            .contains("File not found"),
        "{value}"
    );
    assert_eq!(results[2]["content"], "1|beta");
}

#[tokio::test]
async fn read_many_files_rejects_batches_outside_two_to_ten() {
    let (context, _) = sandbox_context();
    let tool = EnvironmentReadManyFilesTool::new(config());
    for paths in [json!(["/workspace/a"]), json!(vec!["/workspace/a"; 11])] {
        let result = tool
            .execute_with_context(json!({ "paths": paths }), &context)
            .await;
        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected a tool error, got {result:?}");
        };
        assert!(message.contains("between 2 and 10"), "{message}");
    }
    let result = tool
        .execute_with_context(json!({ "paths": ["/workspace/a", 7] }), &context)
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(m) if m.contains("paths[1]")),
        "{result:?}"
    );
}

/// Skills activated and output persisted after the sandbox started live in the
/// session VFS only. The pointers the runtime hands the model must still open.
#[tokio::test]
async fn read_file_falls_back_to_the_session_vfs_for_skills_and_persisted_output() {
    let (context, files) = sandbox_context();
    let dir = unique_dir();
    // Start the sandbox first, so nothing below is seeded into it.
    write_sandbox(&context, &format!("{dir}/start.txt"), "up").await;

    let id = uuid::Uuid::now_v7().simple().to_string();
    let skill = format!("/.agents/skills/ops-{id}/SKILL.md");
    let output = format!("/outputs/call_{id}.stdout");
    let source = format!("/src/vfs-only-{id}.rs");
    for (path, content) in [
        (skill.as_str(), "# Ops skill"),
        (output.as_str(), "full output"),
        (source.as_str(), "fn vfs() {}"),
    ] {
        files
            .write_file(context.session_id, path, content, "text")
            .await
            .unwrap();
    }

    for (path, expected) in [
        (format!("/workspace{skill}"), "1|# Ops skill"),
        (skill.clone(), "1|# Ops skill"),
        (format!("/workspace{output}"), "1|full output"),
    ] {
        let result = read(&context, &path).await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("{path}: {result:?}");
        };
        assert_eq!(value["content"], expected, "{path}");
        assert_eq!(value["path"], path);
    }

    // Workspace files belong to the sandbox: a VFS-only copy stays unreachable.
    let result = read(&context, &format!("/workspace{source}")).await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(m) if m.contains("File not found")),
        "{result:?}"
    );

    // The sandbox stays primary when it has the file too.
    let persisted = format!("/workspace/outputs/both_{id}.stdout");
    files
        .write_file(context.session_id, &persisted, "vfs", "text")
        .await
        .unwrap();
    write_sandbox(&context, &persisted, "sandbox").await;
    let result = read(&context, &persisted).await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("{result:?}");
    };
    assert_eq!(value["content"], "1|sandbox");

    // read_many_files goes through the same read path.
    let result = EnvironmentReadManyFilesTool::new(config())
        .execute_with_context(
            json!({"paths": [format!("/workspace{skill}"), format!("{dir}/start.txt")]}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("{result:?}");
    };
    assert_eq!(value["results"][0]["content"], "1|# Ops skill");
    assert_eq!(value["results"][1]["content"], "1|up");
}

#[test]
fn vfs_owned_paths_cover_skills_and_outputs_only() {
    for path in [
        "/.agents/skills/a/SKILL.md",
        "/workspace/.agents/skills/a/SKILL.md",
        "/outputs/x.stdout",
        "/workspace/outputs/x.stdout",
        "/workspace/outputs",
    ] {
        assert!(is_session_vfs_owned(path), "{path}");
    }
    for path in [
        "/workspace/src/main.rs",
        "/workspace/.agentsx/a",
        "/workspace/outputsx/a",
        "/workspace/yolop/outputs/a",
        "/workspaceoutputs/a",
    ] {
        assert!(!is_session_vfs_owned(path), "{path}");
    }
}

#[test]
fn list_directory_paths_stay_in_the_workspace() {
    assert_eq!(workspace_path("/workspace").unwrap(), "/workspace");
    assert_eq!(workspace_path("/workspace/").unwrap(), "/workspace");
    assert_eq!(workspace_path("src/").unwrap(), "/workspace/src");
    assert_eq!(workspace_path("/workspace/src").unwrap(), "/workspace/src");
    assert_eq!(
        workspace_path("/.agents/skills").unwrap(),
        "/workspace/.agents/skills"
    );
    assert!(workspace_path("/workspace/../etc").is_err());
}

/// The listing command runs where the sandbox's does: a POSIX shell with GNU
/// find. Run it on a real directory so its quoting and paging are proven.
#[test]
fn list_command_pages_a_real_directory_sorted_by_name() {
    let root = std::env::temp_dir().join(format!("everruns-list-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("b.txt"), "abc").unwrap();
    std::fs::write(root.join("c d.md"), "x").unwrap();
    let dir = root.to_str().unwrap().to_string();

    let run = |start: usize, end: usize| {
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(list_command(&shell_escape(&dir), start, end))
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
        )
    };

    let (status, stdout) = run(0, 10);
    assert_eq!(status, Some(0));
    let listing = parse_listing(&stdout, "/workspace/demo");
    assert_eq!(listing.total, 3);
    let names: Vec<_> = listing.entries.iter().map(|e| e["name"].clone()).collect();
    assert_eq!(names, vec![json!("b.txt"), json!("c d.md"), json!("src")]);
    assert_eq!(listing.entries[0]["size_bytes"], 3);
    assert_eq!(listing.entries[0]["path"], "/workspace/demo/b.txt");
    assert_eq!(listing.entries[2]["is_directory"], true);

    let (_, stdout) = run(1, 2);
    let listing = parse_listing(&stdout, "/workspace/demo");
    assert_eq!(listing.total, 3);
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0]["name"], "c d.md");

    let ToolExecutionResult::Success(page) = listing_result("/workspace/demo", listing, 1, 1)
    else {
        panic!("expected a page");
    };
    assert_eq!(page["total_count"], 3);
    assert_eq!(page["truncation"]["next_offset"], 2);

    std::fs::remove_dir_all(&root).unwrap();
    let (status, _) = run(0, 10);
    assert_eq!(status, Some(2), "a missing directory exits 2");
}

#[tokio::test]
async fn list_directory_rejects_bad_paging_before_touching_the_sandbox() {
    let context = ToolContext::new(everruns_contracts::typed_id::SessionId::new());
    let tool = EnvironmentListDirectoryTool::new(json!({"provider": "missing-provider"}));
    for arguments in [json!({"limit": 0}), json!({"offset": -1})] {
        let result = tool.execute_with_context(arguments, &context).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(_)),
            "{result:?}"
        );
    }
    let result = tool
        .execute_with_context(json!({"path": "/workspace/../etc"}), &context)
        .await;
    assert!(
        matches!(&result, ToolExecutionResult::ToolError(m) if m.contains("inside /workspace")),
        "{result:?}"
    );
}
