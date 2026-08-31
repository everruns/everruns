//! Delivering background-run completions to a host that owns the turn loop.
//!
//! `spawn_background` detaches a tool from the turn and, when it finishes,
//! signals the session through `PlatformStore::send_message`. For an embedded
//! host that is not as simple as running a turn: if the session has a *live
//! host loop* — a terminal UI, an editor session, anything already driving
//! turns for it — the completion must reach that loop and be run when it is
//! idle. Running it underneath the host instead means two turns on one session
//! at once.
//!
//! [`HostRoutedRunner`] is that rule, and only that rule. It decorates any
//! [`LocalSessionRunner`]:
//!
//! - session has a registered route → hand the message to the host's channel
//! - no route (a child/subagent session with nobody watching) → delegate to the
//!   inner runner, which runs the turn synchronously
//!
//! Everything else passes straight through. What the host does with a delivered
//! wake — coalescing, enriching it with session state, when to run it — stays
//! with the host, because that part is genuinely host-specific.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::session::ExecutionSession;
use everruns_platform::{PlatformCreateSessionRequest, PlatformMessage};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::platform_store::LocalSessionRunner;

/// Registry of sessions with a live host loop.
///
/// A host registers a session when it starts driving it and drops the receiver
/// when it stops; a send to a dropped receiver prunes the route, so a crashed
/// or closed host does not keep swallowing wakes.
#[derive(Debug, Clone, Default)]
pub struct WakeRoutes {
    routes: Arc<AsyncMutex<HashMap<SessionId, UnboundedSender<String>>>>,
}

impl WakeRoutes {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `session_id` as host-driven and take the receiving end.
    ///
    /// Registering a session that already has a route replaces it — the newest
    /// loop wins, which is what a reconnecting host wants.
    pub async fn register(&self, session_id: SessionId) -> UnboundedReceiver<String> {
        let (sender, receiver) = unbounded_channel();
        self.routes.lock().await.insert(session_id, sender);
        receiver
    }

    /// Stop routing to `session_id`.
    pub async fn unregister(&self, session_id: SessionId) {
        self.routes.lock().await.remove(&session_id);
    }

    /// Sessions currently claimed by a host loop.
    pub async fn live_sessions(&self) -> Vec<SessionId> {
        self.routes.lock().await.keys().copied().collect()
    }
}

/// Wraps a [`LocalSessionRunner`] so background completions reach a live host
/// loop instead of running a turn underneath it.
pub struct HostRoutedRunner<R: LocalSessionRunner> {
    inner: R,
    routes: WakeRoutes,
}

impl<R: LocalSessionRunner> HostRoutedRunner<R> {
    /// Decorate `inner`, routing through `routes`.
    pub fn new(inner: R, routes: WakeRoutes) -> Self {
        Self { inner, routes }
    }

    /// The route registry, for hosts that want to register sessions after
    /// construction.
    pub fn routes(&self) -> &WakeRoutes {
        &self.routes
    }

    /// The wrapped runner.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

#[async_trait]
impl<R: LocalSessionRunner> LocalSessionRunner for HostRoutedRunner<R> {
    async fn routable_session_ids(&self) -> Result<Option<Vec<SessionId>>> {
        // Live host sessions are always routable. If the inner runner scopes
        // its own routes, union the two; if it routes everything (`None`), so
        // do we.
        let live = self.routes.live_sessions().await;
        match self.inner.routable_session_ids().await? {
            None => Ok(None),
            Some(mut inner) => {
                for session_id in live {
                    if !inner.contains(&session_id) {
                        inner.push(session_id);
                    }
                }
                Ok(Some(inner))
            }
        }
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let mut routes = self.routes.routes.lock().await;
        if let Some(sender) = routes.get(&session_id) {
            if sender.send(content.to_string()).is_ok() {
                return Ok(());
            }
            routes.remove(&session_id);
        }

        // Nobody is driving this session, or its host went away. Deliver this
        // wake synchronously now because immediate background completions are
        // not retried. Keep registration serialized until the turn finishes so
        // a reconnecting host cannot start driving the same session underneath it.
        self.inner.send_message(session_id, content).await
    }

    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        self.inner
            .create_session(harness_id, agent_id, title, locale, parent_session_id)
            .await
    }

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        self.inner.create_session_with_options(request).await
    }

    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>> {
        self.inner.list_sessions(limit, agent_id).await
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.inner.get_session(session_id).await
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        self.inner.get_messages(session_id, limit).await
    }

    async fn get_session_status(&self, session_id: SessionId) -> Result<Option<String>> {
        self.inner.get_session_status(session_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::error::AgentLoopError;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct RecordingRunner {
        delivered: Mutex<Vec<(SessionId, String)>>,
        turns_run: AtomicUsize,
    }

    #[async_trait]
    impl LocalSessionRunner for RecordingRunner {
        async fn create_session(
            &self,
            harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _title: Option<&str>,
            _locale: Option<&str>,
            _parent_session_id: Option<SessionId>,
        ) -> Result<ExecutionSession> {
            let _ = harness_id;
            Err(AgentLoopError::tool("create_session unused in these tests"))
        }

        async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
            // Stands in for "ran the turn synchronously".
            self.turns_run.fetch_add(1, Ordering::SeqCst);
            self.delivered
                .lock()
                .unwrap()
                .push((session_id, content.to_string()));
            Ok(())
        }

        async fn list_sessions(
            &self,
            _limit: Option<usize>,
            _agent_id: Option<AgentId>,
        ) -> Result<Vec<ExecutionSession>> {
            Ok(vec![])
        }

        async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
            Ok(None)
        }

        async fn get_messages(
            &self,
            _session_id: SessionId,
            _limit: Option<usize>,
        ) -> Result<Vec<PlatformMessage>> {
            Ok(vec![])
        }

        async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
            Ok(Some("idle".to_string()))
        }
    }

    #[tokio::test]
    async fn a_live_host_session_receives_the_wake_instead_of_running_a_turn() {
        let routes = WakeRoutes::new();
        let session_id = SessionId::new_random();
        let mut receiver = routes.register(session_id).await;
        let runner = HostRoutedRunner::new(RecordingRunner::default(), routes);

        runner
            .send_message(session_id, "Background run completed.")
            .await
            .expect("wake should be routed");

        assert_eq!(
            receiver.try_recv().expect("host receives the wake"),
            "Background run completed."
        );
        assert_eq!(
            runner.inner().turns_run.load(Ordering::SeqCst),
            0,
            "a turn must not run underneath the host loop"
        );
    }

    #[tokio::test]
    async fn a_session_with_no_host_loop_falls_through_to_a_synchronous_turn() {
        // Child/subagent sessions have nobody watching; the inner runner drives
        // them directly.
        let runner = HostRoutedRunner::new(RecordingRunner::default(), WakeRoutes::new());
        let session_id = SessionId::new_random();

        runner
            .send_message(session_id, "Background run completed.")
            .await
            .expect("wake should reach the inner runner");

        assert_eq!(runner.inner().turns_run.load(Ordering::SeqCst), 1);
        assert_eq!(runner.inner().delivered.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_closed_host_channel_prunes_the_route_and_delivers_synchronously() {
        let routes = WakeRoutes::new();
        let session_id = SessionId::new_random();
        let receiver = routes.register(session_id).await;
        drop(receiver); // the host went away
        let runner = HostRoutedRunner::new(RecordingRunner::default(), routes.clone());

        runner
            .send_message(session_id, "Background run completed.")
            .await
            .expect("a dead receiver should fall through to the inner runner");
        assert!(
            routes.live_sessions().await.is_empty(),
            "the dead route must be pruned"
        );
        assert_eq!(runner.inner().turns_run.load(Ordering::SeqCst), 1);
        assert_eq!(
            runner.inner().delivered.lock().unwrap().as_slice(),
            &[(session_id, "Background run completed.".to_string())]
        );
    }

    #[tokio::test]
    async fn re_registering_replaces_the_route_and_a_stale_send_does_not_prune_it() {
        let routes = WakeRoutes::new();
        let session_id = SessionId::new_random();
        let stale = routes.register(session_id).await;
        drop(stale);
        let mut fresh = routes.register(session_id).await;
        let runner = HostRoutedRunner::new(RecordingRunner::default(), routes.clone());

        runner
            .send_message(session_id, "second wake")
            .await
            .expect("the newest loop wins");

        assert_eq!(
            fresh.try_recv().expect("fresh host receives"),
            "second wake"
        );
        assert_eq!(routes.live_sessions().await, vec![session_id]);
    }

    #[tokio::test]
    async fn reconnect_waits_for_a_synchronous_turn_to_finish() {
        use tokio::sync::Notify;
        use tokio::time::{Duration, timeout};

        struct BlockingRunner {
            entered: Arc<Notify>,
            release: Arc<Notify>,
        }

        #[async_trait]
        impl LocalSessionRunner for BlockingRunner {
            async fn create_session(
                &self,
                _harness_id: HarnessId,
                _agent_id: Option<AgentId>,
                _title: Option<&str>,
                _locale: Option<&str>,
                _parent_session_id: Option<SessionId>,
            ) -> Result<ExecutionSession> {
                Err(AgentLoopError::tool("create_session unused in these tests"))
            }

            async fn send_message(&self, _session_id: SessionId, _content: &str) -> Result<()> {
                self.entered.notify_one();
                self.release.notified().await;
                Ok(())
            }

            async fn list_sessions(
                &self,
                _limit: Option<usize>,
                _agent_id: Option<AgentId>,
            ) -> Result<Vec<ExecutionSession>> {
                Ok(vec![])
            }

            async fn get_session(
                &self,
                _session_id: SessionId,
            ) -> Result<Option<ExecutionSession>> {
                Ok(None)
            }

            async fn get_messages(
                &self,
                _session_id: SessionId,
                _limit: Option<usize>,
            ) -> Result<Vec<PlatformMessage>> {
                Ok(vec![])
            }

            async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
                Ok(None)
            }
        }

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let routes = WakeRoutes::new();
        let session_id = SessionId::new_random();
        let runner = Arc::new(HostRoutedRunner::new(
            BlockingRunner {
                entered: entered.clone(),
                release: release.clone(),
            },
            routes.clone(),
        ));

        let send = tokio::spawn({
            let runner = runner.clone();
            async move { runner.send_message(session_id, "first wake").await }
        });
        entered.notified().await;

        let mut registration = tokio::spawn({
            let routes = routes.clone();
            async move { routes.register(session_id).await }
        });
        assert!(
            timeout(Duration::from_millis(20), &mut registration)
                .await
                .is_err(),
            "a reconnect must not become live while the inner turn is running"
        );

        release.notify_one();
        send.await.expect("send task should finish").unwrap();
        let mut receiver = registration.await.expect("registration should finish");

        runner
            .send_message(session_id, "second wake")
            .await
            .expect("wake should reach the reconnected host");
        assert_eq!(receiver.try_recv().unwrap(), "second wake");
    }

    #[tokio::test]
    async fn live_sessions_are_reported_routable_alongside_the_inner_scope() {
        struct ScopedRunner(SessionId);

        #[async_trait]
        impl LocalSessionRunner for ScopedRunner {
            async fn routable_session_ids(&self) -> Result<Option<Vec<SessionId>>> {
                Ok(Some(vec![self.0]))
            }
            async fn create_session(
                &self,
                harness_id: HarnessId,
                _agent_id: Option<AgentId>,
                _title: Option<&str>,
                _locale: Option<&str>,
                _parent: Option<SessionId>,
            ) -> Result<ExecutionSession> {
                let _ = harness_id;
                Err(AgentLoopError::tool("create_session unused in these tests"))
            }
            async fn send_message(&self, _session_id: SessionId, _content: &str) -> Result<()> {
                Ok(())
            }
            async fn list_sessions(
                &self,
                _limit: Option<usize>,
                _agent_id: Option<AgentId>,
            ) -> Result<Vec<ExecutionSession>> {
                Ok(vec![])
            }
            async fn get_session(
                &self,
                _session_id: SessionId,
            ) -> Result<Option<ExecutionSession>> {
                Ok(None)
            }
            async fn get_messages(
                &self,
                _session_id: SessionId,
                _limit: Option<usize>,
            ) -> Result<Vec<PlatformMessage>> {
                Ok(vec![])
            }
            async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
                Ok(None)
            }
        }

        let child = SessionId::new_random();
        let host_session = SessionId::new_random();
        let routes = WakeRoutes::new();
        let _receiver = routes.register(host_session).await;
        let runner = HostRoutedRunner::new(ScopedRunner(child), routes);

        let routable = runner
            .routable_session_ids()
            .await
            .expect("routable")
            .expect("scoped");

        assert!(routable.contains(&child));
        assert!(routable.contains(&host_session));
    }
}
