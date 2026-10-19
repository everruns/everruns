//! A database file as pages: a consistent copy of a live SQLite file, which
//! pages changed since the last copy, and the segment format that carries
//! them.
//!
//! Decisions:
//! - The copy goes through SQLite's online backup into a mirror file, so it
//!   is one consistent read (WAL frames included) while the engine keeps
//!   writing. Backup keeps page numbers, so two copies compare page by page.
//! - Pages are compared by SHA-256, kept in memory per file. Reading the copy
//!   costs its size on local disk; what goes to the bucket is only the pages
//!   that changed.
//! - A segment is a small binary object: a header, then each changed page
//!   with its number. Restoring writes the snapshot, then every segment in
//!   order, then cuts the file to the last page count.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

const SEGMENT_MAGIC: &[u8; 8] = b"EVRSEG1\0";
const HEADER_LEN: usize = 8 + 4 + 4 + 4;

/// What the file system says about a database file and its WAL: when it
/// moves, the file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    db: Option<(u64, SystemTime)>,
    wal: Option<(u64, SystemTime)>,
}

pub(super) fn fingerprint(path: &Path) -> Fingerprint {
    let stat = |path: &Path| {
        std::fs::metadata(path)
            .ok()
            .and_then(|meta| Some((meta.len(), meta.modified().ok()?)))
    };
    Fingerprint {
        db: stat(path),
        wal: stat(&sidecar(path, "-wal")),
    }
}

/// `path` with `suffix` appended to its file name (`local.db-wal`).
pub(super) fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// A consistent copy of the database at `path`, made through `mirror`.
pub(super) fn copy(path: &Path, mirror: &Path) -> crate::Result<Vec<u8>> {
    if let Some(parent) = mirror.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for stale in [mirror.to_path_buf(), sidecar(mirror, "-journal")] {
        match std::fs::remove_file(&stale) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    everruns::sqlite::backup(path, mirror)
        .with_context(|| format!("copying {}", path.display()))?;
    Ok(std::fs::read(mirror)?)
}

/// The page size a database file's header records.
pub(super) fn page_size(bytes: &[u8]) -> crate::Result<u32> {
    if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
        bail!("not a SQLite database");
    }
    let size = u32::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    Ok(if size == 1 { 65_536 } else { size })
}

/// The page hashes of the copy last sent to the bucket.
#[derive(Default)]
pub(super) struct Pages {
    page_size: u32,
    hashes: Vec<[u8; 32]>,
}

/// What changed between the copy last sent and a new one.
pub(super) struct Changes {
    pub page_size: u32,
    pub page_count: u32,
    /// Page numbers, from 1, whose content changed or is new.
    pub changed: Vec<u32>,
    hashes: Vec<[u8; 32]>,
}

impl Pages {
    pub(super) fn of(bytes: &[u8]) -> crate::Result<Self> {
        let page_size = page_size(bytes)?;
        Ok(Self {
            page_size,
            hashes: hashes(bytes, page_size),
        })
    }

    /// Compare a new copy with this one. `None` when the page size changed,
    /// which only a new snapshot can carry.
    pub(super) fn changes(&self, bytes: &[u8]) -> crate::Result<Option<Changes>> {
        let page_size = page_size(bytes)?;
        if self.page_size != 0 && page_size != self.page_size {
            return Ok(None);
        }
        let hashes = hashes(bytes, page_size);
        let changed = hashes
            .iter()
            .enumerate()
            .filter(|(index, hash)| self.hashes.get(*index) != Some(hash))
            .map(|(index, _)| page_number(index))
            .collect();
        Ok(Some(Changes {
            page_size,
            page_count: u32::try_from(hashes.len()).unwrap_or(u32::MAX),
            changed,
            hashes,
        }))
    }

    /// Record a change set as sent.
    pub(super) fn accept(&mut self, changes: Changes) {
        self.page_size = changes.page_size;
        self.hashes = changes.hashes;
    }
}

fn page_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

fn hashes(bytes: &[u8], page_size: u32) -> Vec<[u8; 32]> {
    bytes
        .chunks(page_size as usize)
        .map(|page| Sha256::digest(page).into())
        .collect()
}

/// A segment holding the `changes` of the copy `bytes`.
pub(super) fn encode_segment(bytes: &[u8], changes: &Changes) -> Vec<u8> {
    let size = changes.page_size as usize;
    let mut out = Vec::with_capacity(HEADER_LEN + changes.changed.len() * (4 + size));
    out.extend_from_slice(SEGMENT_MAGIC);
    out.extend_from_slice(&changes.page_size.to_be_bytes());
    out.extend_from_slice(&changes.page_count.to_be_bytes());
    let count = u32::try_from(changes.changed.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_be_bytes());
    for &page in &changes.changed {
        let start = (page as usize - 1) * size;
        let mut data = bytes[start..bytes.len().min(start + size)].to_vec();
        data.resize(size, 0);
        out.extend_from_slice(&page.to_be_bytes());
        out.extend_from_slice(&data);
    }
    out
}

/// Apply a segment to a file's bytes.
pub(super) fn apply_segment(file: &mut Vec<u8>, segment: &[u8]) -> crate::Result<()> {
    if segment.len() < HEADER_LEN || &segment[..8] != SEGMENT_MAGIC {
        bail!("not a page segment");
    }
    let read = |at: usize| {
        u32::from_be_bytes([
            segment[at],
            segment[at + 1],
            segment[at + 2],
            segment[at + 3],
        ])
    };
    let size = read(8) as usize;
    let page_count = read(12) as usize;
    let count = read(16) as usize;
    if size == 0 || segment.len() != HEADER_LEN + count * (4 + size) {
        bail!("a page segment has the wrong length");
    }
    for entry in 0..count {
        let at = HEADER_LEN + entry * (4 + size);
        let page = read(at) as usize;
        if page == 0 {
            bail!("a page segment names page 0");
        }
        let start = (page - 1) * size;
        if file.len() < start + size {
            file.resize(start + size, 0);
        }
        file[start..start + size].copy_from_slice(&segment[at + 4..at + 4 + size]);
    }
    file.truncate(page_count * size);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::sqlite as rusqlite;

    fn db(dir: &Path, rows: usize) -> std::path::PathBuf {
        let path = dir.join("test.db");
        let conn = rusqlite::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, body TEXT);",
        )
        .unwrap();
        for i in 0..rows {
            conn.execute(
                "INSERT INTO t (body) VALUES (?1)",
                [format!("row {i} {}", "x".repeat(200))],
            )
            .unwrap();
        }
        path
    }

    #[test]
    fn segments_rebuild_the_copy_they_were_cut_from() {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join("mirror/test.db");
        let path = db(dir.path(), 50);
        let first = copy(&path, &mirror).unwrap();
        let mut pages = Pages::of(&first).unwrap();

        db(dir.path(), 400);
        let second = copy(&path, &mirror).unwrap();
        let changes = pages.changes(&second).unwrap().unwrap();
        assert!(!changes.changed.is_empty());
        assert!(changes.changed.len() < second.len() / changes.page_size as usize + 1);
        let segment = encode_segment(&second, &changes);
        pages.accept(changes);

        let mut rebuilt = first.clone();
        apply_segment(&mut rebuilt, &segment).unwrap();
        assert_eq!(rebuilt, second);

        let restored = dir.path().join("restored.db");
        std::fs::write(&restored, &rebuilt).unwrap();
        let conn = rusqlite::open(&restored).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 450);

        // Nothing new: no page changes.
        let third = copy(&path, &mirror).unwrap();
        assert!(pages.changes(&third).unwrap().unwrap().changed.is_empty());
    }

    #[test]
    fn a_bad_segment_is_refused() {
        let mut file = vec![0; 8];
        assert!(apply_segment(&mut file, b"nope").is_err());
        let mut segment = SEGMENT_MAGIC.to_vec();
        segment.extend_from_slice(&4096u32.to_be_bytes());
        segment.extend_from_slice(&1u32.to_be_bytes());
        segment.extend_from_slice(&1u32.to_be_bytes());
        assert!(apply_segment(&mut file, &segment).is_err());
    }
}
