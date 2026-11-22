//! Images and files in a v2 session's messages (data plane v2, step
//! 11.6): an inline image is kept in the session's files and the model
//! sees it; an upload the control plane copied there is seen too; a part
//! that names an attachment the session does not hold is refused, and a
//! kind the data plane does not take is named.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use base64::Engine as _;
use everruns_agentd::dp::{ConfigAck, ConfigUpdate};
use everruns_contracts::typed_id::{FileId, ImageId, MessageId, SessionId};
use reqwest::{Method, StatusCode};
use serde_json::json;
use support::org::{ORG, command, org_fleet};
use support::{Fleet, answers};

/// The first bytes of a PNG: enough for the media type to be sniffed.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_image_is_kept_in_the_sessions_files_and_seen() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("eyes").await;
    let path = format!("/v1/sessions/{session}/messages");

    let (status, _, sent) = fleet
        .post_once(
            &path,
            json!({ "message": { "role": "user", "content": [
                { "type": "text", "text": "what is this?" },
                { "type": "image", "base64": b64(PNG), "media_type": "image/png" },
            ]}}),
        )
        .await;
    assert_eq!(status, 201, "{sent}");
    assert_eq!(sent["content"][1]["type"], "image_file", "{sent}");
    fleet.turns(&session, 1).await;
    let said = answers(&fleet.events(&session).await);
    assert_eq!(said, vec!["Saw image data:image/png;base64."]);

    // An attachment the session does not hold is never named in a turn.
    let (status, _, refused) = fleet
        .post_once(
            &path,
            json!({ "message": { "role": "user", "content": [
                { "type": "image_file", "image_id": ImageId::new() },
            ]}}),
        )
        .await;
    assert_eq!(status, 409, "{refused}");
    // A kind the data plane does not take is named.
    let (status, _, refused) = fleet
        .post_once(
            &path,
            json!({ "message": { "role": "user", "content": [
                { "type": "input_audio", "url": "data:audio/wav;base64,AA==" },
            ]}}),
        )
        .await;
    assert_eq!(status, 501, "{refused}");
    assert_eq!(
        refused["feature"], "message content input_audio",
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_the_control_plane_copied_is_seen() {
    let (fleet, cp) = org_fleet(&[]).await;
    let package = everruns_agentd_testkit::definition("eyes").unwrap();
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &ConfigUpdate {
                epoch: 1,
                version: 2,
                base_version: 1,
                changes: vec![
                    json!({ "kind": "Agent", "agent_id": "eyes", "revision": 1, "package": package }),
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 2 });
    let id = SessionId::new();
    let (status, created) = cp
        .call(
            &fleet.base(),
            Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": id, "agent_ref": "eyes" }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let session = id.to_string();

    // The control plane copies the upload, then sends the message.
    let image = ImageId::new();
    let copy = json!({ "op": "seed", "files": [{
        "path": format!("/attachments/{image}/photo.png"),
        "content": b64(PNG),
        "encoding": "base64",
        "is_readonly": true,
    }]});
    let (status, copied) = command(&fleet, &cp, ORG, &session, "fs", copy).await;
    assert_eq!(status, StatusCode::OK, "{copied}");
    assert_eq!(copied["result"], "done", "{copied}");
    let message = json!({
        "id": MessageId::new(),
        "parts": [
            { "type": "text", "text": "and this?" },
            { "type": "image_file", "image_id": image, "filename": "photo.png" },
        ],
    });
    let (status, sent) = command(&fleet, &cp, ORG, &session, "messages", message).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    fleet.turns(&session, 1).await;
    let said = answers(&fleet.events(&session).await);
    assert_eq!(said, vec!["Saw image data:image/png;base64."]);

    // A file it was never given is refused.
    let missing = json!({
        "id": MessageId::new(),
        "parts": [{ "type": "file", "file_id": FileId::new(), "filename": "a.pdf" }],
    });
    let (status, refused) = command(&fleet, &cp, ORG, &session, "messages", missing).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
}
