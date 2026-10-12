// Page images of an in-memory SQLite database: snapshot, diff, rebuild.
//
// Decision: the persisted backend keeps each database as an in-memory SQLite
// cache and stores it in PostgreSQL as fixed-size pages, numbered from 1 like
// SQLite's own pages. After a write it serializes the database
// (`sqlite3_serialize`, already enabled through rusqlite's `serialize`
// feature, so no VFS and no new crate) and compares each page's SHA-256 with
// the digest of what was last persisted. Only changed pages are written, and
// pages past the new end are removed. Digests, not a second copy of the
// image, are what the cache keeps, so a 50 MB database costs 400 KB of
// bookkeeping rather than another 50 MB.

use super::error::SqlDbError;
use super::executor::configure_connection;
use super::limits::PAGE_SIZE;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// SHA-256 of one page.
pub type PageDigest = [u8; 32];

/// What a write changed, relative to the last persisted image.
#[derive(Debug)]
pub struct PageDiff {
    /// `(page_number, bytes)` for every page whose content changed or is new.
    pub changed: Vec<(i32, Vec<u8>)>,
    /// Pages in the new image; stored pages above this are removed.
    pub page_count: i32,
    /// Digests of the whole new image, which become the cache's baseline.
    pub digests: Vec<PageDigest>,
}

impl PageDiff {
    /// True when the image is identical to the baseline it was diffed against.
    pub fn is_unchanged(&self, baseline_pages: usize) -> bool {
        self.changed.is_empty() && self.page_count as usize == baseline_pages
    }

    pub fn size_bytes(&self) -> i64 {
        i64::from(self.page_count) * PAGE_SIZE as i64
    }
}

fn digest(page: &[u8]) -> PageDigest {
    Sha256::digest(page).into()
}

/// Digests of every page of `image`.
pub fn page_digests(image: &[u8]) -> Vec<PageDigest> {
    image.chunks(PAGE_SIZE).map(digest).collect()
}

/// Diff the current state of `conn` against `baseline`.
pub fn diff_connection(conn: &Connection, baseline: &[PageDigest]) -> Result<PageDiff, SqlDbError> {
    // A database nothing has been written to has no pages, and SQLite refuses
    // to serialize an empty image.
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(|e| SqlDbError::Internal(format!("failed to read page count: {e}")))?;
    if page_count == 0 {
        return diff_image(&[], baseline);
    }
    let image = conn
        .serialize("main")
        .map_err(|e| SqlDbError::Internal(format!("failed to serialize database: {e}")))?;
    diff_image(&image, baseline)
}

/// Diff a serialized image against `baseline`.
pub fn diff_image(image: &[u8], baseline: &[PageDigest]) -> Result<PageDiff, SqlDbError> {
    if !image.len().is_multiple_of(PAGE_SIZE) {
        return Err(SqlDbError::Internal(format!(
            "database image of {} bytes is not a whole number of {PAGE_SIZE}-byte pages",
            image.len()
        )));
    }
    let mut changed = Vec::new();
    let mut digests = Vec::with_capacity(image.len() / PAGE_SIZE);
    for (index, page) in image.chunks(PAGE_SIZE).enumerate() {
        let page_digest = digest(page);
        if baseline.get(index) != Some(&page_digest) {
            changed.push((index as i32 + 1, page.to_vec()));
        }
        digests.push(page_digest);
    }
    Ok(PageDiff {
        changed,
        page_count: digests.len() as i32,
        digests,
    })
}

/// Open a configured in-memory connection holding the given stored pages.
///
/// `pages` must be ordered and numbered `1..=n` with no gaps; anything else
/// means the stored database is damaged, which is refused rather than opened
/// half-empty.
pub fn open_from_pages(
    pages: &[(i32, Vec<u8>)],
) -> Result<(Connection, Vec<PageDigest>), SqlDbError> {
    let mut image = Vec::with_capacity(pages.len() * PAGE_SIZE);
    for (index, (page_number, data)) in pages.iter().enumerate() {
        if *page_number != index as i32 + 1 || data.len() != PAGE_SIZE {
            return Err(SqlDbError::Internal(format!(
                "stored database page {page_number} is missing or malformed"
            )));
        }
        image.extend_from_slice(data);
    }

    let mut conn = Connection::open_in_memory()
        .map_err(|e| SqlDbError::Internal(format!("failed to open in-memory db: {e}")))?;
    if !image.is_empty() {
        conn.deserialize_read_exact("main", image.as_slice(), image.len(), false)
            .map_err(|e| SqlDbError::Internal(format!("failed to load stored database: {e}")))?;
    }
    configure_connection(&conn)?;
    Ok((conn, page_digests(&image)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_sqldb::executor::{execute_query, execute_statement};

    fn fresh() -> Connection {
        open_from_pages(&[]).unwrap().0
    }

    #[test]
    fn empty_database_has_no_pages() {
        let diff = diff_connection(&fresh(), &[]).unwrap();
        assert_eq!(diff.page_count, 0);
        assert!(diff.is_unchanged(0));
    }

    #[test]
    fn first_write_changes_every_page_and_round_trips() {
        let conn = fresh();
        execute_statement(&conn, "CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('a')").unwrap();
        let diff = diff_connection(&conn, &[]).unwrap();
        assert!(diff.page_count > 0);
        assert_eq!(diff.changed.len(), diff.page_count as usize);

        let (reopened, digests) = open_from_pages(&diff.changed).unwrap();
        assert_eq!(digests, diff.digests);
        let rows = execute_query(&reopened, "SELECT v FROM t").unwrap();
        assert_eq!(rows.rows[0][0], serde_json::json!("a"));
        // The reopened database keeps accepting writes past its loaded size.
        execute_statement(
            &reopened,
            "INSERT INTO t SELECT hex(randomblob(2000)) FROM (SELECT 1 UNION SELECT 2 UNION SELECT 3)",
        )
        .unwrap();
        assert!(diff_connection(&reopened, &digests).unwrap().page_count > diff.page_count);
    }

    #[test]
    fn small_update_rewrites_few_pages() {
        let conn = fresh();
        execute_statement(
            &conn,
            "CREATE TABLE a (v TEXT); CREATE TABLE b (v TEXT);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 200)
             INSERT INTO a SELECT hex(randomblob(100)) FROM n;",
        )
        .unwrap();
        let base = diff_connection(&conn, &[]).unwrap();
        assert!(base.page_count > 10);

        execute_statement(&conn, "INSERT INTO b VALUES ('x')").unwrap();
        let diff = diff_connection(&conn, &base.digests).unwrap();
        assert!(!diff.changed.is_empty());
        assert!(
            diff.changed.len() < base.page_count as usize / 2,
            "{} of {} pages changed",
            diff.changed.len(),
            base.page_count
        );
    }

    #[test]
    fn read_leaves_image_unchanged() {
        let conn = fresh();
        execute_statement(&conn, "CREATE TABLE t (v INTEGER)").unwrap();
        let base = diff_connection(&conn, &[]).unwrap();
        execute_query(&conn, "SELECT * FROM t").unwrap();
        assert!(
            diff_connection(&conn, &base.digests)
                .unwrap()
                .is_unchanged(base.digests.len())
        );
    }

    #[test]
    fn gaps_in_stored_pages_are_refused() {
        let page = vec![0u8; PAGE_SIZE];
        let err = open_from_pages(&[(1, page.clone()), (3, page)]).unwrap_err();
        assert!(matches!(err, SqlDbError::Internal(_)));
        let err = open_from_pages(&[(1, vec![0u8; 10])]).unwrap_err();
        assert!(matches!(err, SqlDbError::Internal(_)));
    }
}
