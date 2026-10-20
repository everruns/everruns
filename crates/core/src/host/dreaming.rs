//! Host-generic dreaming, shared by embedded and platform execution.
#[path = "dreaming/review.rs"]
mod review;
use super::RuntimeHostAdapter;
use crate::driver_registry::{LlmCallConfig, Message, MessageRole};
use crate::runtime_context::AssembledTurnContext;
use crate::{
    dreaming::*,
    engine::dreaming::runner::*,
    error::{AgentLoopError, Result},
    typed_id::SessionId,
};
use async_trait::async_trait;
use everruns_contracts::rt::utc_now;
use std::sync::Arc;

pub struct RuntimeDreamingHost<A: RuntimeHostAdapter> {
    adapter: A,
    org: i64,
    assembled: AssembledTurnContext,
    store: Arc<dyn DreamingStore>,
}
impl<A: RuntimeHostAdapter> RuntimeDreamingHost<A> {
    pub async fn new(adapter: A, org: i64, session: SessionId) -> Result<Self> {
        let store = adapter
            .dreaming_store()
            .ok_or_else(|| AgentLoopError::config("host does not support dreaming persistence"))?;
        let assembled = super::runtime_context::inspect_turn_context_for_session(
            adapter.harness_store(org).as_ref(),
            adapter.agent_store(org).as_ref(),
            adapter.session_store(org).as_ref(),
            adapter.message_store().as_ref(),
            adapter.provider_store(org).as_ref(),
            &adapter.capability_registry(),
            &adapter.driver_registry(),
            session,
            &[],
            Some(adapter.file_store(org)),
            adapter.storage_store(org),
        )
        .await?;
        Ok(Self {
            adapter,
            org,
            assembled,
            store,
        })
    }
    pub fn config(&self) -> Result<Option<DreamingConfig>> {
        self.assembled
            .resolved_capability_configs
            .iter()
            .find(|c| c.capability_id() == DREAMING_CAPABILITY_ID)
            .map(|c| {
                serde_json::from_value(c.config_value().clone())
                    .map_err(|_| AgentLoopError::config("invalid dreaming config"))
            })
            .transpose()
    }
    pub async fn enqueue(&self, manual: bool) -> Result<bool> {
        if manual
            && self
                .adapter
                .session_store(self.org)
                .get_session(self.assembled.session_id())
                .await?
                .is_some_and(|s| {
                    matches!(
                        s.status,
                        crate::session::SessionExecutionState::Active
                            | crate::session::SessionExecutionState::WaitingForToolResults
                    )
                })
        {
            return Err(AgentLoopError::config(
                "dreaming requires completed experience; wait for the active turn",
            ));
        }
        let Some(config) = self.config()? else {
            return Ok(false);
        };
        let evidence = crate::engine::dreaming::evidence::select(&self.assembled.messages);
        enqueue(
            self,
            self.org,
            self.assembled.session_id(),
            config,
            evidence,
            utc_now().timestamp(),
            manual,
        )
        .await
    }
    fn prompt(&self, job: &DreamJob, existing: &str) -> String {
        let d = &job.config.destinations[job.destination_index];
        serde_json::json!({"instructions":d.instructions,"evidence":job.evidence,"existing_memories":existing}).to_string()
    }
    async fn model(
        &self,
        job: &DreamJob,
    ) -> Result<crate::runtime_context::ResolvedModelExecution> {
        match &job.config.model {
            Some(model) => {
                super::runtime_context::resolve_model_execution_for_session(
                    self.adapter.provider_store(self.org).as_ref(),
                    &self.adapter.driver_registry(),
                    everruns_contracts::model_spec::ModelSpec::on(
                        self.assembled.model.provider.clone(),
                        model.clone(),
                    ),
                    Some(self.assembled.session_id()),
                )
                .await
            }
            None => Ok(self.assembled.model.clone()),
        }
    }
    fn files(&self) -> Arc<dyn crate::session_files::SessionFileSystem> {
        self.adapter.file_store(self.org)
    }
}
#[async_trait]
impl<A: RuntimeHostAdapter> DreamingHost for RuntimeDreamingHost<A> {
    async fn cancelled(&self, job: &DreamJob) -> Result<bool> {
        let Some(registry) = self.adapter.session_task_registry(self.org) else {
            return Ok(false);
        };
        let task_id = format!("task_dream_{}", job.operation_id.replace('-', ""));
        Ok(registry
            .get(job.session_id, &task_id)
            .await?
            .is_some_and(|t| t.cancel_requested_at.is_some()))
    }

    fn store(&self) -> Arc<dyn DreamingStore> {
        self.store.clone()
    }
    async fn destination(
        &self,
        session: SessionId,
        d: &DreamDestination,
    ) -> Result<DreamMemoryScope> {
        if self.config()?.is_none_or(|c| !c.destinations.contains(d)) {
            return Err(AgentLoopError::config(
                "dreaming destination configuration was revoked",
            ));
        }
        let scope = self
            .files()
            .dreaming_scope(session, &d.mount)
            .await?
            .ok_or_else(|| {
                AgentLoopError::config(
                    "dreaming destination is not an authorized writable Memory mount",
                )
            })?;
        if !scope.private
            && self
                .files()
                .dreaming_scope(session, "/memory/user")
                .await?
                .is_some()
        {
            return Err(AgentLoopError::config(
                "private user experience cannot be promoted into shared Memory",
            ));
        }
        Ok(scope)
    }
    async fn existing(&self, session: SessionId, d: &DreamDestination) -> Result<String> {
        let root = format!("{}{}", d.mount, d.path.trim_end_matches('/'));
        if self.files().stat_file(session, &root).await?.is_none() {
            return Ok(String::new());
        }
        let mut entries = self.files().list_directory(session, &root).await?;
        entries.sort_by_key(|f| std::cmp::Reverse(f.updated_at));
        let mut result = String::new();
        for f in entries
            .into_iter()
            .filter(|f| !f.is_directory && f.size_bytes <= MAX_MEMORY_BYTES as i64)
            .take(16)
        {
            if let Some(file) = self.files().read_file(session, &f.path).await?
                && let Some(text) = file.content
            {
                if result.len() + text.len() > MAX_MEMORY_BYTES {
                    break;
                }
                result.push_str(&text);
                result.push('\n');
            }
        }
        Ok(result)
    }
    async fn maximum_cost(&self, job: &DreamJob, existing: &str) -> Result<Option<f64>> {
        if self.config()?.as_ref() != Some(&job.config) {
            return Err(AgentLoopError::config(
                "dreaming configuration changed; retained work is no longer authorized",
            ));
        }

        let model = self.model(job).await?;
        let input =
            ((self.prompt(job, existing).len() + DREAM_SYSTEM_PROMPT.len() + 4096) * 2) as u32;
        use everruns_contracts::model_profiles::{cache_write_1h_surcharge_usd, estimate_cost_usd};
        let output = job.config.budget.max_output_tokens_per_run;
        let Some(uncached) =
            estimate_cost_usd(&model.provider_type, &model.model, input, output, 0, 0)
        else {
            return Ok(None);
        };
        // Cache writes can cost more than uncached input. Reserve the larger
        // full-prompt bill, including the provider's long-retention surcharge.
        let cached = estimate_cost_usd(&model.provider_type, &model.model, 0, output, 0, input)
            .unwrap_or(uncached)
            + cache_write_1h_surcharge_usd(&model.provider_type, &model.model, input, input)
                .unwrap_or(0.0);
        Ok(Some(uncached.max(cached)))
    }
    async fn generate(&self, job: &DreamJob, existing: &str) -> Result<DreamOutput> {
        if let Some(checker) = self
            .adapter
            .budget_checker(self.org, self.assembled.snapshot.agent_id)
        {
            let budget = checker.check_budgets(&job.session_id.to_string()).await?;
            if ["paused", "exhausted"].contains(&budget.status.as_str()) {
                return Err(AgentLoopError::config("host budget blocks dreaming"));
            }
        }

        let model = self.model(job).await?;
        let messages = vec![
            Message::text(MessageRole::System, DREAM_SYSTEM_PROMPT),
            Message::text(MessageRole::User, self.prompt(job, existing)),
        ];
        let mut config = LlmCallConfig::new(&model.model);
        config.max_tokens = Some(job.config.budget.max_output_tokens_per_run);
        config.reasoning_effort = Some(everruns_contracts::model::ReasoningEffort::Low);
        config
            .metadata
            .insert("dreaming_operation_id".into(), job.operation_id.clone());
        let started = everruns_contracts::rt::Instant::now();
        let response = everruns_contracts::rt::timeout(
            std::time::Duration::from_secs(240),
            model.driver.chat_completion(
                &everruns_contracts::ProviderEndpoint::default(),
                messages,
                &config,
            ),
        )
        .await
        .map_err(|_| AgentLoopError::llm("dreaming model call timed out"))??;
        let metadata = &response.metadata;
        let mut usage = crate::events::TokenUsage::new(
            metadata.prompt_tokens.unwrap_or(0),
            metadata.completion_tokens.unwrap_or(0),
        );
        usage.cache_read_tokens = metadata.cache_read_tokens;
        usage.cache_creation_tokens = metadata.cache_creation_tokens;
        usage.actual_cost_usd = metadata.provider_cost_usd;
        usage.estimated_cost_usd = everruns_contracts::model_profiles::estimate_cost_usd_for_speed(
            &model.provider_type,
            &model.model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens.unwrap_or(0),
            usage.cache_creation_tokens.unwrap_or(0),
            metadata.service_tier.as_deref(),
        );
        if let Some(surcharge) = everruns_contracts::model_profiles::cache_write_1h_surcharge_usd(
            &model.provider_type,
            &model.model,
            usage
                .input_tokens
                .saturating_add(usage.cache_read_tokens.unwrap_or(0))
                .saturating_add(usage.cache_creation_tokens.unwrap_or(0)),
            metadata.cache_creation_1h_tokens.unwrap_or(0),
        ) {
            usage.estimated_cost_usd = usage.estimated_cost_usd.map(|cost| cost + surcharge);
        }
        // Reuse canonical billing/budget listeners without publishing evidence,
        // prompts, request capture or generated Memory in the session log.
        let mut generation = crate::events::LlmGenerationData::success(
            Vec::new(),
            Vec::new(),
            None,
            Vec::new(),
            model.model.clone(),
            Some(model.provider_type.to_string()),
            Some(usage.clone()),
            Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            None,
        );
        generation.metadata.response_model = metadata.response_model.clone();
        let provider_id = model.provider.to_string();
        generation.metadata.provider_id =
            provider_id.starts_with("provider_").then_some(provider_id);
        self.adapter
            .dreaming_event_emitter()
            .emit(
                crate::events::EventRequest::new(job.session_id, Default::default(), generation)
                    .with_metadata(serde_json::json!({"dreaming_operation_id":job.operation_id})),
            )
            .await?;
        let _ = self
            .adapter
            .dreaming_event_emitter()
            .emit(crate::events::EventRequest::new(
                job.session_id,
                Default::default(),
                crate::events::DreamingActivityData {
                    operation_id: job.operation_id.clone(),
                    status: "generated".into(),
                    note_count: job.note_count,
                    reason: None,
                    cost_usd: None,
                    model: Some(model.model.clone()),
                    usage: Some(usage),
                },
            ))
            .await;
        serde_json::from_str(response.text.trim())
            .map_err(|_| AgentLoopError::llm("dreaming returned invalid JSON"))
    }
    async fn commit(
        &self,
        job: &DreamJob,
        d: &DreamDestination,
        output: &DreamOutput,
    ) -> Result<()> {
        if output.notes.is_empty() || job.config.commit_mode == DreamCommitMode::Propose {
            return Ok(());
        }
        let root = format!("{}{}", d.mount, d.path.trim_end_matches('/'));
        if self
            .files()
            .stat_file(job.session_id, &root)
            .await?
            .is_none()
        {
            self.files().create_directory(job.session_id, &root).await?;
        }
        let path = format!(
            "{root}/dream-{}-{}.md",
            job.operation_id, job.destination_index
        );
        let mut content = format!(
            "<!-- everruns dreaming; session={}; operation={} -->\n",
            job.session_id, job.operation_id
        );
        for n in &output.notes {
            content.push_str(&format!(
                "\n### {}\n\n{}\n\nSources: {}\n",
                n.kind,
                n.text,
                n.evidence.join(", ")
            ));
        }
        if let Some(existing) = self.files().read_file(job.session_id, &path).await? {
            if existing.content.as_deref() == Some(&content) {
                return Ok(());
            }
            return Err(AgentLoopError::store(
                "dreaming artifact conflicts with existing content",
            ));
        }
        if !self
            .files()
            .create_file_if_absent(job.session_id, &path, &content)
            .await?
        {
            let current = self.files().read_file(job.session_id, &path).await?;
            if current.and_then(|f| f.content).as_deref() != Some(&content) {
                return Err(AgentLoopError::store(
                    "dreaming artifact changed concurrently",
                ));
            }
        }
        Ok(())
    }
    async fn activity(&self, session: SessionId, a: DreamActivity) -> Result<()> {
        if a.operation_id != "maintenance"
            && let Some(registry) = self.adapter.session_task_registry(self.org)
        {
            use crate::session_task::{
                CreateSessionTask, SessionTaskState, SessionTaskUpdate, TaskWakePolicy,
            };
            let task_id = format!("task_dream_{}", a.operation_id.replace('-', ""));
            let _ = registry
                .create(CreateSessionTask {
                    session_id: session,
                    id: Some(task_id.clone()),
                    kind: "dreaming".into(),
                    display_name: "Dreaming".into(),
                    spec: serde_json::json!({"operation_id":a.operation_id}),
                    state: SessionTaskState::Queued,
                    links: Default::default(),
                    wake_policy: TaskWakePolicy::Silent,
                })
                .await;
            let state = match a.status.as_str() {
                "started" | "retrying" | "progress" => SessionTaskState::Running,
                "completed" | "proposed" | "accepted" | "discarded" => SessionTaskState::Succeeded,
                "failed" => SessionTaskState::Failed,
                "canceled" => SessionTaskState::Canceled,
                _ => SessionTaskState::Queued,
            };
            let _ = registry
                .update(
                    session,
                    &task_id,
                    SessionTaskUpdate {
                        state: Some(state),
                        summary: Some(format!("{}: {} notes", a.status, a.note_count)),
                        ..Default::default()
                    },
                )
                .await;
        }

        // Only bounded metadata: memory text, destinations and evidence are never copied into session events.
        self.adapter
            .dreaming_event_emitter()
            .emit(crate::events::EventRequest::new(
                session,
                Default::default(),
                crate::events::DreamingActivityData {
                    operation_id: a.operation_id,
                    status: a.status,
                    note_count: a.note_count,
                    reason: a.reason,
                    cost_usd: a.cost_usd,
                    model: a.model,
                    usage: None,
                },
            ))
            .await?;
        Ok(())
    }
}
/// Sweep due jobs using an adapter that can load every returned session.
/// Session-scoped hosts dispatch `Engine::maintain_dreaming(session)` instead.
pub async fn run_due<A: RuntimeHostAdapter>(adapter: A) -> Result<usize> {
    let Some(store) = adapter.dreaming_store() else {
        return Ok(0);
    };
    use futures::{StreamExt, TryStreamExt};
    let due = store.due(utc_now().timestamp(), 32).await?;
    futures::stream::iter(due)
        .map(|(org, session)| run_one(adapter.clone(), store.clone(), org, session))
        .buffer_unordered(4)
        .try_fold(
            0usize,
            |count, ran| async move { Ok(count + usize::from(ran)) },
        )
        .await
}
async fn run_one<A: RuntimeHostAdapter>(
    adapter: A,
    store: Arc<dyn DreamingStore>,
    org: i64,
    session: SessionId,
) -> Result<bool> {
    let host = match RuntimeDreamingHost::new(adapter.clone(), org, session).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(%e,"dreaming host unavailable");
            postpone(store.as_ref(), org, session, false).await?;
            return Ok(false);
        }
    };
    if host.config()?.is_none() {
        postpone(store.as_ref(), org, session, true).await?;
        return Ok(false);
    }
    match run_session(&host, org, session).await {
        Ok(true) => return Ok(true),
        Ok(false) => {}
        Err(_) => {
            let (operation, note_count) = store
                .get(org, &job_key(session))
                .await?
                .and_then(|r| serde_json::from_str::<DreamSessionState>(&r.value).ok())
                .and_then(|s| s.job.map(|j| (j.operation_id, j.note_count)))
                .unwrap_or_else(|| ("maintenance".into(), 0));
            host.activity(session,DreamActivity{operation_id:operation,status:"retrying".into(),destination:None,note_count,reason:Some("Dreaming will retry; check destination access and budget if failures persist.".into()),cost_usd:None,model:None}).await?;
        }
    }
    Ok(false)
}
/// Called after a successful turn. The foreground pays only the durable enqueue.
pub fn after_turn<A: RuntimeHostAdapter>(
    adapter: A,
    org: i64,
    session: SessionId,
    turn: crate::typed_id::TurnId,
) -> futures::future::BoxFuture<'static, Result<()>> {
    // Keep the maintenance context/model future out of the foreground lifecycle's
    // state-machine layout. Scheduling still durably enqueues before returning.
    Box::pin(async move {
        let snapshot = super::execution_snapshot::load_execution_snapshot(
            adapter.harness_store(org).as_ref(),
            adapter.agent_store(org).as_ref(),
            adapter.session_store(org).as_ref(),
            session,
        )
        .await?;
        if !snapshot
            .capabilities
            .iter()
            .any(|c| c.capability_id() == DREAMING_CAPABILITY_ID)
        {
            return Ok(());
        }
        let host = RuntimeDreamingHost::new(adapter.clone(), org, session).await?;
        record_turn(host.store.as_ref(), org, session, turn.to_string()).await?;
        if !host.enqueue(false).await? {
            return Ok(());
        }
        let delay = host
            .config()?
            .map(|c| {
                if c.trigger.mode == DreamTriggerMode::Idle {
                    c.trigger.idle_after_seconds
                } else {
                    0
                }
            })
            .unwrap_or(0);
        let store = host.store.clone();
        everruns_contracts::rt::spawn(async move {
            everruns_contracts::rt::sleep(std::time::Duration::from_secs(u64::from(delay))).await;
            // Embedded adapters can be scoped to one session. A local timer must
            // never discover or postpone another session's durable obligations.
            let _ = run_one(adapter, store, org, session).await;
        });
        Ok(())
    })
}

#[async_trait]
pub trait DreamingCommand: Send + Sync {
    async fn proposals(&self, accept: bool, discard: bool) -> Result<String>;
    async fn enqueue(&self) -> Result<bool>;
}
pub struct RuntimeDreamingCommand<A: RuntimeHostAdapter> {
    adapter: A,
    org: i64,
    session: SessionId,
    local_wakeup: bool,
}
impl<A: RuntimeHostAdapter> RuntimeDreamingCommand<A> {
    /// Queue work for the host's durable maintenance dispatcher.
    pub fn new(adapter: A, org: i64, session: SessionId) -> Self {
        Self {
            adapter,
            org,
            session,
            local_wakeup: false,
        }
    }

    /// Also kick embedded execution. Durable hosts still need persistent wakeups.
    pub fn with_local_wakeup(mut self) -> Self {
        self.local_wakeup = true;
        self
    }
}
#[async_trait]
impl<A: RuntimeHostAdapter> DreamingCommand for RuntimeDreamingCommand<A> {
    async fn proposals(&self, accept: bool, discard: bool) -> Result<String> {
        RuntimeDreamingHost::new(self.adapter.clone(), self.org, self.session)
            .await?
            .proposals(accept, discard)
            .await
    }

    async fn enqueue(&self) -> Result<bool> {
        let queued = RuntimeDreamingHost::new(self.adapter.clone(), self.org, self.session)
            .await?
            .enqueue(true)
            .await?;
        if self.local_wakeup {
            let adapter = self.adapter.clone();
            let org = self.org;
            let session = self.session;
            everruns_contracts::rt::spawn(async move {
                if let Some(store) = adapter.dreaming_store() {
                    let _ = run_one(adapter, store, org, session).await;
                }
            });
        }
        Ok(queued)
    }
}

// Keep unavailable sessions from monopolizing the bounded due scan. Removing
// the capability cancels retained work instead of repeatedly rediscovering it.
async fn postpone(
    store: &dyn DreamingStore,
    org: i64,
    session: SessionId,
    cancel: bool,
) -> Result<()> {
    let key = job_key(session);
    if let Some(row) = store.get(org, &key).await? {
        let value = if cancel {
            let mut state: DreamSessionState = serde_json::from_str(&row.value)
                .map_err(|_| AgentLoopError::store("invalid dreaming state"))?;
            if let Some(job) = state.job.as_mut() {
                job.completed = true;
                job.output = None;
                job.proposals.clear();
            }
            serde_json::to_string(&state)
                .map_err(|_| AgentLoopError::store("serialize canceled job"))?
        } else {
            row.value
        };
        store
            .compare_exchange(
                org,
                &key,
                Some(row.revision),
                &value,
                Some(session),
                (!cancel).then(|| utc_now().timestamp() + 600),
            )
            .await?;
    }
    Ok(())
}
