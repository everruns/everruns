use super::*;

#[derive(Clone)]
struct LostReply {
    inner: MemoryHosting,
    lost: Arc<std::sync::atomic::AtomicBool>,
}
#[async_trait::async_trait]
impl SessionHosting for LostReply {
    async fn command(
        &self,
        session: SessionId,
        command: SessionCommand,
    ) -> everruns_contracts::Result<Value> {
        let completion = matches!(&command, SessionCommand::CompleteClientDispatch { .. });
        let result = self.inner.command(session, command).await?;
        if completion && !self.lost.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(everruns_contracts::error::AgentLoopError::store(
                "injected response loss",
            ));
        }
        Ok(result)
    }
    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> everruns_contracts::Result<crate::hosting::SessionQuerySnapshot> {
        self.inner.query_snapshot(id).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> everruns_contracts::Result<crate::hosting::HostingStatus> {
        self.inner.status(id).await
    }
    async fn receipt(
        &self,
        id: SessionId,
        message: crate::MessageId,
    ) -> everruns_contracts::Result<Option<crate::hosting::AdmissionReceipt>> {
        self.inner.receipt(id, message).await
    }
    fn reader(&self, id: SessionId) -> crate::hosting::spi::HostBackends {
        self.inner.reader(id)
    }
    async fn record(&self, id: SessionId, key: &str) -> everruns_contracts::Result<Option<Value>> {
        self.inner.record(id, key).await
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> everruns_contracts::Result<Option<crate::hosting::SessionActivation>> {
        self.inner.claim(id, holder, ttl).await
    }
    fn definitions(&self) -> Option<&dyn crate::hosting::AgentDefinitions> {
        self.inner.definitions()
    }
    fn process_scheduler(&self) -> bool {
        true
    }
}
#[tokio::test]
async fn personal_browser_lost_completion_reply_still_wakes_and_never_replays() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = LostReply {
        inner: MemoryHosting::default(),
        lost: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let engine = engine(
        profile.clone(),
        agent(ToolCall {
            id: "lost".into(),
            name: STATUS_TOOL.into(),
            arguments: json!({}),
        }),
    );
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let admission = session.send("status").await.unwrap();
    wait_pending(&engine, id).await;
    assert_eq!(
        worker.run_pending(&engine, id).await.unwrap_err(),
        CoordinatorError::Authority
    );
    tokio::time::timeout(Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(profile.lost.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(worker.run_pending(&engine, id).await.unwrap().completed, 0);
    worker.close_session(&engine, id).await.unwrap();
}
#[tokio::test]
async fn personal_browser_slow_init_does_not_block_other_session_or_close() {
    let Some(cfg) = config() else {
        return;
    };
    let mut slow = cfg.clone();
    slow.args = vec![
        "-u".into(),
        "-c".into(),
        "import time;time.sleep(60)".into(),
    ];
    let engine = engine(
        MemoryHosting::default(),
        Agent::builder()
            .instructions("Test personal browser host tools.")
            .model(Model::simulated("ok"))
            .build()
            .unwrap(),
    );
    let first = SessionId::new();
    let second = SessionId::new();
    engine.create_registered(first, &build()).await.unwrap();
    engine.create_registered(second, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(
        move |session| {
            if session == first {
                slow.clone()
            } else {
                cfg.clone()
            }
        },
        2,
    )
    .unwrap();
    let init = {
        let worker = worker.clone();
        let engine = engine.clone();
        tokio::spawn(async move { worker.bind_session(&engine, first).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if worker.inner.bindings.lock().await.contains_key(&first) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), worker.bind_session(&engine, second))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), worker.close_session(&engine, first))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(init.await.unwrap().unwrap_err(), CoordinatorError::Closed);
    worker.close_session(&engine, second).await.unwrap();
}

#[tokio::test]
async fn personal_browser_denied_pretool_hook_never_becomes_dispatchable() {
    let Some(cfg) = config() else {
        return;
    };
    let profile = MemoryHosting::default();
    let call = ToolCall {
        id: "denied".into(),
        name: HANDOFF_TOOL.into(),
        arguments: json!({"tab":42,"message":"not allowed"}),
    };
    let agent = Agent::builder()
        .instructions("Test denied client tool.")
        .model(Model::simulated_with_config(
            LlmSimConfig::fixed("done").with_tool_call_sequence(vec![vec![call], vec![]]),
        ))
        .on_tool_start(|_| async { Err::<(), _>("Denied by test policy") })
        .build()
        .unwrap();
    let engine = engine(profile.clone(), agent);
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.enable_session(&engine, id).await.unwrap();
    let admission = session.send("blocked").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(engine.session_status(id).await.unwrap().waiting.is_empty());
    let report = worker.run_pending(&engine, id).await.unwrap();
    assert_eq!(report.completed, 0);
    assert_eq!(report.interrupted, 0);
    worker.close_session(&engine, id).await.unwrap();
}

#[tokio::test]
async fn personal_browser_close_reaps_even_when_configuration_cleanup_fails() {
    let Some(cfg) = config() else {
        return;
    };
    let engine = engine(
        MemoryHosting::default(),
        Agent::builder()
            .instructions("Test cleanup failure.")
            .model(Model::simulated("done"))
            .client_tool(
                STATUS_TOOL,
                "Conflicting host tool",
                json!({"type":"object"}),
            )
            .build()
            .unwrap(),
    );
    let id = SessionId::new();
    engine.create_registered(id, &build()).await.unwrap();
    let worker = PersonalBrowserCoordinator::new(move |_| cfg.clone(), 1).unwrap();
    worker.bind_session(&engine, id).await.unwrap();
    let binding = worker
        .inner
        .bindings
        .lock()
        .await
        .get(&id)
        .cloned()
        .unwrap();
    let state = binding
        .supervisor
        .lock()
        .await
        .as_ref()
        .unwrap()
        .watch_state();
    assert_eq!(*state.borrow(), BrowseState::Ready);
    assert_eq!(
        worker.close_session(&engine, id).await.unwrap_err(),
        CoordinatorError::InvalidConfiguration
    );
    assert_eq!(*state.borrow(), BrowseState::Closed);
    assert!(binding.supervisor.lock().await.is_none());
    assert_eq!(
        worker.run_pending(&engine, id).await.unwrap_err(),
        CoordinatorError::Closed
    );
}
