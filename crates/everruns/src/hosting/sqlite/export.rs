//! JSONL as a rebuildable export of the authority journal (EVE-1298).
//!
//! Decision: after cutover the SQLite authority is the only writer. JSONL is a
//! projection rebuilt from one read snapshot of the canonical journal, in the
//! exact `JsonlEventLog` line format, so tools that read legacy JSONL keep
//! working. Rebuilding overwrites the projection; nothing appends to it.
use super::*;
use std::io::Write;

/// Journal rows read per transaction while exporting.
const EXPORT_PAGE: usize = 512;

impl SqliteHosting {
    /// Write `session`'s canonical journal as JSONL, one complete event
    /// envelope per line in sequence order, and return the number of events.
    ///
    /// Reads pin the high watermark at the start, so the export is one
    /// consistent prefix even while the session keeps running. The output is
    /// a projection: rebuild it whenever needed and never write it back.
    /// Stability: experimental.
    pub fn export_jsonl(&self, session: SessionId, out: &mut impl Write) -> Result<usize> {
        let through = self.transaction(session, |_, state| {
            state.binding(session)?;
            Ok(state.head)
        })?;
        let mut after = 0;
        let mut written = 0;
        loop {
            let rows: Vec<(i32, String)> = self.transaction(session, |tx, _| {
                let mut query = tx
                    .prepare(
                        "SELECT seq,value FROM framework_events WHERE session=? AND seq>? AND seq<=? ORDER BY seq LIMIT ?",
                    )
                    .map_err(error)?;
                query
                    .query_map(
                        params![session.to_string(), after, through, EXPORT_PAGE as i64],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(error)?
                    .map(|row| row.map_err(error))
                    .collect()
            })?;
            let Some((last, _)) = rows.last() else {
                break;
            };
            after = *last;
            for (_, value) in &rows {
                // Stored rows are already the canonical envelope JSON.
                out.write_all(value.as_bytes()).map_err(error)?;
                out.write_all(b"\n").map_err(error)?;
                written += 1;
            }
            if rows.len() < EXPORT_PAGE {
                break;
            }
        }
        out.flush().map_err(error)?;
        Ok(written)
    }

    /// Rebuild a JSONL projection file of `session` atomically: write a
    /// private temporary file next to `path`, sync it, then rename it over
    /// `path`. A reader sees the previous projection or the new one, never a
    /// partial file. Stability: experimental.
    pub fn rebuild_jsonl_export(
        &self,
        session: SessionId,
        path: impl AsRef<Path>,
    ) -> Result<usize> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| AgentLoopError::config("export path has no file name"))?;
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // THREAT[TM-FS-014]: the export holds full conversation content.
            options.mode(0o600);
        }
        let result = (|| {
            let file = options.open(&temporary).map_err(error)?;
            let mut writer = std::io::BufWriter::new(file);
            let written = self.export_jsonl(session, &mut writer)?;
            writer
                .into_inner()
                .map_err(|error| AgentLoopError::store(error.to_string()))?
                .sync_all()
                .map_err(error)?;
            std::fs::rename(&temporary, path).map_err(error)?;
            Ok(written)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}
