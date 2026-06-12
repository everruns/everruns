use super::queries as q;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use everruns_core::SessionTask;
use everruns_core::session_task::{
    NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState, TaskMessage,
    TaskMessagePart, find_task_executor,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn registry_err(e: everruns_core::AgentLoopError) -> CommandError {
    CommandError::internal(anyhow::anyhow!(e))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionTasks {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled).
    #[serde(default)]
    pub state: Option<String>,
    /// Optional kind filter (subagent, external_agent, background_tool, ...).
    #[serde(default)]
    pub kind: Option<String>,
}

impl Command for ListSessionTasks {
    type Output = Vec<SessionTask>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_tasks",
            category: "session_tasks",
            description: "List background tasks owned by a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionTask>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        let state = match self.state.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(SessionTaskState::parse(raw).ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                ))
            })?),
            None => None,
        };
        let filter = SessionTaskFilter {
            kind: self.kind,
            state,
        };
        q::registry_for_ctx(ctx)
            .list(session_id, Some(&filter))
            .await
            .map_err(registry_err)
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionTasks>() }

/// Task snapshot plus the recent message thread.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionTaskDetail {
    pub task: SessionTask,
    pub messages: Vec<TaskMessage>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for GetSessionTask {
    type Output = SessionTaskDetail;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_task",
            category: "session_tasks",
            description: "Get one session task with its recent message thread.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks/{task_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTaskDetail, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let task = q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let messages = q::registry_for_ctx(ctx)
            .list_messages(session_id, &self.task_id, Some(50))
            .await
            .map_err(registry_err)?;
        Ok(SessionTaskDetail { task, messages })
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionTask>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostSessionTaskMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Plain-text message (alternative to `content`).
    #[serde(default)]
    pub text: Option<String>,
    /// Structured message parts (alternative to `text`).
    #[serde(default)]
    pub content: Option<Vec<TaskMessagePart>>,
    /// Input request ID this message answers, when applicable.
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

impl Command for PostSessionTaskMessage {
    type Output = TaskMessage;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "post_session_task_message",
            category: "session_tasks",
            description: "Send an inbound message to a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/messages",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<TaskMessage, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        let content = match (self.content, self.text) {
            (Some(content), _) if !content.is_empty() => content,
            (_, Some(text)) if !text.trim().is_empty() => vec![TaskMessagePart::text(text)],
            _ => {
                return Err(CommandError::bad_request(
                    "Message requires non-empty `content` parts or `text`",
                ));
            }
        };
        let task_id = self.task_id.clone();
        let mut message = NewTaskMessage {
            direction: everruns_core::session_task::TaskMessageDirection::Inbound,
            content,
            in_reply_to: self.in_reply_to,
            // API writers are unfenced: user messages apply regardless of
            // executor attempt.
            expected_attempt: None,
        };
        message.in_reply_to = message.in_reply_to.filter(|s| !s.is_empty());

        let recorded = q::registry_for_ctx(ctx)
            .record_message(session_id, &task_id, message)
            .await
            .map_err(registry_err)?;

        // Best-effort executor delivery: re-fetch the task (it may have
        // transitioned to running if the message answered an input request),
        // then call deliver so the running work receives the message.
        // The message is durably recorded regardless — a delivery error (or
        // a re-fetch error) is logged but never fails the HTTP call.
        let refetch = q::registry_for_ctx(ctx).get(session_id, &task_id).await;
        match refetch {
            Ok(Some(task)) if find_task_executor(&task.kind).is_some() => {
                let executor = find_task_executor(&task.kind).unwrap();
                match q::tool_context_for_ctx(ctx, session_id).await {
                    Ok(tool_ctx) => {
                        if let Err(e) = executor.deliver(&task, &recorded, &tool_ctx).await {
                            tracing::warn!(
                                task_id = %task_id,
                                kind = %task.kind,
                                error = %e,
                                "PostSessionTaskMessage: executor deliver failed (best-effort; message is recorded)"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %e,
                            "PostSessionTaskMessage: could not build ToolContext; skipping executor deliver (message is recorded)"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "PostSessionTaskMessage: re-fetch after record failed; skipping executor deliver (message is recorded)"
                );
            }
            _ => {}
        }

        Ok(recorded)
    }
}

inventory::submit! { CommandDescriptor::of::<PostSessionTaskMessage>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for CancelSessionTask {
    type Output = SessionTask;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "cancel_session_task",
            category: "session_tasks",
            description: "Request cooperative cancellation of a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTask, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        let task = q::registry_for_ctx(ctx)
            .request_cancel(session_id, &self.task_id)
            .await
            .map_err(registry_err)?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        // Best-effort executor cancel: skip for already-terminal tasks to
        // avoid side effects (e.g. disabling a schedule for a task that
        // already succeeded). For non-terminal tasks, invoke the kind-specific
        // cancel so active executors (notably MonitorTaskExecutor) can act
        // immediately. MonitorTaskExecutor.cancel disables the linked schedule
        // AND transitions the task to Canceled in the registry — so we
        // re-fetch the task afterwards to return the freshest snapshot.
        if !task.state.is_terminal()
            && let Some(executor) = find_task_executor(&task.kind)
        {
            match q::tool_context_for_ctx(ctx, session_id).await {
                Ok(tool_ctx) => {
                    if let Err(e) = executor.cancel(&task, &tool_ctx).await {
                        tracing::warn!(
                            task_id = %task.id,
                            kind = %task.kind,
                            error = %e,
                            "CancelSessionTask: executor cancel failed (best-effort)"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task.id,
                        error = %e,
                        "CancelSessionTask: could not build ToolContext; skipping executor cancel (cancel intent is recorded)"
                    );
                }
            }
        }

        // Re-fetch to return the freshest snapshot: the executor may have
        // transitioned the task to Canceled (e.g. MonitorTaskExecutor does).
        // On re-fetch error, return the previously fetched snapshot rather
        // than failing the HTTP call (cancel intent is already recorded).
        let fresh = match q::registry_for_ctx(ctx)
            .get(session_id, &self.task_id)
            .await
        {
            Ok(Some(t)) => t,
            Ok(None) => task,
            Err(e) => {
                tracing::warn!(
                    task_id = %self.task_id,
                    error = %e,
                    "CancelSessionTask: re-fetch after cancel failed; returning snapshot (cancel intent is recorded)"
                );
                task
            }
        };

        Ok(fresh)
    }
}

inventory::submit! { CommandDescriptor::of::<CancelSessionTask>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateHarnessRow;
    use crate::storage::{CreateSessionRow, StorageBackend};
    use everruns_core::network_access::NetworkAccessList;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, SessionTaskState, TASK_KIND_BACKGROUND_TOOL,
        TASK_KIND_MONITOR, TaskLinks, TaskWakePolicy,
    };
    use everruns_core::{Caller, DEFAULT_ORG_ID, PrincipalId};
    use std::sync::Arc;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Build a minimal test Ctx backed by an in-memory StorageBackend.
    fn test_ctx(db: Arc<StorageBackend>) -> Ctx {
        Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
    }

    /// Create a session in the in-memory database, returning its ID.
    async fn create_session(db: &Arc<StorageBackend>) -> everruns_core::SessionId {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("test session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .await
        .unwrap()
        .id
    }

    // -------------------------------------------------------------------------
    // CancelSessionTask — monitor task
    // -------------------------------------------------------------------------

    /// API cancel of a monitor task must disable the linked schedule via
    /// MonitorTaskExecutor and return the task in Canceled state.
    #[tokio::test]
    async fn cancel_monitor_task_cancels_linked_schedule() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a schedule directly in storage so we have a real schedule_id.
        let schedule = db
            .create_session_schedule(crate::storage::CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                description: "test monitor schedule".to_string(),
                cron_expression: Some("0 * * * *".to_string()),
                scheduled_at: None,
                timezone: "UTC".to_string(),
                next_trigger_at: None,
            })
            .await
            .unwrap();
        assert!(schedule.enabled, "schedule must start enabled");
        let schedule_id = schedule.id;

        // Create a monitor task with the schedule_id in spec.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Test Monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Call the cancel command via execute (bypassing policy).
        let result = CancelSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        // Task must be Canceled (MonitorTaskExecutor transitions it).
        assert_eq!(
            result.state,
            SessionTaskState::Canceled,
            "monitor task must be Canceled after API cancel"
        );

        // Schedule must be disabled (MonitorTaskExecutor called cancel_schedule).
        let updated_schedule = db
            .get_session_schedule(DEFAULT_ORG_ID, schedule_id)
            .await
            .unwrap()
            .expect("schedule must still exist");
        assert!(
            !updated_schedule.enabled,
            "schedule must be disabled after monitor task cancel"
        );
    }

    // -------------------------------------------------------------------------
    // PostSessionTaskMessage — unknown/no-executor kind
    // -------------------------------------------------------------------------

    /// Posting a message to a task whose kind has no executor registered
    /// must still return 200 and a recorded TaskMessage (no executor → no-op
    /// delivery, message is durably stored).
    #[tokio::test]
    async fn post_message_to_no_executor_kind_returns_recorded_message() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a task with a kind that has no registered executor.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "unknown_test_kind".to_string(),
                display_name: "Unknown Kind Task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("hello from API".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await
        .unwrap();

        // The returned message must be the recorded inbound message.
        assert_eq!(
            result.task_id, task.id,
            "recorded message must belong to the task"
        );
        assert_eq!(
            result.direction,
            everruns_core::session_task::TaskMessageDirection::Inbound
        );

        // The message thread must contain the message.
        let messages = registry
            .list_messages(session_id, &task.id, Some(10))
            .await
            .unwrap();
        assert_eq!(messages.len(), 1, "message must be persisted");
    }

    // -------------------------------------------------------------------------
    // PostSessionTaskMessage — background_tool kind (executor registered, deliver
    // returns unsupported — best-effort: HTTP call still succeeds)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn post_message_to_background_tool_still_returns_200() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "Background Tool Task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // BackgroundToolTaskExecutor.deliver returns an error (unsupported).
        // The command must still return Ok with the recorded message.
        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("steer the tool".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(result.task_id, task.id);
        let messages = registry
            .list_messages(session_id, &task.id, Some(10))
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    // -------------------------------------------------------------------------
    // CancelSessionTask — terminal task must not invoke executor
    // -------------------------------------------------------------------------

    /// API cancel of an already-terminal (succeeded) monitor task must NOT
    /// invoke the executor. The linked schedule must remain enabled because
    /// MonitorTaskExecutor.cancel was never called.
    #[tokio::test]
    async fn cancel_terminal_monitor_task_skips_executor() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a schedule that must remain enabled.
        let schedule = db
            .create_session_schedule(crate::storage::CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                description: "terminal monitor schedule".to_string(),
                cron_expression: Some("0 * * * *".to_string()),
                scheduled_at: None,
                timezone: "UTC".to_string(),
                next_trigger_at: None,
            })
            .await
            .unwrap();
        let schedule_id = schedule.id;
        assert!(schedule.enabled, "schedule must start enabled");

        // Create the monitor task already in Succeeded state.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Succeeded Monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Succeeded,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // API cancel on a terminal task.
        let result = CancelSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        // request_cancel records cancel_requested_at but does NOT change state
        // for terminal tasks (registry invariant); the returned task reflects
        // the terminal state.
        assert_eq!(
            result.state,
            SessionTaskState::Succeeded,
            "terminal task state must not regress after API cancel"
        );

        // Schedule must still be enabled — MonitorTaskExecutor.cancel was NOT called.
        let updated_schedule = db
            .get_session_schedule(DEFAULT_ORG_ID, schedule_id)
            .await
            .unwrap()
            .expect("schedule must still exist");
        assert!(
            updated_schedule.enabled,
            "schedule must remain enabled when cancel skips executor for terminal task"
        );
    }

    // -------------------------------------------------------------------------
    // tool_context_for_ctx — network_access is populated from folded overlays
    // -------------------------------------------------------------------------

    /// The factory must derive the effective network ACL by folding harness →
    /// agent → session overlays and set it on the returned ToolContext.
    ///
    /// Test setup:
    ///   harness allows [a.example.com, b.example.com]
    ///   session allows [b.example.com]
    ///   expected intersection: [b.example.com]  (a.example.com is narrowed out)
    #[tokio::test]
    async fn tool_context_for_ctx_populates_network_access() {
        let db = Arc::new(StorageBackend::in_memory());

        // Create a harness with a network_access list.
        let harness_network_access =
            NetworkAccessList::allow_only(["a.example.com", "b.example.com"]);
        let harness = db
            .create_harness(
                DEFAULT_ORG_ID,
                CreateHarnessRow {
                    name: "acl-test-harness".to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: String::new(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: Some(serde_json::to_value(&harness_network_access).unwrap()),
                    is_built_in: false,
                },
            )
            .await
            .unwrap();

        // Create a session with a narrower network_access list, linked to the harness.
        let session_network_access = NetworkAccessList::allow_only(["b.example.com"]);
        let session_id = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(harness.id),
                agent_id: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("acl test session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: Some(serde_json::to_value(&session_network_access).unwrap()),
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
            })
            .await
            .unwrap()
            .id;

        let ctx = test_ctx(db.clone());
        let tool_ctx = q::tool_context_for_ctx(&ctx, session_id)
            .await
            .expect("tool_context_for_ctx must succeed");

        let acl = tool_ctx
            .network_access
            .expect("network_access must be populated");

        // b.example.com is in both layers → allowed after intersection.
        assert!(
            acl.is_url_allowed("https://b.example.com/ok"),
            "b.example.com must be allowed (in both harness and session)"
        );
        // a.example.com is only in harness, not session → blocked by intersection.
        assert!(
            !acl.is_url_allowed("https://a.example.com/ok"),
            "a.example.com must be blocked (not in session allow list)"
        );
        // Unrelated host must be blocked.
        assert!(
            !acl.is_url_allowed("https://other.example.com/ok"),
            "other.example.com must be blocked"
        );
    }
}
