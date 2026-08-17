// Named local environment configuration for embedded hosts.
//
// A `LocalProfile` bundles the on-disk locations and identity defaults an
// embedder needs to stand up the local stores: where the SQLite database and
// workspace files live, the UI base URL used for tool result links, and the
// local org/principal identity stamped onto created records.

use std::path::{Path, PathBuf};

use everruns_provider::typed_id::PrincipalId;

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
        let data_dir = default_data_dir();
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

    /// Override the directory exposed as the local workspace root.
    pub fn with_workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = root.into();
        self
    }

    /// Override the base URL used in locally projected platform records.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Select the public organization identifier for local records.
    pub fn with_org_public_id(mut self, org_public_id: impl Into<String>) -> Self {
        self.org_public_id = org_public_id.into();
        self
    }

    /// Select the principal that owns locally created sessions.
    pub fn with_owner_principal_id(mut self, owner_principal_id: PrincipalId) -> Self {
        self.owner_principal_id = owner_principal_id;
        self
    }

    /// Ensure `data_dir` and `workspace_root` exist on disk with private
    /// permissions. Local stores can contain prompts, tool output, and schedule
    /// metadata, so pre-existing unsafe paths are rejected instead of followed.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        ensure_private_dir(&self.data_dir)?;
        ensure_private_dir(&self.workspace_root)?;
        Ok(())
    }

    /// Root directory containing local databases and workspace state.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("everruns")
        .join("local")
}

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    std::fs::create_dir_all(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing to use symlinked local profile directory {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("local profile path is not a directory: {}", path.display()),
        ));
    }
    if metadata.uid() != current_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "local profile directory is not owned by the current user: {}",
                path.display()
            ),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_uses_user_local_data_dir() {
        assert!(
            !LocalProfile::default()
                .data_dir
                .starts_with(std::env::temp_dir())
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_dirs_hardens_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let profile = LocalProfile::new(root.path().join("profile"));
        profile.ensure_dirs().unwrap();

        let data_mode = std::fs::metadata(&profile.data_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let workspace_mode = std::fs::metadata(&profile.workspace_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(data_mode, 0o700);
        assert_eq!(workspace_mode, 0o700);
    }
}
