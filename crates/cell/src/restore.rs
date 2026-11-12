//! Rebuilding a cell from its restore points (data plane v2 design,
//! "Disaster recovery"): after a cell lost quorum, or to take it back to a
//! moment before a bad release.
//!
//! ```text
//! app restore --topology cluster.json --cell 0 --store s3://... \
//!             --node 1=/var/cell/c0n1 [--at 2026-11-12T09:00:00Z] [--dry-run]
//! app restore --topology cluster.json --cell 0 --store s3://... \
//!             --node 2=/var/cell/c0n2 --incarnation 2
//! ```
//!
//! [`state_at`] reads the cell's state as of a time (or its newest) from
//! the object store alone; [`rebuild`] writes it as the starting point of
//! new nodes, which then start as usual. Nodes on other machines are seeded
//! from the rebuilt incarnation's archived snapshot with [`seed`].
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
use everruns_contracts::typed_id::SessionId;

use crate::state::{CellId, CellState, Command, Response, SessionState};
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
    // `read`: the last entry read; entries without a time wait in
    // `pending` until the next one that has a time says whether they
    // happened before the point.
    let (mut read, mut replayed) = (snapshot, 0);
    let mut pending: Vec<Entry> = Vec::new();
    let mut changes = Vec::new();
    'ranges: for (first, end) in store.shipped(cell, inc).await? {
        if end <= read {
            continue;
        }
        if first > read + 1 {
            anyhow::bail!(
                "shipped log of cell {cell} (incarnation {inc}) has a hole: {} to {}",
                read + 1,
                first - 1
            );
        }
        let entries: Vec<Entry> =
            serde_json::from_slice(&store.get_log(cell, inc, first, end).await?)?;
        for entry in entries {
            if entry.log_id.index <= read {
                continue;
            }
            read = entry.log_id.index;
            let time = match &entry.payload {
                EntryPayload::Normal(command) => command.time(),
                _ => None,
            };
            let Some(at) = at_ms else {
                replay(&mut state, entry, &mut changes);
                replayed += 1;
                continue;
            };
            match time {
                Some(time) if time > at => break 'ranges,
                Some(_) => {
                    for entry in pending.drain(..).chain(std::iter::once(entry)) {
                        replay(&mut state, entry, &mut changes);
                        replayed += 1;
                    }
                }
                None => pending.push(entry),
            }
        }
    }
    // Entries left in `pending` have no later time to place them before
    // the point, so they stay out.
    Ok(Restored {
        last: state.last_applied.map_or(0, |id| id.index),
        state,
        from: base,
        replayed,
    })
}

/// Apply one shipped entry to `state`, as a replica would.
///
/// Decision: a command without a time of its own (a move step, a segment
/// record, a grant) is placed by the next command that has one: it
/// committed before that one, so it was proposed before that one was
/// answered.
/// A point-in-time restore then never takes in a step from after the point
/// (an import carrying a session's later history, say), at the cost of
/// leaving out, at the very end, steps it cannot place.
fn replay(state: &mut CellState, entry: Entry, changes: &mut Vec<crate::state::Change>) {
    match entry.payload {
        EntryPayload::Blank => {}
        EntryPayload::Normal(command) => {
            state.apply(command, changes);
        }
        EntryPayload::Membership(membership) => {
            state.membership = StoredMembership::new(Some(entry.log_id), membership);
        }
    }
    state.last_applied = Some(entry.log_id);
    changes.clear();
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

/// Rebuild `cell` as of `at_ms` (its newest without one) as a new
/// incarnation whose voters are `members` (id to address), writing the
/// starting point of the nodes in `local` (id to an empty data directory).
/// Seed the others with [`seed`] on their machines, then start every node
/// as usual.
pub async fn rebuild(
    store: &Store,
    cell: CellId,
    members: &BTreeMap<NodeId, String>,
    local: &BTreeMap<NodeId, PathBuf>,
    at_ms: Option<i64>,
) -> anyhow::Result<Rebuilt> {
    if members.is_empty() {
        anyhow::bail!("a rebuilt cell needs at least one member");
    }
    for (node, dir) in local {
        if !members.contains_key(node) {
            anyhow::bail!("node {node} is not a member of cell {cell}");
        }
        refuse_used(*node, dir)?;
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
        .map(|(id, addr)| (*id, BasicNode { addr: addr.clone() }))
        .collect();
    state.membership =
        StoredMembership::new(Some(last_log_id), Membership::new(vec![voters], nodes));
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
    for (node, dir) in local {
        seed_dir(dir, *node, &meta, &data).await?;
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

/// What [`restore_org`] did with each session of the org.
#[derive(Debug, Default, serde::Serialize)]
pub struct OrgRestored {
    pub org: String,
    pub at_ms: i64,
    /// Sessions taken back to the point.
    pub restored: Vec<SessionId>,
    /// Sessions of the org at the point that no cell holds any more.
    pub missing: Vec<SessionId>,
    /// Sessions a cell refused for now (moving); run the restore again.
    pub refused: Vec<(SessionId, String)>,
}

/// `org`'s sessions as of `at_ms`, from the object store alone: each
/// cell's state at the point, the newest copy of each session. A session
/// that was moving at the point has two copies; the higher generation is
/// the one the move was installing.
pub async fn org_at(
    store: &Store,
    cells: impl IntoIterator<Item = CellId>,
    org: &str,
    at_ms: i64,
) -> anyhow::Result<std::collections::HashMap<SessionId, SessionState>> {
    let mut sessions = std::collections::HashMap::<SessionId, SessionState>::new();
    for cell in cells {
        let state = state_at(store, cell, Some(at_ms)).await?.state;
        for (id, session) in state.sessions {
            if session.tenant.as_deref() != Some(org) {
                continue;
            }
            if sessions
                .get(&id)
                .is_none_or(|s| s.generation < session.generation)
            {
                sessions.insert(id, session);
            }
        }
    }
    Ok(sessions)
}

/// Take `org`'s sessions back to `at_ms` where they live now, through each
/// one's cell (`Command::Revert`). No other org's sessions change.
///
/// Decisions:
/// - A session goes back in place, on a new history branch: no move, so
///   its directory entry and generation stay, and a running turn's
///   executor is fenced out by the jump of its epoch counter.
/// - Sessions created after the point stay as they are, and a session
///   deleted since is reported, not brought back: a restore never removes
///   or resurrects anything by itself.
/// - Spend and grants are not restored; they come from the control plane.
pub async fn restore_org(
    store: &Store,
    gateway: &crate::gateway::Gateway,
    org: &str,
    at_ms: i64,
) -> anyhow::Result<OrgRestored> {
    let cells: Vec<CellId> = gateway
        .topology()
        .map_err(|(_, error)| anyhow::anyhow!(error))?
        .cells
        .keys()
        .copied()
        .collect();
    let images = org_at(store, cells, org, at_ms).await?;
    let mut report = OrgRestored {
        org: org.to_owned(),
        at_ms,
        ..Default::default()
    };
    let branch = crate::state::new_branch(crate::node::now_ms());
    for (session, image) in images {
        let command = Command::Revert {
            session,
            image: Box::new(image),
            branch,
            now_ms: crate::node::now_ms(),
        };
        let mut outcome = Err("the session keeps moving".to_owned());
        for _ in 0..4 {
            let Some(placed) = gateway
                .placement(session)
                .await
                .map_err(|(_, error)| anyhow::anyhow!(error))?
            else {
                outcome = Ok(false);
                break;
            };
            match gateway
                .propose(placed.cell, &command)
                .await
                .map_err(|(_, error)| anyhow::anyhow!(error))?
            {
                Response::Ok | Response::Duplicate => {
                    outcome = Ok(true);
                    break;
                }
                Response::NotFound => {
                    outcome = Ok(false);
                    break;
                }
                Response::Moved { .. } => continue,
                Response::Rejected { reason } => {
                    outcome = Err(reason);
                    break;
                }
                other => {
                    outcome = Err(format!("{other:?}"));
                    break;
                }
            }
        }
        match outcome {
            Ok(true) => report.restored.push(session),
            Ok(false) => report.missing.push(session),
            Err(reason) => report.refused.push((session, reason)),
        }
    }
    Ok(report)
}

/// Refuse a data directory that already holds a node.
fn refuse_used(node: NodeId, dir: &std::path::Path) -> anyhow::Result<()> {
    if dir.join("wal").exists() || dir.join("snapshots").join("current.snap").exists() {
        anyhow::bail!(
            "node {node}'s data directory {} already holds a node; a restore needs an empty one",
            dir.display()
        );
    }
    Ok(())
}

/// Write a node's starting point: the snapshot, an empty log, and a vote at
/// the snapshot's term.
async fn seed_dir(
    dir: &std::path::Path,
    node: NodeId,
    meta: &crate::sm::Meta,
    data: &[u8],
) -> anyhow::Result<()> {
    crate::sm::write_snapshot_file(&dir.join("snapshots"), meta, data)?;
    let term = meta.last_log_id.map_or(0, |id| id.leader_id.term);
    let mut wal = crate::wal::Wal::open(dir.join("wal"))?;
    wal.save_vote(&Vote::new(term, node))
        .await
        .map_err(|error| anyhow::anyhow!("write vote: {error}"))?;
    Ok(())
}

/// Seed `node`'s empty data directory `dir` for incarnation `incarnation`
/// of `cell`, already rebuilt elsewhere: from that incarnation's newest
/// archived snapshot. The node then starts as usual and catches up from
/// its cell's leader.
///
/// Decision: a node of a rebuilt cell never starts empty. An empty node
/// begins at term 0 with no membership of its own, so two of them could
/// elect each other over an empty log; a seeded one holds the restored
/// membership and a vote at its term.
pub async fn seed(
    store: &Store,
    cell: CellId,
    incarnation: u64,
    node: NodeId,
    dir: &std::path::Path,
) -> anyhow::Result<u64> {
    refuse_used(node, dir)?;
    let Some(copy) = store
        .snapshot_copies(cell)
        .await?
        .into_iter()
        .filter(|c| c.inc == incarnation)
        .max_by_key(|c| c.index)
    else {
        anyhow::bail!("cell {cell} has no snapshot of incarnation {incarnation}; rebuild it first");
    };
    let bytes = store.get_snapshot(&copy).await?;
    let (meta, at) = crate::sm::split_snapshot(&bytes)?;
    if !meta
        .last_membership
        .membership()
        .voter_ids()
        .any(|id| id == node)
    {
        anyhow::bail!("node {node} is not a member of cell {cell}'s incarnation {incarnation}");
    }
    seed_dir(dir, node, &meta, &bytes[at..]).await?;
    Ok(copy.index)
}

/// Make `state` incarnation `incarnation`: every session's epochs jump past
/// anything the lost incarnation handed out, no claim survives, and every
/// session writes its history on a new branch.
fn renew(state: &mut CellState, incarnation: u64) {
    state.incarnation = incarnation;
    let branch = crate::state::new_branch(crate::node::now_ms());
    for session in state.sessions.values_mut() {
        session.branch = branch;
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

    /// A grant has no time of its own; the next timed command places it.
    #[tokio::test]
    async fn a_step_without_a_time_goes_where_the_next_timed_one_does() {
        let store = Store::open("memory:///").unwrap();
        let entry = |index, command| Entry {
            log_id: openraft::LogId::new(openraft::CommittedLeaderId::new(1, 1), index),
            payload: EntryPayload::Normal(command),
        };
        let timed = |now_ms| Command::Reserve {
            session: SessionId::new(),
            now_ms,
        };
        let grant = |id: &str| Command::Grant {
            tenant: "org".into(),
            grant_id: id.into(),
            grant: crate::allowance::Grant {
                granted_micros: 1,
                per_call_micros: 1,
                spend_until_ms: 1,
                used_micros: 0,
                overrun_micros: 0,
            },
        };
        let log = vec![
            entry(1, timed(100)),
            entry(2, grant("before-300")),
            entry(3, timed(300)),
            entry(4, grant("unplaced")),
        ];
        store
            .put_log(7, 0, 1, 4, serde_json::to_vec(&log).unwrap())
            .await
            .unwrap();
        let grants = |r: &Restored| -> Vec<String> {
            r.state
                .allowances
                .tenants
                .get("org")
                .map(|a| a.grants.keys().cloned().collect())
                .unwrap_or_default()
        };
        let at = |ms| state_at(&store, 7, Some(ms));
        let r = at(200).await.unwrap();
        assert_eq!((r.last, r.state.clock_ms), (1, 100));
        assert!(grants(&r).is_empty(), "it came after 200 at the latest");
        let r = at(300).await.unwrap();
        assert_eq!(r.last, 3);
        assert_eq!(
            grants(&r),
            ["before-300"],
            "the tail has no time to place it"
        );
        let r = state_at(&store, 7, None).await.unwrap();
        assert_eq!(r.last, 4);
    }

    #[tokio::test]
    async fn an_empty_store_restores_an_empty_cell_and_rebuilds_nothing() {
        let store = Store::open("memory:///").unwrap();
        let restored = state_at(&store, 7, None).await.unwrap();
        assert_eq!(restored.last, 0);
        assert!(restored.state.sessions.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let members = BTreeMap::from([(1, "127.0.0.1:1".to_owned())]);
        let local = BTreeMap::from([(1, dir.path().join("n1"))]);
        let error = rebuild(&store, 7, &members, &local, None)
            .await
            .unwrap_err();
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
