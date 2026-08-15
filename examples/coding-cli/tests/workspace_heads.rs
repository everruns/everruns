//! Temporary-Git acceptance tests for the coding CLI's public workspace flow.

use std::path::Path;
use std::process::Command;

use everruns::{
    Agent, InMemoryEngine, LlmSimConfig, Model, SessionId, ToolCall, WorkspaceHeadAccess,
    WorkspacePolicy,
};
use everruns_coding_cli::{CodingWorkspace, agent_builder};
use serde_json::json;

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir().expect("repository");
    git(repository.path(), &["init", "--quiet"]);
    git(
        repository.path(),
        &["config", "user.name", "Coding CLI Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "coding-cli@example.test"],
    );
    std::fs::write(repository.path().join("README.md"), "base\n").expect("base file");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "base"]);
    repository
}

fn writing_agent(workspace: &CodingWorkspace, content: &'static str, read_only: bool) -> Agent {
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: format!("write-{content}"),
                name: "write_file".into(),
                arguments: json!({
                    "path": "/workspace/result.txt",
                    "content": content
                }),
            }],
            vec![],
        ]));
    let policy = if read_only {
        WorkspacePolicy::read_only()
    } else {
        WorkspacePolicy::read_write()
    };
    workspace
        .configure_agent(agent_builder().model(model), policy)
        .build()
        .expect("agent builds")
}

#[test]
fn binary_opens_a_named_git_head_in_offline_mode() {
    let repository = repository();
    let state = tempfile::tempdir().expect("state");
    let output = Command::new(env!("CARGO_BIN_EXE_ercode"))
        .args([
            "--offline",
            "--cwd",
            repository.path().to_str().expect("UTF-8 repository"),
            "--state-dir",
            state.path().to_str().expect("UTF-8 state"),
            "--head",
            "smoke",
            "--base",
            "HEAD",
            "hello",
        ])
        .output()
        .expect("run ercode");
    assert!(
        output.status.success(),
        "ercode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("name=\"smoke\""), "{stderr}");
    assert!(stderr.contains("access=isolated"), "{stderr}");
    assert!(stderr.contains("session=session_"), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Offline simulator: configure a real model to execute coding tool calls."
    );
}

#[tokio::test]
async fn isolated_heads_from_one_base_write_without_colliding() {
    let repository = repository();
    let state = tempfile::tempdir().expect("state");
    let workspace = CodingWorkspace::open(repository.path(), state.path())
        .await
        .expect("workspace opens");
    let engine = InMemoryEngine::new();
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    let left = workspace
        .create_head("left", Some(&base), false)
        .await
        .expect("left head");
    let right = workspace
        .create_head("right", Some(&base), false)
        .await
        .expect("right head");

    assert_ne!(left.id(), right.id());
    assert_eq!(left.access(), WorkspaceHeadAccess::Isolated);
    assert_eq!(right.access(), WorkspaceHeadAccess::Isolated);

    let left_session = workspace
        .start_session(
            &engine,
            &writing_agent(&workspace, "left", false),
            left.clone(),
        )
        .await
        .expect("left session");
    let right_session = workspace
        .start_session(
            &engine,
            &writing_agent(&workspace, "right", false),
            right.clone(),
        )
        .await
        .expect("right session");
    let (left_turn, right_turn) = tokio::join!(
        left_session.run("write the result"),
        right_session.run("write the result")
    );
    left_turn.expect("left turn");
    right_turn.expect("right turn");

    let left_file = left
        .file_system()
        .read_file(left_session.session_id(), "/workspace/result.txt")
        .await
        .expect("read left")
        .expect("left result");
    let right_file = right
        .file_system()
        .read_file(right_session.session_id(), "/workspace/result.txt")
        .await
        .expect("read right")
        .expect("right result");
    assert_eq!(left_file.content.as_deref(), Some("left"));
    assert_eq!(right_file.content.as_deref(), Some("right"));
    assert!(!repository.path().join("result.txt").exists());

    let base_file = left
        .file_system()
        .read_file(left_session.session_id(), "/workspace/README.md")
        .await
        .expect("read base file")
        .expect("README in selected head");
    assert_eq!(base_file.content.as_deref(), Some("base\n"));
}

#[tokio::test]
async fn framework_policy_denies_writes_on_the_selected_head() {
    let repository = repository();
    let state = tempfile::tempdir().expect("state");
    let workspace = CodingWorkspace::open(repository.path(), state.path())
        .await
        .expect("workspace opens");
    let engine = InMemoryEngine::new();
    let head = workspace
        .create_head("read-only", None, false)
        .await
        .expect("head");
    let session = workspace
        .start_session(
            &engine,
            &writing_agent(&workspace, "blocked", true),
            head.clone(),
        )
        .await
        .expect("session");

    session
        .run("write the result")
        .await
        .expect("turn completes");
    assert!(
        head.file_system()
            .read_file(session.session_id(), "/workspace/result.txt")
            .await
            .expect("read result")
            .is_none(),
        "WorkspacePolicy::read_only must be enforced by Framework tools"
    );
}

#[tokio::test]
async fn typed_resume_reopens_the_recorded_head_and_drop_does_not_delete_it() {
    let repository = repository();
    let state = tempfile::tempdir().expect("state");
    let workspace = CodingWorkspace::open(repository.path(), state.path())
        .await
        .expect("workspace opens");
    let engine = InMemoryEngine::new();
    let head = workspace
        .create_head("resume", None, false)
        .await
        .expect("head");
    let agent = writing_agent(&workspace, "persisted", false);
    let session = workspace
        .start_session(&engine, &agent, head.clone())
        .await
        .expect("session");
    session.run("write the result").await.expect("materialize");
    let session_id = session.session_id();
    let head_id = head.id();
    let branch = head.status().await.expect("head status").metadata["branch"].clone();
    drop(session);
    drop(agent);
    drop(head);
    drop(workspace);

    let worktrees = git(repository.path(), &["worktree", "list", "--porcelain"]);
    assert!(
        worktrees.contains(&format!("branch refs/heads/{branch}")),
        "dropping CLI handles must not remove the provider-owned worktree"
    );

    let reopened = CodingWorkspace::from_state(state.path()).expect("Framework state reopens");
    let reopened_engine = InMemoryEngine::new();
    let resumed = reopened
        .resume_session(
            &reopened_engine,
            &writing_agent(&reopened, "unused", false),
            session_id,
        )
        .await
        .expect("typed resume");
    assert_eq!(
        resumed.workspace_head().expect("recorded head").id(),
        head_id
    );
    let file = resumed
        .workspace_head()
        .expect("recorded head")
        .file_system()
        .read_file(session_id, "/workspace/result.txt")
        .await
        .expect("read result")
        .expect("persisted result");
    assert_eq!(file.content.as_deref(), Some("persisted"));

    resumed
        .workspace_head()
        .expect("recorded head")
        .clone()
        .destroy()
        .await
        .expect("explicit destroy");
    drop(resumed);
    drop(reopened);

    let missing_workspace =
        CodingWorkspace::from_state(state.path()).expect("Framework state reopens");
    let missing_engine = InMemoryEngine::new();
    let missing_agent = writing_agent(&missing_workspace, "unused", false);
    let missing = missing_workspace
        .resume_session(&missing_engine, &missing_agent, session_id)
        .await
        .err()
        .expect("destroyed recorded head must not be substituted");
    assert!(
        format!("{missing:#}").contains("recorded workspace head is unavailable"),
        "unexpected resume error: {missing:#}"
    );
}

#[tokio::test]
async fn shared_head_requires_explicit_creation_and_existing_session_id() {
    let repository = repository();
    let state = tempfile::tempdir().expect("state");
    let workspace = CodingWorkspace::open(repository.path(), state.path())
        .await
        .expect("workspace opens");
    let engine = InMemoryEngine::new();
    let shared = workspace
        .create_head("team", None, true)
        .await
        .expect("shared head");
    let agent = writing_agent(&workspace, "shared", false);
    let first = workspace
        .start_session(&engine, &agent, shared.clone())
        .await
        .expect("first session");
    first.run("write the result").await.expect("materialize");
    let second = workspace
        .start_shared_session(&engine, &agent, first.session_id())
        .await
        .expect("second shared session");
    assert_ne!(first.session_id(), second.session_id());
    assert_eq!(
        first.workspace_head().expect("first head").id(),
        second.workspace_head().expect("second head").id()
    );

    let isolated = workspace
        .create_head("private", None, false)
        .await
        .expect("isolated head");
    let isolated_session = workspace
        .start_session(&engine, &agent, isolated)
        .await
        .expect("isolated session");
    isolated_session
        .run("materialize")
        .await
        .expect("materialize isolated session");
    let error = workspace
        .start_shared_session(&engine, &agent, isolated_session.session_id())
        .await
        .err()
        .expect("isolated head must not be shared implicitly");
    assert!(format!("{error:#}").contains("records an isolated head"));
}

#[test]
fn typed_session_ids_are_required_for_resume_inputs() {
    assert!("not-a-session".parse::<SessionId>().is_err());
}
