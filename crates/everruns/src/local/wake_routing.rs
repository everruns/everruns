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
use std::pin::pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::session::ExecutionSession;
use everruns_platform::{PlatformCreateSessionRequest, PlatformMessage};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::platform_store::LocalSessionRunner;

#[derive(Debug, Default)]
struct RouteState {
    /// Sessions with a live host loop.
    routes: HashMap<SessionId, UnboundedSender<String>>,
    /// Sessions with a synchronous fallback turn in flight, by depth.
    ///
    /// A depth rather than a flag because a turn can wake its own session
    /// again before it finishes; counting means the nested wake proceeds
    /// instead of waiting on something it is itself inside of.
    delivering: HashMap<SessionId, usize>,
}

/// Registry of sessions with a live host loop.
///
/// A host registers a session when it starts driving it and drops the receiver
/// when it stops; a send to a dropped receiver prunes the route, so a crashed
/// or closed host does not keep swallowing wakes.
#[derive(Debug, Clone, Default)]
pub struct WakeRoutes {
    state: Arc<Mutex<RouteState>>,
    /// Notified when a session's last in-flight fallback turn finishes.
    idle: Arc<Notify>,
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
    ///
    /// Async because it waits out a synchronous fallback turn already running
    /// for *this* session: becoming live underneath one is exactly the
    /// two-loops-on-one-session state this module exists to prevent. Other
    /// sessions' turns never block it, and no lock is held across the wait.
    pub async fn register(&self, session_id: SessionId) -> UnboundedReceiver<String> {
        let (sender, receiver) = unbounded_channel();
        loop {
            // Arm the waiter *before* reading `delivering`, so a turn that
            // finishes in the gap wakes us rather than leaving us parked.
            let mut idle = pin!(self.idle.notified());
            idle.as_mut().enable();

            {
                let mut state = self.state.lock().unwrap();
                if !state.delivering.contains_key(&session_id) {
                    state.routes.insert(session_id, sender);
                    return receiver;
                }
            }

            idle.await;
        }
    }

    /// Stop routing to `session_id`.
    pub fn unregister(&self, session_id: SessionId) {
        self.state.lock().unwrap().routes.remove(&session_id);
    }

    /// Sessions currently claimed by a host loop.
    pub fn live_sessions(&self) -> Vec<SessionId> {
        self.state.lock().unwrap().routes.keys().copied().collect()
    }

    /// Hand `content` to a live host, or claim the synchronous fallback.
    ///
    /// `None` means delivered. `Some(guard)` means the caller owns the fallback
    /// turn and must hold the guard until it finishes. Both the route decision
    /// and the claim happen under one lock, so `register` cannot slip between
    /// them; the lock is released before the turn runs.
    fn deliver_or_claim(&self, session_id: SessionId, content: &str) -> Option<Delivery> {
        let mut state = self.state.lock().unwrap();
        if let Some(sender) = state.routes.get(&session_id) {
            if sender.send(content.to_string()).is_ok() {
                return None;
            }
            // The host went away. Prune atomically with the failed send so a
            // concurrent registration cannot be removed.
            state.routes.remove(&session_id);
        }
        *state.delivering.entry(session_id).or_insert(0) += 1;
        Some(Delivery {
            routes: self.clone(),
            session_id,
        })
    }
}

/// Marks a synchronous fallback turn as in flight for one session.
struct Delivery {
    routes: WakeRoutes,
    session_id: SessionId,
}

impl Drop for Delivery {
    fn drop(&mut self) {
        let now_idle = {
            let mut state = self.routes.state.lock().unwrap();
            match state.delivering.get_mut(&self.session_id) {
                Some(depth) if *depth > 1 => {
                    *depth -= 1;
                    false
                }
                _ => {
                    state.delivering.remove(&self.session_id);
                    true
                }
            }
        };
        if now_idle {
            self.routes.idle.notify_waiters();
        }
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
        let live = self.routes.live_sessions();
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
        let Some(_delivery) = self.routes.deliver_or_claim(session_id, content) else {
            return Ok(());
        };

        // Nobody is driving this session, or its host went away. Deliver this
        // wake synchronously now because immediate background completions are
        // not retried. `_delivery` lives until this call returns, which is what
        // holds off a reconnecting host for *this* session; every other session
        // keeps running.
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
            routes.live_sessions().is_empty(),
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
        assert_eq!(routes.live_sessions(), vec![session_id]);
    }

    /// Stands in for a runner whose turn takes real time: parks in
    /// `send_message` until released, so a test can observe the window during
    /// which a fallback turn is in flight.
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
            Ok(None)
        }
    }

    #[tokio::test]
    async fn reconnect_waits_for_a_synchronous_turn_to_finish() {
        use tokio::time::{Duration, timeout};

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
    async fn a_fallback_turn_only_holds_off_its_own_sessions_reconnect() {
        // The hold-off above is per session. A fallback turn for one session
        // must not stall registration, unregistration, or delivery for any
        // other — those are independent host loops.
        use tokio::time::{Duration, timeout};

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let routes = WakeRoutes::new();
        let busy = SessionId::new_random();
        let other = SessionId::new_random();
        let runner = Arc::new(HostRoutedRunner::new(
            BlockingRunner {
                entered: entered.clone(),
                release: release.clone(),
            },
            routes.clone(),
        ));

        let send = tokio::spawn({
            let runner = runner.clone();
            async move { runner.send_message(busy, "wake for the busy session").await }
        });
        entered.notified().await;

        let mut receiver = timeout(Duration::from_secs(5), routes.register(other))
            .await
            .expect("another session's reconnect must not wait on the busy turn");
        assert_eq!(routes.live_sessions(), vec![other]);

        timeout(
            Duration::from_secs(5),
            runner.send_message(other, "wake for the other session"),
        )
        .await
        .expect("delivery to another session must not wait on the busy turn")
        .expect("wake should be routed");
        assert_eq!(
            receiver.try_recv().unwrap(),
            "wake for the other session",
            "the other host loop keeps receiving while the busy turn runs"
        );

        release.notify_one();
        send.await.expect("send task should finish").unwrap();
    }

    #[tokio::test]
    async fn a_turn_that_wakes_its_own_session_again_does_not_deadlock() {
        // `send_message` is reachable from inside a turn. A nested wake for the
        // same session must fall through to another turn, not park forever
        // waiting for the turn it is already inside of to finish.
        use tokio::time::{Duration, timeout};

        let runner = HostRoutedRunner::new(RecordingRunner::default(), WakeRoutes::new());
        let session_id = SessionId::new_random();

        let outer = runner.routes().deliver_or_claim(session_id, "outer wake");
        assert!(outer.is_some(), "no host loop, so this claims the fallback");

        timeout(
            Duration::from_secs(5),
            runner.send_message(session_id, "nested wake"),
        )
        .await
        .expect("a nested wake for the same session must not deadlock")
        .expect("wake should be delivered");

        drop(outer);
        assert_eq!(runner.inner().turns_run.load(Ordering::SeqCst), 1);
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
