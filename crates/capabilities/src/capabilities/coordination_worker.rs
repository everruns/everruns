//! Worker side of the coordination capability: the tools a thread gets while it
//! works an assignment. Split from `coordination.rs` to keep both files small.

use super::*;

// =============================================================================
// Worker tools (injected into threads by PlatformToolAugmentor)
// =============================================================================

/// The open assignment a thread session is working on.
#[derive(Debug, Clone)]
pub struct ThreadAssignment {
    coordinator: SessionId,
    thread: SessionId,
    task_id: String,
    attempt: i32,
}

impl ThreadAssignment {
    fn check<'a>(
        &self,
        context: &'a ToolContext,
    ) -> Result<&'a Arc<dyn SessionTaskRegistry>, ToolExecutionResult> {
        if context.session_id != self.thread {
            return Err(ToolExecutionResult::tool_error(
                "This tool only works inside the thread that owns the assignment.",
            ));
        }
        require_registry(context)
    }

    async fn update(
        &self,
        registry: &dyn SessionTaskRegistry,
        update: SessionTaskUpdate,
    ) -> Result<SessionTask, ToolExecutionResult> {
        registry
            .update(
                self.coordinator,
                &self.task_id,
                SessionTaskUpdate {
                    expected_attempt: Some(self.attempt),
                    ..update
                },
            )
            .await
            .map_err(ToolExecutionResult::internal_error)?
            .ok_or_else(|| ToolExecutionResult::tool_error("The assignment no longer exists."))
    }

    async fn post(
        &self,
        registry: &dyn SessionTaskRegistry,
        content: Vec<TaskMessagePart>,
    ) -> Result<TaskMessage, ToolExecutionResult> {
        registry
            .record_message(
                self.coordinator,
                &self.task_id,
                NewTaskMessage {
                    direction: TaskMessageDirection::Outbound,
                    content,
                    in_reply_to: None,
                    expected_attempt: Some(self.attempt),
                },
            )
            .await
            .map_err(ToolExecutionResult::internal_error)
    }
}

/// The newest open assignment linked to `thread`, if the session is a thread.
/// Costs one session read and one task list.
pub async fn open_assignment_for_thread(
    thread: SessionId,
    session_store: &dyn SessionStore,
    task_registry: &dyn SessionTaskRegistry,
) -> everruns_contracts::error::Result<Option<ThreadAssignment>> {
    let Some(session) = session_store.get_session(thread).await? else {
        return Ok(None);
    };
    let Some(coordinator) = session.parent_session_id else {
        return Ok(None);
    };
    let latest = task_registry
        .list(
            coordinator,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_ASSIGNMENT.to_string()),
                state: None,
            }),
        )
        .await?
        .into_iter()
        .filter(|task| task.links.child_session_id == Some(thread) && !task.state.is_terminal())
        .max_by_key(|task| task.created_at);
    Ok(latest.map(|task| ThreadAssignment {
        coordinator,
        thread,
        task_id: task.id,
        attempt: task.attempt,
    }))
}

/// Worker tools bound to one open assignment.
pub fn worker_tools(assignment: &ThreadAssignment) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(UpdateChecklistTool(assignment.clone())),
        Box::new(CompleteAssignmentTool(assignment.clone())),
        Box::new(AskDecisionTool(assignment.clone())),
        Box::new(ReportToCoordinatorTool(assignment.clone())),
        Box::new(RedirectToCoordinatorTool(assignment.clone())),
    ]
}

/// Tool definitions a thread's reason step sees.
pub fn worker_tool_definitions(assignment: &ThreadAssignment) -> Vec<ToolDefinition> {
    worker_tools(assignment)
        .iter()
        .map(|tool| tool.to_definition())
        .collect()
}

struct UpdateChecklistTool(ThreadAssignment);

#[async_trait]
impl Tool for UpdateChecklistTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    context_tool!("update_checklist", "Update Checklist");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("update_checklist requires session context.")
    }

    fn description(&self) -> &str {
        "Replace this assignment's checklist. The person and the coordinator see it live. Plan the steps first, then mark each in_progress and done as you go."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "maxItems": MAX_STEPS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done", "skipped"] }
                        },
                        "required": ["title", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["steps"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let registry = match self.0.check(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let steps: Vec<ProgressStep> =
            match serde_json::from_value(arguments.get("steps").cloned().unwrap_or(Value::Null)) {
                Ok(steps) => steps,
                Err(error) => {
                    return ToolExecutionResult::tool_error(format!("Invalid steps: {error}"));
                }
            };
        if steps.len() > MAX_STEPS {
            return ToolExecutionResult::tool_error(format!(
                "A checklist holds at most {MAX_STEPS} steps."
            ));
        }
        if steps
            .iter()
            .any(|step| step.title.trim().is_empty() || step.title.len() > MAX_TITLE_BYTES)
        {
            return ToolExecutionResult::tool_error(format!(
                "Each step needs a title of 1 to {MAX_TITLE_BYTES} bytes."
            ));
        }
        let done = steps
            .iter()
            .filter(|step| step.status == ProgressStepStatus::Done)
            .count() as u64;
        let current = steps
            .iter()
            .find(|step| step.status == ProgressStepStatus::InProgress)
            .map(|step| step.title.clone());
        let total = steps.len() as u64;
        match self
            .0
            .update(
                registry.as_ref(),
                SessionTaskUpdate {
                    state_detail: current,
                    progress: Some(BackgroundProgress {
                        current: Some(done),
                        total: Some(total),
                        unit: Some("steps".to_string()),
                        label: None,
                        steps,
                    }),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(
                json!({ "status": "updated", "done": done, "total": total }),
            ),
            Err(error) => error,
        }
    }
}

struct CompleteAssignmentTool(ThreadAssignment);

#[async_trait]
impl Tool for CompleteAssignmentTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    context_tool!("complete_assignment", "Complete Assignment");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("complete_assignment requires session context.")
    }

    fn description(&self) -> &str {
        "Finish this assignment and report to the coordinator. Call it once the work is done (outcome done) or cannot be done (outcome failed). Say what you did, how you checked it, and link what you produced."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "What was done, in a few sentences, leading with the result." },
                "validation": { "type": "string", "description": "How you checked it: tests, output, a link." },
                "outcome": { "type": "string", "enum": ["done", "failed"], "default": "done" },
                "artifacts": {
                    "type": "array",
                    "maxItems": MAX_ARTIFACTS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": { "type": "string", "description": "file, url, pr, session, ..." },
                            "url": { "type": "string" },
                            "path": { "type": "string" }
                        },
                        "required": ["name", "type"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["summary"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let registry = match self.0.check(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let summary = match bounded_text(&arguments, "summary", MAX_REPORT_BYTES) {
            Ok(summary) => summary,
            Err(error) => return error,
        };
        let validation = match optional_bounded_text(&arguments, "validation", MAX_REPORT_BYTES) {
            Ok(validation) => validation,
            Err(error) => return error,
        };
        let artifacts: Vec<TaskArtifact> = match arguments.get("artifacts") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(artifacts) => artifacts,
                Err(error) => {
                    return ToolExecutionResult::tool_error(format!("Invalid artifacts: {error}"));
                }
            },
        };
        if artifacts.len() > MAX_ARTIFACTS {
            return ToolExecutionResult::tool_error(format!(
                "At most {MAX_ARTIFACTS} artifacts per assignment."
            ));
        }
        let failed = arguments.get("outcome").and_then(Value::as_str) == Some("failed");
        let report = match validation {
            Some(validation) => format!("{summary}\n\nValidation: {validation}"),
            None => summary.to_string(),
        };
        match self
            .0
            .update(
                registry.as_ref(),
                SessionTaskUpdate {
                    state: Some(if failed {
                        SessionTaskState::Failed
                    } else {
                        SessionTaskState::Succeeded
                    }),
                    summary: Some(report.clone()),
                    artifacts: Some(artifacts),
                    error: failed.then(|| TaskError {
                        kind: "assignment_failed".to_string(),
                        message: truncate_summary(summary),
                    }),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(task) if task.state.is_terminal() => ToolExecutionResult::success(json!({
                "status": task.state.to_string(),
                "note": "The coordinator was told. Reply to the person in a sentence, then stop.",
            })),
            Ok(_) => ToolExecutionResult::tool_error("The assignment was already closed."),
            Err(error) => error,
        }
    }
}

pub(super) struct AskDecisionTool(pub(super) ThreadAssignment);

#[async_trait]
impl Tool for AskDecisionTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    context_tool!("ask_decision", "Ask for a Decision");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("ask_decision requires session context.")
    }

    fn description(&self) -> &str {
        "Ask the person a decision only they can make. Ask once: a short question naming the action, up to four options, one recommended. Keep working on the recommended option when you can; otherwise end your turn and wait."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": { "type": "string" },
                "options": { "type": "array", "items": { "type": "string" }, "maxItems": MAX_OPTIONS },
                "recommended": { "type": "integer", "minimum": 0, "description": "Index of the recommended option." }
            },
            "required": ["question"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let registry = match self.0.check(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let question = match bounded_text(&arguments, "question", MAX_REPORT_BYTES) {
            Ok(question) => question,
            Err(error) => return error,
        };
        let options: Vec<String> = arguments
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if options.len() > MAX_OPTIONS {
            return ToolExecutionResult::tool_error(format!("At most {MAX_OPTIONS} options."));
        }
        let recommended = arguments.get("recommended").and_then(Value::as_u64);
        if recommended.is_some_and(|index| index as usize >= options.len()) {
            return ToolExecutionResult::tool_error("recommended must index an option.");
        }
        let request = TaskInputRequest {
            id: format!("ask_{}", uuid::Uuid::now_v7().simple()),
            prompt: question.to_string(),
            expected: (!options.is_empty())
                .then(|| json!({ "options": options, "recommended": recommended })),
        };
        match self
            .0
            .update(
                registry.as_ref(),
                SessionTaskUpdate {
                    input_request: Some(request),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "status": "asked",
                "note": "Put the question to the person in your reply too. The answer arrives as your next message.",
            })),
            Err(error) => error,
        }
    }
}

struct ReportToCoordinatorTool(ThreadAssignment);

#[async_trait]
impl Tool for ReportToCoordinatorTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    context_tool!("report_to_coordinator", "Report to Coordinator");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("report_to_coordinator requires session context.")
    }

    fn description(&self) -> &str {
        "Tell the coordinator something it should know before you finish: a finding that changes the plan, or news another thread needs. Not for routine progress; keep the checklist for that."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "message": { "type": "string" } },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let registry = match self.0.check(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let message = match bounded_text(&arguments, "message", MAX_REPORT_BYTES) {
            Ok(message) => message,
            Err(error) => return error,
        };
        match self
            .0
            .post(registry.as_ref(), vec![TaskMessagePart::text(message)])
            .await
        {
            Ok(stored) => {
                ToolExecutionResult::success(json!({ "status": "sent", "message_id": stored.id }))
            }
            Err(error) => error,
        }
    }
}

pub(super) struct RedirectToCoordinatorTool(pub(super) ThreadAssignment);

#[async_trait]
impl Tool for RedirectToCoordinatorTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        narrate_coordination(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    context_tool!("redirect_to_coordinator", "Redirect to Coordinator");

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("redirect_to_coordinator requires session context.")
    }

    fn description(&self) -> &str {
        "Hand a request back to the coordinator when it does not belong in this thread (other work, another thread's topic, or out of your scope). The coordinator routes it; you keep working on your own assignment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request": { "type": "string", "description": "The request to route, in the person's words." },
                "reason": { "type": "string", "description": "Why it belongs elsewhere." }
            },
            "required": ["request", "reason"],
            "additionalProperties": false
        })
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let registry = match self.0.check(context) {
            Ok(registry) => registry,
            Err(error) => return error,
        };
        let request = match bounded_text(&arguments, "request", MAX_BRIEF_BYTES) {
            Ok(request) => request,
            Err(error) => return error,
        };
        let reason = match bounded_text(&arguments, "reason", MAX_REPORT_BYTES) {
            Ok(reason) => reason,
            Err(error) => return error,
        };
        let content = vec![
            TaskMessagePart::text(format!(
                "Redirect: this belongs elsewhere ({reason}). Request: {request}"
            )),
            TaskMessagePart::Data {
                data: json!({ "type": "redirect", "reason": reason, "request": request }),
            },
        ];
        match self.0.post(registry.as_ref(), content).await {
            Ok(stored) => ToolExecutionResult::success(json!({
                "status": "redirected",
                "message_id": stored.id,
                "note": "Tell the person the coordinator will pick this up.",
            })),
            Err(error) => error,
        }
    }
}

// =============================================================================
// Thread turn settlement (shared by the server listener and the framework)
// =============================================================================

/// How a thread's turn changed, as far as its assignment cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadTurn {
    Started,
    Completed,
    Cancelled,
    Failed,
}

/// Apply one thread turn to the thread's open assignment, if it has one.
///
/// A thread finishes an assignment with `complete_assignment`. A turn that ends
/// without it flags the assignment as needing attention, which wakes the
/// coordinator; a failed turn fails it; a turn that starts while the
/// assignment waits on an answer puts it back to running.
pub async fn settle_thread_turn(
    registry: &dyn SessionTaskRegistry,
    coordinator: SessionId,
    thread: SessionId,
    turn: ThreadTurn,
) -> everruns_contracts::error::Result<()> {
    let Some(assignment) = registry
        .list(
            coordinator,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_ASSIGNMENT.to_string()),
                state: None,
            }),
        )
        .await?
        .into_iter()
        .filter(|task| task.links.child_session_id == Some(thread) && !task.state.is_terminal())
        .max_by_key(|task| task.created_at)
    else {
        return Ok(());
    };

    let update = match (turn, assignment.state) {
        (ThreadTurn::Started, SessionTaskState::AwaitingInput) => SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
        (
            ThreadTurn::Completed | ThreadTurn::Cancelled,
            SessionTaskState::Running | SessionTaskState::Queued,
        ) => {
            let how = if turn == ThreadTurn::Cancelled {
                "was cancelled"
            } else {
                "ended its turn"
            };
            SessionTaskUpdate {
                input_request: Some(TaskInputRequest {
                    id: format!("stopped_{}", uuid::Uuid::now_v7().simple()),
                    prompt: format!(
                        "The thread {how} without completing the assignment. Read its last reply with get_thread, then message it or tell the person."
                    ),
                    expected: None,
                }),
                ..Default::default()
            }
        }
        (ThreadTurn::Failed, _) => SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            error: Some(TaskError {
                kind: "turn_failed".to_string(),
                message: "The thread's turn failed.".to_string(),
            }),
            ..Default::default()
        },
        _ => return Ok(()),
    };
    registry
        .update(
            coordinator,
            &assignment.id,
            SessionTaskUpdate {
                expected_attempt: Some(assignment.attempt),
                ..update
            },
        )
        .await?;
    Ok(())
}
