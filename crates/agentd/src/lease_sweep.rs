//! Releasing what sessions' tools leased at a provider and left behind: a
//! sandbox, a browser session (`session_records.rs` keeps the leases).
//!
//! Decisions:
//! - agentd sweeps its own leases (agentd vs control plane review,
//!   2026-11-25). The control plane's cleanup activity used to sweep the
//!   copies it mirrors from the feed, so a standalone agentd, with no
//!   control plane, leaked every sandbox a session left running. The
//!   control plane now skips mirrored leases (`swept_by = data_plane`) and
//!   keeps sweeping v1's.
//! - Executors sweep, from the sessions they mirror: they hold what the
//!   provider calls need (the org's credentials and sandbox keys). A lease
//!   is claimed through its session's shard first, a compare-and-set on its
//!   state, so of several executors one cleans it; a claim older than
//!   [`STALE_AFTER`] whose sweep never reported is due again.
//! - The release itself is the worker's (`everruns_capabilities::
//!   leased_cleanup`), so both planes stop a provider resource the same way.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::runtime::LeasedResource;
use everruns_contracts::typed_id::SessionId;

use crate::hosting::RaftHosting;

/// How often an executor looks for due leases.
pub const EVERY: Duration = Duration::from_secs(60);
/// Leases one pass claims at most; the next pass takes the rest.
pub const BATCH: usize = 25;
/// A claim whose sweep did not report for this long is due again.
pub const STALE_AFTER: u32 = 5 * 60;
/// How long a failed release waits before it is tried again.
pub const RETRY_AFTER: u32 = 60;

/// What one pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pass {
    pub released: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Claim and release each of `due` (a session and one of its leases).
pub async fn sweep(hosting: &RaftHosting, due: Vec<(SessionId, LeasedResource)>) -> Pass {
    let mut pass = Pass::default();
    for (session, lease) in due {
        let (records, resolver, storage) = hosting.lease_tools(session);
        let claimed = match records.claim_lease(&lease, STALE_AFTER).await {
            Ok(Some(claimed)) => claimed,
            Ok(None) => {
                pass.skipped += 1;
                continue;
            }
            Err(error) => {
                tracing::debug!(%session, %error, "lease claim failed");
                pass.skipped += 1;
                continue;
            }
        };
        let outcome = everruns_capabilities::leased_cleanup::cleanup_resource(
            &claimed,
            resolver.as_ref(),
            storage.as_ref(),
        )
        .await;
        let error = match &outcome {
            Ok(detail) => {
                tracing::info!(%session, provider = %claimed.provider, external_id = %claimed.external_id, %detail, "released a leased resource");
                None
            }
            Err(error) => {
                tracing::warn!(%session, provider = %claimed.provider, external_id = %claimed.external_id, %error, "leased resource release failed; retrying later");
                Some(error.to_string())
            }
        };
        let failed = error.is_some();
        match records.lease_cleaned(&claimed, error, RETRY_AFTER).await {
            Ok(Some(_)) if failed => pass.failed += 1,
            Ok(Some(_)) => pass.released += 1,
            Ok(None) | Err(_) => pass.skipped += 1,
        }
    }
    pass
}

/// A resolver with no sandbox keys, for an executor that runs no managed
/// sandboxes: a lease's token then comes from the org's credentials.
pub(crate) struct NoKeys;

#[async_trait::async_trait]
impl everruns_core::UserConnectionResolver for NoKeys {
    async fn get_connection_token(
        &self,
        _session: SessionId,
        _provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }
}

pub(crate) fn no_keys() -> Arc<dyn everruns_core::UserConnectionResolver> {
    Arc::new(NoKeys)
}
