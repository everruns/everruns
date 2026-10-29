//! Canonical, server-owned authority for the worker RPC and byte listener.
//!
//! Values carry resolved authority, not credentials or a wire format. The
//! legacy bearer retains its existing broad authority. No production listener
//! issues journal authority; enrollment, scoped listener qualification and
//! direct hosting qualification remain prerequisites to enabling it.

use super::assignment_scope::ActorJournalScope;
use super::journal_capability::VerifiedJournalScope;
use axum::http::Extensions;
use tonic::{Request, Status};

#[cfg(test)]
mod tests;

/// Authority resolved by the server, never by caller metadata or selectors.
/// There is deliberately no default: losing this context must fail closed.
#[derive(Clone, Debug)]
pub enum WorkerAuthority {
    /// Shared bearer verified by the existing trusted-worker auth adapter.
    LegacyWorker,
    /// Explicit authentication-disabled development adapter.
    Development,
    /// A trusted adapter's resolved assignment, not an identity proof itself.
    ActorJournal(ActorJournalScope),
    /// mTLS peer and opaque capability resolved against live server records.
    VerifiedJournal(VerifiedJournalScope),
}

fn invalid_context() -> Status {
    Status::unauthenticated("Missing or ambiguous worker authority context")
}

/// A legacy authenticator must not overwrite a previous scoped assertion.
pub(super) fn ensure_unset(extensions: &Extensions) -> Result<(), Status> {
    if extensions.get::<WorkerAuthority>().is_some()
        || extensions.get::<ActorJournalScope>().is_some()
        || extensions.get::<VerifiedJournalScope>().is_some()
    {
        return Err(invalid_context());
    }
    Ok(())
}

/// Both transports consume this one context. An old adapter's standalone scope
/// is refused even beside canonical legacy authority, rather than downgraded.
pub(crate) fn resolve(extensions: &Extensions) -> Result<&WorkerAuthority, Status> {
    if extensions.get::<ActorJournalScope>().is_some()
        || extensions.get::<VerifiedJournalScope>().is_some()
    {
        return Err(invalid_context());
    }
    extensions
        .get::<WorkerAuthority>()
        .ok_or_else(invalid_context)
}

/// Every generated RPC delegate passes this gate; scoped future RPCs inherit
/// denial. Explicit legacy/development authority preserves its trusted mode.
pub(super) fn check_rpc<T>(request: &Request<T>, rpc: &str) -> Result<(), Status> {
    match resolve(request.extensions())? {
        WorkerAuthority::LegacyWorker | WorkerAuthority::Development => Ok(()),
        WorkerAuthority::ActorJournal(_) | WorkerAuthority::VerifiedJournal(_)
            if rpc == "read_actor_journal" =>
        {
            Ok(())
        }
        WorkerAuthority::ActorJournal(_) | WorkerAuthority::VerifiedJournal(_) => Err(
            Status::permission_denied("Worker assignment does not permit this operation"),
        ),
    }
}

/// Current wire-only protection contract. Unsupported future floors fail closed.
const INGRESS_PROTECTION_VERSION: i32 = 1;

/// A global persisted floor is necessary while legacy global RPCs and signed
/// byte URLs have no trustworthy assignment selector. Never cache absence: a
/// legacy channel must not survive cutover by reconnecting or changing IDs.
pub(crate) async fn admit_ingress(
    db: &crate::storage::StorageBackend,
    authority: &WorkerAuthority,
) -> Result<(), Status> {
    let floor = db.worker_ingress_minimum_version().await.map_err(|error| {
        super::worker::support::internal_status("Failed to validate worker ingress", error)
    })?;
    let Some(version) = floor else {
        return if matches!(authority, WorkerAuthority::VerifiedJournal(_)) {
            Err(Status::failed_precondition(
                "Protected worker ingress is not enabled",
            ))
        } else {
            Ok(())
        };
    };
    match authority {
        WorkerAuthority::VerifiedJournal(_) if version <= INGRESS_PROTECTION_VERSION => Ok(()),
        WorkerAuthority::VerifiedJournal(_) => Err(Status::unavailable(
            "Worker ingress protection version is unsupported",
        )),
        WorkerAuthority::LegacyWorker
        | WorkerAuthority::Development
        | WorkerAuthority::ActorJournal(_) => Err(Status::permission_denied(
            "Protected worker ingress requires verified assignment authority",
        )),
    }
}
