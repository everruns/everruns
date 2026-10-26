// Streaming reads and writes of session files, with HTTP byte-range support.
//
// Decision: downloads never go through `read_file`. That path loads the whole
// file, base64-encodes it into the JSON contract and decodes it again, which
// holds a large recording in memory several times. Files stored in the
// workspace table stream from the repository instead (object storage or the
// requested slice of the inline column). Knowledge, Memory and virtual mounts
// are small generated files, so they keep using `read_file` and are sliced in
// memory.
//
// Writes stream the same way: `write_file_stream` hands the body to the
// repository, which uploads it to object storage in parts. Memory and other
// mounted paths are refused there; they hold small notes, written through the
// regular create/update path.

use super::WorkspaceFileService;
use crate::kernel_imports::SessionFile;
use crate::storage::blob_store::BlobByteStream;
use anyhow::{Result, anyhow};
use futures::StreamExt;
use std::ops::Range;
use uuid::Uuid;

/// Files of unknown type up to this size are sniffed for UTF-8 to pick between
/// `text/plain` and `application/octet-stream`, matching the JSON contract's
/// text/base64 choice. Larger unknown files are served as binary.
const SNIFF_UNKNOWN_TYPE_MAX_BYTES: i64 = 1024 * 1024;

/// One `Range: bytes=...` request, before it is resolved against a file size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRangeRequest {
    /// `bytes=a-b` (both inclusive).
    Bounded { first: u64, last: u64 },
    /// `bytes=a-`.
    From { first: u64 },
    /// `bytes=-n`: the last `n` bytes.
    Suffix { length: u64 },
}

impl ByteRangeRequest {
    /// Parse a `Range` header value. Multi-range and malformed values yield
    /// `None`, which HTTP allows a server to answer with the whole file.
    pub fn parse(header: &str) -> Option<Self> {
        let spec = header.trim().strip_prefix("bytes=")?.trim();
        if spec.contains(',') {
            return None;
        }
        let (first, last) = spec.split_once('-')?;
        let (first, last) = (first.trim(), last.trim());
        match (first.is_empty(), last.is_empty()) {
            (false, true) => Some(Self::From {
                first: first.parse().ok()?,
            }),
            (true, false) => Some(Self::Suffix {
                length: last.parse().ok()?,
            }),
            (false, false) => {
                let (first, last) = (first.parse().ok()?, last.parse().ok()?);
                (first <= last).then_some(Self::Bounded { first, last })
            }
            (true, true) => None,
        }
    }

    /// Resolve against a file of `size` bytes into a half-open range, or
    /// `None` when the range cannot be satisfied (answered with 416).
    pub fn resolve(self, size: u64) -> Option<Range<u64>> {
        let range = match self {
            Self::Bounded { first, last } => first..last.saturating_add(1).min(size),
            Self::From { first } => first..size,
            Self::Suffix { length } => size.saturating_sub(length)..size,
        };
        (range.start < range.end).then_some(range)
    }
}

/// The requested range lies outside the file.
#[derive(Debug, thiserror::Error)]
#[error("Requested range is outside the {size} byte file")]
pub struct RangeNotSatisfiable {
    pub size: u64,
}

/// A file opened for a streaming download.
pub struct OpenedFile {
    pub path: String,
    pub name: String,
    /// Size of the whole file.
    pub size: u64,
    pub content_type: String,
    /// Strong validator for `If-Range` and caching.
    pub etag: String,
    /// The served range, `None` for the whole file.
    pub range: Option<Range<u64>>,
    pub body: BlobByteStream,
}

impl WorkspaceFileService {
    /// Open a file for streaming, optionally a byte range of it.
    ///
    /// `Ok(None)` when the path does not exist. Directories are an error; the
    /// HTTP layer checks for them first to answer 400.
    pub async fn open_file(
        &self,
        session_id: Uuid,
        path: &str,
        range: Option<ByteRangeRequest>,
    ) -> Result<Option<OpenedFile>> {
        self.restore_virtual_mounts(session_id).await?;
        let path = Self::normalize_path(path);
        let routed_elsewhere = everruns_contracts::knowledge::is_mount(&path)
            || self.route_memory(session_id, &path).await.is_some()
            || self
                .virtual_registry
                .as_ref()
                .is_some_and(|registry| registry.read_file(&session_id, &path).is_some());
        if routed_elsewhere {
            return self.open_loaded_file(session_id, &path, range).await;
        }

        let Some(info) = self.db.get_session_file_info(session_id, &path).await? else {
            return Ok(None);
        };
        if info.is_directory {
            return Err(anyhow!("Cannot download a directory: {path}"));
        }
        let size = info.size_bytes.max(0) as u64;
        let resolved = match range {
            Some(request) => Some(request.resolve(size).ok_or(RangeNotSatisfiable { size })?),
            None => None,
        };
        let known_type = guessed_content_type(&path);
        if known_type.is_none() && info.size_bytes <= SNIFF_UNKNOWN_TYPE_MAX_BYTES {
            // Small file of unknown type: the JSON path already decides text
            // vs binary by loading it, so reuse that.
            return self.open_loaded_file(session_id, &path, range).await;
        }

        let Some(content) = self
            .db
            .open_session_file_content(session_id, &path, resolved.clone())
            .await?
        else {
            return Ok(None);
        };
        let etag = match &content.content_sha256 {
            Some(sha) => format!("\"{sha}\""),
            None => format!(
                "\"{}-{}-{}\"",
                info.id,
                info.size_bytes,
                info.updated_at.timestamp_micros()
            ),
        };
        Ok(Some(OpenedFile {
            name: crate::kernel_imports::FileInfo::name_from_path(&path),
            path,
            size,
            content_type: known_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            etag,
            range: resolved,
            body: content.body,
        }))
    }

    /// Create or replace a file from a byte stream.
    ///
    /// Applies the same path, mount, read-only and quota rules as
    /// `create_file` / `update_file`. `declared_bytes` (a `Content-Length`)
    /// is checked against the quota before any byte is read, and the bytes
    /// actually received are capped by both the per-file limit and the room
    /// left in the session.
    pub async fn write_file_stream(
        &self,
        session_id: Uuid,
        path: &str,
        body: BlobByteStream,
        declared_bytes: Option<u64>,
        is_readonly: Option<bool>,
    ) -> Result<crate::kernel_imports::FileInfo> {
        self.restore_virtual_mounts(session_id).await?;
        let path = Self::normalize_path(path);
        Self::validate_path(&path)?;
        self.ensure_path_not_virtual(session_id, &path)?;
        if everruns_contracts::knowledge::is_mount(&path)
            || self.route_memory(session_id, &path).await.is_some()
        {
            return Err(anyhow!(
                "Cannot upload into a mounted folder: {path}; write the file instead"
            ));
        }
        self.ensure_parent_path_writable(session_id, &path).await?;

        let existing = self.db.get_session_file_info(session_id, &path).await?;
        let existing_size = match &existing {
            Some(file) if file.is_directory => {
                return Err(anyhow!("Cannot update directory: {path}"));
            }
            Some(file) if file.is_readonly => {
                return Err(anyhow!("Cannot modify readonly file: {path}"));
            }
            Some(file) => file.size_bytes,
            None => 0,
        };
        if let Some(declared) = declared_bytes {
            self.check_write_quota(session_id, declared as i64, existing_size)
                .await?;
        }
        if let Some(parent) = crate::kernel_imports::FileInfo::parent_path(&path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        // The stream may not say its size up front, so bound it by what the
        // session can still hold as well as by the per-file limit.
        let session_room = self.quota.per_session_bytes
            - (self.db.total_session_file_bytes(session_id).await? - existing_size);
        let max_bytes = self.quota.per_file_bytes.min(session_room).max(0) as u64;
        let row = self
            .db
            .write_session_file_stream(session_id, &path, body, max_bytes, is_readonly)
            .await?;
        Ok(Self::row_to_file_info_from_info(row))
    }

    /// Serve a file through the whole-file `read_file` path, slicing in memory.
    async fn open_loaded_file(
        &self,
        session_id: Uuid,
        path: &str,
        range: Option<ByteRangeRequest>,
    ) -> Result<Option<OpenedFile>> {
        let Some(file) = self.read_file(session_id, path).await? else {
            return Ok(None);
        };
        if file.is_directory {
            return Err(anyhow!("Cannot download a directory: {path}"));
        }
        let bytes = match file.content.as_deref() {
            Some(content) => SessionFile::decode_content(content, &file.encoding)?,
            None => Vec::new(),
        };
        let size = bytes.len() as u64;
        let resolved = match range {
            Some(request) => Some(request.resolve(size).ok_or(RangeNotSatisfiable { size })?),
            None => None,
        };
        let content_type = guessed_content_type(&file.path).unwrap_or_else(|| {
            if file.encoding == "base64" {
                "application/octet-stream".to_string()
            } else {
                "text/plain; charset=utf-8".to_string()
            }
        });
        let etag = {
            use sha2::{Digest, Sha256};
            format!("\"{}\"", hex::encode(Sha256::digest(&bytes)))
        };
        let slice = match &resolved {
            Some(range) => bytes[range.start as usize..range.end as usize].to_vec(),
            None => bytes,
        };
        let body = futures::stream::once(async move { Ok(bytes::Bytes::from(slice)) }).boxed();
        Ok(Some(OpenedFile {
            path: file.path,
            name: file.name,
            size,
            content_type,
            etag,
            range: resolved,
            body,
        }))
    }
}

fn guessed_content_type(path: &str) -> Option<String> {
    mime_guess::from_path(path)
        .first_raw()
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_ranges() {
        assert_eq!(
            ByteRangeRequest::parse("bytes=0-99"),
            Some(ByteRangeRequest::Bounded { first: 0, last: 99 })
        );
        assert_eq!(
            ByteRangeRequest::parse("bytes=100-"),
            Some(ByteRangeRequest::From { first: 100 })
        );
        assert_eq!(
            ByteRangeRequest::parse("bytes=-500"),
            Some(ByteRangeRequest::Suffix { length: 500 })
        );
    }

    #[test]
    fn ignores_ranges_it_does_not_serve() {
        for header in [
            "bytes=0-1,5-9",
            "bytes=9-1",
            "bytes=-",
            "items=0-1",
            "bytes=a-b",
            "",
        ] {
            assert_eq!(ByteRangeRequest::parse(header), None, "{header}");
        }
    }

    #[test]
    fn resolves_against_file_size() {
        let size = 1000;
        assert_eq!(
            ByteRangeRequest::Bounded { first: 0, last: 99 }.resolve(size),
            Some(0..100)
        );
        // A last byte past the end is clamped.
        assert_eq!(
            ByteRangeRequest::Bounded {
                first: 900,
                last: 5000
            }
            .resolve(size),
            Some(900..1000)
        );
        assert_eq!(
            ByteRangeRequest::From { first: 990 }.resolve(size),
            Some(990..1000)
        );
        assert_eq!(
            ByteRangeRequest::Suffix { length: 10 }.resolve(size),
            Some(990..1000)
        );
        // A suffix longer than the file is the whole file.
        assert_eq!(
            ByteRangeRequest::Suffix { length: 5000 }.resolve(size),
            Some(0..1000)
        );
    }

    #[test]
    fn rejects_unsatisfiable_ranges() {
        assert_eq!(ByteRangeRequest::From { first: 1000 }.resolve(1000), None);
        assert_eq!(ByteRangeRequest::Suffix { length: 0 }.resolve(1000), None);
        assert_eq!(ByteRangeRequest::From { first: 0 }.resolve(0), None);
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::domains::session_files::limits::QuotaLimits;
    use crate::storage::StorageBackend;
    use crate::storage::blob_store::BlobTooLarge;
    use std::sync::Arc;

    fn chunks(parts: &[&[u8]]) -> BlobByteStream {
        let parts: Vec<Result<bytes::Bytes>> = parts
            .iter()
            .map(|part| Ok(bytes::Bytes::copy_from_slice(part)))
            .collect();
        futures::stream::iter(parts).boxed()
    }

    async fn collect(file: OpenedFile) -> Vec<u8> {
        let mut body = file.body;
        let mut out = Vec::new();
        while let Some(chunk) = body.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        out
    }

    fn service(db: StorageBackend, per_file: i64, per_session: i64) -> WorkspaceFileService {
        WorkspaceFileService::new(Arc::new(db)).with_quota_limits(QuotaLimits {
            per_file_bytes: per_file,
            per_session_bytes: per_session,
        })
    }

    #[tokio::test]
    async fn streamed_write_round_trips_whole_and_ranges() {
        let db = StorageBackend::test_database();
        let sid = db.create_test_session().await.uuid();
        let svc = service(db, 1024, 4096);

        let info = svc
            .write_file_stream(
                sid,
                "/recordings/turn-1.mp4",
                chunks(&[b"0123", b"456789"]),
                None,
                Some(true),
            )
            .await
            .unwrap();
        assert_eq!(info.size_bytes, 10);
        assert!(info.is_readonly);

        let whole = svc
            .open_file(sid, "/recordings/turn-1.mp4", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(whole.content_type, "video/mp4");
        assert_eq!(whole.size, 10);
        assert_eq!(whole.range, None);
        assert_eq!(collect(whole).await, b"0123456789");

        let range = svc
            .open_file(
                sid,
                "/recordings/turn-1.mp4",
                ByteRangeRequest::parse("bytes=2-5"),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(range.range, Some(2..6));
        assert_eq!(collect(range).await, b"2345");

        let error = svc
            .open_file(
                sid,
                "/recordings/turn-1.mp4",
                ByteRangeRequest::parse("bytes=10-"),
            )
            .await
            .err()
            .expect("range past the end");
        assert_eq!(
            error.downcast_ref::<RangeNotSatisfiable>().map(|e| e.size),
            Some(10)
        );
    }

    #[tokio::test]
    async fn streamed_write_respects_read_only_and_limits() {
        let db = StorageBackend::test_database();
        let sid = db.create_test_session().await.uuid();
        let svc = service(db, 8, 12);

        svc.write_file_stream(sid, "/a.bin", chunks(&[b"12345"]), Some(5), Some(true))
            .await
            .unwrap();
        let error = svc
            .write_file_stream(sid, "/a.bin", chunks(&[b"x"]), Some(1), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("readonly"), "{error}");

        // Over the per-file limit while streaming, with no declared size.
        let error = svc
            .write_file_stream(sid, "/b.bin", chunks(&[b"12345", b"6789"]), None, None)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<BlobTooLarge>().is_some(), "{error}");

        // Within the per-file limit, but past the room left in the session.
        let error = svc
            .write_file_stream(sid, "/c.bin", chunks(&[b"12345678"]), None, None)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<BlobTooLarge>().is_some(), "{error}");
        assert!(svc.stat(sid, "/c.bin").await.unwrap().is_none());

        // A declared size over the quota is refused before reading.
        let error = svc
            .write_file_stream(sid, "/d.bin", chunks(&[]), Some(100), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("too large"), "{error}");
    }

    #[tokio::test]
    async fn unknown_types_keep_the_text_or_binary_choice() {
        let db = StorageBackend::test_database();
        let sid = db.create_test_session().await.uuid();
        let svc = service(db, 1024, 4096);

        svc.write_file_stream(sid, "/NOTES", chunks(&[b"plain text"]), None, None)
            .await
            .unwrap();
        svc.write_file_stream(sid, "/BLOB", chunks(&[&[0xff, 0xfe, 0x00]]), None, None)
            .await
            .unwrap();
        let text = svc.open_file(sid, "/NOTES", None).await.unwrap().unwrap();
        assert_eq!(text.content_type, "text/plain; charset=utf-8");
        let binary = svc.open_file(sid, "/BLOB", None).await.unwrap().unwrap();
        assert_eq!(binary.content_type, "application/octet-stream");
    }

    #[tokio::test]
    async fn offloaded_streamed_write_and_shared_copy() {
        use crate::storage::blob_store::ObjectStoreBlobStore;
        let (pool, database) = crate::storage::test_database::create_test_database();
        let db = StorageBackend::from_database(
            crate::storage::Database::new(pool)
                .with_test_database(database)
                .with_blob_store(Some(Arc::new(ObjectStoreBlobStore::in_memory()))),
        );
        let sid = db.create_test_session().await.uuid();
        let svc = service(db, 64 * 1024 * 1024, 128 * 1024 * 1024);
        // More than one multipart part.
        let big: Vec<u8> = (0..=255u8).cycle().take(9 * 1024 * 1024 + 7).collect();
        let parts: Vec<&[u8]> = big.chunks(1024 * 1024).collect();

        let info = svc
            .write_file_stream(sid, "/video.webm", chunks(&parts), None, None)
            .await
            .unwrap();
        assert_eq!(info.size_bytes, big.len() as i64);

        let tail = svc
            .open_file(sid, "/video.webm", ByteRangeRequest::parse("bytes=-100"))
            .await
            .unwrap()
            .unwrap();
        assert!(tail.etag.len() > 60, "etag is the content hash");
        assert_eq!(collect(tail).await, big[big.len() - 100..].to_vec());

        // Replacing streams a new revision; a copy shares it.
        svc.write_file_stream(sid, "/video.webm", chunks(&[b"short"]), None, None)
            .await
            .unwrap();
        svc.copy_file(
            sid,
            super::super::CopyFileInput {
                src_path: "/video.webm".to_string(),
                dst_path: "/copy.webm".to_string(),
            },
        )
        .await
        .unwrap();
        let copy = svc
            .open_file(sid, "/copy.webm", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(collect(copy).await, b"short");
    }
}
