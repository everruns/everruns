//! Session actors: the runnable index and lease protocol (actor-based design).
//!
//! Handler bodies for the `WorkerService` RPCs in this group; the trait impl in
//! `super::super::worker_service_impl` forwards to them.
//!
//! Decision: these are dedicated RPCs, not internal commands. Like the leased
//! resource sweeper, a lease call picks work across every org, outside any one
//! org's turn, so there is no org for a command to run as. The bearer token on
//! the worker link is the authorization, as for every worker RPC. The
//! in-process path (`DirectWorkerAdapters`) calls the same repository methods.

use std::time::Duration;

use crate::storage::DEFAULT_ACTOR_LEASES_PER_ORG;
use crate::worker_link::grpc_service::worker::support::{
    internal_status, session_not_found_status,
};
use crate::worker_link::grpc_service::*;
use everruns_contracts::typed_id::SessionId;

/// The lease life a request asks for; 0 means the core default.
fn requested_actor_lease_ttl(ttl_ms: u32) -> Duration {
    if ttl_ms == 0 {
        everruns_core::host::DEFAULT_SESSION_LEASE_TTL
    } else {
        Duration::from_millis(u64::from(ttl_ms))
    }
}

/// Encoded bytes one journal page may carry. Decision: a quarter of the gRPC
/// message cap, so a page always fits with room for the envelope. Counting
/// events alone did not: one turn's 400 events encoded to ~200 MB, over the
/// 64 MiB cap, and every lease of that session failed on the read.
const ACTOR_JOURNAL_PAGE_BYTES: usize = everruns_internal_protocol::MAX_GRPC_MESSAGE_SIZE / 4;

/// Keep the leading `events` that fit `budget` encoded bytes, always at least
/// one so paging advances. `true` when some were left for the next page.
fn fit_journal_page(events: &mut Vec<proto::Event>, budget: usize) -> bool {
    let mut used = 0;
    let fits = events
        .iter()
        .position(|event| {
            used += prost::Message::encoded_len(event);
            used > budget
        })
        .map_or(events.len(), |over| over.max(1));
    let cut = fits < events.len();
    events.truncate(fits);
    cut
}

fn requested_max_per_org(max_leased_per_org: u32) -> i64 {
    if max_leased_per_org == 0 {
        DEFAULT_ACTOR_LEASES_PER_ORG
    } else {
        i64::from(max_leased_per_org)
    }
}

fn require_holder(holder: &str) -> Result<(), Status> {
    if holder.trim().is_empty() {
        return Err(Status::invalid_argument("Missing holder"));
    }
    Ok(())
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_lease_actors(
        &self,
        request: Request<proto::LeaseActorsRequest>,
    ) -> Result<Response<proto::LeaseActorsResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let rows = self
            .db
            .lease_session_actors(
                &req.holder,
                i64::from(req.limit),
                requested_actor_lease_ttl(req.ttl_ms),
                requested_max_per_org(req.max_leased_per_org),
            )
            .await
            .map_err(|e| internal_status("Failed to lease actors", e))?;
        Ok(Response::new(proto::LeaseActorsResponse {
            actors: rows
                .into_iter()
                .map(|row| proto::LeasedActor {
                    session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                        row.session_id.uuid(),
                    )),
                    org_id: row.org_id,
                    fence: row.fence,
                    inbox_seq: row.inbox_seq,
                    cancel_requested: row.cancel_requested(),
                    cancel_inbox_seq: row.cancel_inbox_seq,
                    consumed_seq: row.consumed_seq,
                    next_input_seq: row.next_input_seq,
                    attempts: u32::try_from(row.attempts).unwrap_or(0),
                })
                .collect(),
        }))
    }

    pub(crate) async fn handle_renew_actor_lease(
        &self,
        request: Request<proto::RenewActorLeaseRequest>,
    ) -> Result<Response<proto::RenewActorLeaseResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let session_id = SessionId::from_uuid(parse_uuid(req.session_id.as_ref())?);
        let renewal = self
            .db
            .renew_session_actor_lease(
                session_id,
                &req.holder,
                req.fence,
                requested_actor_lease_ttl(req.ttl_ms),
                req.consumed_seq,
            )
            .await
            .map_err(|e| internal_status("Failed to renew actor lease", e))?;
        // A lease lost to a missing actor row is a deleted session: the row
        // goes only with it (FK cascade). `NotFound` naming it lets the worker
        // stop the turn quietly (EVE-1235), as the in-process store does.
        if renewal.is_none()
            && self
                .db
                .get_session_actor(session_id)
                .await
                .map_err(|e| internal_status("Failed to renew actor lease", e))?
                .is_none()
        {
            return Err(session_not_found_status(session_id));
        }
        Ok(Response::new(match renewal {
            Some(renewal) => proto::RenewActorLeaseResponse {
                held: true,
                inbox_seq: renewal.inbox_seq,
                cancel_requested: renewal.cancel_requested(),
                cancel_inbox_seq: renewal.cancel_inbox_seq,
            },
            None => proto::RenewActorLeaseResponse::default(),
        }))
    }

    pub(crate) async fn handle_release_actor(
        &self,
        request: Request<proto::ReleaseActorRequest>,
    ) -> Result<Response<proto::ReleaseActorResponse>, Status> {
        let req = request.into_inner();
        require_holder(&req.holder)?;
        let session_id = SessionId::from_uuid(parse_uuid(req.session_id.as_ref())?);
        let released = self
            .db
            .release_session_actor(session_id, &req.holder, req.fence, req.consumed_inbox_seq)
            .await
            .map_err(|e| internal_status("Failed to release actor", e))?;
        Ok(Response::new(proto::ReleaseActorResponse {
            released: released.is_some(),
            still_runnable: released.unwrap_or(false),
        }))
    }

    pub(crate) async fn handle_read_actor_journal(
        &self,
        request: Request<proto::ReadActorJournalRequest>,
    ) -> Result<Response<proto::ReadActorJournalResponse>, Status> {
        let scope = match super::super::authority::resolve(request.extensions())? {
            super::super::authority::WorkerAuthority::ActorJournal(scope) => Some(scope.clone()),
            super::super::authority::WorkerAuthority::LegacyWorker
            | super::super::authority::WorkerAuthority::Development => None,
        };
        let req = request.into_inner();
        let session_id = SessionId::from_uuid(parse_uuid(req.session_id.as_ref())?);
        if let Some(scope) = scope {
            scope.admit_read(&self.db, session_id).await?;
        }
        let journal = self
            .db
            .read_session_actor_journal(session_id, req.after_sequence, i64::from(req.limit))
            .await
            .map_err(|e| internal_status("Failed to read actor journal", e))?;
        let mut events: Vec<proto::Event> = journal
            .events
            .into_iter()
            .map(|row| schema_event_to_proto(&EventService::row_to_event(row)))
            .collect();
        let cut = fit_journal_page(&mut events, ACTOR_JOURNAL_PAGE_BYTES);
        Ok(Response::new(proto::ReadActorJournalResponse {
            events,
            has_more: journal.has_more || cut,
            head_sequence: journal.head_sequence,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: i32, payload: usize) -> proto::Event {
        proto::Event {
            event_type: "x".repeat(payload),
            sequence,
            ..Default::default()
        }
    }

    fn sequences(events: &[proto::Event]) -> Vec<i32> {
        events.iter().map(|event| event.sequence).collect()
    }

    #[test]
    fn a_journal_page_stops_at_its_byte_budget() {
        let mut events = vec![event(1, 40), event(2, 40), event(3, 40)];
        assert!(fit_journal_page(&mut events, 100));
        assert_eq!(sequences(&events), vec![1, 2]);
    }

    #[test]
    fn a_journal_page_keeps_one_event_over_the_budget_so_paging_advances() {
        let mut events = vec![event(1, 500), event(2, 10)];
        assert!(fit_journal_page(&mut events, 100));
        assert_eq!(sequences(&events), vec![1]);
    }

    #[test]
    fn a_journal_page_within_the_budget_is_whole() {
        let mut events = vec![event(1, 10), event(2, 10)];
        assert!(!fit_journal_page(&mut events, 100));
        assert_eq!(sequences(&events), vec![1, 2]);
    }
}
