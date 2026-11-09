//! Host-facing MCP server readiness (EVE-1342).
//!
//! A projection of the per-session [`McpDiscoveryCache`] state, so embedding
//! hosts can render MCP status, drive reconnect and feed doctor output without
//! keeping a parallel readiness model. The cache stays the single owner of
//! discovery state; nothing here is stored separately.
//!
//! [`McpDiscoveryCache`]: crate::host::McpDiscoveryCache

use std::time::SystemTime;

use tokio::sync::watch;

use crate::host::McpDiscoveryStatus;

/// Readiness of one configured MCP server that discovers tools.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpServerReadiness {
    /// Configured server name.
    pub name: String,
    /// Discovery state of the server's current configuration.
    pub status: McpDiscoveryStatus,
    /// Deferred and not yet revealed in this session. Turns do not connect to
    /// it until it is revealed; an explicit probe does.
    pub deferred: bool,
    /// A discovery attempt (turn, background refresh or probe) is running.
    pub probing: bool,
    /// Tools in the catalog turns currently use. A failed refresh keeps the
    /// previous complete catalog, so `Failed` can still report tools.
    pub tool_count: usize,
    /// Why discovery failed or needs authentication. Bounded upstream text;
    /// `None` when ready or pending.
    pub reason: Option<String>,
    /// When the last discovery attempt for this configuration completed;
    /// `None` while no attempt has completed.
    pub last_probe: Option<SystemTime>,
}

impl McpServerReadiness {
    pub(crate) fn new(name: String, status: McpDiscoveryStatus) -> Self {
        Self {
            name,
            status,
            deferred: false,
            probing: false,
            tool_count: 0,
            reason: None,
            last_probe: None,
        }
    }
}

/// Readiness of every configured MCP server of one session.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpReadiness {
    /// Discovery revision this snapshot was read at. It increases whenever
    /// readiness may have changed; compare it, or wait on an
    /// [`McpReadinessWatch`], to know when to re-read.
    pub revision: u64,
    /// One entry per configured server that discovers tools, in name order.
    pub servers: Vec<McpServerReadiness>,
}

impl McpReadiness {
    /// The entry for `name`, if the session configures that server.
    pub fn server(&self, name: &str) -> Option<&McpServerReadiness> {
        self.servers.iter().find(|server| server.name == name)
    }
}

/// Change notifications for MCP readiness.
///
/// The revision is shared by every session using the same discovery cache,
/// so a change may belong to another session; re-read readiness and compare.
#[derive(Clone, Debug)]
pub struct McpReadinessWatch {
    receiver: watch::Receiver<u64>,
}

impl McpReadinessWatch {
    pub(crate) fn new(receiver: watch::Receiver<u64>) -> Self {
        Self { receiver }
    }

    /// The latest revision, marking it seen.
    pub fn revision(&mut self) -> u64 {
        *self.receiver.borrow_and_update()
    }

    /// Wait until the revision moves past the last one seen and return it.
    /// `None` when the runtime that owned the discovery cache is gone.
    pub async fn changed(&mut self) -> Option<u64> {
        self.receiver.changed().await.ok()?;
        Some(*self.receiver.borrow_and_update())
    }
}
