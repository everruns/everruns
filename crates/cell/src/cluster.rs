//! The seed topology: which nodes formed which cell at start, and which
//! cell holds the directory.
//!
//! Decision: the file only seeds. After start, a cell's nodes are its Raft
//! membership and the fleet's cells are the directory's cell registry (see
//! `Gateway::topology`), so nodes and cells are added without editing it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::CellId;
use crate::types::NodeId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topology {
    /// The cell whose state machine holds the directory.
    #[serde(default)]
    pub directory_cell: CellId,
    /// cell -> node -> `host:port`.
    pub cells: BTreeMap<CellId, BTreeMap<NodeId, String>>,
}

impl Topology {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn nodes(&self, cell: CellId) -> Vec<String> {
        self.cells
            .get(&cell)
            .map(|nodes| nodes.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// A topology file that is re-read on demand.
#[derive(Clone)]
pub struct TopologySource {
    pub path: PathBuf,
}

impl TopologySource {
    pub fn read(&self) -> anyhow::Result<Topology> {
        Topology::load(&self.path)
    }
}
