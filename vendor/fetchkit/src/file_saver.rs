//! File saving abstractions for FetchKit
//!
//! Consumers implement [`FileSaver`] to control where fetched bytes land:
//! - CLI: writes to real filesystem ([`LocalFileSaver`])
//! - Everruns: writes to session virtual filesystem
//! - Tests: in-memory buffer

use async_trait::async_trait;
#[cfg(unix)]
use std::ffi::{CString, OsStr};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Result of a save operation
#[derive(Debug, Clone)]
pub struct SaveResult {
    /// Canonical/normalized path where file was saved
    pub path: String,
    /// Bytes written
    pub bytes_written: u64,
}

/// Errors that can occur during file save operations
#[derive(Debug, Error)]
pub enum FileSaveError {
    /// Path is not allowed (traversal, outside base dir, etc.)
    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),
    /// IO error during save
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Other save error
    #[error("Save error: {0}")]
    Other(String),
}

/// Destination for saving fetched content to files.
///
/// Consumers implement this trait to control where bytes land:
/// - CLI: writes to real filesystem ([`LocalFileSaver`])
/// - Everruns: writes to session virtual filesystem
/// - Tests: in-memory buffer
#[async_trait]
pub trait FileSaver: Send + Sync {
    /// Save raw bytes to the given path.
    /// Returns the canonical path where the file was written and bytes written.
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError>;

    /// Check if a path is writable / allowed before fetching.
    /// Default: always allowed.
    async fn validate_path(&self, path: &str) -> Result<(), FileSaveError> {
        let _ = path;
        Ok(())
    }
}

/// Saves fetched content to the real filesystem.
///
/// Ships with fetchkit as a built-in implementation for CLI usage.
///
/// # Path resolution
///
/// - With `base_dir`: relative paths resolved against it; path traversal
///   outside base_dir is rejected
/// - Without `base_dir`: only absolute paths are accepted
///
/// Parent directories are created automatically.
///
/// # Examples
///
/// ```
/// use fetchkit::LocalFileSaver;
/// use std::path::PathBuf;
///
/// // Save relative to a base directory
/// let saver = LocalFileSaver::new(Some(PathBuf::from("/tmp/downloads")));
///
/// // Save with absolute paths only
/// let saver = LocalFileSaver::new(None);
/// ```
pub struct LocalFileSaver {
    /// Optional base directory. Paths resolved relative to this.
    /// If None, paths must be absolute.
    base_dir: Option<PathBuf>,
}

impl LocalFileSaver {
    /// Create a new LocalFileSaver with an optional base directory.
    pub fn new(base_dir: Option<PathBuf>) -> Self {
        Self { base_dir }
    }

    /// Resolve and validate a path, returning the normalized absolute path.
    fn resolve_path(&self, path: &str) -> Result<PathBuf, FileSaveError> {
        if path.is_empty() {
            return Err(FileSaveError::PathNotAllowed(
                "Path must name a file".into(),
            ));
        }

        let input = PathBuf::from(path);

        if let Some(base) = &self.base_dir {
            let joined = if input.is_absolute() {
                input
            } else {
                base.join(&input)
            };
            let normalized = normalize_path(&joined);
            let normalized_base = normalize_path(base);

            if !normalized.starts_with(&normalized_base) {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Path escapes base directory: {}",
                    path
                )));
            }
            Ok(normalized)
        } else {
            if !input.is_absolute() {
                return Err(FileSaveError::PathNotAllowed(
                    "Path must be absolute when no base_dir is set".into(),
                ));
            }
            Ok(normalize_path(&input))
        }
    }

    #[cfg(not(unix))]
    async fn canonicalize_base_dir(&self, base: &Path) -> Result<PathBuf, FileSaveError> {
        tokio::fs::create_dir_all(base).await?;

        let meta = tokio::fs::symlink_metadata(base).await?;
        if meta.file_type().is_symlink() {
            return Err(FileSaveError::PathNotAllowed(
                "Base directory must not be a symlink".into(),
            ));
        }
        if !meta.is_dir() {
            return Err(FileSaveError::PathNotAllowed(
                "Base directory must be a directory".into(),
            ));
        }

        Ok(tokio::fs::canonicalize(base).await?)
    }

    #[cfg(not(unix))]
    async fn prepare_parent_dir(&self, resolved: &Path) -> Result<PathBuf, FileSaveError> {
        let Some(base) = &self.base_dir else {
            return Ok(resolved
                .parent()
                .ok_or_else(|| FileSaveError::PathNotAllowed("Path must name a file".into()))?
                .to_path_buf());
        };

        let normalized_base = normalize_path(base);
        let relative = resolved
            .strip_prefix(&normalized_base)
            .map_err(|_| FileSaveError::PathNotAllowed("Path escapes base directory".into()))?;
        let canonical_base = self.canonicalize_base_dir(base).await?;
        let mut current = canonical_base.clone();

        for component in relative
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .components()
        {
            let Component::Normal(name) = component else {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Unsupported path component in save path: {}",
                    resolved.display()
                )));
            };

            let candidate = current.join(name);
            let meta = match tokio::fs::symlink_metadata(&candidate).await {
                Ok(meta) => meta,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    if let Err(create_err) = tokio::fs::create_dir(&candidate).await {
                        if create_err.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(create_err.into());
                        }
                    }
                    tokio::fs::symlink_metadata(&candidate).await?
                }
                Err(err) => return Err(err.into()),
            };

            if meta.file_type().is_symlink() {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Path traverses symlink: {}",
                    candidate.display()
                )));
            }
            if !meta.is_dir() {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Parent path is not a directory: {}",
                    candidate.display()
                )));
            }

            let canonical_candidate = tokio::fs::canonicalize(&candidate).await?;
            if !canonical_candidate.starts_with(&canonical_base) {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Path escapes base directory via symlink: {}",
                    candidate.display()
                )));
            }
            current = canonical_candidate;
        }

        Ok(current)
    }

    #[cfg(unix)]
    async fn write_resolved_path(
        &self,
        resolved: PathBuf,
        bytes: &[u8],
    ) -> Result<PathBuf, FileSaveError> {
        let bytes = bytes.to_vec();
        let task = if let Some(base_dir) = &self.base_dir {
            let normalized_base = normalize_path(base_dir);
            let relative = resolved
                .strip_prefix(&normalized_base)
                .map_err(|_| FileSaveError::PathNotAllowed("Path escapes base directory".into()))?
                .to_path_buf();
            let base_dir = base_dir.clone();
            tokio::task::spawn_blocking(move || {
                save_under_base_no_follow(&base_dir, &relative, &bytes)
            })
        } else {
            tokio::task::spawn_blocking(move || save_absolute_no_follow(&resolved, &bytes))
        };

        task.await
            .map_err(|err| FileSaveError::Other(format!("File save task failed: {err}")))?
    }

    #[cfg(not(unix))]
    async fn write_resolved_path(
        &self,
        resolved: PathBuf,
        bytes: &[u8],
    ) -> Result<PathBuf, FileSaveError> {
        let file_name = resolved
            .file_name()
            .ok_or_else(|| FileSaveError::PathNotAllowed("Path must name a file".into()))?;
        let parent_dir = self.prepare_parent_dir(&resolved).await?;
        let final_path = parent_dir.join(file_name);

        if self.base_dir.is_none() {
            if let Some(parent) = final_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        match tokio::fs::symlink_metadata(&final_path).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(FileSaveError::PathNotAllowed(format!(
                    "Refusing to write through symlink: {}",
                    final_path.display()
                )));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        tokio::fs::write(&final_path, bytes).await?;
        Ok(final_path)
    }
}

#[async_trait]
impl FileSaver for LocalFileSaver {
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError> {
        let resolved = self.resolve_path(path)?;
        if let Some(base_dir) = &self.base_dir {
            if resolved == normalize_path(base_dir) {
                return Err(FileSaveError::PathNotAllowed(
                    "Path must name a file".into(),
                ));
            }
        }
        resolved
            .file_name()
            .ok_or_else(|| FileSaveError::PathNotAllowed("Path must name a file".into()))?;

        let final_path = self.write_resolved_path(resolved, bytes).await?;

        Ok(SaveResult {
            path: final_path.to_string_lossy().to_string(),
            bytes_written: bytes.len() as u64,
        })
    }

    async fn validate_path(&self, path: &str) -> Result<(), FileSaveError> {
        self.resolve_path(path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn save_under_base_no_follow(
    base: &Path,
    relative: &Path,
    bytes: &[u8],
) -> Result<PathBuf, FileSaveError> {
    // Keep traversal and final creation anchored to directory descriptors so
    // attacker-controlled symlink swaps cannot redirect a later path open.
    std::fs::create_dir_all(base)?;

    let meta = std::fs::symlink_metadata(base)?;
    if meta.file_type().is_symlink() {
        return Err(FileSaveError::PathNotAllowed(
            "Base directory must not be a symlink".into(),
        ));
    }
    if !meta.is_dir() {
        return Err(FileSaveError::PathNotAllowed(
            "Base directory must be a directory".into(),
        ));
    }

    let canonical_base = std::fs::canonicalize(base)?;
    let mut current_dir = open_dir_no_follow(&canonical_base)?;

    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(FileSaveError::PathNotAllowed(format!(
                "Unsupported path component in save path: {}",
                relative.display()
            )));
        };

        mkdirat_if_missing(&current_dir, name)?;
        current_dir = open_child_dir_no_follow(&current_dir, name)?;
    }

    let file_name = relative
        .file_name()
        .ok_or_else(|| FileSaveError::PathNotAllowed("Path must name a file".into()))?;
    let final_path = canonical_base.join(relative);
    let mut file = open_child_file_no_follow(&current_dir, file_name, &final_path)?;
    file.write_all(bytes)?;

    Ok(final_path)
}

#[cfg(unix)]
fn save_absolute_no_follow(path: &Path, bytes: &[u8]) -> Result<PathBuf, FileSaveError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = open_path_no_follow(path)?;
    file.write_all(bytes)?;
    Ok(path.to_path_buf())
}

#[cfg(unix)]
fn open_dir_no_follow(path: &Path) -> Result<OwnedFd, FileSaveError> {
    let path = cstring_path(path)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    owned_fd_from_result(fd, "Refusing to traverse symlink")
}

#[cfg(unix)]
fn open_child_dir_no_follow(parent: &OwnedFd, name: &OsStr) -> Result<OwnedFd, FileSaveError> {
    let name = cstring_component(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    owned_fd_from_result(fd, "Refusing to traverse symlink")
}

#[cfg(unix)]
fn open_child_file_no_follow(
    parent: &OwnedFd,
    name: &OsStr,
    path_for_error: &Path,
) -> Result<std::fs::File, FileSaveError> {
    let name = cstring_component(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o666,
        )
    };
    file_from_result(
        fd,
        format!(
            "Refusing to write through symlink: {}",
            path_for_error.display()
        ),
    )
}

#[cfg(unix)]
fn open_path_no_follow(path: &Path) -> Result<std::fs::File, FileSaveError> {
    let c_path = cstring_path(path)?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o666,
        )
    };
    file_from_result(
        fd,
        format!("Refusing to write through symlink: {}", path.display()),
    )
}

#[cfg(unix)]
fn mkdirat_if_missing(parent: &OwnedFd, name: &OsStr) -> Result<(), FileSaveError> {
    let name = cstring_component(name)?;
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) };
    if result == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::AlreadyExists {
        Ok(())
    } else {
        Err(err.into())
    }
}

#[cfg(unix)]
fn owned_fd_from_result(fd: libc::c_int, symlink_message: &str) -> Result<OwnedFd, FileSaveError> {
    if fd >= 0 {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    } else {
        Err(open_error(symlink_message.to_string()))
    }
}

#[cfg(unix)]
fn file_from_result(
    fd: libc::c_int,
    symlink_message: String,
) -> Result<std::fs::File, FileSaveError> {
    if fd >= 0 {
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    } else {
        Err(open_error(symlink_message))
    }
}

#[cfg(unix)]
fn open_error(symlink_message: String) -> FileSaveError {
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ELOOP) {
        FileSaveError::PathNotAllowed(symlink_message)
    } else {
        FileSaveError::Io(err)
    }
}

#[cfg(unix)]
fn cstring_path(path: &Path) -> Result<CString, FileSaveError> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| FileSaveError::PathNotAllowed("Path contains NUL byte".into()))
}

#[cfg(unix)]
fn cstring_component(component: &OsStr) -> Result<CString, FileSaveError> {
    CString::new(component.as_bytes())
        .map_err(|_| FileSaveError::PathNotAllowed("Path contains NUL byte".into()))
}

/// Lexically normalize a path by resolving `.` and `..` components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Only pop if the last component is a normal component
                if matches!(components.last(), Some(Component::Normal(_))) {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(
            normalize_path(Path::new("/a/b/./c")),
            PathBuf::from("/a/b/c")
        );
        assert_eq!(
            normalize_path(Path::new("/a/b/c/../..")),
            PathBuf::from("/a")
        );
    }

    #[test]
    fn test_local_file_saver_resolve_relative() {
        let saver = LocalFileSaver::new(Some(PathBuf::from("/tmp/downloads")));
        let resolved = saver.resolve_path("file.txt").unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/downloads/file.txt"));
    }

    #[test]
    fn test_local_file_saver_resolve_subdirectory() {
        let saver = LocalFileSaver::new(Some(PathBuf::from("/tmp/downloads")));
        let resolved = saver.resolve_path("sub/dir/file.txt").unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/downloads/sub/dir/file.txt"));
    }

    #[test]
    fn test_local_file_saver_reject_traversal() {
        let saver = LocalFileSaver::new(Some(PathBuf::from("/tmp/downloads")));
        let result = saver.resolve_path("../../etc/passwd");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FileSaveError::PathNotAllowed(_)
        ));
    }

    #[test]
    fn test_local_file_saver_reject_traversal_absolute() {
        let saver = LocalFileSaver::new(Some(PathBuf::from("/tmp/downloads")));
        let result = saver.resolve_path("/etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_local_file_saver_no_base_requires_absolute() {
        let saver = LocalFileSaver::new(None);
        let result = saver.resolve_path("relative.txt");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FileSaveError::PathNotAllowed(_)
        ));
    }

    #[test]
    fn test_local_file_saver_no_base_accepts_absolute() {
        let saver = LocalFileSaver::new(None);
        let resolved = saver.resolve_path("/tmp/file.txt").unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/file.txt"));
    }

    #[tokio::test]
    async fn test_local_file_saver_save_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));

        let result = saver.save("test.txt", b"hello world").await.unwrap();
        assert_eq!(result.bytes_written, 11);
        assert!(result.path.ends_with("test.txt"));

        let content = tokio::fs::read_to_string(dir.path().join("test.txt"))
            .await
            .unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_local_file_saver_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));

        let result = saver
            .save("sub/dir/file.bin", &[0xFF, 0x00, 0xAB])
            .await
            .unwrap();
        assert_eq!(result.bytes_written, 3);

        let content = tokio::fs::read(dir.path().join("sub/dir/file.bin"))
            .await
            .unwrap();
        assert_eq!(content, vec![0xFF, 0x00, 0xAB]);
    }

    #[tokio::test]
    async fn test_local_file_saver_validate_path() {
        let dir = tempfile::tempdir().unwrap();
        let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));

        assert!(saver.validate_path("safe.txt").await.is_ok());
        assert!(saver.validate_path("sub/dir/safe.txt").await.is_ok());
        assert!(saver.validate_path("../../escape.txt").await.is_err());
    }
}
