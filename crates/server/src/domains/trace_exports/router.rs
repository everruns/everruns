//! Non-blocking event fan-out. A turn keeps its initial routing; exports never change globals.
use super::transport::DestinationTransport;
use super::{DestinationConfig, TraceProvider, TraceSecrets};
use crate::storage::{EncryptionService, StorageBackend, trace_exports::TracingDestinationRow};
use async_trait::async_trait;
use everruns_core::host::observability::braintrust::{
    BraintrustContentConfig, BraintrustDeliveryConfig, BraintrustPayloadMode,
    BraintrustThinkingMode,
};
use everruns_core::host::observability::{BraintrustConfig, BraintrustListener};
use everruns_core::{EgressScope, EgressService, Event, EventData, EventListener};
use moka::future::Cache;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};
use uuid::Uuid;

pub struct CustomerTracingListener {
    sender: mpsc::Sender<RouterMessage>,
    budget: Arc<Semaphore>,
    terminal_budget: Arc<Semaphore>,
}
struct ExportSlot {
    sender: mpsc::Sender<ExportMessage>,
}
struct QueuedEvent {
    event: Event,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}
enum RouterMessage {
    Event(Arc<QueuedEvent>),
    Flush(tokio::sync::oneshot::Sender<Result<(), String>>),
}
enum ExportMessage {
    Event(Arc<QueuedEvent>),
    Flush(tokio::sync::oneshot::Sender<Result<(), String>>),
}
type Routes = Vec<(TracingDestinationRow, Arc<ExportSlot>)>;
impl CustomerTracingListener {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Arc<EncryptionService>,
        egress: Arc<dyn EgressService>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel::<RouterMessage>(1024);
        let budget = Arc::new(Semaphore::new(32 * 1024 * 1024));
        let terminal_budget = Arc::new(Semaphore::new(1024 * 1024));
        let startup = chrono::Utc::now();
        tokio::spawn(async move {
            // Never re-resolve a completed, evicted or pre-restart turn under newer consent.
            // At capacity refuse new turns rather than evicting their routing decisions.
            let mut turns = std::collections::HashMap::<String, Routes>::new();
            let mut expirations = std::collections::VecDeque::new();
            let mut cutoff = startup;
            let exporters: Cache<(i64, Uuid, i64), Arc<ExportSlot>> = Cache::builder()
                .max_capacity(64)
                .time_to_idle(Duration::from_secs(900))
                .build();
            let permits = Arc::new(Semaphore::new(64));
            while let Some(message) = receiver.recv().await {
                let queued = match message {
                    RouterMessage::Event(event) => event,
                    RouterMessage::Flush(done) => {
                        let mut result = Ok(());
                        for (_, slot) in exporters.iter() {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            if slot.sender.send(ExportMessage::Flush(tx)).await.is_ok() {
                                if !matches!(rx.await, Ok(Ok(()))) {
                                    result = Err("Destination flush failed".into());
                                }
                            } else {
                                result = Err("Destination exporter stopped".into());
                            }
                        }
                        let _ = done.send(result);
                        continue;
                    }
                };
                let event = &queued.event;
                let Some(turn) = event.context.turn_id else {
                    continue;
                };
                let now = tokio::time::Instant::now();
                while expirations
                    .front()
                    .is_some_and(|(expires, _, _)| *expires <= now)
                {
                    let (_, key, ts) = expirations.pop_front().expect("checked above");
                    turns.remove(&key);
                    cutoff = cutoff.max(ts);
                }
                let key = format!("{}:{turn}", event.session_id);
                if event.event_type == "turn.started" && !turns.contains_key(&key) {
                    if event.ts <= cutoff {
                        continue;
                    }
                    if turns.len() >= 10_000 {
                        cutoff = cutoff.max(event.ts);
                        metrics::counter!("everruns_customer_tracing_router_dropped").increment(1);
                        continue;
                    }
                    match resolve(&db, event).await {
                        Ok((rows, scope)) => {
                            let mut routes = Vec::new();
                            for row in rows {
                                let cache_key = (row.org_id, row.id, row.revision);
                                let cached = exporters
                                    .get(&cache_key)
                                    .await
                                    .filter(|s| !s.sender.is_closed());
                                let slot = if let Some(slot) = cached {
                                    Some(slot)
                                } else {
                                    match create_slot(
                                        db.clone(),
                                        encryption.clone(),
                                        egress.clone(),
                                        row.clone(),
                                        scope.clone(),
                                        permits.clone(),
                                    )
                                    .await
                                    {
                                        Ok(slot) => {
                                            let slot = Arc::new(slot);
                                            exporters.insert(cache_key, slot.clone()).await;
                                            Some(slot)
                                        }
                                        Err(_) => {
                                            let _ = db
                                                .record_tracing_delivery(
                                                    row.org_id,
                                                    row.id,
                                                    row.revision,
                                                    Some("Exporter unavailable or at capacity"),
                                                )
                                                .await;
                                            let _ = super::health::refresh(&db, row.org_id).await;
                                            None
                                        }
                                    }
                                };
                                if let Some(slot) = slot {
                                    routes.push((row, slot));
                                }
                            }
                            turns.insert(key.clone(), routes);
                        }
                        Err(_) => {
                            tracing::warn!("Customer trace routing unavailable; turn not exported");
                            turns.insert(key.clone(), vec![]);
                        }
                    }
                    expirations.push_back((now + Duration::from_secs(3600), key.clone(), event.ts));
                }
                if let Some(routes) = turns.get(&key) {
                    for (row, slot) in routes {
                        if slot
                            .sender
                            .try_send(ExportMessage::Event(queued.clone()))
                            .is_err()
                        {
                            let _=sqlx::query("UPDATE tracing_destinations SET dropped_events=dropped_events+1 WHERE org_id=$1 AND id=$2 AND revision=$3")
                                .bind(row.org_id).bind(row.id).bind(row.revision).execute(db.database().pool()).await;
                        }
                    }
                }
                if matches!(
                    event.event_type.as_str(),
                    "turn.completed" | "turn.failed" | "turn.cancelled"
                ) && let Some(routes) = turns.get_mut(&key)
                {
                    routes.clear();
                }
            }
        });
        Self {
            sender,
            budget,
            terminal_budget,
        }
    }
    #[cfg(test)]
    pub(super) fn exhaust_budget_for_test(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.budget
            .clone()
            .try_acquire_many_owned(self.budget.available_permits() as u32)
            .unwrap()
    }
}
#[async_trait]
impl EventListener for CustomerTracingListener {
    fn name(&self) -> &'static str {
        "CustomerTracing"
    }
    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            "turn.started",
            "turn.completed",
            "turn.failed",
            "turn.cancelled",
            "reason.started",
            "reason.completed",
            "act.started",
            "act.completed",
            "llm.generation",
            "tool.started",
            "tool.completed",
            "reason.thinking.started",
            "reason.thinking.completed",
        ])
    }
    async fn on_event(&self, event: &Event) {
        // THREAT[TM-DOS-001]: bound aggregate queued bytes, not just event count.
        let size = serde_json::to_vec(event).map_or(usize::MAX, |bytes| bytes.len());
        if size > 256 * 1024 {
            metrics::counter!("everruns_customer_tracing_router_dropped").increment(1);
            return;
        }
        // Completion must remain admissible when open spans consume the regular budget.
        let budget = if matches!(
            event.event_type.as_str(),
            "turn.completed" | "turn.failed" | "turn.cancelled"
        ) {
            &self.terminal_budget
        } else {
            &self.budget
        };
        let Ok(bytes) = budget.clone().try_acquire_many_owned(size as u32) else {
            metrics::counter!("everruns_customer_tracing_router_dropped").increment(1);
            return;
        };
        if self
            .sender
            .try_send(RouterMessage::Event(Arc::new(QueuedEvent {
                event: event.clone(),
                _bytes: bytes,
            })))
            .is_err()
        {
            metrics::counter!("everruns_customer_tracing_router_dropped").increment(1);
        }
    }
    async fn flush(&self) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.sender
            .send(RouterMessage::Flush(tx))
            .await
            .map_err(|_| "Trace router stopped")?;
        tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| "Trace router flush timed out".to_string())?
            .map_err(|_| "Trace router flush stopped".to_string())?
    }
}

async fn resolve(
    db: &StorageBackend,
    event: &Event,
) -> anyhow::Result<(Vec<TracingDestinationRow>, EgressScope)> {
    let session = db
        .get_session_unscoped(event.session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Missing session"))?;
    // Most organizations have no customer exporters. Skip ancestry/invocation/policy work entirely.
    let destinations = db.tracing_export_destinations(session.org_id).await?;
    if destinations.is_empty() {
        return Ok((vec![], EgressScope::default()));
    }
    // THREAT[TM-OBS-010]: personal Chat and descendants never inherit organization exports.
    if session.source == "chat"
        || crate::platform_chat_agent::is_platform_chat(db, session.org_id, session.agent_id)
            .await?
    {
        return Ok((vec![], EgressScope::default()));
    }
    let mut parent = session.parent_session_id;
    for _ in 0..32 {
        let Some(id) = parent else { break };
        let ancestor = db
            .get_session(session.org_id, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Missing ancestor"))?;
        if ancestor.source == "chat"
            || crate::platform_chat_agent::is_platform_chat(db, session.org_id, ancestor.agent_id)
                .await?
        {
            return Ok((vec![], EgressScope::default()));
        }
        parent = ancestor.parent_session_id;
    }
    anyhow::ensure!(parent.is_none(), "Session ancestry limit");
    // Worker-supplied labels are not authority. Only an invocation proved to belong to the session can select a guest responder.
    let Some(message) = event.context.input_message_id else {
        return Ok((vec![], EgressScope::default()));
    };
    if !db
        .runtime_invocation_exists(session.id, message.uuid())
        .await?
    {
        return Ok((vec![], EgressScope::default()));
    }
    let agent = db
        .runtime_invocation_responder(session.id, message.uuid())
        .await?
        .map(everruns_contracts::typed_id::AgentId::from_uuid)
        .or(session.agent_id);
    let Some(agent) = agent else {
        return Ok((vec![], EgressScope::default()));
    };
    if crate::platform_chat_agent::is_platform_chat(db, session.org_id, Some(agent)).await? {
        return Ok((vec![], EgressScope::default()));
    }
    anyhow::ensure!(
        db.get_agent(session.org_id, agent).await?.is_some(),
        "Responder outside organization"
    );
    let defaults = db.tracing_policy(session.org_id, None).await?;
    let policy = db
        .tracing_policy(session.org_id, Some(agent.uuid()))
        .await?;
    // THREAT[TM-OBS-016]: asynchronous admission must not apply newer consent backward.
    if db
        .tracing_policy_changed_after(session.org_id, agent.uuid(), event.ts)
        .await?
    {
        return Ok((vec![], EgressScope::default()));
    }
    let ids = policy.policy.effective(&defaults.policy.destination_ids);
    let mut rows = Vec::new();
    for row in destinations {
        let config: DestinationConfig = serde_json::from_value(row.config.clone())?;
        if ids.contains(&row.id) && config.enabled && row.configured_at <= event.ts {
            rows.push(row);
        }
    }
    let organization = db
        .get_organization(session.org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization missing"))?;
    Ok((
        rows,
        EgressScope {
            org_id: Some(organization.public_id.parse()?),
            session_id: None,
        },
    ))
}

pub async fn make_listener(
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    egress: Arc<dyn EgressService>,
    row: TracingDestinationRow,
    scope: EgressScope,
) -> anyhow::Result<Arc<dyn EventListener>> {
    let config: DestinationConfig = serde_json::from_value(row.config.clone())?;
    let encrypted = db
        .tracing_secrets(row.org_id, row.connection_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Missing credentials"))?;
    let secrets: TraceSecrets = serde_json::from_str(&encryption.decrypt_to_string(&encrypted)?)?;
    let key = secrets
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.strip_prefix("Bearer ").unwrap_or(v).to_string())
        .unwrap_or_default();
    let service = Arc::new(DestinationTransport {
        db,
        inner: egress,
        row,
        scope,
        secrets,
    });
    Ok(match config.provider {
        TraceProvider::Otlp => Arc::new(
            everruns_core::host::observability::customer::otlp_listener(
                &config.endpoint,
                service,
                config.capture_content,
            )
            .map_err(|_| anyhow::anyhow!("Unable to initialize OTLP exporter"))?,
        ),
        TraceProvider::Braintrust => Arc::new(
            BraintrustListener::new(BraintrustConfig {
                api_key: key,
                project_id: config
                    .project_id
                    .ok_or_else(|| anyhow::anyhow!("Project missing"))?
                    .to_string(),
                api_url: config.endpoint,
                delivery: BraintrustDeliveryConfig {
                    queue_capacity: 128,
                    max_batch_size: 16,
                    ..Default::default()
                },
                content: BraintrustContentConfig {
                    record_content: config.capture_content,
                    record_thinking: if config.capture_reasoning {
                        BraintrustThinkingMode::Full
                    } else {
                        BraintrustThinkingMode::None
                    },
                    tool_args_mode: if config.capture_content {
                        BraintrustPayloadMode::Full
                    } else {
                        BraintrustPayloadMode::None
                    },
                    tool_results_mode: if config.capture_content {
                        BraintrustPayloadMode::Full
                    } else {
                        BraintrustPayloadMode::None
                    },
                    debug_payloads: false,
                },
                deployment_grade: everruns_core::DeploymentGrade::from_env(),
            })?
            .with_egress(service)
            .map_err(anyhow::Error::msg)?,
        ),
    })
}
async fn create_slot(
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    egress: Arc<dyn EgressService>,
    row: TracingDestinationRow,
    scope: EgressScope,
    permits: Arc<Semaphore>,
) -> anyhow::Result<ExportSlot> {
    let permit = permits.try_acquire_owned()?;
    let config: DestinationConfig = serde_json::from_value(row.config.clone())?;
    let listener = make_listener(db, encryption, egress, row, scope).await?;
    let (sender, mut receiver) = mpsc::channel::<ExportMessage>(128);
    let capture_reasoning = config.capture_reasoning;
    tokio::spawn(async move {
        let _permit = permit;
        // Keep the byte charge for open turns: SDK span state must share the router's memory budget.
        let mut retained = std::collections::HashMap::<
            everruns_contracts::typed_id::TurnId,
            Vec<Arc<QueuedEvent>>,
        >::new();
        let expires = tokio::time::Instant::now() + Duration::from_secs(3600);
        while let Ok(Some(message)) = tokio::time::timeout(
            expires
                .saturating_duration_since(tokio::time::Instant::now())
                .min(Duration::from_secs(900)),
            receiver.recv(),
        )
        .await
        {
            match message {
                ExportMessage::Event(queued) => {
                    let terminal = matches!(
                        queued.event.event_type.as_str(),
                        "turn.completed" | "turn.failed" | "turn.cancelled"
                    );
                    if let Some(turn) = queued.event.context.turn_id {
                        if terminal {
                            retained.remove(&turn);
                        } else {
                            retained.entry(turn).or_default().push(queued.clone());
                        }
                    }
                    if capture_reasoning {
                        listener.on_event(&queued.event).await;
                    } else {
                        let mut event = queued.event.clone();
                        strip_reasoning(&mut event);
                        listener.on_event(&event).await;
                    }
                }
                ExportMessage::Flush(done) => {
                    let result = tokio::time::timeout(Duration::from_secs(15), listener.flush())
                        .await
                        .map_err(|_| "Destination flush timed out".to_string())
                        .and_then(|r| r);
                    let _ = done.send(result);
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(10), listener.flush()).await;
    });
    Ok(ExportSlot { sender })
}
fn strip_reasoning(event: &mut Event) {
    match &mut event.data {
        EventData::ReasonThinkingCompleted(data) => data.thinking.clear(),
        EventData::LlmGeneration(data) => {
            for message in &mut data.messages {
                message.content.retain(|p| {
                    !matches!(
                        p,
                        everruns_core::message::ContentPart::Reasoning(_)
                            | everruns_core::message::ContentPart::ProviderOpaque(_)
                    )
                });
            }
            data.request_body = None;
        }
        _ => {}
    }
}

#[cfg(test)]
pub(super) async fn resolve_for_test(
    db: &StorageBackend,
    event: &Event,
) -> anyhow::Result<Vec<Uuid>> {
    Ok(resolve(db, event)
        .await?
        .0
        .into_iter()
        .map(|row| row.id)
        .collect())
}
