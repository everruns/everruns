use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::{error, info};
use uuid::Uuid;

use super::SlackDeliveryDispatcher;

/// Bound independent session workers so one workspace cannot monopolize the
/// runtime while still allowing unrelated workspaces to make progress.
const MAX_CONCURRENT_SESSION_DELIVERIES: usize = 32;

pub(super) struct SessionDeliveryScheduler {
    workers: JoinSet<Uuid>,
    running: HashSet<Uuid>,
    pending: VecDeque<Uuid>,
    pending_set: HashSet<Uuid>,
}

impl SessionDeliveryScheduler {
    pub(super) fn new() -> Self {
        Self {
            workers: JoinSet::new(),
            running: HashSet::new(),
            pending: VecDeque::new(),
            pending_set: HashSet::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    pub(super) fn schedule(&mut self, dispatcher: Arc<SlackDeliveryDispatcher>, session_id: Uuid) {
        if self.running.contains(&session_id)
            || self.workers.len() >= MAX_CONCURRENT_SESSION_DELIVERIES
        {
            if self.pending_set.insert(session_id) {
                self.pending.push_back(session_id);
            }
            return;
        }

        self.spawn(dispatcher, session_id);
    }

    pub(super) async fn join_next(&mut self, dispatcher: Arc<SlackDeliveryDispatcher>) {
        match self.workers.join_next().await {
            Some(Ok(session_id)) => {
                self.running.remove(&session_id);
            }
            Some(Err(error)) => error!(%error, "Slack delivery worker failed"),
            None => {}
        }

        while self.workers.len() < MAX_CONCURRENT_SESSION_DELIVERIES {
            let Some(session_id) = self.pending.pop_front() else {
                break;
            };
            self.pending_set.remove(&session_id);
            if !self.running.contains(&session_id) {
                self.spawn(dispatcher.clone(), session_id);
            }
        }
    }

    pub(super) fn abort_all(&mut self) {
        info!("Slack delivery dispatcher shutting down");
        self.workers.abort_all();
    }

    fn spawn(&mut self, dispatcher: Arc<SlackDeliveryDispatcher>, session_id: Uuid) {
        self.running.insert(session_id);
        self.workers.spawn(async move {
            dispatcher.process_session_events(session_id).await;
            session_id
        });
    }
}
