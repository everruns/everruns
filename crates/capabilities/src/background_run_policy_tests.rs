//! `spawn_background` must not be a way around the target tool's policy
//! (EVE-1186). These drive the tool through the real act phase, so the outer
//! call meets the same pre-tool chain a model call does, and assert that the
//! nested target call meets it too.

use super::tests::{
    InMemoryTaskRegistry, NoopStorageStore, TestFileStore, TestScheduleStore, TestSubagentDelegate,
};
use super::*;
use everruns_contracts::tool_types::{ToolCall, ToolDefinition, ToolHints, ToolResult};
use everruns_contracts::typed_id::{AgentId, EventId, HarnessId, MessageId, TurnId};
use everruns_core::background::BackgroundExecutableTool;
use everruns_core::engine::{ActAtom, ActInput, ActResult, ExecutionContext};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_task::{SessionTaskRegistry, SessionTaskState};
use everruns_core::tool_context::ToolContextServices;
use everruns_core::tool_hooks::{PostToolExecHook, PreToolUseDecision, PreToolUseHook};
use std::sync::Mutex;

/// Stand-in for the sandbox `bash` tool: background-capable, with a schema
/// that requires `command`, and a record of every command it actually ran.
#[derive(Default)]
struct RecordingBash {
    ran: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl BackgroundExecutableTool for RecordingBash {
    async fn execute_background(
        &self,
        arguments: Value,
        _context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        let command = arguments["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.ran.lock().unwrap().push(command.clone());
        sink.output("stdout", &format!("ran: {command}\n"))
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        Ok(BackgroundOutcome {
            summary: format!("ran {command}"),
            result: json!({ "stdout": format!("ran: {command}") }),
            raw_output: None,
        })
    }
}

#[async_trait]
impl Tool for RecordingBash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "test bash"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let command = arguments["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.ran.lock().unwrap().push(command.clone());
        ToolExecutionResult::success(json!({ "stdout": format!("ran: {command}") }))
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_supports_background(true)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

/// A Bash-specific policy: deny `bash` calls that delete things. It names the
/// target tool, as a guardrail `tool_pattern` rule or a user hook would.
struct DenyBashRm;

#[async_trait]
impl PreToolUseHook for DenyBashRm {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        let command = tool_call.arguments["command"].as_str().unwrap_or_default();
        if tool_call.name == "bash" && command.contains("rm ") {
            return PreToolUseDecision::Block {
                tool_call,
                reason: "bash may not delete files".to_string(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

#[derive(Clone, Default)]
struct NullEmitter;

#[async_trait]
impl EventEmitter for NullEmitter {
    async fn emit(&self, request: EventRequest) -> everruns_contracts::error::Result<Event> {
        Ok(request.into_event(EventId::new(), 0))
    }
}

struct Harness {
    session_id: SessionId,
    bash: Arc<Mutex<Vec<String>>>,
    tasks: Arc<InMemoryTaskRegistry>,
    schedules: Arc<TestScheduleStore>,
    files: Arc<TestFileStore>,
    signals: Arc<TestSubagentDelegate>,
    registry: Arc<ToolRegistry>,
}

impl Harness {
    fn new() -> Self {
        let bash = RecordingBash::default();
        let ran = bash.ran.clone();
        let registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(bash)
            .build();
        Self {
            session_id: SessionId::new(),
            bash: ran,
            tasks: Arc::new(InMemoryTaskRegistry::default()),
            schedules: Arc::new(TestScheduleStore::default()),
            files: Arc::new(TestFileStore::default()),
            signals: Arc::new(TestSubagentDelegate::default()),
            registry: Arc::new(registry),
        }
    }

    fn atom(
        &self,
        pre: Vec<Arc<dyn PreToolUseHook>>,
        post: Vec<Arc<dyn PostToolExecHook>>,
    ) -> ActAtom<ToolRegistry, NullEmitter> {
        let services = ToolContextServices {
            file_store: Some(self.files.clone()),
            storage_store: Some(Arc::new(NoopStorageStore)),
            schedule_store: Some(self.schedules.clone()),
            subagent_delegate: Some(self.signals.clone()),
            session_task_registry: Some(self.tasks.clone()),
            tool_registry: Some(self.registry.clone()),
            ..Default::default()
        };
        ActAtom::new((*self.registry).clone(), NullEmitter)
            .with_context_services(services)
            .with_pre_tool_hooks(pre)
            .with_post_tool_hooks(post)
    }

    async fn call(
        &self,
        atom: &ActAtom<ToolRegistry, NullEmitter>,
        name: &str,
        arguments: Value,
    ) -> ActResult {
        let context = ExecutionContext::new(self.session_id, TurnId::new(), MessageId::new());
        atom.execute(ActInput {
            org_id: None,
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: format!("call_{name}"),
                name: name.to_string(),
                arguments,
            }],
            tool_definitions: self.registry.tool_definitions(),
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        })
        .await
        .expect("act phase runs")
    }

    async fn background_tasks(&self) -> Vec<everruns_core::session_task::SessionTask> {
        self.tasks.list(self.session_id, None).await.unwrap()
    }

    async fn wait_for_task(&self, task_id: &str) -> everruns_core::session_task::SessionTask {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(Some(task)) = self.tasks.get(self.session_id, task_id).await
                    && task.state != SessionTaskState::Running
                {
                    break task;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background run settles")
    }

    fn ran(&self) -> Vec<String> {
        self.bash.lock().unwrap().clone()
    }
}

fn only_result(result: &ActResult) -> &ToolResult {
    assert_eq!(result.results.len(), 1);
    &result.results[0].result
}

#[tokio::test]
async fn bash_denial_also_blocks_the_same_command_through_spawn_background() {
    let harness = Harness::new();
    let atom = harness.atom(vec![Arc::new(DenyBashRm)], vec![]);

    // The policy denies the command when the model calls bash directly...
    let direct = harness
        .call(&atom, "bash", json!({ "command": "rm -rf /workspace" }))
        .await;
    assert!(
        only_result(&direct).error.is_some(),
        "direct bash is denied"
    );

    // ...and wrapping it as background work must not change the answer.
    let wrapped = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "command": "rm -rf /workspace" } }),
        )
        .await;
    let error = only_result(&wrapped)
        .error
        .clone()
        .expect("spawn_background of a denied bash command fails");
    assert!(
        error.contains("bash may not delete files"),
        "the target tool's denial is reported, got: {error}"
    );
    assert!(
        harness.background_tasks().await.is_empty(),
        "no background run is scheduled for a denied call"
    );

    // Nothing ran, in the foreground or the background.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        harness.ran().is_empty(),
        "denied command ran: {:?}",
        harness.ran()
    );
}

#[tokio::test]
async fn allowed_background_commands_still_run() {
    let harness = Harness::new();
    let atom = harness.atom(vec![Arc::new(DenyBashRm)], vec![]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "command": "ls /workspace" } }),
        )
        .await;
    let value = only_result(&result)
        .result
        .clone()
        .expect("allowed background run starts");
    assert!(only_result(&result).error.is_none(), "{value}");
    let task = harness
        .wait_for_task(value["task_id"].as_str().unwrap())
        .await;

    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(harness.ran(), vec!["ls /workspace".to_string()]);
}

/// Rewrites every bash command, the way a user `pre_tool_use` hook may.
struct ForceDryRun;

#[async_trait]
impl PreToolUseHook for ForceDryRun {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.name == "bash" {
            let command = tool_call.arguments["command"].as_str().unwrap_or_default();
            tool_call.arguments = json!({ "command": format!("{command} --dry-run") });
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

#[tokio::test]
async fn the_detached_run_executes_the_arguments_the_policy_authorized() {
    let harness = Harness::new();
    let atom = harness.atom(vec![Arc::new(ForceDryRun)], vec![]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "command": "make deploy" } }),
        )
        .await;
    let value = only_result(&result).result.clone().unwrap();
    let task = harness
        .wait_for_task(value["task_id"].as_str().unwrap())
        .await;

    assert_eq!(harness.ran(), vec!["make deploy --dry-run".to_string()]);
    // The task records what ran, so a re-attach cannot revive the original.
    assert_eq!(
        task.spec["arguments"],
        json!({ "command": "make deploy --dry-run" })
    );
}

#[tokio::test]
async fn nested_arguments_are_held_to_the_target_schema() {
    let harness = Harness::new();
    let atom = harness.atom(vec![], vec![]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "cmd": "ls" } }),
        )
        .await;

    let error = only_result(&result)
        .error
        .clone()
        .expect("invalid args fail");
    assert!(error.contains("invalid_tool_arguments"), "{error}");
    assert!(harness.background_tasks().await.is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(harness.ran().is_empty());
}

/// Defers every bash call the way the hosted `tool_approval` gate does while
/// nobody has answered.
struct DeferBash;

#[async_trait]
impl PreToolUseHook for DeferBash {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.name != "bash" {
            return PreToolUseDecision::Continue(tool_call);
        }
        let payload = json!({
            "code": everruns_contracts::tool_types::TOOL_APPROVAL_REQUIRED_CODE,
            "error": "`bash` needs a person's approval",
            "tool_call_id": tool_call.id,
            "tool": tool_call.name,
            "arguments": tool_call.arguments,
            "fingerprint": "sha256:ab",
            "risk": "destructive",
            "mode": "normal",
            "asked_at": "2026-10-01T00:00:00Z",
            "expires_at": "2026-10-01T00:15:00Z",
        });
        PreToolUseDecision::Defer {
            result: ToolResult {
                tool_call_id: String::new(),
                result: Some(payload),
                images: None,
                error: Some("`bash` needs a person's approval".to_string()),
                connection_required: None,
                raw_output: None,
            },
            tool_call,
        }
    }
}

#[tokio::test]
async fn a_target_needing_approval_parks_the_turn_instead_of_running() {
    let harness = Harness::new();
    let atom = harness.atom(vec![Arc::new(DeferBash)], vec![]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "command": "git push --force" } }),
        )
        .await;

    // The approval request names the target call, under the outer call's id,
    // so the engine parks the turn on it exactly as for a direct bash call.
    let request = everruns_contracts::tool_types::ToolApprovalRequired::from_tool_result(
        only_result(&result),
    )
    .expect("structured approval request");
    assert_eq!(request.tool, "bash");
    assert_eq!(request.tool_call_id, "call_spawn_background");
    assert_eq!(request.arguments, json!({ "command": "git push --force" }));
    assert!(
        result
            .client_tool_calls
            .iter()
            .any(|call| call.name == everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL),
        "the turn parks on an approval request"
    );
    assert!(harness.background_tasks().await.is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(harness.ran().is_empty());
}

#[tokio::test]
async fn a_denied_target_is_not_scheduled_either() {
    let harness = Harness::new();
    let atom = harness.atom(vec![Arc::new(DenyBashRm)], vec![]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({
                "tool": "bash",
                "args": { "command": "rm -rf /workspace" },
                "schedule": { "cron_expression": "*/10 * * * *" }
            }),
        )
        .await;

    assert!(only_result(&result).error.is_some());
    assert!(harness.schedules.schedules.lock().unwrap().is_empty());
    assert!(harness.background_tasks().await.is_empty());
}

/// Withholds bash output that mentions a secret, as a `tool_output`
/// guardrail does: the notice replaces the result and the raw output.
struct WithholdSecrets;

#[async_trait]
impl PostToolExecHook for WithholdSecrets {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        let seen = format!(
            "{}{}",
            result.result.clone().unwrap_or_default(),
            result.raw_output.clone().unwrap_or_default()
        );
        if tool_call.name == "bash" && seen.contains("secret") {
            result.result = Some(json!("[withheld by guardrail]"));
            result.error = None;
            result.raw_output = None;
        }
    }
}

#[tokio::test]
async fn post_tool_hooks_see_the_background_result_before_anyone_else() {
    let harness = Harness::new();
    let atom = harness.atom(vec![], vec![Arc::new(WithholdSecrets)]);

    let result = harness
        .call(
            &atom,
            "spawn_background",
            json!({ "tool": "bash", "args": { "command": "cat secret.txt" } }),
        )
        .await;
    let value = only_result(&result).result.clone().unwrap();
    let task = harness
        .wait_for_task(value["task_id"].as_str().unwrap())
        .await;
    let run_id = value["run_id"].as_str().unwrap();

    let read = |path: String| {
        let files = harness.files.clone();
        let session_id = harness.session_id;
        async move {
            use everruns_core::session_files::SessionFileSystem;
            files
                .read_file(session_id, &path)
                .await
                .unwrap()
                .and_then(|file| file.content)
                .unwrap_or_default()
        }
    };
    let result_json = read(format!("/.background/{run_id}/result.json")).await;
    let log = read(format!("/.background/{run_id}/output.log")).await;
    let signals = harness.signals.sent_messages.lock().unwrap().clone();

    assert_eq!(harness.ran(), vec!["cat secret.txt".to_string()]);
    assert_eq!(task.summary.as_deref(), Some("[withheld by guardrail]"));
    assert!(
        result_json.contains("[withheld by guardrail]"),
        "{result_json}"
    );
    for text in [&result_json, &log, &signals.join("\n")] {
        assert!(
            !text.contains("ran: cat secret.txt"),
            "output leaked: {text}"
        );
    }
}

#[tokio::test]
async fn spawn_background_refuses_to_run_outside_the_act_phase() {
    let harness = Harness::new();
    // A context without the act phase's chains: nothing to hold the target to.
    let context = ToolContext::with_stores(
        harness.session_id,
        harness.files.clone(),
        Arc::new(NoopStorageStore),
    )
    .with_tool_registry(harness.registry.clone())
    .with_session_task_registry(harness.tasks.clone());

    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({ "tool": "bash", "args": { "command": "ls" } }),
            &context,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("spawn_background must fail closed without a tool policy");
    };
    assert!(message.contains("tool policy"), "{message}");
    assert!(harness.background_tasks().await.is_empty());
}
