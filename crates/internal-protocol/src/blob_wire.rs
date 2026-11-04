//! Wire format for worker uploads on the byte path beside gRPC.
//!
//! Decision: an upload is one HTTP request whose body is a single JSON line
//! naming the object, then its raw bytes. The bytes never become a protobuf
//! field or base64, and the name and metadata (an image prompt can be long)
//! travel in the body, where no header or URL size limit applies. Serialized
//! JSON never contains a raw newline, so the first `\n` ends the head.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Path a worker POSTs a new image to, with `?org_id=`.
pub const IMAGE_UPLOAD_PATH: &str = "/internal/blobs/images";

/// What the upload is, before its bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadHead {
    pub filename: String,
    pub content_type: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// The stored image, as the server answers an upload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UploadedImage {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Build an upload body: the head as one JSON line, then `data`.
pub fn encode_image_upload(head: &ImageUploadHead, data: &[u8]) -> serde_json::Result<Vec<u8>> {
    let head = serde_json::to_vec(head)?;
    let mut body = Vec::with_capacity(head.len() + 1 + data.len());
    body.extend_from_slice(&head);
    body.push(b'\n');
    body.extend_from_slice(data);
    Ok(body)
}

/// Split an upload body into its head and bytes; `None` when it is malformed.
pub fn decode_image_upload(body: &[u8]) -> Option<(ImageUploadHead, &[u8])> {
    let end = body.iter().position(|b| *b == b'\n')?;
    let head = serde_json::from_slice(&body[..end]).ok()?;
    Some((head, &body[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_round_trips_with_newlines_in_metadata_and_bytes() {
        let head = ImageUploadHead {
            filename: "cat.png".into(),
            content_type: "image/png".into(),
            metadata: serde_json::json!({"prompt": "a cat\non two lines"}),
        };
        let data = b"\x89PNG\n\r\n";
        let body = encode_image_upload(&head, data).unwrap();
        let (decoded, bytes) = decode_image_upload(&body).unwrap();
        assert_eq!(decoded, head);
        assert_eq!(bytes, data);
        assert!(decode_image_upload(b"no head line").is_none());
        assert!(decode_image_upload(b"{not json}\nbytes").is_none());
    }
}
