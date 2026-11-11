//! Raft type configuration for a cell.

use std::io::Cursor;

/// A node within a cell.
pub type NodeId = u64;

openraft::declare_raft_types!(
    /// A cell's Raft types: commands in, responses out, JSON snapshots.
    pub TypeConfig:
        D = crate::state::Command,
        R = crate::state::Response,
        NodeId = NodeId,
        Node = openraft::BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = openraft::TokioRuntime,
);

pub type Raft = openraft::Raft<TypeConfig>;
pub type Entry = openraft::Entry<TypeConfig>;
pub type LogId = openraft::LogId<NodeId>;
pub type Vote = openraft::Vote<NodeId>;
pub type StorageError = openraft::StorageError<NodeId>;
