//! Rebuilding a cell from its restore points (data plane v2 design,
//! "Disaster recovery"): after a cell lost quorum, or to take it back to a
//! moment before a bad release.
//!
//! ```text
//! app restore --cell 0 --store s3://... --data /var/cell/c0 [--at <unix ms>]
//! ```
//!
//! [`state_at`] reads the cell's state as of a time (or its newest) from
//! the object store alone; [`rebuild`] writes it as the starting point of
//! new nodes, which then start as usual.
//!
//! Decisions:
//! - A rebuilt cell is a new incarnation, claimed in the store so two
//!   rebuilds never share one. Its segments, shipped log and feed carry the
//!   number, so nothing the lost incarnation wrote is read as its own.
//! - Every session's epoch numbers jump by [`FENCE_GAP`] and its claim is
//!   dropped. An executor still running a turn of the lost incarnation
//!   holds an epoch the rebuilt cell never hands out again, so all it
//!   writes is refused; the turn resumes from the log under a new epoch.
//!   Feed records and moves need no such step: the feed carries the
//!   incarnation, and every move step shipped before it was answered.
//! - A point-in-time restore stops at the first command proposed after the
//!   point. Command times come from the proposers (`Command::time`), the
//!   same values every replica applied.
//! - The rebuilt state is archived as the new incarnation's first snapshot
//!   before any node starts, so the new incarnation's shipped log always
//!   has a base.
//! - The new nodes start from that snapshot with an empty log and a vote at
//!   the restored log's term, so their first election is above every log
//!   entry they hold. A data directory that already holds a node is
//!   refused: a rebuild never overwrites a node that might still be alive.

use std::collections::BTreeMap;
use std::path::PathBuf;

use openraft::storage::RaftLogStorage;
use openraft::{BasicNode, EntryPayload, Membership, SnapshotMeta, StoredMembership};

use crate::restore_points::SnapshotCopy;
use crate::state::{CellId, CellState};
use crate::store::Store;
use crate::types::{Entry, NodeId, Vote};

/// How far a rebuild moves every session's epoch numbers.
pub const FENCE_GAP: u64 = 1 << 32;

/// What [`state_at`] found.
#[derive(Debug)]
pub struct Restored {
    pub state: CellState,
    /// The archived snapshot it started from, if any.
    pub from: Option<SnapshotCopy>,
    /// Index of the last entry applied.
    pub last: u64,
    /// Shipped entries applied after the snapshot.
    pub replayed: usize,
}

/// `cell`'s state as of `at_ms` (its newest without one), from the object
/// store alone: the newest archived snapshot at or before the point, then
/// its incarnation's shipped log after it, up to the point.
pub async fn state_at(store: &Store, cell: CellId, at_ms: Option<i64>) -> anyhow::Result<Restored> {
    let base = store
        .snapshot_copies(cell)
        .await?
        .into_iter()
        .filter(|c| at_ms.is_none_or(|at| c.clock_ms <= at))
        .max_by_key(|c| (c.inc, c.index));
    let (mut state, inc) = match &base {
        Some(copy) => (
            crate::sm::decode_archived(&store.get_snapshot(copy).await?)?,
            copy.inc,
        ),
        // No snapshot: the log of the cell's first life, from the start.
        None => (CellState::default(), 0),
    };
    let snapshot = state.last_applied.map_or(0, |id| id.index);
    let mut last = snapshot;
    let mut replayed = 0;
    'ranges: for (first, end) in store.shipped(cell, inc).await? {
        if end <= last {
            continue;
        }
        if first > last + 1 {
            anyhow::bail!(
                "shipped log of cell {cell} (incarnation {inc}) has a hole: {} to {}",
                last + 1,
                first - 1
            );
        }
        let entries: Vec<Entry> =
            serde_json::from_slice(&store.get_log(cell, inc, first, end).await?)?;
        let mut changes = Vec::new();
        for entry in entries {
            if entry.log_id.index <= last {
                continue;
            }
            match entry.payload {
                EntryPayload::Blank => {}
                EntryPayload::Normal(command) => {
                    if let (Some(at), Some(time)) = (at_ms, command.time())
                        && time > at
                    {
                        break 'ranges;
                    }
                    state.apply(command, &mut changes);
                }
                EntryPayload::Membership(membership) => {
                    state.membership = StoredMembership::new(Some(entry.log_id), membership);
                }
            }
            state.last_applied = Some(entry.log_id);
            last = entry.log_id.index;
            replayed += 1;
            changes.clear();
        }
    }
    Ok(Restored {
        state,
        from: base,
        last,
        replayed,
    })
}

/// What [`rebuild`] wrote.
#[derive(Debug, serde::Serialize)]
pub struct Rebuilt {
    pub cell: CellId,
    pub incarnation: u64,
    /// The snapshot it started from: (incarnation, index).
    pub from: Option<(u64, u64)>,
    pub last_index: u64,
    pub clock_ms: i64,
    pub sessions: usize,
    pub replayed: usize,
}

/// Rebuild `cell` as of `at_ms` (its newest without one) into fresh data
/// directories for `members`, one per node, as a new incarnation. Start the
/// nodes afterwards as usual.
pub async fn rebuild(
    store: &Store,
    cell: CellId,
    members: &BTreeMap<NodeId, (String, PathBuf)>,
    at_ms: Option<i64>,
) -> anyhow::Result<Rebuilt> {
    for (node, (_, dir)) in members {
        if dir.join("wal").exists() || dir.join("snapshots").join("current.snap").exists() {
            anyhow::bail!(
                "node {node}'s data directory {} already holds a node; a rebuild needs empty ones",
                dir.display()
            );
        }
    }
    let restored = state_at(store, cell, at_ms).await?;
    let Some(last_log_id) = restored.state.last_applied else {
        anyhow::bail!("nothing to restore for cell {cell}: no snapshot and no shipped log");
    };
    let mut state = restored.state;
    // A cell never rebuilt is incarnation 0 here and 1 on its feed, so the
    // first rebuild is 2.
    let incarnation = store
        .claim_incarnation(cell, (state.incarnation + 1).max(2))
        .await?;
    renew(&mut state, incarnation);
    let voters: std::collections::BTreeSet<NodeId> = members.keys().copied().collect();
    let nodes: BTreeMap<NodeId, BasicNode> = members
        .iter()
        .map(|(id, (addr, _))| (*id, BasicNode { addr: addr.clone() }))
        .collect();
    state.membership = StoredMembership::new(
        Some(last_log_id),
        Membership::new(vec![voters], nodes),
    );
    let meta = SnapshotMeta {
        last_log_id: Some(last_log_id),
        last_membership: state.membership.clone(),
        snapshot_id: format!("restore-i{incarnation}-{}", last_log_id.index),
    };
    let (clock_ms, sessions) = (state.clock_ms, state.sessions.len());
    state.share_definitions();
    let data = bytes::Bytes::from(serde_json::to_vec(&state)?);
    store
        .put_snapshot(
            cell,
            &meta.snapshot_id,
            incarnation,
            last_log_id.index,
            clock_ms,
            crate::sm::archived_parts(&meta, data.clone())?,
        )
        .await?;
    for (node, (_, dir)) in members {
        crate::sm::write_snapshot_file(&dir.join("snapshots"), &meta, &data)?;
        let mut wal = crate::wal::Wal::open(dir.join("wal"))?;
        wal.save_vote(&Vote::new(last_log_id.leader_id.term, *node))
            .await
            .map_err(|error| anyhow::anyhow!("write vote: {error}"))?;
    }
    Ok(Rebuilt {
        cell,
        incarnation,
        from: restored.from.map(|c| (c.inc, c.index)),
        last_index: last_log_id.index,
        clock_ms,
        sessions,
        replayed: restored.replayed,
    })
}

/// Make `state` incarnation `incarnation`: every session's epochs jump past
/// anything the lost incarnation handed out, and no claim survives.
fn renew(state: &mut CellState, incarnation: u64) {
    state.incarnation = incarnation;
    for session in state.sessions.values_mut() {
        let a = &mut session.authority;
        a.fence += FENCE_GAP;
        a.holder = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hole_in_the_shipped_log_fails_the_restore() {
        let store = Store::open("memory:///").unwrap();
        store.put_log(7, 0, 5, 6, b"[]".to_vec()).await.unwrap();
        let error = state_at(&store, 7, None).await.unwrap_err();
        assert!(error.to_string().contains("hole: 1 to 4"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_store_restores_an_empty_cell_and_rebuilds_nothing() {
        let store = Store::open("memory:///").unwrap();
        let restored = state_at(&store, 7, None).await.unwrap();
        assert_eq!(restored.last, 0);
        assert!(restored.state.sessions.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let members = BTreeMap::from([(1, ("127.0.0.1:1".to_owned(), dir.path().join("n1")))]);
        let error = rebuild(&store, 7, &members, None).await.unwrap_err();
        assert!(error.to_string().contains("nothing to restore"), "{error}");
    }

    #[test]
    fn a_rebuild_fences_every_epoch_the_lost_incarnation_handed_out() {
        let mut state = CellState::default();
        let mut session = crate::state::SessionState::default();
        session.authority.fence = 7;
        session.authority.holder = Some(crate::authority::Holder {
            name: "exec-1".into(),
            node: 1,
            fence: 7,
            expires_ms: i64::MAX,
        });
        let id = everruns::SessionId::new();
        state.sessions.insert(id, session);
        renew(&mut state, 3);
        let a = &state.sessions[&id].authority;
        assert_eq!(state.incarnation, 3);
        assert_eq!(a.fence, 7 + FENCE_GAP);
        assert!(a.holder.is_none());
    }
}
