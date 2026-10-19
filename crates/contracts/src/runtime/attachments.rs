//! Where message attachments live in a session's file storage.
//!
//! Every attachment gets a stable path under `/attachments`, derived from its
//! id and file name, so the model can be told where to open it again after
//! compaction drops its bytes from the request. The host writes these files
//! read-only; the model can read them but not change them.

/// Root of the session file storage that holds message attachments.
pub const ATTACHMENTS_ROOT: &str = "/attachments";

/// Longest file name kept in an attachment path, in characters.
const MAX_FILE_NAME_CHARS: usize = 100;

/// Stable session path for an attachment: `/attachments/<id>/<file name>`.
///
/// `id` is the attachment's public id (`img_...`, `file_...`). The file name is
/// reduced to letters, digits, `.`, `-` and `_`, so it is safe in a path and in
/// a shell command; `fallback_name` is used when nothing is left.
pub fn attachment_path(id: &str, filename: Option<&str>, fallback_name: &str) -> String {
    let id = sanitize(id, usize::MAX);
    let name = filename
        .map(|name| {
            sanitize(
                name.rsplit(['/', '\\']).next().unwrap_or(name),
                MAX_FILE_NAME_CHARS,
            )
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    format!("{ATTACHMENTS_ROOT}/{id}/{name}")
}

/// Text label that introduces an attachment to the model.
///
/// Example: `Attached file /attachments/file_01.../report.pdf (application/pdf)`.
pub fn attachment_label(kind: &str, path: &str, media_type: Option<&str>) -> String {
    match media_type {
        Some(media_type) => format!("Attached {kind} {path} ({media_type})"),
        None => format!("Attached {kind} {path}"),
    }
}

/// Whether `text` is a label produced by [`attachment_label`].
pub fn is_attachment_label(text: &str) -> bool {
    ["Attached image ", "Attached file "].iter().any(|prefix| {
        text.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(ATTACHMENTS_ROOT))
    })
}

/// Media type of a zip attachment, which the host unpacks next to itself.
pub const ZIP_MEDIA_TYPE: &str = "application/zip";

/// Folder a zip attachment unpacks to: its path without `.zip`
/// (`/attachments/<id>/data.zip` → `/attachments/<id>/data`), or with
/// `_unpacked` appended when it has no `.zip` extension.
pub fn unpacked_dir(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    match lower.strip_suffix(".zip") {
        Some(stem) if !stem.ends_with('/') => path[..stem.len()].to_string(),
        _ => format!("{path}_unpacked"),
    }
}

/// One file or folder name made safe for a session path and a shell
/// command: letters, digits, `.`, `-` and `_`, no leading dot, at most 100
/// characters.
pub fn sanitize_file_name(name: &str) -> String {
    sanitize(name, MAX_FILE_NAME_CHARS)
}

fn sanitize(value: &str, max_chars: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(max_chars)
        .collect();
    cleaned.trim_start_matches('.').to_string()
}

/// Media type the bytes themselves show, for the types the model reads
/// natively or the host unpacks: PNG, JPEG, GIF, WebP, PDF and zip.
pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some(ZIP_MEDIA_TYPE)
    } else {
        None
    }
}

/// Media type to store an attachment under, checked against its bytes.
///
/// The bytes win when they show a known type. A declared image, PDF or zip
/// type the bytes contradict becomes `application/octet-stream`, so the model
/// is never sent bytes under a type they are not. Other declared types (a CSV,
/// a Word file) are kept: the model only gets their label.
pub fn verified_media_type(declared: Option<&str>, bytes: &[u8]) -> String {
    if let Some(sniffed) = sniff_media_type(bytes) {
        return sniffed.to_string();
    }
    let declared = declared
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty() && value.contains('/'));
    match declared {
        Some(value)
            if !(value.starts_with("image/")
                || value == "application/pdf"
                || value == ZIP_MEDIA_TYPE) =>
        {
            value
        }
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_unpacks_next_to_itself() {
        assert_eq!(
            unpacked_dir("/attachments/file_01/data.zip"),
            "/attachments/file_01/data"
        );
        assert_eq!(
            unpacked_dir("/attachments/file_01/DATA.ZIP"),
            "/attachments/file_01/DATA"
        );
        assert_eq!(
            unpacked_dir("/attachments/file_01/file"),
            "/attachments/file_01/file_unpacked"
        );
    }

    #[test]
    fn path_keeps_safe_file_names() {
        assert_eq!(
            attachment_path("file_01ab", Some("report-2026.pdf"), "file"),
            "/attachments/file_01ab/report-2026.pdf"
        );
    }

    #[test]
    fn path_cleans_unsafe_file_names() {
        assert_eq!(
            attachment_path("img_01", Some("../My Photo (1).png"), "image"),
            "/attachments/img_01/My_Photo__1_.png"
        );
        assert_eq!(
            attachment_path("img_01", Some("dir/sub/x;rm -rf.png"), "image"),
            "/attachments/img_01/x_rm_-rf.png"
        );
    }

    #[test]
    fn path_falls_back_when_no_name_is_left() {
        assert_eq!(
            attachment_path("img_01", None, "image"),
            "/attachments/img_01/image"
        );
        assert_eq!(
            attachment_path("img_01", Some("..."), "image"),
            "/attachments/img_01/image"
        );
    }

    #[test]
    fn path_bounds_long_names() {
        let long = format!("{}.pdf", "a".repeat(300));
        let path = attachment_path("file_01", Some(&long), "file");
        assert_eq!(path.len(), "/attachments/file_01/".len() + 100);
    }

    #[test]
    fn labels_round_trip() {
        let label = attachment_label(
            "file",
            "/attachments/file_01/a.pdf",
            Some("application/pdf"),
        );
        assert_eq!(
            label,
            "Attached file /attachments/file_01/a.pdf (application/pdf)"
        );
        assert!(is_attachment_label(&label));
        assert!(!is_attachment_label("Attached file somewhere else"));
    }

    #[test]
    fn media_type_comes_from_the_bytes() {
        assert_eq!(sniff_media_type(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(
            sniff_media_type(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_media_type(b"a,b\n"), None);
        assert_eq!(
            verified_media_type(Some("image/png"), b"%PDF-1.7"),
            "application/pdf"
        );
        assert_eq!(
            verified_media_type(Some("image/png"), b"not a png"),
            "application/octet-stream"
        );
        assert_eq!(
            verified_media_type(Some("text/csv; charset=utf-8"), b"a,b\n"),
            "text/csv"
        );
        assert_eq!(
            verified_media_type(None, b"a,b\n"),
            "application/octet-stream"
        );
    }
}
