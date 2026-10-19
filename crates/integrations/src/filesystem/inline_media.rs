//! Sniffs the formats `read_file` returns to the model as native content.

/// Detect the MIME type of a file `read_file` returns as native content: an
/// image format supported by model providers, or a PDF document.
pub(super) fn inline_media_type(content: &str) -> Option<&'static str> {
    use base64::Engine as _;

    // Decode only the prefix needed by supported formats. This avoids trusting an
    // attacker-controlled extension and avoids decoding large payloads twice.
    let encoded = content.as_bytes();
    let prefix_len = encoded.len().min(16);
    let prefix_len = prefix_len - (prefix_len % 4);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded[..prefix_len])
        .ok()?;

    match bytes.as_slice() {
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        // Attachments are reopened by path after compaction drops their
        // bytes (knowledge/runtime-resources/session-attachments.md).
        [b'%', b'P', b'D', b'F', b'-', ..] => Some("application/pdf"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::inline_media_type;

    fn encoded_prefix(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn supported_formats() {
        for (bytes, expected) in [
            (&b"\x89PNG\r\n\x1a\nrest"[..], "image/png"),
            (b"\xff\xd8\xff\xe0rest", "image/jpeg"),
            (b"GIF87arest", "image/gif"),
            (b"GIF89arest", "image/gif"),
            (b"RIFF\x04\x00\x00\x00WEBPrest", "image/webp"),
            (b"%PDF-1.7\n%rest", "application/pdf"),
        ] {
            assert_eq!(inline_media_type(&encoded_prefix(bytes)), Some(expected));
        }
    }

    #[test]
    fn rejects_other_files_and_invalid_base64() {
        assert_eq!(inline_media_type(&encoded_prefix(b"not an image")), None);
        assert_eq!(inline_media_type(&encoded_prefix(b"%PDX-1.7 rest")), None);
        assert_eq!(inline_media_type("not base64!"), None);
        assert_eq!(inline_media_type(""), None);
    }
}
