use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use everruns::{
    Agent, Environment, InMemoryEngine, LlmSimConfig, LocalConfig, LocalGitWorkspaceProvider,
    Model, ResumeError, Session, SessionEnvironmentError, ToolCall, Workspace, WorkspaceHeadId,
    WorkspacePolicy,
};
use everruns_core::session_files::SessionFileSystem;
use serde_json::json;

struct ComputeFileSystem(Arc<dyn SessionFileSystem>);

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "--quiet"]);
    git(
        repository.path(),
        &["config", "user.name", "Workspace Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "workspace@example.test"],
    );
    std::fs::write(repository.path().join("README.md"), "base\n").unwrap();
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "base"]);
    repository
}

fn agent(config: LocalConfig) -> Agent {
    Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .local(config)
        .build()
        .unwrap()
}

fn create_session(agent: Agent) -> Session {
    InMemoryEngine::new().create(agent)
}

async fn restore(agent: Agent, session_id: everruns::SessionId) -> Result<Session, ResumeError> {
    let engine = InMemoryEngine::new();
    engine.attach(session_id, agent).await?;
    engine.resume(session_id).await
}

#[tokio::test]
async fn ordinary_sessions_select_a_permanent_head_before_execution() {
    let engine = InMemoryEngine::new();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .build()
        .unwrap();
    let session = engine.create(agent.clone());
    session.start().await.unwrap();
    let head_id = session.workspace_head().unwrap().id();
    let session_id = session.session_id();

    drop(session);
    drop(agent);
    let resumed = engine.resume(session_id).await.unwrap();
    assert_eq!(resumed.workspace_head().unwrap().id(), head_id);
}

#[tokio::test]
async fn workspace_shorthand_is_one_explicitly_shared_reopenable_head() {
    let root = tempfile::tempdir().unwrap();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .workspace(root.path())
        .build()
        .unwrap();
    let engine = InMemoryEngine::new();
    let first = engine.create(agent.clone());
    let second = engine.create(agent);
    first.start().await.unwrap();
    second.start().await.unwrap();

    let first_head = first.workspace_head().unwrap();
    let second_head = second.workspace_head().unwrap();
    assert_eq!(first_head.id(), second_head.id());
    assert_eq!(first_head.workspace_id(), second_head.workspace_id());
    assert_eq!(first_head.access(), everruns::WorkspaceHeadAccess::Shared);

    let resumed = engine.resume(first.session_id()).await.unwrap();
    assert_eq!(resumed.workspace_head().unwrap().id(), first_head.id());
}

#[tokio::test]
async fn typed_resume_reopens_exact_head_and_isolation_is_enforced() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let provider_state = data.path().join("git-heads");
    let provider = Arc::new(LocalGitWorkspaceProvider::new(&provider_state).unwrap());
    let workspace = Workspace::open(provider.clone(), repository.path().to_string_lossy())
        .await
        .unwrap();
    let head = workspace.head("session").create().await.unwrap();
    let environment = Environment::builder()
        .workspace(head.clone())
        .build()
        .unwrap();
    let config = LocalConfig::new(data.path().join("runtime"));

    let first_agent = agent(config.clone());
    let session = create_session(first_agent.clone())
        .environment(environment.clone())
        .start()
        .await
        .unwrap();
    assert_eq!(session.workspace_head().unwrap().id(), head.id());

    let collision = create_session(first_agent.clone())
        .environment(environment)
        .start()
        .await
        .err()
        .expect("isolated head must reject a second session");
    assert_eq!(collision, SessionEnvironmentError::AlreadyBound);

    let session_id = session.session_id();
    drop(session);
    drop(first_agent);

    let reopened_provider = Arc::new(LocalGitWorkspaceProvider::new(&provider_state).unwrap());
    let resumed_agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .local(config)
        .workspace_provider(reopened_provider)
        .build()
        .unwrap();
    let resumed = restore(resumed_agent, session_id).await.unwrap();
    assert_eq!(resumed.workspace_head().unwrap().id(), head.id());
    assert_eq!(
        resumed.workspace_head().unwrap().workspace_id(),
        workspace.id()
    );

    head.clone().destroy().await.unwrap();
    let missing_agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .local(LocalConfig::new(data.path().join("runtime")))
        .workspace_provider(Arc::new(
            LocalGitWorkspaceProvider::new(&provider_state).unwrap(),
        ))
        .build()
        .unwrap();
    let missing = restore(missing_agent, session_id)
        .await
        .err()
        .expect("destroyed head must never be substituted");
    assert!(matches!(
        missing,
        everruns::ResumeError::WorkspaceUnavailable
    ));
}

#[tokio::test]
async fn shared_head_requires_explicit_shared_creation() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let provider = Arc::new(LocalGitWorkspaceProvider::new(data.path().join("heads")).unwrap());
    let workspace = Workspace::open(provider, repository.path().to_string_lossy())
        .await
        .unwrap();
    let shared = workspace.head("shared").shared().create().await.unwrap();
    let environment = Environment::builder().workspace(shared).build().unwrap();
    let agent = agent(LocalConfig::new(data.path().join("runtime")));

    let first = create_session(agent.clone())
        .environment(environment.clone())
        .start()
        .await
        .unwrap();
    let second = create_session(agent)
        .environment(environment)
        .start()
        .await
        .unwrap();
    assert_ne!(first.session_id(), second.session_id());
    assert_eq!(
        first.workspace_head().unwrap().id(),
        second.workspace_head().unwrap().id()
    );
}

#[tokio::test]
async fn workspace_scoped_compute_extension_addresses_the_selected_head() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let provider = Arc::new(LocalGitWorkspaceProvider::new(data.path().join("heads")).unwrap());
    let workspace = Workspace::open(provider, repository.path().to_string_lossy())
        .await
        .unwrap();
    let head = workspace.head("compute").create().await.unwrap();
    let environment = Environment::builder()
        .workspace(head.clone())
        .workspace_extension(|selected| Arc::new(ComputeFileSystem(selected.file_system())))
        .unwrap()
        .build()
        .unwrap();
    let session = create_session(agent(LocalConfig::new(data.path().join("runtime"))))
        .environment(environment)
        .start()
        .await
        .unwrap();

    let compute = session
        .environment_extension::<ComputeFileSystem>()
        .unwrap();
    compute
        .0
        .write_file(
            session.session_id(),
            "/from-compute.txt",
            "same head",
            "text",
        )
        .await
        .unwrap();
    let visible = head
        .file_system()
        .read_file(session.session_id(), "/from-compute.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible.content.as_deref(), Some("same head"));
}

#[tokio::test]
async fn head_filesystem_keeps_policy_and_symlink_containment() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let provider_state = data.path().join("heads");
    let provider = Arc::new(LocalGitWorkspaceProvider::new(&provider_state).unwrap());
    let workspace = Workspace::open(provider, repository.path().to_string_lossy())
        .await
        .unwrap();
    let readonly_head = workspace.head("readonly").create().await.unwrap();
    let readonly_path = provider_state
        .join("worktrees")
        .join(workspace.id().to_string())
        .join(readonly_head.id().to_string());
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "write-blocked".into(),
                name: "write_file".into(),
                arguments: json!({"path": "/workspace/blocked.txt", "content": "blocked"}),
            }],
            vec![],
        ]));
    let readonly_agent = Agent::builder()
        .instructions("Write the requested file.")
        .model(model)
        .local(LocalConfig::new(data.path().join("readonly-runtime")))
        .build()
        .unwrap();
    create_session(readonly_agent)
        .environment(
            Environment::builder()
                .workspace(readonly_head)
                .build()
                .unwrap(),
        )
        .start()
        .await
        .unwrap()
        .run("write")
        .await
        .unwrap();
    assert!(!readonly_path.join("blocked.txt").exists());

    let writable_head = workspace.head("writable").create().await.unwrap();
    let writable_path = provider_state
        .join("worktrees")
        .join(workspace.id().to_string())
        .join(writable_head.id().to_string());
    let outside = data.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, writable_path.join("escape")).unwrap();
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "write-escape".into(),
                name: "write_file".into(),
                arguments: json!({"path": "/workspace/escape/secret.txt", "content": "no"}),
            }],
            vec![],
        ]));
    let writable_agent = Agent::builder()
        .instructions("Write the requested file.")
        .model(model)
        .local(LocalConfig::new(data.path().join("writable-runtime")))
        .workspace_policy(WorkspacePolicy::read_write())
        .build()
        .unwrap();
    create_session(writable_agent)
        .environment(
            Environment::builder()
                .workspace(writable_head)
                .build()
                .unwrap(),
        )
        .start()
        .await
        .unwrap()
        .run("write")
        .await
        .unwrap();
    #[cfg(unix)]
    assert!(!outside.join("secret.txt").exists());
}

#[tokio::test]
async fn concurrent_sessions_write_separate_heads_without_collisions() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let provider_state = data.path().join("heads");
    let provider = Arc::new(LocalGitWorkspaceProvider::new(&provider_state).unwrap());
    let workspace = Workspace::open(provider, repository.path().to_string_lossy())
        .await
        .unwrap();
    let left = workspace.head("left").create().await.unwrap();
    let right = workspace.head("right").create().await.unwrap();

    let make_agent = |content: &'static str, runtime: &'static str| {
        let model = Model::simulated_with_config(
            LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: format!("write-{content}"),
                    name: "write_file".into(),
                    arguments: json!({"path": "/workspace/result.txt", "content": content}),
                }],
                vec![],
            ]),
        );
        Agent::builder()
            .instructions("Write the requested file.")
            .model(model)
            .local(LocalConfig::new(data.path().join(runtime)))
            .workspace_policy(WorkspacePolicy::read_write())
            .build()
            .unwrap()
    };
    let left_agent = make_agent("left", "left-runtime");
    let right_agent = make_agent("right", "right-runtime");
    let left_session = create_session(left_agent)
        .environment(
            Environment::builder()
                .workspace(left.clone())
                .build()
                .unwrap(),
        )
        .start()
        .await
        .unwrap();
    let right_session = create_session(right_agent)
        .environment(
            Environment::builder()
                .workspace(right.clone())
                .build()
                .unwrap(),
        )
        .start()
        .await
        .unwrap();

    let (left_result, right_result) = tokio::join!(
        left_session.run("write left"),
        right_session.run("write right")
    );
    left_result.unwrap();
    right_result.unwrap();

    let worktree = |head_id: WorkspaceHeadId| {
        provider_state
            .join("worktrees")
            .join(workspace.id().to_string())
            .join(head_id.to_string())
            .join("result.txt")
    };
    assert_eq!(
        std::fs::read_to_string(worktree(left.id())).unwrap(),
        "left"
    );
    assert_eq!(
        std::fs::read_to_string(worktree(right.id())).unwrap(),
        "right"
    );
}
