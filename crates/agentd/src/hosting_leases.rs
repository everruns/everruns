//! What the lease sweep reaches a session through (`lease_sweep.rs`).
//! Split from `hosting.rs` for size.

use super::*;

impl RaftHosting {
    /// What the lease sweep needs for `session` (`lease_sweep.rs`): its
    /// records through its shard, its connection resolver (the org's
    /// credentials, and sandbox keys when this process runs sandboxes),
    /// and its storage for the provider state a release clears.
    pub(crate) fn lease_tools(
        &self,
        session: SessionId,
    ) -> (
        crate::session_records::ShardRecords,
        Arc<dyn everruns_core::UserConnectionResolver>,
        Arc<dyn everruns_core::session_services::SessionStorageStore>,
    ) {
        let records = crate::session_records::ShardRecords::new(Port(self.inner.clone()), session);
        let resolver = match self.inner.sandboxes.get() {
            Some(sandboxes) => sandboxes.connections(session),
            None => crate::lookups::connections(
                crate::lease_sweep::no_keys(),
                self.inner.material.get(),
                session,
            ),
        };
        (records, resolver, Arc::new(self.ports(session, None)))
    }
}
