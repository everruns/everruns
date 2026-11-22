//! The seed topology: which nodes formed which shard at start, and which
//! shard holds the directory.
//!
//! Decision: the file only seeds. After start, a shard's nodes are its Raft
//! membership and the fleet's shards are the directory's shard registry (see
//! `Gateway::topology`), so nodes and shards are added without editing it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::ShardId;
use crate::types::NodeId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topology {
    /// The shard whose state machine holds the directory.
    #[serde(default)]
    pub directory_shard: ShardId,
    /// shard -> node -> `host:port`.
    pub shards: BTreeMap<ShardId, BTreeMap<NodeId, String>>,
}

impl Topology {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn nodes(&self, shard: ShardId) -> Vec<String> {
        self.shards
            .get(&shard)
            .map(|nodes| nodes.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// A topology file that is re-read on demand.
#[derive(Debug, Clone)]
pub struct TopologySource {
    pub path: PathBuf,
}

impl TopologySource {
    pub fn read(&self) -> anyhow::Result<Topology> {
        Topology::load(&self.path)
    }
}
