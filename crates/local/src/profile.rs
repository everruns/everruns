// Named local environment configuration for embedded hosts.
//
// A `LocalProfile` bundles the on-disk locations and identity defaults an
// embedder needs to stand up the local stores: where the SQLite database and
// workspace files live, the UI base URL used for tool result links, and the
// local org/principal identity stamped onto created records.

use std::path::{Path, PathBuf};

use everruns_core::typed_id::PrincipalId;

/// Local environment configuration.
#[derive(Debug, Clone)]
pub struct LocalProfile {
    /// Directory holding the local SQLite database(s).
    pub data_dir: PathBuf,
    /// Root directory for the session workspace filesystem.
    pub workspace_root: PathBuf,
    /// Base URL used to build UI links in tool results.
    pub base_url: String,
    /// Public organization id for created records.
    pub org_public_id: String,
    /// Owning principal stamped on schedules/sessions created locally.
    pub owner_principal_id: PrincipalId,
}

impl Default for LocalProfile {
    fn default() -> Self {
        let data_dir = std::env::temp_dir().join("everruns-local");
        Self {
            workspace_root: data_dir.join("workspace"),
            data_dir,
            base_url: "http://localhost:9300".to_string(),
            org_public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            owner_principal_id: PrincipalId::from_seed(1),
        }
    }
}

impl LocalProfile {
    /// Start from defaults rooted at `data_dir`. The workspace defaults to
    /// `data_dir/workspace`.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            workspace_root: data_dir.join("workspace"),
            data_dir,
            ..Self::default()
        }
    }

    /// Path to the local stores database file under `data_dir`.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("local.db")
    }

    pub fn with_workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = root.into();
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_org_public_id(mut self, org_public_id: impl Into<String>) -> Self {
        self.org_public_id = org_public_id.into();
        self
    }

    pub fn with_owner_principal_id(mut self, owner_principal_id: PrincipalId) -> Self {
        self.owner_principal_id = owner_principal_id;
        self
    }

    /// Ensure `data_dir` and `workspace_root` exist on disk.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.workspace_root)?;
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
