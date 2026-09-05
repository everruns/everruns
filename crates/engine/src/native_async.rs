//! Opt-in native-call coordinator for streaming hosts. Normal Reason/Act hosts
//! keep synchronous execution unless they install this coordinator explicitly.

use async_trait::async_trait;
use everruns_provider::{
    LlmResponseStream, LlmStreamEvent,
    error::{AgentLoopError, Result},
    native_async::{Delivery, NativeAsyncCheckpoint, NativeToolCall, PendingCallState},
};
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

/// An exclusive, durable conversation journal. Keep the ownership fence for the
/// lifetime of the coordinator, including provider requests between pump calls.
#[async_trait]
pub trait NativeAsyncJournal: Send + Sync {
    async fn load(&self) -> Result<NativeAsyncCheckpoint>;
    /// Atomically persist and flush before returning success.
    async fn save(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct NativeCallPolicy {
    pub allow_async: bool,
    pub replay_safe: bool,
    pub concurrency_class: Option<String>,
}

/// Hosts must use their ordinary tool authorization, argument validation,
/// pre/post execution hooks, resource scoping and outbound limits here. Native
/// metadata from the provider never grants permission to execute a tool.
#[async_trait]
pub trait NativeAsyncExecutor: Send + Sync + 'static {
    async fn authorize(&self, call: &NativeToolCall) -> Result<NativeCallPolicy>;
    async fn execute(&self, call: NativeToolCall) -> Result<String>;
}

type Job = BoxFuture<'static, (String, Result<String>)>;

pub struct NativeAsyncCoordinator {
    journal: Box<dyn NativeAsyncJournal>,
    executor: Arc<dyn NativeAsyncExecutor>,
    checkpoint: NativeAsyncCheckpoint,
    jobs: FuturesUnordered<Job>,
    permits: Arc<Semaphore>,
    classes: HashMap<String, Arc<Mutex<()>>>,
    serialize_all: bool,
    poisoned: bool,
}

impl NativeAsyncCoordinator {
    /// Recover only after acquiring the journal's exclusive ownership fence.
    /// Safe calls are reauthorized and restarted; unsafe calls become explicit
    /// interrupted outputs. Ambiguous HTTP delivery requires receipt recovery.
    pub async fn open(
        journal: Box<dyn NativeAsyncJournal>,
        executor: Arc<dyn NativeAsyncExecutor>,
        max_concurrency: usize,
        parallel_tool_calls: bool,
    ) -> Result<Self> {
        let mut checkpoint = journal.load().await?;
        checkpoint.recover()?;
        journal.save(&checkpoint).await?;
        let mut this = Self {
            journal,
            executor,
            checkpoint,
            jobs: FuturesUnordered::new(),
            permits: Arc::new(Semaphore::new(max_concurrency.max(1))),
            classes: HashMap::new(),
            serialize_all: !parallel_tool_calls,
            poisoned: false,
        };
        let queued: Vec<_> = this
            .checkpoint
            .order
            .iter()
            .filter_map(|id| this.checkpoint.calls.get(id))
            .filter(|pending| pending.state == PendingCallState::Queued)
            .map(|pending| pending.call.clone())
            .collect();
        for call in queued {
            let policy = this.executor.authorize(&call).await?;
            this.launch(call, policy).await?;
        }
        Ok(this)
    }

    pub fn checkpoint(&self) -> &NativeAsyncCheckpoint {
        &self.checkpoint
    }

    fn healthy(&self) -> Result<()> {
        if self.poisoned {
            Err(AgentLoopError::store(
                "native coordinator lost durable state; reopen under a fresh ownership fence",
            ))
        } else {
            Ok(())
        }
    }

    async fn save(&mut self) -> Result<()> {
        match self.journal.save(&self.checkpoint).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.poisoned = true;
                self.jobs.clear();
                Err(error)
            }
        }
    }

    async fn launch(&mut self, call: NativeToolCall, policy: NativeCallPolicy) -> Result<()> {
        if call.is_async() && !policy.allow_async {
            return Err(AgentLoopError::config(
                "tool is not authorized for native asynchronous execution",
            ));
        }
        if self
            .checkpoint
            .calls
            .get(call.id())
            .is_some_and(|pending| pending.replay_safe != policy.replay_safe)
        {
            return Err(AgentLoopError::config(
                "native replay policy changed; explicit reconciliation required",
            ));
        }
        self.checkpoint.start(call.id())?;
        self.save().await?;
        let class = if self.serialize_all {
            Some(String::new())
        } else {
            policy.concurrency_class
        };
        let lock = class.map(|class| {
            self.classes
                .entry(class)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        });
        let permits = self.permits.clone();
        let executor = self.executor.clone();
        self.jobs.push(Box::pin(async move {
            let _class_guard = match lock {
                Some(lock) => Some(lock.lock_owned().await),
                None => None,
            };
            let _permit = permits
                .acquire()
                .await
                .expect("coordinator never closes permits");
            let id = call.id().to_owned();
            (id, executor.execute(call).await)
        }));
        Ok(())
    }

    pub async fn register(&mut self, call: NativeToolCall) -> Result<()> {
        self.healthy()?;
        let policy = self.executor.authorize(&call).await?;
        if call.is_async() && !policy.allow_async {
            return Err(AgentLoopError::config(
                "tool is not authorized for native asynchronous execution",
            ));
        }
        if !self.checkpoint.register(call.clone(), policy.replay_safe)? {
            return Ok(());
        }
        self.save().await?;
        self.launch(call, policy).await
    }

    async fn settle(&mut self, id: String, result: Result<String>) -> Result<()> {
        // Executors own disclosure: errors must already be safe for the model.
        let output = result
            .unwrap_or_else(|error| serde_json::json!({"error":error.to_string()}).to_string());
        self.checkpoint.settle(&id, output)?;
        self.save().await
    }

    /// Consume a response while jobs execute. Returns once the provider response
    /// is complete, even if async work remains. Independent follow-up responses
    /// may be pumped before waiting for jobs. `observe` receives prose/reasoning
    /// unchanged; only executable call events are consumed by the coordinator.
    pub async fn pump(
        &mut self,
        stream: LlmResponseStream,
        mut observe: impl FnMut(LlmStreamEvent),
    ) -> Result<()> {
        self.healthy()?;
        if self.checkpoint.response_in_flight {
            return Err(AgentLoopError::store(
                "prior native response requires reconciliation",
            ));
        }
        let result = self.pump_inner(stream, &mut observe).await;
        if result.is_err() {
            self.poisoned = true;
            self.jobs.clear();
        }
        result
    }

    async fn pump_inner(
        &mut self,
        mut stream: LlmResponseStream,
        mut observe: impl FnMut(LlmStreamEvent),
    ) -> Result<()> {
        self.healthy()?;
        self.checkpoint.response_in_flight = true;
        self.save().await?;
        loop {
            tokio::select! {
                finished = self.jobs.next(), if !self.jobs.is_empty() => {
                    let (id, result) = finished.expect("nonempty job set");
                    self.settle(id, result).await?;
                }
                event = stream.next() => {
                    let event = event.ok_or_else(|| AgentLoopError::llm("native response stream ended without a completion receipt"))??;
                    match event {
                        LlmStreamEvent::NativeToolCall(call) => self.register(call).await?,
                        LlmStreamEvent::ToolCalls(calls) => {
                            for call in calls {
                                self.register(NativeToolCall::Function { call_id: call.id, name: call.name, arguments: serde_json::to_string(&call.arguments).map_err(|e| AgentLoopError::config(e.to_string()))?, asynchronous: false }).await?;
                            }
                        }
                        LlmStreamEvent::Done(metadata) => {
                            let response_id = metadata.response_id.clone().ok_or_else(|| AgentLoopError::llm("native response omitted response ID"))?;
                            if self.checkpoint.delivery.is_some() { self.checkpoint.acknowledge_delivery(response_id)?; }
                            else { self.checkpoint.response_completed(response_id)?; }
                            self.checkpoint.response_in_flight = false;
                            self.save().await?;
                            if metadata.finish_reason.as_deref().is_some_and(|reason| !matches!(reason, "stop" | "tool_calls" | "end_turn")) {
                                return Err(AgentLoopError::llm("native response did not finish successfully"));
                            }
                            observe(LlmStreamEvent::Done(metadata));
                            return Ok(());
                        }
                        LlmStreamEvent::Error(error) => return Err(AgentLoopError::llm(error.to_string())),
                        other => observe(other),
                    }
                }
            }
        }
    }

    /// Wait for one completion; outputs may be delivered out of launch order.
    pub async fn wait_next(&mut self) -> Result<bool> {
        self.healthy()?;
        if let Some((id, result)) = self.jobs.next().await {
            self.settle(id, result).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Persist the delivery intent before the caller submits its HTTP request.
    /// Synchronous calls must all finish before the provider can continue.
    pub async fn prepare_delivery(&mut self) -> Result<Option<Delivery>> {
        self.healthy()?;
        while self.checkpoint.calls.values().any(|pending| {
            !pending.call.is_async()
                && matches!(
                    pending.state,
                    PendingCallState::Queued | PendingCallState::Running
                )
        }) {
            if !self.wait_next().await? {
                return Err(AgentLoopError::store("synchronous call has no running job"));
            }
        }
        let delivery = self.checkpoint.prepare_delivery()?.cloned();
        self.save().await?;
        Ok(delivery)
    }

    /// Dropping owned futures cancels local execution. Persist cancellation
    /// outputs; the conversation remains incomplete until they are delivered.
    pub async fn cancel(&mut self) -> Result<()> {
        self.healthy()?;
        self.jobs.clear();
        self.checkpoint.cancel();
        self.save().await
    }
}

impl NativeAsyncCoordinator {
    /// Drive HTTP response continuations through completion, returning only when
    /// every accepted call's output has a provider receipt. The caller supplies
    /// request construction; no transport or model defaults are chosen here.
    pub async fn run<Request, RequestFuture>(
        &mut self,
        max_responses: usize,
        mut request: Request,
        mut observe: impl FnMut(LlmStreamEvent),
    ) -> Result<()>
    where
        Request: FnMut(Option<Delivery>, Option<String>) -> RequestFuture,
        RequestFuture: std::future::Future<Output = Result<LlmResponseStream>>,
    {
        self.healthy()?;
        for _ in 0..max_responses {
            let mut delivery = self.prepare_delivery().await?;
            // On a restart with a completed response, resume its pending jobs
            // before making a continuation request with no new input.
            if delivery.is_none()
                && self.checkpoint.latest_response_id.is_some()
                && !self.jobs.is_empty()
            {
                self.wait_next().await?;
                delivery = self.prepare_delivery().await?;
            }
            let stream = request(delivery, self.checkpoint.latest_response_id.clone()).await?;
            self.pump(stream, &mut observe).await?;
            if self.checkpoint.can_complete() {
                return Ok(());
            }
            if !self
                .checkpoint
                .calls
                .values()
                .any(|pending| matches!(pending.state, PendingCallState::Ready { .. }))
            {
                self.wait_next().await?;
            }
        }
        Err(AgentLoopError::config(
            "native async response limit reached; pending work remains checkpointed",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::LlmCompletionMetadata;
    use std::{
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[derive(Clone, Default)]
    struct MemoryJournal(Arc<StdMutex<NativeAsyncCheckpoint>>);
    #[async_trait]
    impl NativeAsyncJournal for MemoryJournal {
        async fn load(&self) -> Result<NativeAsyncCheckpoint> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn save(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<()> {
            *self.0.lock().unwrap() = checkpoint.clone();
            Ok(())
        }
    }
    #[derive(Default)]
    struct Executor {
        started: AtomicUsize,
        active: AtomicUsize,
        maximum: AtomicUsize,
    }
    #[async_trait]
    impl NativeAsyncExecutor for Executor {
        async fn authorize(&self, call: &NativeToolCall) -> Result<NativeCallPolicy> {
            if call.name() == "forbidden" {
                return Err(AgentLoopError::tool("not authorized"));
            }
            Ok(NativeCallPolicy {
                allow_async: true,
                replay_safe: true,
                concurrency_class: None,
            })
        }
        async fn execute(&self, call: NativeToolCall) -> Result<String> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _active = Active(&self.active);
            tokio::time::sleep(Duration::from_millis(if call.id() == "slow" {
                100
            } else {
                1
            }))
            .await;
            Ok(call.id().into())
        }
    }
    fn call(id: &str, asynchronous: bool) -> NativeToolCall {
        NativeToolCall::Function {
            call_id: id.into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            asynchronous,
        }
    }
    fn done(id: &str) -> LlmStreamEvent {
        LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
            response_id: Some(id.into()),
            ..Default::default()
        }))
    }
    fn stream(events: Vec<LlmStreamEvent>) -> LlmResponseStream {
        Box::pin(futures::stream::iter(events.into_iter().map(Ok)))
    }

    #[tokio::test]
    async fn early_dispatch_mixed_calls_out_of_order_and_independent_work() {
        let journal = MemoryJournal::default();
        let executor = Arc::new(Executor::default());
        let mut coordinator =
            NativeAsyncCoordinator::open(Box::new(journal.clone()), executor.clone(), 2, true)
                .await
                .unwrap();
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        sender
            .unbounded_send(Ok(LlmStreamEvent::NativeToolCall(call("slow", true))))
            .unwrap();
        let observed = executor.clone();
        let producer = async move {
            // The stream is not complete yet; the tool must already be running.
            while observed.started.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            sender
                .unbounded_send(Ok(LlmStreamEvent::NativeToolCall(call("fast", true))))
                .unwrap();
            sender
                .unbounded_send(Ok(LlmStreamEvent::TextDelta("independent answer".into())))
                .unwrap();
            sender.unbounded_send(Ok(done("launch"))).unwrap();
        };
        let mut text = String::new();
        let (result, _) = tokio::join!(
            coordinator.pump(Box::pin(receiver), |event| {
                if let LlmStreamEvent::TextDelta(delta) = event {
                    text.push_str(&delta);
                }
            }),
            producer
        );
        result.unwrap();
        assert_eq!(text, "independent answer");
        assert!(!coordinator.checkpoint().can_complete());
        coordinator.wait_next().await.unwrap();
        coordinator
            .pump(stream(vec![done("independent_followup")]), |_| {})
            .await
            .unwrap();
        let delivery = coordinator.prepare_delivery().await.unwrap().unwrap();
        assert_eq!(delivery.previous_response_id, "independent_followup");
        assert_eq!(delivery.call_ids, vec!["fast"]);
        coordinator
            .pump(
                stream(vec![
                    LlmStreamEvent::NativeToolCall(call("sync", false)),
                    done("fast_receipt"),
                ]),
                |_| {},
            )
            .await
            .unwrap();
        let next = coordinator.prepare_delivery().await.unwrap().unwrap();
        assert!(next.call_ids.contains(&"sync".to_string()));
        coordinator
            .pump(stream(vec![done("sync_receipt")]), |_| {})
            .await
            .unwrap();
        while coordinator.wait_next().await.unwrap() {}
        coordinator.prepare_delivery().await.unwrap();
        coordinator
            .pump(stream(vec![done("final")]), |_| {})
            .await
            .unwrap();
        assert!(coordinator.checkpoint().can_complete());
        assert!(executor.maximum.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn restart_cancellation_duplicate_calls_and_serial_limit() {
        let journal = MemoryJournal::default();
        let executor = Arc::new(Executor::default());
        let mut coordinator =
            NativeAsyncCoordinator::open(Box::new(journal.clone()), executor.clone(), 8, false)
                .await
                .unwrap();
        coordinator.register(call("slow", true)).await.unwrap();
        coordinator.register(call("slow", true)).await.unwrap();
        coordinator.register(call("fast", true)).await.unwrap();
        coordinator
            .pump(stream(vec![done("before_restart")]), |_| {})
            .await
            .unwrap();
        drop(coordinator);
        let mut recovered =
            NativeAsyncCoordinator::open(Box::new(journal), executor.clone(), 8, false)
                .await
                .unwrap();
        while recovered.wait_next().await.unwrap() {}
        assert!(executor.maximum.load(Ordering::SeqCst) <= 1);
        let delivery = recovered.prepare_delivery().await.unwrap().unwrap();
        assert_eq!(delivery.call_ids.len(), 2);
        recovered
            .pump(stream(vec![done("receipt")]), |_| {})
            .await
            .unwrap();
        recovered.register(call("cancel", true)).await.unwrap();
        recovered.cancel().await.unwrap();
        assert!(!recovered.wait_next().await.unwrap());
        assert!(!recovered.checkpoint().can_complete());
        assert!(
            recovered.prepare_delivery().await.unwrap().unwrap().input[0]["output"]
                .as_str()
                .unwrap()
                .contains("cancelled")
        );
    }

    #[tokio::test]
    async fn failed_journal_write_prevents_dispatch() {
        struct FailsAfterOpen(AtomicUsize);
        #[async_trait]
        impl NativeAsyncJournal for FailsAfterOpen {
            async fn load(&self) -> Result<NativeAsyncCheckpoint> {
                Ok(NativeAsyncCheckpoint::default())
            }
            async fn save(&self, _: &NativeAsyncCheckpoint) -> Result<()> {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(AgentLoopError::store("disk unavailable"))
                }
            }
        }
        let executor = Arc::new(Executor::default());
        let mut coordinator = NativeAsyncCoordinator::open(
            Box::new(FailsAfterOpen(AtomicUsize::new(0))),
            executor.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        assert!(
            coordinator
                .register(call("must_not_run", true))
                .await
                .is_err()
        );
        assert_eq!(executor.started.load(Ordering::SeqCst), 0);
        assert!(coordinator.wait_next().await.is_err());
        assert!(
            coordinator
                .register(call("must_not_run", true))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn authorization_and_incomplete_stream_do_not_silently_finish() {
        let journal = MemoryJournal::default();
        let executor = Arc::new(Executor::default());
        let mut coordinator =
            NativeAsyncCoordinator::open(Box::new(journal.clone()), executor.clone(), 1, true)
                .await
                .unwrap();
        let forbidden = NativeToolCall::Function {
            call_id: "bad".into(),
            name: "forbidden".into(),
            arguments: "{}".into(),
            asynchronous: true,
        };
        assert!(coordinator.register(forbidden).await.is_err());
        assert!(coordinator.checkpoint().calls.is_empty());
        assert!(
            coordinator
                .pump(
                    stream(vec![LlmStreamEvent::NativeToolCall(call("pending", true))]),
                    |_| {}
                )
                .await
                .is_err()
        );
        drop(coordinator);
        assert!(
            NativeAsyncCoordinator::open(Box::new(journal), executor, 1, true)
                .await
                .is_err()
        );
    }
}
