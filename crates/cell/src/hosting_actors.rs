//! The cell's session actor store: Engine's epochs (claim, renew,
//! progress, release, hand-off) and the journal its holders read, as Raft
//! commands, and the spend allowance check before each provider call.
//! Split from `hosting.rs` for size.

use super::*;

/// `backends` with the cell's allowance as the turn's budget authority: the
/// Engine's budget gate asks it before every provider call, and the cell
/// reserves the call's maximum from the session's tenant (see `allowance`).
pub(super) fn with_allowance(
    inner: Arc<Inner>,
    session: SessionId,
    mut backends: HostBackends,
) -> HostBackends {
    let checker: Arc<dyn everruns_contracts::runtime::tool_execution::BudgetChecker> =
        Arc::new(CellBudget { inner, session });
    backends.extensions.authorities.budget = Some(Arc::new(move |_| Some(checker.clone())));
    backends
}

struct CellBudget {
    inner: Arc<Inner>,
    session: SessionId,
}

#[async_trait]
impl everruns_contracts::runtime::tool_execution::BudgetChecker for CellBudget {
    async fn check_budgets(
        &self,
        _session_id: &str,
    ) -> Result<everruns_contracts::runtime::budget::BudgetToolResponse> {
        let command = Command::Reserve {
            session: self.session,
            now_ms: now_ms(),
        };
        match self.inner.propose(self.session, command).await? {
            Response::Value { value } => serde_json::from_value(value).map_err(store_err),
            other => Err(store_err(format!("reserve: {other:?}"))),
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
