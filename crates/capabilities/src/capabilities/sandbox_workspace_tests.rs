use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use everruns_contracts::session_sandbox::SessionSandboxContext;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::InMemorySessionStorageStore;
use serde_json::json;

use super::*;
use crate::session_sandbox::{
    SessionSandboxInstance, SessionSandboxProvider, SessionSandboxProviderPlugin,
    SessionSandboxReadFileResponse, SessionSandboxState, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};

const PROVIDER: &str = "secret-file-test-sandbox";
const SECRET: &str = "ghs_supersecret";

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Exec(String),
    Write(String, String),
    Read(String),
    /// Secret files still present when the workspace was checkpointed.
    Checkpoint(Vec<String>),
}

#[derive(Default)]
struct Sandbox {
    events: Vec<Event>,
    files: BTreeSet<String>,
    fail_command: bool,
    review_content: String,
    review_encoding: Option<String>,
    reported_size: Option<usize>,
}

/// Per-sandbox record, keyed by external id so parallel tests stay apart.
static SANDBOXES: LazyLock<Mutex<HashMap<String, Sandbox>>> = LazyLock::new(Default::default);

fn with_sandbox<T>(id: &str, f: impl FnOnce(&mut Sandbox) -> T) -> T {
    let mut all = SANDBOXES.lock().unwrap();
    f(all.entry(id.to_string()).or_default())
}

struct SecretFileTestProvider;

inventory::submit! {
    SessionSandboxProviderPlugin { factory: || Box::new(SecretFileTestProvider) }
}

fn unquoted_path(command: &str, prefix: &str) -> Option<String> {
    let rest = command.split(prefix).nth(1)?;
    Some(rest.split('\'').next()?.to_string())
}

#[async_trait]
impl SessionSandboxProvider for SecretFileTestProvider {
    fn id(&self) -> &str {
        PROVIDER
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(SessionSandboxInstance {
            external_id: context.session_id().to_string(),
            workspace_path: Some("/home/sandbox/workspace".to_string()),
            ..Default::default()
        })
    }

    async fn resume(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn pause(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn delete(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        Ok(())
    }

    /// Plays the shell's part: `: >` creates the file, `rm -f` and the trap
    /// remove it, and a failing command still runs its trap.
    async fn exec(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let command = request.command.clone();
        let exit_code = with_sandbox(&instance.external_id, |sandbox| {
            sandbox.events.push(Event::Exec(command.clone()));
            if let Some(path) = unquoted_path(&command, ": > '") {
                sandbox.files.insert(path);
            }
            if let Some(path) = unquoted_path(&command, "rm -f -- '") {
                sandbox.files.remove(&path);
            }
            // The trap removes the file when the wrapped command exits.
            if let Some(path) = unquoted_path(&command, "EVR_SECRET_FILE='") {
                sandbox.files.remove(&path);
            }
            let failed = sandbox.fail_command && command.contains("git fetch");
            i32::from(failed)
        });
        Ok(SessionSandboxExecResponse {
            exit_code,
            stdout: if command.contains("wc -c") {
                with_sandbox(&instance.external_id, |s| {
                    s.reported_size.unwrap_or(s.review_content.len())
                })
                .to_string()
            } else {
                format!("token={SECRET} in /home/sandbox/workspace")
            },
            stderr: String::new(),
            success: exit_code == 0,
            truncated: false,
            total_lines: 1,
            raw_output: None,
            hint: None,
        })
    }

    async fn read_file(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        Ok(with_sandbox(&instance.external_id, |s| {
            s.events.push(Event::Read(path.into()));
            SessionSandboxReadFileResponse {
                path: path.to_string(),
                content: s.review_content.clone(),
                encoding: s.review_encoding.clone().unwrap_or_else(|| "text".into()),
            }
        }))
    }

    async fn write_file(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        with_sandbox(&instance.external_id, |sandbox| {
            sandbox.events.push(Event::Write(
                path.to_string(),
                String::from_utf8_lossy(content).into_owned(),
            ));
            sandbox.files.insert(path.to_string());
        });
        Ok(SessionSandboxWriteFileResponse {
            path: path.to_string(),
            bytes_written: content.len(),
        })
    }

    async fn checkpoint(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        with_sandbox(&instance.external_id, |sandbox| {
            let live = sandbox.files.iter().cloned().collect();
            sandbox.events.push(Event::Checkpoint(live));
        });
        Ok(instance.clone())
    }

    async fn status(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        Ok(SessionSandboxStatusResponse {
            provider: state.provider.clone(),
            session_status: state.status,
            external_id: state.instance.external_id.clone(),
            display_name: None,
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({}),
        })
    }
}

fn workspace() -> (ManagedSandboxWorkspace, ToolContext, String) {
    let session_id = SessionId::new();
    let context =
        ToolContext::with_storage_store(session_id, Arc::new(InMemorySessionStorageStore::new()));
    let config = SessionSandboxConfig {
        provider: PROVIDER.to_string(),
        ..Default::default()
    };
    (
        ManagedSandboxWorkspace::new(config),
        context,
        session_id.to_string(),
    )
}

#[tokio::test]
async fn slack_review_prefers_the_sandbox_and_never_falls_back_on_read_failure() {
    use crate::slack_action::{
        SlackAction, SlackActionError, SlackActionInvoker, SlackActionInvokerExt,
        SlackActionOutcome,
    };
    use everruns_core::capabilities::Capability;
    use everruns_core::host::InMemorySessionFileStore;
    use everruns_core::session_files::SessionFileSystem;
    struct Recorder(Mutex<Vec<SlackAction>>);
    #[async_trait]
    impl SlackActionInvoker for Recorder {
        async fn invoke(
            &self,
            action: SlackAction,
        ) -> Result<SlackActionOutcome, SlackActionError> {
            self.0.lock().unwrap().push(action);
            Ok(SlackActionOutcome::CodeTaskUpdated { revision: 1 })
        }
    }
    for oversized in [false, true] {
        let (workspace, mut context, id) = workspace();
        let files = Arc::new(InMemorySessionFileStore::new());
        files
            .write_file(
                context.workspace_fs_key(),
                "/workspace/review.diff",
                "STALE",
                "text",
            )
            .await
            .unwrap();
        context.file_store = Some(files);
        let diff = "diff --git a/a b/a\n--- a/a\n+++ b/a\n";
        with_sandbox(&id, |s| {
            s.review_content = diff.into();
            s.reported_size = oversized.then_some(1_000_001);
        });
        context.extensions.insert(Arc::new(workspace));
        let invoker = Arc::new(Recorder(Mutex::new(Vec::new())));
        context
            .extensions
            .insert(Arc::new(SlackActionInvokerExt(invoker.clone())));
        let tool = crate::capabilities::SlackCapability
            .tools()
            .into_iter()
            .find(|tool| tool.name() == "slack_code_task")
            .unwrap();
        let result = tool.execute_with_context(json!({
            "expected_revision":0,"context":[],"summary":"Review","diff_path":"/workspace/review.diff"
        }), &context).await;
        let seen = invoker.0.lock().unwrap();
        if oversized {
            assert!(matches!(result, ToolExecutionResult::ToolError(_)));
            assert!(seen.is_empty(), "must not publish a stale VFS copy");
        } else {
            assert!(
                matches!(result, ToolExecutionResult::Success(_)),
                "{result:?}"
            );
            let SlackAction::CodeTask { request } = &seen[0] else {
                panic!("wrong action")
            };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(request).unwrap()["diff"],
                diff
            );
        }
    }
}

#[tokio::test]
async fn secret_file_is_gone_before_the_checkpoint_and_never_in_a_command() {
    let (workspace, context, id) = workspace();
    let response = workspace
        .exec_with_secret_file(
            &context,
            "git fetch origin && ls /workspace/app",
            "/workspace",
            SECRET,
        )
        .await
        .unwrap();

    let events = with_sandbox(&id, |sandbox| sandbox.events.clone());
    let written: Vec<&String> = events
        .iter()
        .filter_map(|event| match event {
            Event::Write(path, content) => {
                assert_eq!(content, SECRET);
                Some(path)
            }
            _ => None,
        })
        .collect();
    assert_eq!(written.len(), 1);
    let path = written[0];
    assert!(path.starts_with("/tmp/.everruns-secret-"), "{path}");

    let execs: Vec<&String> = events
        .iter()
        .filter_map(|event| match event {
            Event::Exec(command) => Some(command),
            _ => None,
        })
        .collect();
    assert!(execs.iter().all(|command| !command.contains(SECRET)));
    let main = execs
        .iter()
        .find(|command| command.contains("git fetch"))
        .unwrap();
    assert!(
        main.contains(&format!("EVR_SECRET_FILE='{path}'")),
        "{main}"
    );
    assert!(main.contains("trap 'rm -f -- '\\''"), "{main}");
    assert!(main.contains("ls /home/sandbox/workspace/app"), "{main}");

    let position = |wanted: fn(&Event) -> bool| events.iter().position(wanted).unwrap();
    assert!(
        position(|e| matches!(e, Event::Write(..)))
            < position(|e| matches!(e, Event::Exec(c) if c.contains("git fetch")))
    );
    assert_eq!(events.last(), Some(&Event::Checkpoint(vec![])));
    assert_eq!(
        response.stdout, "token=[redacted] in /workspace",
        "output is redacted and mapped back to /workspace"
    );
}

#[tokio::test]
async fn a_failed_command_still_removes_the_file_before_the_checkpoint() {
    let (workspace, context, id) = workspace();
    with_sandbox(&id, |sandbox| sandbox.fail_command = true);
    let response = workspace
        .exec_with_secret_file(&context, "git fetch origin", "/workspace", SECRET)
        .await
        .unwrap();
    assert_eq!(response.exit_code, 1);
    let events = with_sandbox(&id, |sandbox| sandbox.events.clone());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Exec(c) if c.starts_with("rm -f -- '/tmp/")))
    );
    assert_eq!(events.last(), Some(&Event::Checkpoint(vec![])));
}

#[test]
fn installs_only_with_a_managed_sandbox() {
    let mut extensions = ToolContextExtensions::default();
    install_sandbox_workspace(&mut extensions, &[CapabilityRef::new("github")]);
    assert!(extensions.get::<SandboxWorkspaceExt>().is_none());

    install_sandbox_workspace(
        &mut extensions,
        &[CapabilityRef::with_config(
            SESSION_SANDBOX_CAPABILITY_ID,
            json!({"provider": PROVIDER}),
        )],
    );
    assert!(extensions.get::<SandboxWorkspaceExt>().is_some());
}

#[tokio::test]
async fn rejects_a_cwd_outside_the_workspace() {
    let (workspace, context, _) = workspace();
    assert!(
        workspace
            .exec_with_secret_file(&context, "true", "/workspace/../etc", SECRET)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn review_read_uses_the_primary_workspace_and_quotes_the_path() {
    let (workspace, context, id) = workspace();
    let diff = "diff --git a/a b/a\n--- a/a\n+++ b/a\n";
    with_sandbox(&id, |s| s.review_content = diff.into());
    let read = workspace
        .read_review_file(&context, "/workspace/o'wn.diff", 1_000_000)
        .await
        .unwrap();
    assert_eq!(read, diff);
    let events = with_sandbox(&id, |s| s.events.clone());
    assert!(events.contains(&Event::Read("/home/sandbox/workspace/o'wn.diff".into())));
    let quoted = shell_escape("/home/sandbox/workspace/o'wn.diff");
    assert!(events.iter().any(
        |e| matches!(e, Event::Exec(c) if c == &format!("test -f {quoted} && wc -c < {quoted}"))
    ));
    assert!(!events.iter().any(|e| matches!(e, Event::Checkpoint(_))));
}

#[tokio::test]
async fn review_read_refuses_oversize_growth_and_binary_content() {
    for (reported, content, encoding, should_read) in [
        (11, "short", "text", false),
        (1, "grown beyond limit", "text", true),
        (1, "YmluYXJ5", "base64", true),
    ] {
        let (workspace, context, id) = workspace();
        with_sandbox(&id, |s| {
            s.reported_size = Some(reported);
            s.review_content = content.into();
            s.review_encoding = Some(encoding.into());
        });
        assert!(
            workspace
                .read_review_file(&context, "/workspace/review.diff", 10)
                .await
                .is_err()
        );
        assert_eq!(
            with_sandbox(&id, |s| s
                .events
                .iter()
                .any(|e| matches!(e, Event::Read(_)))),
            should_read
        );
    }
}
