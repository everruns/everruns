//! What the allocator holds, beside what the node counts itself.
//!
//! Decision: at the 2026-11-23 75k gate a shard node's own accounting
//! (resident events plus the Raft log) covered about 30% of its resident
//! set: 2.5 of 11.5 GB at the peak. The rest was never measured, so every
//! memory failure so far surfaced only at full size. jemalloc's own figures
//! split that rest into live allocations (`allocated`), pages the allocator
//! keeps for reuse (`active` and `resident` above `allocated`) and its own
//! bookkeeping (`metadata`), which says whether a gap is data the node
//! holds or allocator overhead.
//!
//! Both binaries (`agentd`, `agentd-testnode`) run on jemalloc. In a process
//! that does not (the library's own tests), the figures cover only what went
//! through jemalloc directly, which is next to nothing.

use serde::{Deserialize, Serialize};
use tikv_jemalloc_ctl::{epoch, stats};

/// The allocator's view of this process, in MB.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heap {
    /// Bytes the program has allocated and not freed.
    pub allocated_mb: u64,
    /// Pages backing those allocations, including their free space.
    pub active_mb: u64,
    /// Pages the allocator has resident, including dirty pages it keeps
    /// for reuse and its metadata.
    pub resident_mb: u64,
    /// The allocator's own bookkeeping.
    pub metadata_mb: u64,
    /// Virtual memory kept mapped but returned to the system.
    pub retained_mb: u64,
}

/// Read the allocator's current figures. Zeros when they cannot be read.
pub fn heap() -> Heap {
    // jemalloc refreshes its statistics only when the epoch advances.
    if epoch::advance().is_err() {
        return Heap::default();
    }
    let mb = |v: Result<usize, tikv_jemalloc_ctl::Error>| v.map_or(0, |b| (b >> 20) as u64);
    Heap {
        allocated_mb: mb(stats::allocated::read()),
        active_mb: mb(stats::active::read()),
        resident_mb: mb(stats::resident::read()),
        metadata_mb: mb(stats::metadata::read()),
        retained_mb: mb(stats::retained::read()),
    }
}
