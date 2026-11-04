//! Native JSONL persistence; replay and projection contracts stay portable.
use super::*;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct JsonlState {
    file: tokio::fs::File,
    lock_file: std::fs::File,
    committed_len: u64,
    index: EventIndex,
}

struct FileLockGuard(std::fs::File);

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// Crash-durable JSONL log containing complete canonical [`Event`] envelopes.
///
/// An append is acknowledged only after newline-delimited bytes are flushed and
/// `sync_data` succeeds. A torn final record is an uncommitted tail and is
/// truncated on open; any malformed complete line is corruption. The in-memory
/// index is disposable and rebuilt solely from the canonical log.
pub struct JsonlEventLog {
    path: PathBuf,
    state: Mutex<JsonlState>,
}

impl JsonlEventLog {
    /// Open or create a canonical event JSONL file and rebuild its index.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, EventLogError> {
        Self::open_with_limits(path, MAX_JSONL_RECOVERY_BYTES, MAX_JSONL_RECOVERY_EVENTS).await
    }

    pub(super) async fn open_with_limits(
        path: impl AsRef<Path>,
        max_bytes: usize,
        max_events: usize,
    ) -> Result<Self, EventLogError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            // THREAT[TM-FS-014]: open and validate the log through one handle
            // so canonical envelopes cannot be redirected through a symlink.
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path).await?;
        ensure_private_event_log(&file, &path).await?;
        // Cloned from the validated handle so the advisory lock guards the same
        // file the symlink check just accepted, not whatever the path resolves
        // to later.
        let lock_file = file.try_clone().await?.into_std().await;

        // THREAT[TM-DOS-035]: local state is not assumed trustworthy after a
        // crash or operator edit. Read through `take` so a concurrently growing
        // file cannot force an unbounded allocation during index recovery.
        let mut bytes = Vec::new();
        (&mut file)
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > max_bytes {
            return Err(EventLogError::RecoveryLimitExceeded {
                detail: format!(
                    "{} exceeds the {max_bytes}-byte local recovery limit",
                    path.display()
                ),
            });
        }
        let committed_len = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        let mut index = EventIndex::default();
        for (line_index, line) in bytes[..committed_len]
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .enumerate()
        {
            if line_index >= max_events {
                return Err(EventLogError::RecoveryLimitExceeded {
                    detail: format!(
                        "{} exceeds the {max_events}-event local recovery limit",
                        path.display()
                    ),
                });
            }
            let event: Event =
                serde_json::from_slice(line).map_err(|error| EventLogError::Corruption {
                    detail: format!("line {}: {error}", line_index + 1),
                })?;
            index.insert_existing(event)?;
        }
        if bytes.len() != committed_len {
            file.set_len(committed_len as u64).await?;
            file.sync_data().await?;
        }
        Ok(Self {
            path,
            state: Mutex::new(JsonlState {
                file,
                lock_file,
                committed_len: committed_len as u64,
                index,
            }),
        })
    }

    /// Backing canonical event-log path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
async fn ensure_private_event_log(file: &tokio::fs::File, path: &Path) -> std::io::Result<()> {
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("event log path is not a regular file: {}", path.display()),
        ));
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    if metadata.uid() != unsafe { libc::getuid() } {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "event log is not owned by the current user: {}",
                path.display()
            ),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    Ok(())
}

#[cfg(not(unix))]
async fn ensure_private_event_log(_file: &tokio::fs::File, _path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[async_trait]
impl EventReader for JsonlEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.state.lock().await.index.page(request)
    }
}

impl JsonlEventLog {
    /// `expected`: `None` appends unconditionally, `Some(last)` only when the
    /// session's log ends at `last`. Checked while holding the state mutex and
    /// the advisory file lock the append writes under, after confirming no
    /// other writer grew the file, so the check and the write are one step.
    async fn append_checked(
        &self,
        request: EventRequest,
        expected: Option<Option<i32>>,
    ) -> Result<Event, EventLogError> {
        reject_ephemeral(&request)?;
        let mut state = self.state.lock().await;
        let lock_file = state.lock_file.try_clone()?;
        fs2::FileExt::try_lock_exclusive(&lock_file).map_err(|error| EventLogError::Backend {
            detail: format!("cannot lock {} for append: {error}", self.path.display()),
        })?;
        let _file_lock = FileLockGuard(lock_file);
        let actual_len = state.file.metadata().await?.len();
        if actual_len != state.committed_len {
            return Err(EventLogError::Backend {
                detail: format!(
                    "{} was modified by another writer (expected {} bytes, found {actual_len})",
                    self.path.display(),
                    state.committed_len
                ),
            });
        }
        if let Some(expected) = expected {
            state
                .index
                .check_last_sequence(request.session_id, expected)?;
        }
        let sequence = state.index.next_sequence(request.session_id)?;
        let event = request.into_event(EventId::new(), sequence);
        let mut encoded =
            serde_json::to_vec(&event).map_err(|error| EventLogError::InvalidAppend {
                detail: error.to_string(),
            })?;
        encoded.push(b'\n');
        let previous_len = state.committed_len;
        let write_result = async {
            state.file.write_all(&encoded).await?;
            state.file.flush().await?;
            state.file.sync_data().await
        }
        .await;
        if let Err(error) = write_result {
            let _ = state.file.set_len(previous_len).await;
            let _ = state.file.sync_data().await;
            return Err(EventLogError::Backend {
                detail: error.to_string(),
            });
        }
        state.committed_len += encoded.len() as u64;
        state.index.insert_existing(event.clone())?;
        Ok(event)
    }
}

#[async_trait]
impl EventLog for JsonlEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        self.append_checked(request, None).await
    }

    async fn append_conditional(
        &self,
        request: EventRequest,
        expected_last_sequence: Option<i32>,
    ) -> Result<Event, EventLogError> {
        self.append_checked(request, Some(expected_last_sequence))
            .await
    }

    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}
