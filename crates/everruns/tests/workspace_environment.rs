use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use everruns::{
    Agent, Environment, InMemoryEngine, LlmSimConfig, LocalConfig, LocalGitWorkspace, Model,
    ResumeError, Session, SessionEnvironmentError, ToolCall, Workspace, WorkspaceHeadId,
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

#[test]
#[allow(deprecated)]
fn duplicate_workspace_backend_keeps_emitting_legacy_build_error() {
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    let error = Agent::builder()
        .workspace_backend(Arc::new(
            LocalGitWorkspace::new(first_state.path()).unwrap(),
        ))
        .workspace_backend(Arc::new(
            LocalGitWorkspace::new(second_state.path()).unwrap(),
        ))
        .build()
        .expect_err("duplicate backend ids must fail");

    assert!(matches!(
        error,
        everruns::BuildError::DuplicateWorkspaceProvider { .. }
    ));
}

#[test]
#[allow(deprecated)]
fn deprecated_workspace_names_forward_to_backend_api() {
    fn implements_backend<T: everruns::WorkspaceProvider>() {}
    fn implements_prelude_backend<T: everruns::prelude::WorkspaceProvider>() {}

    implements_backend::<everruns::LocalGitWorkspaceProvider>();
    implements_prelude_backend::<everruns::prelude::LocalGitWorkspaceProvider>();
    let state = tempfile::tempdir().unwrap();
    let backend = Arc::new(everruns::LocalGitWorkspaceProvider::new(state.path()).unwrap());
    let _builder = Agent::builder().workspace_provider(backend);
    let _id: Option<everruns::WorkspaceProviderId> = None;
    let _prelude_id: Option<everruns::prelude::WorkspaceProviderId> = None;
    assert_eq!(
        everruns::WorkspaceError::BackendUnavailable("offline".into()).to_string(),
        "workspace backend is unavailable: offline"
    );
    assert_eq!(
        everruns::WorkspaceError::Backend("failed".into()).to_string(),
        "workspace backend failed: failed"
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
#[allow(deprecated)]
async fn typed_resume_reopens_exact_head_and_isolation_is_enforced() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let backend_state = data.path().join("git-heads");
    let backend = Arc::new(LocalGitWorkspace::new(&backend_state).unwrap());
    let workspace = Workspace::open(backend.clone(), repository.path().to_string_lossy())
        .await
        .unwrap();
    let head = workspace.head("session").create().await.unwrap();
    let legacy = head.provider();
    let canonical = head.backend();
    assert_eq!(legacy.id(), canonical.id());
    assert!(Arc::ptr_eq(&legacy, &canonical));
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

    let unavailable = restore(agent(config.clone()), session_id)
        .await
        .err()
        .expect("unregistered workspace backend must fail");
    assert!(matches!(
        unavailable,
        ResumeError::WorkspaceProviderUnavailable { .. }
    ));

    let reopened_backend = Arc::new(LocalGitWorkspace::new(&backend_state).unwrap());
    let resumed_agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .local(config)
        .workspace_backend(reopened_backend)
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
        .workspace_backend(Arc::new(LocalGitWorkspace::new(&backend_state).unwrap()))
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
#[allow(deprecated)]
async fn workspace_backend_failures_keep_emitting_legacy_workspace_errors() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("not-a-directory");
    std::fs::write(&file, "file").unwrap();
    let unavailable =
        LocalGitWorkspace::new(file.join("state")).expect_err("invalid state parent must fail");
    assert!(matches!(
        unavailable,
        everruns::WorkspaceError::ProviderUnavailable(_)
    ));

    let state = tempfile::tempdir().unwrap();
    let not_repository = tempfile::tempdir().unwrap();
    let backend = Arc::new(LocalGitWorkspace::new(state.path()).unwrap());
    let failure = Workspace::open(backend, not_repository.path().to_string_lossy())
        .await
        .expect_err("non-repository locator must fail");
    assert!(matches!(failure, everruns::WorkspaceError::Provider(_)));
}

#[tokio::test]
#[allow(deprecated)]
async fn backend_identity_conflicts_keep_emitting_legacy_session_error() {
    let repository = repository();
    let configured_state = tempfile::tempdir().unwrap();
    let selected_state = tempfile::tempdir().unwrap();
    let configured = Arc::new(LocalGitWorkspace::new(configured_state.path()).unwrap());
    let selected = Arc::new(LocalGitWorkspace::new(selected_state.path()).unwrap());
    let workspace = Workspace::open(selected, repository.path().to_string_lossy())
        .await
        .unwrap();
    let environment = Environment::builder()
        .workspace(workspace.head("conflict").create().await.unwrap())
        .build()
        .unwrap();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .workspace_backend(configured)
        .build()
        .unwrap();
    let error = create_session(agent)
        .environment(environment)
        .start()
        .await
        .err()
        .expect("different backend instance with the same id must fail");

    assert_eq!(error, SessionEnvironmentError::ProviderConflict);
}

#[tokio::test]
async fn shared_head_requires_explicit_shared_creation() {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let backend = Arc::new(LocalGitWorkspace::new(data.path().join("heads")).unwrap());
    let workspace = Workspace::open(backend, repository.path().to_string_lossy())
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
    let backend = Arc::new(LocalGitWorkspace::new(data.path().join("heads")).unwrap());
    let workspace = Workspace::open(backend, repository.path().to_string_lossy())
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
    let backend_state = data.path().join("heads");
    let backend = Arc::new(LocalGitWorkspace::new(&backend_state).unwrap());
    let workspace = Workspace::open(backend, repository.path().to_string_lossy())
        .await
        .unwrap();
    let readonly_head = workspace.head("readonly").create().await.unwrap();
    let readonly_path = backend_state
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
    let writable_path = backend_state
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
    let backend_state = data.path().join("heads");
    let backend = Arc::new(LocalGitWorkspace::new(&backend_state).unwrap());
    let workspace = Workspace::open(backend, repository.path().to_string_lossy())
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
        backend_state
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

/// A target that fixes its own boundary, the way Bashkit and a Daytona VM do.
struct IsolatedCompute;

#[async_trait::async_trait]
impl everruns::Compute for IsolatedCompute {
    fn id(&self) -> &str {
        "test-isolated"
    }

    fn kind(&self) -> everruns::ComputeKind {
        everruns::ComputeKind::Vfs
    }

    fn capabilities(&self) -> everruns::ComputeCapabilities {
        everruns::ComputeCapabilities {
            portable_checkpoint: true,
            network_enforced: true,
            ..Default::default()
        }
    }

    fn enforced_containment(&self) -> everruns::ContainmentLevel {
        everruns::ContainmentLevel::Isolated
    }

    fn durability(&self) -> everruns::Durability {
        everruns::Durability::Checkpointed
    }

    async fn connect(
        &self,
        _head: &everruns::WorkspaceHead,
    ) -> Result<Arc<dyn everruns::ComputeSession>, everruns::ComputeError> {
        Err(everruns::ComputeError::Unsupported("connect"))
    }
}

/// A target that contains nothing, the way a real machine does.
struct UncontainedCompute;

#[async_trait::async_trait]
impl everruns::Compute for UncontainedCompute {
    fn id(&self) -> &str {
        "test-uncontained"
    }

    fn kind(&self) -> everruns::ComputeKind {
        everruns::ComputeKind::Host
    }

    fn capabilities(&self) -> everruns::ComputeCapabilities {
        everruns::ComputeCapabilities::full_machine()
    }

    fn enforced_containment(&self) -> everruns::ContainmentLevel {
        everruns::ContainmentLevel::None
    }

    fn durability(&self) -> everruns::Durability {
        everruns::Durability::None
    }

    async fn connect(
        &self,
        _head: &everruns::WorkspaceHead,
    ) -> Result<Arc<dyn everruns::ComputeSession>, everruns::ComputeError> {
        Err(everruns::ComputeError::Unsupported("connect"))
    }
}

async fn test_head() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    everruns::WorkspaceHead,
) {
    let repository = repository();
    let data = tempfile::tempdir().unwrap();
    let backend = Arc::new(LocalGitWorkspace::new(data.path().join("git-heads")).unwrap());
    let workspace = Workspace::open(backend, repository.path().to_string_lossy())
        .await
        .unwrap();
    let head = workspace.head("compute").create().await.unwrap();
    (repository, data, head)
}

#[tokio::test]
async fn an_environment_without_compute_can_do_nothing_and_says_so() {
    let (_repository, _data, head) = test_head().await;

    let environment = Environment::builder().workspace(head).build().unwrap();

    assert!(environment.compute().is_none());
    assert!(!environment.capabilities().native_processes);
    assert_eq!(
        environment.containment().level,
        everruns::ContainmentLevel::None
    );
}

#[tokio::test]
async fn an_omitted_containment_records_what_the_target_enforces() {
    let (_repository, _data, head) = test_head().await;

    let environment = Environment::builder()
        .workspace(head)
        .compute(Arc::new(IsolatedCompute))
        .build()
        .unwrap();

    assert_eq!(
        environment.containment().level,
        everruns::ContainmentLevel::Isolated
    );
    assert_eq!(environment.durability(), everruns::Durability::Checkpointed);
    assert!(environment.capabilities().portable_checkpoint);
}

#[tokio::test]
async fn containment_weaker_than_the_target_is_refused_rather_than_corrected() {
    let (_repository, _data, head) = test_head().await;

    let error = Environment::builder()
        .workspace(head)
        .compute(Arc::new(IsolatedCompute))
        .containment(everruns::Containment::none())
        .build()
        .expect_err("a target's own boundary cannot be opted out of");

    assert!(matches!(
        error,
        everruns::EnvironmentError::ContainmentWeakerThanTarget {
            requested: everruns::ContainmentLevel::None,
            enforced: everruns::ContainmentLevel::Isolated,
        }
    ));
}

#[tokio::test]
async fn containment_nothing_implements_yet_is_refused_rather_than_promised() {
    let (_repository, _data, head) = test_head().await;

    let error = Environment::builder()
        .workspace(head)
        .compute(Arc::new(UncontainedCompute))
        .containment(everruns::Containment::native())
        .build()
        .expect_err("kernel containment has no provider yet");

    assert!(matches!(
        error,
        everruns::EnvironmentError::ContainmentUnavailable {
            requested: everruns::ContainmentLevel::Native,
        }
    ));
}

#[tokio::test]
async fn an_uncontained_target_never_reports_itself_as_recoverable() {
    let (_repository, _data, head) = test_head().await;

    let environment = Environment::builder()
        .workspace(head)
        .compute(Arc::new(UncontainedCompute))
        .containment(everruns::Containment::none())
        .build()
        .unwrap();

    // Durability is read off the target, so no profile can promise recovery a
    // real machine cannot deliver.
    assert_eq!(environment.durability(), everruns::Durability::None);
    assert!(environment.capabilities().native_processes);
    assert!(!environment.capabilities().portable_checkpoint);
}
