//! Metadata-only workspace moves with the same authorization as file writes.
use super::*;

impl WorkspaceFileService {
    /// Move/rename a file or directory
    pub async fn move_file(
        &self,
        session_id: Uuid,
        req: MoveFileInput,
    ) -> Result<Option<SessionFile>> {
        self.restore_virtual_mounts(session_id).await?;
        let src_path = Self::normalize_path(&req.src_path);
        let dst_path = Self::normalize_path(&req.dst_path);

        Self::validate_path(&src_path)?;
        Self::validate_path(&dst_path)?;
        anyhow::ensure!(
            src_path != "/" && dst_path != "/",
            "Cannot move a filesystem root"
        );
        anyhow::ensure!(
            src_path != dst_path && !dst_path.starts_with(&format!("{src_path}/")),
            "Cannot move a path into itself"
        );
        self.ensure_path_not_virtual(session_id, &src_path)?;
        self.ensure_path_not_virtual(session_id, &dst_path)?;
        self.ensure_parent_path_writable(session_id, &src_path)
            .await?;
        self.ensure_parent_path_writable(session_id, &dst_path)
            .await?;

        // A move or copy that crosses a Memory mount boundary cannot be one
        // storage statement, so it is read + write (+ delete) through this same
        // service. Directories are refused rather than walked: a recursive
        // cross-store copy is not what any caller needs today.
        if self
            .crosses_memory_mount(session_id, &src_path, &dst_path)
            .await
        {
            return self
                .transfer_across_memory_mount(session_id, &src_path, &dst_path, true)
                .await;
        }

        // Check source exists
        let source = self.db.get_session_file_info(session_id, &src_path).await?;
        if source.is_none() {
            return Err(anyhow!("Source not found: {}", src_path));
        }
        anyhow::ensure!(
            !self
                .db
                .has_readonly_session_files(session_id, &src_path)
                .await?,
            "Cannot move readonly files or directories"
        );
        for path in [&src_path, &dst_path] {
            anyhow::ensure!(
                !self.is_mount_ancestor(session_id, path).await,
                "Cannot move or replace Memory mount ancestors"
            );
            if let Some(registry) = &self.virtual_registry {
                anyhow::ensure!(
                    !registry.contains_mount(session_id, path),
                    "Cannot move or replace virtual mounts"
                );
            }
        }

        // Check destination doesn't exist
        if self.db.session_file_exists(session_id, &dst_path).await? {
            return Err(anyhow!("Destination already exists: {}", dst_path));
        }

        // Ensure destination parent exists
        if let Some(parent) = FileInfo::parent_path(&dst_path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        let row = self
            .db
            .move_session_file(session_id, &src_path, &dst_path)
            .await?;
        Ok(row.map(Self::row_to_session_file))
    }
}

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod tests;
