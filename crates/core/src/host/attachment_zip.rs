//! Reads a zip attachment into the files it unpacks to.
//!
//! Decision: a zip attachment is unpacked next to itself in the session's
//! file storage (`/attachments/<id>/data.zip` → `/attachments/<id>/data/`), so
//! the agent works with its contents as ordinary read-only files. Unpacking is
//! all or nothing: the same check rejects a zip at upload, so a stored upload
//! always unpacks and the label that names the folder is true.

// Design: knowledge/runtime-resources/session-attachments.md.
// THREAT[TM-DOS-001, TM-FS-020]: sizes are measured while decompressing, never
// taken from entry headers, and every limit fails the whole archive. Paths
// that climb out (`..`, absolute), symlinks and encrypted entries are refused.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use everruns_contracts::runtime::attachments::sanitize_file_name;

/// Most files one zip may unpack to.
pub const MAX_ZIP_FILES: usize = 1_000;
/// Most bytes one zip may unpack to.
pub const MAX_ZIP_UNPACKED_BYTES: usize = 100 * 1024 * 1024;
/// Largest single file a zip may unpack to.
pub const MAX_ZIP_FILE_BYTES: usize = 25 * 1024 * 1024;
/// Highest unpacked-to-packed ratio, for one entry and for the whole zip.
const MAX_COMPRESSION_RATIO: usize = 100;
/// Entries below this size may exceed the ratio (a small run of zeros does).
const RATIO_EXEMPT_BYTES: usize = 1024 * 1024;
/// Raw entries read before giving up, directories and skipped files included.
const MAX_ZIP_ENTRIES: usize = 2 * MAX_ZIP_FILES;
/// Deepest folder nesting kept.
const MAX_PATH_DEPTH: usize = 16;

/// Whether `bytes` start like a zip archive with at least one entry.
pub fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Read a zip into `relative path → bytes`, or say why it cannot be unpacked.
///
/// Folders are implied by paths. macOS metadata (`__MACOSX/`, `.DS_Store`)
/// and symlinks are skipped; anything that breaks a limit or tries to leave
/// the folder rejects the whole archive.
pub fn unpack_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a valid zip: {e}"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!("more than {MAX_ZIP_FILES} files"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("cannot read an entry: {e}"))?;
        if entry.encrypted() {
            return Err("encrypted zips are not supported".to_string());
        }
        let Some(path) = entry_path(entry.name())? else {
            continue;
        };
        if entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            continue;
        }
        if files.len() == MAX_ZIP_FILES {
            return Err(format!("more than {MAX_ZIP_FILES} files"));
        }
        let packed = entry.compressed_size() as usize;
        let mut content = Vec::new();
        (&mut entry)
            .take(MAX_ZIP_FILE_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|e| format!("cannot unpack {path}: {e}"))?;
        if content.len() > MAX_ZIP_FILE_BYTES {
            return Err(format!(
                "{path} is larger than {} MiB",
                MAX_ZIP_FILE_BYTES >> 20
            ));
        }
        if content.len() > RATIO_EXEMPT_BYTES
            && content.len() > packed.max(1) * MAX_COMPRESSION_RATIO
        {
            return Err(format!(
                "{path} is compressed more than {MAX_COMPRESSION_RATIO}:1"
            ));
        }
        total += content.len();
        if total > MAX_ZIP_UNPACKED_BYTES {
            return Err(format!(
                "unpacks to more than {} MiB",
                MAX_ZIP_UNPACKED_BYTES >> 20
            ));
        }
        // Names that differ only in unsafe characters collapse to one path;
        // the first entry wins.
        files.entry(path).or_insert(content);
    }
    if total > RATIO_EXEMPT_BYTES && total > bytes.len() * MAX_COMPRESSION_RATIO {
        return Err(format!("compressed more than {MAX_COMPRESSION_RATIO}:1"));
    }
    Ok(files)
}

/// Safe relative path for an entry, `None` for metadata to skip, or an error
/// for a path that tries to leave the folder.
fn entry_path(name: &str) -> Result<Option<String>, String> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.split('/').next().is_some_and(|s| s.ends_with(':')) {
        return Err(format!("{name} is an absolute path"));
    }
    let segments: Vec<&str> = name
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if segments.contains(&"..") {
        return Err(format!("{name} points outside the archive"));
    }
    if segments.first() == Some(&"__MACOSX") || segments.last().is_some_and(|s| *s == ".DS_Store") {
        return Ok(None);
    }
    if segments.len() > MAX_PATH_DEPTH {
        return Err(format!(
            "{name} is nested more than {MAX_PATH_DEPTH} folders deep"
        ));
    }
    let cleaned: Vec<String> = segments
        .iter()
        .map(|segment| sanitize_file_name(segment))
        .collect();
    if cleaned.is_empty() || cleaned.iter().any(String::is_empty) {
        return Ok(None);
    }
    Ok(Some(cleaned.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacks_files_with_folders_and_skips_mac_metadata() {
        let bytes = zip_of(&[
            ("data/report.csv", b"a,b\n1,2\n"),
            ("data/notes/My Notes.md", b"# notes"),
            ("__MACOSX/data/._report.csv", b"junk"),
            ("data/.DS_Store", b"junk"),
        ]);

        let files = unpack_zip(&bytes).unwrap();

        assert!(looks_like_zip(&bytes));
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            ["data/notes/My_Notes.md", "data/report.csv"]
        );
        assert_eq!(files["data/report.csv"], b"a,b\n1,2\n");
    }

    #[test]
    fn rejects_paths_that_leave_the_folder() {
        for name in [
            "../evil.sh",
            "a/../../evil.sh",
            "/etc/passwd",
            "C:/evil.bat",
        ] {
            let bytes = zip_of(&[(name, b"x")]);
            assert!(unpack_zip(&bytes).is_err(), "{name}");
        }
    }

    #[test]
    fn rejects_a_zip_bomb_by_ratio() {
        let zeros = vec![0u8; 2 * RATIO_EXEMPT_BYTES];
        let bytes = zip_of(&[("zeros.bin", &zeros)]);

        let error = unpack_zip(&bytes).unwrap_err();

        assert!(error.contains("compressed more than"), "{error}");
    }

    #[test]
    fn rejects_too_many_files() {
        let names: Vec<String> = (0..=MAX_ZIP_FILES).map(|i| format!("f{i}.txt")).collect();
        let entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), &b"x"[..]))
            .collect();

        let error = unpack_zip(&zip_of(&entries)).unwrap_err();

        assert!(error.contains("more than 1000 files"), "{error}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(!looks_like_zip(b"%PDF-1.7"));
        assert!(unpack_zip(b"PK\x03\x04 not really").is_err());
    }
}
