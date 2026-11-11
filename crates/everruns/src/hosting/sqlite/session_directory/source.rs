//! Read and transfer earlier layouts under the same family ownership fence.
use super::*;

pub(super) enum SessionSource {
    Shared(PathBuf),
    Owned(PathBuf),
}

impl SessionSource {
    fn open(&self) -> Result<rusqlite::Connection> {
        let path = match self {
            Self::Shared(path) | Self::Owned(path) => path,
        };
        let source =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(error)?;
        source.busy_timeout(Duration::from_secs(5)).map_err(error)?;
        Ok(source)
    }

    fn control(&self, session: SessionId) -> Result<Option<Control>> {
        source_control(&self.open()?, session)
    }

    fn children(&self, session: SessionId) -> Result<Vec<SessionId>> {
        let source = self.open()?;
        let sql = match self {
            Self::Owned(_) => {
                "SELECT session FROM framework_session_children WHERE ?1 IS NOT NULL LIMIT 65537"
            }
            Self::Shared(_) => {
                "SELECT session FROM framework_authority WHERE json_extract(control,'$.binding.metadata.parent_session_id')=?1 LIMIT 65537"
            }
        };
        let mut query = source.prepare(sql).map_err(error)?;
        query
            .query_map([session.to_string()], |row| row.get::<_, String>(0))
            .map_err(error)?
            .map(|row| row.map_err(error)?.parse().map_err(error))
            .collect()
    }

    pub(super) fn transfer(&self, destination: &Path, session: SessionId) -> Result<()> {
        match self {
            Self::Shared(path) => migrate_shared(path, destination, session),
            Self::Owned(_) => self.transfer_owned(destination),
        }
    }

    fn validate_owned(&self, session: SessionId) -> Result<()> {
        if matches!(self, Self::Owned(_)) {
            let source = self.open()?;
            for (table, column) in [
                ("framework_authority", "session"),
                ("framework_events", "session"),
                ("framework_objects", "session"),
                ("framework_commands", "session"),
                ("local_tasks", "session_id"),
                ("local_schedules", "session_id"),
                ("local_task_recovery_wakes", "session_id"),
                ("dreaming_state", "session_id"),
            ] {
                let foreign: bool = source
                    .query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}<>?)"),
                        [session.to_string()],
                        |row| row.get(0),
                    )
                    .map_err(error)?;
                if foreign {
                    return Err(error(
                        "session directory database contains foreign session records",
                    ));
                }
            }
        }
        Ok(())
    }

    fn transfer_owned(&self, destination: &Path) -> Result<()> {
        let temporary =
            destination.with_file_name(format!("transfer-{}.sqlite", uuid::Uuid::new_v4()));
        let transfer = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&temporary).map_err(error)?;
            let source = self.open()?;
            let mut target = rusqlite::Connection::open(&temporary).map_err(error)?;
            // SQLite backup includes committed WAL frames and the child registry.
            // A raw filesystem copy could silently omit the last transactions.
            rusqlite::backup::Backup::new(&source, &mut target)
                .map_err(error)?
                .run_to_completion(128, Duration::from_millis(5), None)
                .map_err(error)?;
            target
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .map_err(error)?;
            target.close().map_err(|(_, failure)| error(failure))?;
            std::fs::File::open(&temporary)
                .map_err(error)?
                .sync_all()
                .map_err(error)?;
            std::fs::rename(&temporary, destination).map_err(error)?;
            #[cfg(unix)]
            std::fs::File::open(destination.parent().expect("session database parent"))
                .map_err(error)?
                .sync_all()
                .map_err(error)?;
            Ok(())
        })();
        if transfer.is_err() {
            let _ = std::fs::remove_file(&temporary);
            let _ = std::fs::remove_file(temporary.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(temporary.with_extension("sqlite-shm"));
        }
        transfer
    }
}

pub(super) struct SourceFamily {
    pub(super) sessions: Vec<SessionId>,
    // Source producers stay quiescent until every family snapshot is published.
    _leases: Vec<Box<dyn Send + Sync>>,
}

fn source_control(source: &rusqlite::Connection, session: SessionId) -> Result<Option<Control>> {
    source
        .query_row(
            "SELECT control FROM framework_authority WHERE session=?",
            [session.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(error)?
        .map(decode)
        .transpose()
}

impl SessionDirectory {
    pub(super) fn source_for(&self, session: SessionId) -> Result<Option<SessionSource>> {
        for path in &self.sources {
            if path == &self.root {
                continue;
            }
            if path.is_dir() {
                let database = SqliteHosting::session_db_path(path, session);
                if database.exists() {
                    return Ok(Some(SessionSource::Owned(database)));
                }
            } else if path.exists() {
                let source = SessionSource::Shared(path.clone());
                if source.control(session)?.is_some() {
                    return Ok(Some(source));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn source_family(&self, selected: SessionId) -> Result<SourceFamily> {
        let mut family = SourceFamily {
            sessions: Vec::new(),
            _leases: Vec::new(),
        };
        let mut leased = HashSet::new();
        let mut ancestors = HashSet::new();
        let mut cursor = selected;
        // A worker may be opened first. Quiesce its ancestors and siblings as
        // well: coordinator updates and child reports cross session boundaries.
        loop {
            if !ancestors.insert(cursor) {
                return Err(error("source session parent link contains a cycle"));
            }
            if ancestors.len() > 65_536 {
                return Err(error("source session family exceeds transfer limit"));
            }
            if leased.insert(cursor) {
                family._leases.push((self.ownership)(cursor)?);
                family.sessions.push(cursor);
            }
            let source = self.source_for(cursor)?;
            let state = source
                .as_ref()
                .map(|source| source.control(cursor))
                .transpose()?
                .flatten();
            let Some(state) = state else {
                if matches!(source, Some(SessionSource::Owned(_))) {
                    break;
                }
                if cursor != selected {
                    return Err(error("source session parent is missing"));
                }
                return Ok(family);
            };
            let binding = state.binding(cursor)?;
            let parent = binding.metadata.get("parent_session_id");
            match parent {
                None | Some(Value::Null) => break,
                Some(Value::String(parent)) => cursor = parent.parse().map_err(error)?,
                Some(_) => return Err(error("invalid source session parent link")),
            }
        }
        let root = cursor;
        // A child can create another child just before its source lease is
        // acquired. Rescan under the retained leases until the family is closed.
        loop {
            let mut pending = vec![root];
            let mut visited = HashSet::new();
            let before = leased.len();
            while let Some(session) = pending.pop() {
                if !visited.insert(session) {
                    continue;
                }
                if visited.len() > 65_536 {
                    return Err(error("source session family exceeds transfer limit"));
                }
                if leased.insert(session) {
                    family._leases.push((self.ownership)(session)?);
                    family.sessions.push(session);
                }
                let children = self
                    .source_for(session)?
                    .map(|source| source.children(session))
                    .transpose()?
                    .unwrap_or_default();
                pending.extend(children);
            }
            if leased.len() == before {
                break;
            }
        }
        // Refuse a live epoch anywhere in the family before publishing even
        // the first destination. The host leases cover all source producers.
        for session in &family.sessions {
            let source = self
                .source_for(*session)?
                .ok_or_else(|| error("source family session disappeared"))?;
            source.validate_owned(*session)?;
            let Some(state) = source.control(*session)? else {
                if matches!(source, SessionSource::Owned(_)) {
                    continue;
                }
                return Err(error("source family session disappeared"));
            };
            state.binding(*session)?;
            if state.holder.is_some() && state.expires > now() {
                return Err(error(
                    "cannot transfer a session with a live execution epoch",
                ));
            }
        }
        Ok(family)
    }
}
