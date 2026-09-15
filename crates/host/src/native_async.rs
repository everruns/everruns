//! Private local persistence for custom hosts using native async tools. The
//! journal directory must not be exposed through session tools or shared with
//! another conversation. Distributed hosts need an equivalent fenced store.

use async_trait::async_trait;
use everruns_engine::native_async::NativeAsyncJournal;
use everruns_provider::{
    error::{AgentLoopError, Result},
    native_async::NativeAsyncCheckpoint,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct FileNativeAsyncJournal {
    directory: PathBuf,
    // Held for the entire coordinator lifetime. OS locks release on process death.
    _owner: File,
    writer: Mutex<()>,
}

impl FileNativeAsyncJournal {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory).map_err(store_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(store_error)?;
        }
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("owner.lock"))
            .map_err(store_error)?;
        owner.try_lock().map_err(|_| {
            AgentLoopError::store("native async journal already has an execution owner")
        })?;
        Ok(Self {
            directory,
            _owner: owner,
            writer: Mutex::new(()),
        })
    }
}

fn store_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(format!("native async journal: {error}"))
}

#[async_trait]
impl NativeAsyncJournal for FileNativeAsyncJournal {
    async fn load(&self) -> Result<NativeAsyncCheckpoint> {
        let _guard = self.writer.lock().map_err(store_error)?;
        match fs::read(self.directory.join("checkpoint.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(store_error),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(NativeAsyncCheckpoint::default())
            }
            Err(error) => Err(store_error(error)),
        }
    }
    async fn save(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<()> {
        let _guard = self.writer.lock().map_err(store_error)?;
        let data = serde_json::to_vec(checkpoint).map_err(store_error)?;
        let temporary = self.directory.join("checkpoint.next");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(store_error)?;
        file.write_all(&data).map_err(store_error)?;
        file.sync_all().map_err(store_error)?;
        fs::rename(temporary, self.directory.join("checkpoint.json")).map_err(store_error)?;
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(store_error)?;
        Ok(())
    }
}

/// Shared durable journal acquired through a host's database or gRPC store.
/// A dropped worker loses ownership after the database lease expires.
#[derive(Clone)]
pub struct SharedNativeAsyncJournal {
    store: std::sync::Arc<dyn everruns_core::native_async_store::NativeAsyncStore>,
    lease: everruns_core::native_async_store::NativeAsyncLease,
}
impl SharedNativeAsyncJournal {
    pub async fn acquire(
        store: std::sync::Arc<dyn everruns_core::native_async_store::NativeAsyncStore>,
        org_id: i64,
        session_id: everruns_provider::typed_id::SessionId,
        turn_id: everruns_provider::typed_id::TurnId,
    ) -> Result<Self> {
        let lease = everruns_core::native_async_store::NativeAsyncLease {
            org_id,
            session_id,
            turn_id,
            owner: uuid::Uuid::new_v4(),
        };
        store.acquire(lease).await?;
        Ok(Self { store, lease })
    }
}
#[async_trait]
impl NativeAsyncJournal for SharedNativeAsyncJournal {
    async fn load(&self) -> Result<NativeAsyncCheckpoint> {
        self.store.load(self.lease).await
    }
    async fn save(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<()> {
        self.store.save(self.lease, checkpoint).await
    }
    async fn heartbeat(&self) -> Result<()> {
        self.store.renew(self.lease).await
    }
    async fn release(&self) -> Result<()> {
        self.store.release(self.lease).await
    }
}

/// Executes native calls through the same Act pipeline as ordinary runtime tools.
/// The template supplies already-resolved org/session scope and tool definitions.
/// This initial adapter excludes client-side and approval-gated calls entirely.
pub struct RuntimeNativeAsyncExecutor<A: crate::RuntimeHostAdapter> {
    adapter: A,
    template: everruns_engine::ActInput,
}

impl<A: crate::RuntimeHostAdapter> RuntimeNativeAsyncExecutor<A> {
    pub fn new(adapter: A, mut template: everruns_engine::ActInput) -> Self {
        template.tool_calls.clear();
        Self { adapter, template }
    }
}

#[async_trait]
impl<A: crate::RuntimeHostAdapter> everruns_engine::native_async::NativeAsyncExecutor
    for RuntimeNativeAsyncExecutor<A>
{
    async fn authorize(
        &self,
        call: &everruns_provider::native_async::NativeToolCall,
    ) -> Result<everruns_engine::native_async::NativeCallPolicy> {
        use everruns_provider::tool_types::{SideEffectClass, ToolPolicy};
        let definition = self
            .template
            .tool_definitions
            .iter()
            .find(|definition| definition.name() == call.name())
            .ok_or_else(|| {
                AgentLoopError::config("native tool is not in the authorized tool set")
            })?;
        if definition.policy() != &ToolPolicy::Auto {
            return Err(AgentLoopError::config(
                "native coordinator cannot execute approval-gated or client-side tools",
            ));
        }
        // web_fetch also supports an optional file-saving mode despite its
        // read-only hint. Keep that mode on the ordinary synchronous path.
        let saves_file = if let everruns_provider::native_async::NativeToolCall::Function {
            name,
            arguments,
            ..
        } = call
        {
            name == "web_fetch"
                && serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .is_some_and(|args| {
                        args.get("save_to_file")
                            .is_some_and(|value| !value.is_null() && value != false)
                    })
        } else {
            false
        };
        if saves_file {
            return Err(AgentLoopError::config(
                "native lookup execution excludes file-saving mode",
            ));
        }
        Ok(everruns_engine::native_async::NativeCallPolicy {
            allow_async: definition.hints().readonly == Some(true)
                && definition.hints().concurrency_class.is_none()
                && !saves_file,
            replay_safe: matches!(
                definition.hints().effective_side_effect_class(),
                SideEffectClass::Pure | SideEffectClass::Idempotent
            ),
            concurrency_class: definition.hints().concurrency_class.clone(),
        })
    }
    async fn execute(
        &self,
        call: everruns_provider::native_async::NativeToolCall,
    ) -> Result<String> {
        use everruns_provider::native_async::NativeToolCall;
        let arguments = match &call {
            NativeToolCall::Function { arguments, .. } => serde_json::from_str(arguments)
                .map_err(|_| AgentLoopError::tool("invalid native tool arguments"))?,
            // Custom tools are raw-string tools. The host's implementation owns
            // validation of that string, including its grammar when configured.
            NativeToolCall::Custom { input, .. } => serde_json::Value::String(input.clone()),
        };
        let mut input = self.template.clone();
        input.context.exec_id = everruns_provider::typed_id::ExecId::new();
        input.tool_calls = vec![everruns_provider::tool_types::ToolCall {
            id: call.id().to_owned(),
            name: call.name().to_owned(),
            arguments,
        }];
        let outcome = crate::execute_act_activity(&self.adapter, input)
            .await
            .map_err(|error| AgentLoopError::tool(error.user_facing_message()))?;
        if outcome.blocked || outcome.waiting_for_tool_results {
            return Err(AgentLoopError::tool(
                "native tool paused for required user action; resolve the action before retrying",
            ));
        }
        let result = outcome
            .results
            .into_iter()
            .next()
            .ok_or_else(|| AgentLoopError::tool("native tool execution returned no result"))?
            .result;
        serde_json::to_string(&serde_json::json!({"result":result.result,"error":result.error,"images":result.images})).map_err(store_error)
    }
}

/// Recovery and cancellation can settle a call without running Act again.
/// Persist those terminal errors before delivery so later history cannot invent
/// a missing-tool retry. A completed Act result already has its canonical event.
async fn persist_terminal_errors<A: crate::RuntimeHostAdapter>(
    adapter: &A,
    context: &everruns_core::ExecutionContext,
    checkpoint: &NativeAsyncCheckpoint,
) -> Result<()> {
    use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
    use everruns_provider::native_async::PendingCallState;
    let errors: Vec<_> = checkpoint
        .calls
        .values()
        .filter_map(|pending| {
            let PendingCallState::Ready { output } = &pending.state else {
                return None;
            };
            let value: serde_json::Value = serde_json::from_str(output).ok()?;
            Some((
                pending.call.id(),
                pending.call.name(),
                value.get("error")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    let messages = adapter.message_store().load(context.session_id).await?;
    for (id, name, error) in errors {
        if messages
            .iter()
            .any(|message| message.tool_call_id() == Some(id))
        {
            continue;
        }
        adapter
            .event_emitter()
            .emit(EventRequest::new(
                context.session_id,
                EventContext::from_execution_context(context),
                ToolCompletedData::failure(
                    id.to_owned(),
                    name.to_owned(),
                    if error == "cancelled" {
                        "cancelled"
                    } else {
                        "interrupted"
                    }
                    .to_owned(),
                    error,
                    None,
                ),
            ))
            .await?;
    }
    Ok(())
}

fn recover_host_progress(
    checkpoint: &mut NativeAsyncCheckpoint,
) -> Result<(Option<everruns_core::events::TokenUsage>, Option<u64>)> {
    if checkpoint.transcript_message_id.is_none()
        && checkpoint.host_responses.len() != checkpoint.completed_responses as usize
    {
        return Err(AgentLoopError::store(
            "native response summaries require reconciliation",
        ));
    }
    // A retry after the canonical final message must return its saved summary,
    // even if the activity died before saving the aggregate outcome.
    let mut saved_responses: Vec<everruns_engine::ReasonResult> = checkpoint
        .host_responses
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()
        .map_err(store_error)?;
    let mut saved_usage: Option<everruns_core::events::TokenUsage> = None;
    let mut first_token_ms = None;
    for result in &saved_responses {
        first_token_ms = first_token_ms.or(result.time_to_first_token_ms);
        if let Some(next) = &result.usage {
            if let Some(total) = &mut saved_usage {
                total.add(next);
            } else {
                saved_usage = Some(next.clone());
            }
        }
    }
    if checkpoint.host_outcome.is_none()
        && checkpoint.can_complete()
        && let Some(mut result) = saved_responses.pop()
    {
        result.native_counts = Some(everruns_engine::NativeExecutionCounts {
            llm_calls: checkpoint.completed_responses,
            tool_calls: checkpoint.calls.len().min(u32::MAX as usize) as u32,
        });
        result.tool_calls.clear();
        result.has_tool_calls = false;
        result.usage = saved_usage.clone();
        result.time_to_first_token_ms = first_token_ms;
        checkpoint.host_outcome = Some(serde_json::to_value(result).map_err(store_error)?);
    }
    Ok((saved_usage, first_token_ms))
}

/// Run internal provider continuations within the durable Reason activity. The
/// scheduler sees completion only after every original-call output has a receipt.
pub(crate) async fn execute_reason<A: crate::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: everruns_engine::ReasonInput,
    assembled: everruns_core::AssembledTurnContext,
    atom: everruns_engine::ReasonAtom,
) -> Result<everruns_engine::ReasonResult> {
    use everruns_engine::native_async::NativeAsyncCoordinator;
    use std::sync::Arc;
    let registry = adapter.capability_registry();
    let mut tools = std::collections::BTreeMap::new();
    for config in &assembled.resolved_capability_configs {
        if let Some(capability) = registry.get(config.id())
            && let Some(selected) = capability.native_async_tools(config.config_value())
        {
            tools.extend(selected);
        }
    }
    if tools.is_empty() {
        return atom.execute_with_assembled_context(input, assembled).await;
    }
    let original_driver = assembled.model.driver.clone();
    let supported = original_driver
        .native_async_driver(&assembled.runtime_agent.model, tools.clone(), None)
        .is_some();
    let Some(store) = adapter.native_async_store() else {
        if !supported {
            return atom.execute_with_assembled_context(input, assembled).await;
        }
        return Err(AgentLoopError::config(
            "native async requires a shared durable journal",
        ));
    };
    let journal = SharedNativeAsyncJournal::acquire(
        store,
        org_id,
        input.context.session_id,
        input.context.turn_id,
    )
    .await?;
    let mut checkpoint = journal.load().await?;
    if let Some(id) = checkpoint.transcript_message_id.as_ref()
        && !checkpoint.response_in_flight
    {
        let message_id = id
            .parse()
            .map_err(|_| AgentLoopError::store("invalid native transcript message ID"))?;
        if let Some(message) = adapter
            .message_store()
            .get(input.context.session_id, message_id)
            .await?
        {
            let response_id = message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("response_id"))
                .and_then(serde_json::Value::as_str);
            if response_id == checkpoint.latest_response_id.as_deref()
                && checkpoint.host_responses.len() == checkpoint.completed_responses as usize
            {
                checkpoint.transcript_message_id = None;
                journal.save(&checkpoint).await?;
            }
        }
    }
    let previous_outcome = checkpoint.host_outcome.clone();
    let (saved_usage, mut first_token_ms) = recover_host_progress(&mut checkpoint)?;
    if checkpoint.host_outcome != previous_outcome {
        journal.save(&checkpoint).await?;
    }
    if let Some(outcome) = checkpoint.host_outcome.clone() {
        let outcome = serde_json::from_value(outcome).map_err(store_error)?;
        journal.release().await?;
        return Ok(outcome);
    }
    if !supported {
        if !checkpoint.can_complete() {
            return Err(AgentLoopError::config(
                "pending native outputs cannot switch providers or models",
            ));
        }
        journal.release().await?;
        return atom.execute_with_assembled_context(input, assembled).await;
    }
    if assembled.compaction_policy.is_some() {
        return Err(AgentLoopError::config(
            "native async cannot cross compaction boundaries",
        ));
    }
    for config in &assembled.resolved_capability_configs {
        if let Some(capability) = registry.get(config.id())
            && (!capability.output_guardrails().is_empty()
                || !capability
                    .post_output_guardrails_with_config(config.config_value())
                    .is_empty()
                || capability
                    .finalized_tool_calls_hook(config.config_value())
                    .is_some())
        {
            return Err(AgentLoopError::config(
                "native early dispatch is incompatible with finalized-output or tool-call hooks",
            ));
        }
    }
    let template = everruns_engine::ActInput {
        org_id: Some(org_id),
        context: input.context.clone(),
        harness_id: input.harness_id,
        agent_id: input.agent_id,
        tool_calls: vec![],
        tool_definitions: assembled.runtime_agent.tools.clone(),
        locale: assembled.resolved_locale.clone(),
        blueprint_id: assembled.snapshot.blueprint_id.clone(),
        network_access: assembled.runtime_agent.network_access.clone(),
        parallel_tool_calls: assembled.runtime_agent.parallel_tool_calls,
    };
    let executor = Arc::new(RuntimeNativeAsyncExecutor::new(adapter.clone(), template));
    let lease_heartbeat = journal.clone();
    let coordinator = Arc::new(tokio::sync::Mutex::new(
        NativeAsyncCoordinator::open(
            Box::new(journal),
            executor,
            everruns_engine::configured_max_tool_concurrency(),
            assembled.runtime_agent.parallel_tool_calls != Some(false),
        )
        .await?,
    ));
    let atom = atom.with_native_async(coordinator.clone());
    let outcome = {
        let execution = async {
            let mut usage = saved_usage;
            let prior_responses = checkpoint.completed_responses;
            let max_responses = assembled
                .runtime_agent
                .max_iterations
                .saturating_sub(input.iteration.saturating_sub(1) as usize)
                .saturating_sub(prior_responses as usize);
            for round in 0..max_responses {
                let (delivery, previous_response_id) = {
                    let mut state = coordinator.lock().await;
                    let mut delivery = state.prepare_delivery().await?;
                    if delivery.is_none()
                        && state.checkpoint().latest_response_id.is_some()
                        && !state.checkpoint().can_complete()
                    {
                        state.wait_next().await?;
                        delivery = state.prepare_delivery().await?;
                    }
                    persist_terminal_errors(adapter, &input.context, state.checkpoint()).await?;
                    (delivery, state.checkpoint().latest_response_id.clone())
                };
                let mut context = assembled.clone();
                context.model.driver = original_driver
                    .native_async_driver(&context.runtime_agent.model, tools.clone(), delivery)
                    .ok_or_else(|| AgentLoopError::config("native driver became unavailable"))?;
                let mut next_input = input.clone();
                next_input.context.exec_id = everruns_provider::typed_id::ExecId::new();
                next_input.previous_response_id = previous_response_id;
                next_input.iteration = input.iteration + prior_responses + round as u32;
                let mut result = atom
                    .execute_with_assembled_context(next_input, context)
                    .await?;
                if !result.success {
                    return Ok(result);
                }
                first_token_ms = first_token_ms.or(result.time_to_first_token_ms);
                if let Some(next) = &result.usage {
                    if let Some(total) = &mut usage {
                        total.add(next);
                    } else {
                        usage = Some(next.clone());
                    }
                }
                let mut state = coordinator.lock().await;
                if state.checkpoint().can_complete() {
                    result.native_counts = Some(everruns_engine::NativeExecutionCounts {
                        llm_calls: state.checkpoint().completed_responses,
                        tool_calls: state.checkpoint().calls.len().min(u32::MAX as usize) as u32,
                    });
                    result.tool_calls.clear();
                    result.has_tool_calls = false;
                    result.usage = usage;
                    result.time_to_first_token_ms = first_token_ms;
                    state
                        .persist_host_outcome(serde_json::to_value(&result).map_err(store_error)?)
                        .await?;
                    state.release().await?;
                    return Ok(result);
                }
            }
            Err(AgentLoopError::config(
                "native async response limit reached; pending outputs remain durable",
            ))
        };
        tokio::pin!(execution);
        let cancelled = async {
            if let Some(mut receiver) = adapter.turn_cancellation() {
                loop {
                    if *receiver.borrow() || receiver.changed().await.is_err() {
                        break;
                    }
                }
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(cancelled);
        let maintain_lease = async {
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                heartbeat.tick().await;
                if let Err(error) = lease_heartbeat.heartbeat().await {
                    break error;
                }
            }
        };
        tokio::select! {
            result = &mut execution => result,
            _ = &mut cancelled => Err(AgentLoopError::Cancelled),
            error = maintain_lease => Err(error),
        }
    };
    if !matches!(&outcome, Ok(result) if result.success) {
        let mut state = coordinator.lock().await;
        if state.cancel().await.is_ok() {
            persist_terminal_errors(adapter, &input.context, state.checkpoint()).await?;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_restores_usage_and_final_outcome_without_another_response() {
        use everruns_core::events::TokenUsage;
        use everruns_engine::ReasonResult;
        let first = ReasonResult {
            success: true,
            text: "working".into(),
            usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 3,
                ..Default::default()
            }),
            time_to_first_token_ms: Some(100),
            ..Default::default()
        };
        let last = ReasonResult {
            success: true,
            text: "finished".into(),
            usage: Some(TokenUsage {
                input_tokens: 20,
                output_tokens: 5,
                ..Default::default()
            }),
            time_to_first_token_ms: Some(50),
            ..Default::default()
        };
        let mut checkpoint = NativeAsyncCheckpoint {
            completed_responses: 2,
            latest_response_id: Some("final".into()),
            host_responses: vec![
                serde_json::to_value(first).unwrap(),
                serde_json::to_value(last).unwrap(),
            ],
            ..Default::default()
        };
        recover_host_progress(&mut checkpoint).unwrap();
        let result: ReasonResult =
            serde_json::from_value(checkpoint.host_outcome.clone().unwrap()).unwrap();
        assert_eq!(result.text, "finished");
        assert_eq!(result.usage.unwrap().input_tokens, 30);
        assert_eq!(result.time_to_first_token_ms, Some(100));
        assert_eq!(result.native_counts.unwrap().llm_calls, 2);
        checkpoint.host_outcome = None;
        checkpoint.transcript_message_id = Some("uncommitted".into());
        recover_host_progress(&mut checkpoint).unwrap();
        assert!(checkpoint.host_outcome.is_none());
        checkpoint.transcript_message_id = None;
        checkpoint.host_responses.pop();
        assert!(recover_host_progress(&mut checkpoint).is_err());
    }

    #[tokio::test]
    async fn journal_survives_reopen_and_fences_duplicate_owners() {
        let directory =
            std::env::temp_dir().join(format!("everruns-native-async-{}", uuid::Uuid::new_v4()));
        let journal = FileNativeAsyncJournal::open(&directory).unwrap();
        let mut checkpoint = NativeAsyncCheckpoint::default();
        checkpoint
            .response_completed("persisted_response".into())
            .unwrap();
        journal.save(&checkpoint).await.unwrap();
        assert!(FileNativeAsyncJournal::open(&directory).is_err());
        drop(journal);
        let reopened = FileNativeAsyncJournal::open(&directory).unwrap();
        assert_eq!(reopened.load().await.unwrap(), checkpoint);
        drop(reopened);
        fs::remove_dir_all(directory).unwrap();
    }
}
