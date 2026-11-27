//! The shard's session actor store: Engine's epochs (claim, renew,
//! progress, release, hand-off) and the journal its holders read, as Raft
//! commands, and the spend allowance check before each provider call.
//! Split from `hosting.rs` for size.

use super::*;

/// `backends` with the shard's allowance as the turn's budget authority: the
/// Engine's budget gate asks it before every provider call, and the shard
/// reserves the call's maximum from the session's tenant (see `allowance`).
pub(super) fn with_allowance(
    inner: Arc<Inner>,
    session: SessionId,
    mut backends: HostBackends,
) -> HostBackends {
    let checker: Arc<dyn everruns_contracts::runtime::tool_execution::BudgetChecker> =
        Arc::new(ShardBudget { inner, session });
    backends.extensions.authorities.budget = Some(Arc::new(move |_| Some(checker.clone())));
    backends
}

struct ShardBudget {
    inner: Arc<Inner>,
    session: SessionId,
}

#[async_trait]
impl everruns_contracts::runtime::tool_execution::BudgetChecker for ShardBudget {
    async fn check_budgets(
        &self,
        _session_id: &str,
    ) -> Result<everruns_contracts::runtime::budget::BudgetToolResponse> {
        // Decision: an unmetered session (no tenant) answers from the
        // replica or mirror, as the shard would, without a round trip. Its
        // tenant is set once, before it takes input, so the epoch that
        // runs this call already sees it.
        let unmetered = self
            .inner
            .read(self.session, |s| {
                s.tenant.is_none() && s.placement != crate::state::Placement::Prepared
            })
            .await
            .unwrap_or(false);
        if unmetered {
            return serde_json::from_value(
                serde_json::json!({ "status": "no_budgets", "budgets": [] }),
            )
            .map_err(store_err);
        }
        // Decision: fail closed. Engine's gate lets a call run when the
        // checker errors (fail open), which for a metered session would be
        // spend no reservation covers. So the shard retries through a leader
        // change and, if it still has no answer, reads the allowance as
        // exhausted: the turn ends as budget exhausted instead of spending.
        let mut delay = Duration::from_millis(50);
        let mut last = String::new();
        for attempt in 0..RESERVE_TRIES {
            if attempt > 0 {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(1));
            }
            let command = Command::Reserve {
                session: self.session,
                now_ms: now_ms(),
            };
            match self.inner.propose(self.session, command).await {
                Ok(Response::Value { value }) => {
                    if value["status"] == "exhausted" && self.expired_grant_pending_refresh().await
                    {
                        return self.wait_for_grant_refresh(value).await;
                    }
                    return serde_json::from_value(value).map_err(store_err);
                }
                Ok(other) => last = format!("{other:?}"),
                Err(error) => last = error.to_string(),
            }
        }
        tracing::warn!(session = %self.session, reason = %last, "no reservation; stopping the call");
        serde_json::from_value(serde_json::json!({ "status": "exhausted", "budgets": [] }))
            .map_err(store_err)
    }
}

impl ShardBudget {
    /// A grant may expire between provider calls in one running turn. The
    /// control loop asks for a replacement, so let that answer arrive before
    /// treating the expired grant as an org budget limit.
    async fn expired_grant_pending_refresh(&self) -> bool {
        let Ok(shared) = self.inner.caught_up(self.session).await else {
            return false;
        };
        let state = shared.state.read();
        let Some(tenant) = state
            .sessions
            .get(&self.session)
            .and_then(|s| s.tenant.as_deref())
        else {
            return false;
        };
        state
            .allowances
            .tenants
            .get(tenant)
            .is_some_and(|allowance| {
                allowance.refused.is_none()
                    && allowance
                        .grants
                        .values()
                        .any(|grant| grant.spend_until_ms <= now_ms())
            })
    }

    async fn wait_for_grant_refresh(
        &self,
        exhausted: serde_json::Value,
    ) -> Result<everruns_contracts::runtime::budget::BudgetToolResponse> {
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let command = Command::Reserve {
                session: self.session,
                now_ms: now_ms(),
            };
            if let Ok(Response::Value { value }) = self.inner.propose(self.session, command).await
                && (value["status"] != "exhausted" || self.allowance_refused().await)
            {
                return serde_json::from_value(value).map_err(store_err);
            }
        }
        // The control plane did not replenish the grant: fail closed.
        serde_json::from_value(exhausted).map_err(store_err)
    }

    async fn allowance_refused(&self) -> bool {
        let Ok(shared) = self.inner.caught_up(self.session).await else {
            return false;
        };
        let state = shared.state.read();
        let Some(tenant) = state
            .sessions
            .get(&self.session)
            .and_then(|s| s.tenant.as_deref())
        else {
            return false;
        };
        state
            .allowances
            .tenants
            .get(tenant)
            .is_some_and(|allowance| allowance.refused.is_some())
    }
}

/// Reservation attempts before a call is refused (about 5 s with backoff).
const RESERVE_TRIES: usize = 9;

/// What a running epoch last saw of its inbox, so a change that moves it
/// (new input, a cancel, a resume) pokes the renewer and one that doesn't
/// (the epoch's own writes) does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Seen {
    inbox: i64,
    cancel: Option<i64>,
    resume: u64,
}

impl Seen {
    pub(super) fn of(s: &SessionState) -> Self {
        Self {
            inbox: s.authority.inbox,
            cancel: s.authority.cancel,
            resume: s.authority.resume_generation,
        }
    }
}

/// An epoch's renew poke (`SessionEpoch::renew_poke`).
pub(super) struct Poke {
    pub(super) notify: Arc<tokio::sync::Notify>,
    seen: parking_lot::Mutex<Seen>,
}

impl Poke {
    pub(super) fn new(seen: Seen) -> Self {
        Self {
            notify: Arc::default(),
            seen: parking_lot::Mutex::new(seen),
        }
    }
}

impl Inner {
    /// Poke `session`'s running epoch if its inbox moved. As the in-memory
    /// authority does: a cancel keeps a permit (it must not be missed), an
    /// admission only wakes a renewer that waits (a late one moves the
    /// input to the next turn).
    pub(super) fn poke(&self, session: SessionId) {
        let Some(poke) = self.pokes.lock().get(&session).and_then(Weak::upgrade) else {
            return;
        };
        let shared = match &self.source {
            Source::Node(node) => match node.upgrade() {
                Some(node) => node.shared.clone(),
                None => return,
            },
            Source::Remote(remote) => remote.shared.clone(),
        };
        let Some(now) = shared.state.read().sessions.get(&session).map(Seen::of) else {
            return;
        };
        let mut seen = poke.seen.lock();
        if now == *seen {
            return;
        }
        let cancelled = now.cancel.is_some() && now.cancel != seen.cancel;
        *seen = now;
        drop(seen);
        if cancelled {
            poke.notify.notify_one();
        } else {
            poke.notify.notify_waiters();
        }
    }
}

#[async_trait]
impl SessionActorStore for Ports {
    async fn renew_actor_lease(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.scope(session)?;
        let response = self
            .order(Command::Renew {
                session,
                holder: holder.to_owned(),
                fence,
                ttl_ms: ttl.as_millis() as i64,
                consumed,
                receipts: self.riding(),
                now_ms: now_ms(),
            })
            .await?;
        match response {
            Response::Value { value } => Ok(Some(ActorRenewal {
                inbox_seq: value["inbox_seq"].as_i64().unwrap_or(0),
                cancel_inbox_seq: value["cancel_inbox_seq"].as_i64(),
            })),
            Response::Fenced => Ok(None),
            other => Err(refused(session, other)),
        }
    }

    async fn release_actor(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<ActorRelease> {
        self.release(session, holder, fence, consumed, false).await
    }

    async fn record_actor_progress(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
    ) -> Result<bool> {
        self.scope(session)?;
        match self
            .order(Command::Progress {
                session,
                holder: holder.to_owned(),
                fence,
                now_ms: now_ms(),
                forgive: false,
            })
            .await?
        {
            Response::Ok => Ok(true),
            Response::Fenced => Ok(false),
            other => Err(refused(session, other)),
        }
    }

    async fn forgive_actor_lease(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
    ) -> Result<bool> {
        self.scope(session)?;
        match self
            .order(Command::Progress {
                session,
                holder: holder.to_owned(),
                fence,
                now_ms: now_ms(),
                forgive: true,
            })
            .await?
        {
            Response::Ok => Ok(true),
            Response::Fenced => Ok(false),
            other => Err(refused(session, other)),
        }
    }

    async fn hand_off_actor(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<Option<ActorRelease>> {
        self.release(session, holder, fence, consumed, true)
            .await
            .map(Some)
    }

    async fn read_actor_journal(
        &self,
        session: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.scope(session)?;
        let limit = if limit == 0 {
            256
        } else {
            limit.min(1024) as usize
        };
        // From the latest `turn.started` when the reader names no position.
        let after = match after {
            Some(after) => after,
            None => {
                self.inner
                    .read(session, |s| (s.turn_start - 1).max(0))
                    .await?
            }
        };
        let (mut events, high, partial) = self.window(after, limit).await?;
        let has_more = events.len() > limit || (partial && !events.is_empty());
        events.truncate(limit);
        Ok(ActorJournal {
            events,
            has_more,
            head_sequence: (high > 0).then_some(high),
        })
    }
}

impl Ports {
    /// The receipts waiting to ride on this epoch's next lease write.
    fn riding(&self) -> Vec<(String, Value)> {
        self.epoch
            .as_ref()
            .map(|e| std::mem::take(&mut *e.riding.lock()))
            .unwrap_or_default()
    }

    async fn release(
        &self,
        session: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
        hand_off: bool,
    ) -> Result<ActorRelease> {
        self.scope(session)?;
        let response = self
            .order(Command::Release {
                session,
                holder: holder.to_owned(),
                fence,
                consumed,
                hand_off,
                receipts: self.riding(),
                now_ms: now_ms(),
            })
            .await?;
        match response {
            Response::Value { value } => Ok(ActorRelease::Released {
                still_runnable: value.as_bool().unwrap_or(false),
            }),
            Response::Fenced => Ok(ActorRelease::LeaseLost),
            other => Err(refused(session, other)),
        }
    }
}

pub(super) struct Guard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}
/// Run `f` when the returned guard drops.
pub(super) fn scopeguard<F: FnOnce()>(f: F) -> Guard<F> {
    Guard(Some(f))
}
