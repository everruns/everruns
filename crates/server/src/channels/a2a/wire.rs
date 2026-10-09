// A2A wire versions: negotiation and the 1.0 rendering of protocol objects.
//
// Design Decision: the handlers build every protocol object once, in the A2A
// 0.3 JSON shape (`kind` discriminators, lowercase-hyphen task states), and
// this module renders it for the version the caller negotiated. The 1.0
// rendering is a pure, total mapping of that shape (A2A spec §5.5 ProtoJSON
// enums, Appendix A.2.1 "kind discriminator removed", `final` removed from
// `TaskStatusUpdateEvent`, `SendMessageResponse` / `StreamResponse`
// wrappers), so the two versions cannot drift in what they say, only in how.
//
// Negotiation follows spec §3.6.2: the `A2A-Version` service parameter (header,
// or the query parameter of the same name) picks the version by Major.Minor; an
// empty value MUST mean 0.3; anything else is `VersionNotSupportedError`
// (-32009). One extension: a request with no version but a 1.0 PascalCase
// method name (`SendMessage`, ...) is answered in 1.0, because 0.3 never had
// those names and the clients that send them (a2a-rs, a2a-go 2.x) parse 1.0.
// See `knowledge/integrations/a2a-channel.md`.

use axum::http::{HeaderMap, Uri};
use serde_json::{Map, Value, json};

/// The A2A service parameter carrying the client's protocol version.
pub(super) const A2A_VERSION_PARAM: &str = "A2A-Version";
/// Versions this endpoint speaks, newest first. Advertised in the Agent Card
/// and in `VersionNotSupportedError`.
pub(super) const SUPPORTED_VERSIONS: [&str; 2] = ["1.0", "0.3"];
/// A2A `VersionNotSupportedError` (spec §5.4).
pub(super) const VERSION_NOT_SUPPORTED: i32 = -32009;

/// The A2A protocol binding a request arrived on (spec §5). Both run the same
/// handlers; only the envelope around requests, results and errors differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Binding {
    JsonRpc,
    /// HTTP+JSON (spec §11), served for A2A 1.0 only.
    HttpJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WireVersion {
    V0_3,
    V1_0,
}

/// Resolve the protocol version for a request. `Err` carries the version the
/// client asked for when this endpoint does not speak it.
pub(super) fn negotiate(
    headers: &HeaderMap,
    uri: &Uri,
    method: &str,
) -> Result<WireVersion, String> {
    let requested = headers
        .get(A2A_VERSION_PARAM)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| query_param(uri, A2A_VERSION_PARAM));
    let requested = requested.as_deref().map(str::trim).unwrap_or_default();
    if requested.is_empty() {
        return Ok(if is_v1_method_name(method) {
            WireVersion::V1_0
        } else {
            WireVersion::V0_3
        });
    }
    // Patch versions never affect compatibility (spec §3.6), so compare on
    // Major.Minor only. A bare major ("1") is read as its first minor.
    let mut segments = requested.split('.');
    let major = segments.next().unwrap_or_default();
    let minor = segments.next().unwrap_or("0");
    match (major, minor) {
        ("1", "0") => Ok(WireVersion::V1_0),
        ("0", "3") => Ok(WireVersion::V0_3),
        _ => Err(requested.to_string()),
    }
}

fn query_param(uri: &Uri, name: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        key.eq_ignore_ascii_case(name).then(|| value.to_string())
    })
}

fn is_v1_method_name(method: &str) -> bool {
    method
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        && !method.contains('/')
}

/// The `error.data` detail for a `VersionNotSupportedError`.
pub(super) fn version_not_supported_message(requested: &str) -> String {
    format!(
        "A2A protocol version {requested} is not supported (supported: {})",
        SUPPORTED_VERSIONS.join(", ")
    )
}

/// Render a `Task` (internal 0.3 shape) for the wire.
pub(super) fn task(version: WireVersion, task: Value) -> Value {
    match version {
        WireVersion::V0_3 => task,
        WireVersion::V1_0 => v1_task(task),
    }
}

/// Render the `SendMessage` result: a bare `Task` in 0.3, a
/// `SendMessageResponse` wrapper in 1.0.
pub(super) fn send_message_result(version: WireVersion, task: Value) -> Value {
    match version {
        WireVersion::V0_3 => task,
        WireVersion::V1_0 => json!({ "task": v1_task(task) }),
    }
}

/// Render one streaming frame: a `kind`-tagged object in 0.3, a
/// `StreamResponse` wrapper in 1.0.
pub(super) fn stream_frame(version: WireVersion, frame: Value) -> Value {
    if version == WireVersion::V0_3 {
        return frame;
    }
    match frame.get("kind").and_then(Value::as_str) {
        Some("task") => json!({ "task": v1_task(frame) }),
        Some("message") => json!({ "message": v1_message(frame) }),
        Some("status-update") => {
            let mut update = strip_kind(frame);
            update.remove("final");
            if let Some(status) = update.remove("status") {
                update.insert("status".to_string(), v1_status(status));
            }
            json!({ "statusUpdate": Value::Object(update) })
        }
        Some("artifact-update") => {
            let mut update = strip_kind(frame);
            if let Some(artifact) = update.remove("artifact") {
                update.insert("artifact".to_string(), v1_artifact(artifact));
            }
            json!({ "artifactUpdate": Value::Object(update) })
        }
        _ => frame,
    }
}

/// 0.3 task state → 1.0 `TaskState` enum name. Also accepts the underscore
/// spellings older Everruns builds emitted.
pub(super) fn v1_task_state(state: &str) -> &'static str {
    match state {
        "submitted" => "TASK_STATE_SUBMITTED",
        "working" => "TASK_STATE_WORKING",
        "completed" => "TASK_STATE_COMPLETED",
        "failed" => "TASK_STATE_FAILED",
        "canceled" => "TASK_STATE_CANCELED",
        "input-required" | "input_required" => "TASK_STATE_INPUT_REQUIRED",
        "rejected" => "TASK_STATE_REJECTED",
        "auth-required" | "auth_required" => "TASK_STATE_AUTH_REQUIRED",
        _ => "TASK_STATE_UNSPECIFIED",
    }
}

fn strip_kind(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(mut map) => {
            map.remove("kind");
            map
        }
        _ => Map::new(),
    }
}

fn v1_task(task: Value) -> Value {
    let mut task = strip_kind(task);
    if let Some(status) = task.remove("status") {
        task.insert("status".to_string(), v1_status(status));
    }
    if let Some(Value::Array(artifacts)) = task.remove("artifacts") {
        task.insert(
            "artifacts".to_string(),
            Value::Array(artifacts.into_iter().map(v1_artifact).collect()),
        );
    }
    if let Some(Value::Array(history)) = task.remove("history") {
        task.insert(
            "history".to_string(),
            Value::Array(history.into_iter().map(v1_message).collect()),
        );
    }
    Value::Object(task)
}

fn v1_status(status: Value) -> Value {
    let Value::Object(mut status) = status else {
        return status;
    };
    if let Some(state) = status.get("state").and_then(Value::as_str) {
        let state = v1_task_state(state);
        status.insert("state".to_string(), Value::String(state.to_string()));
    }
    if let Some(message) = status.remove("message") {
        status.insert("message".to_string(), v1_message(message));
    }
    Value::Object(status)
}

fn v1_message(message: Value) -> Value {
    let mut message = strip_kind(message);
    if let Some(role) = message.get("role").and_then(Value::as_str) {
        let role = match role {
            "user" => "ROLE_USER",
            "agent" => "ROLE_AGENT",
            other => other,
        }
        .to_string();
        message.insert("role".to_string(), Value::String(role));
    }
    v1_parts_in(&mut message);
    Value::Object(message)
}

fn v1_artifact(artifact: Value) -> Value {
    let Value::Object(mut artifact) = artifact else {
        return artifact;
    };
    v1_parts_in(&mut artifact);
    Value::Object(artifact)
}

fn v1_parts_in(object: &mut Map<String, Value>) {
    if let Some(Value::Array(parts)) = object.remove("parts") {
        object.insert(
            "parts".to_string(),
            Value::Array(parts.into_iter().map(v1_part).collect()),
        );
    }
}

/// 1.0 `Part` is one flat object whose content is exactly one of `text`,
/// `raw`, `url`, or `data` (spec Appendix A.2.1).
fn v1_part(part: Value) -> Value {
    let mut part = strip_kind(part);
    if let Some(Value::Object(file)) = part.remove("file") {
        if let Some(uri) = file.get("uri") {
            part.insert("url".to_string(), uri.clone());
        }
        if let Some(bytes) = file.get("bytes") {
            part.insert("raw".to_string(), bytes.clone());
        }
        if let Some(mime) = file.get("mimeType") {
            part.insert("mediaType".to_string(), mime.clone());
        }
        if let Some(name) = file.get("name") {
            part.insert("filename".to_string(), name.clone());
        }
    }
    if part.contains_key("data") && !part.contains_key("mediaType") {
        part.insert(
            "mediaType".to_string(),
            Value::String("application/json".to_string()),
        );
    }
    Value::Object(part)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(version: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(version) = version {
            headers.insert(A2A_VERSION_PARAM, version.parse().unwrap());
        }
        headers
    }

    fn uri(path: &str) -> Uri {
        path.parse().unwrap()
    }

    #[test]
    fn empty_version_means_0_3_for_slash_methods() {
        assert_eq!(
            negotiate(&headers(None), &uri("/a2a"), "message/send"),
            Ok(WireVersion::V0_3)
        );
    }

    #[test]
    fn empty_version_with_pascal_case_method_means_1_0() {
        assert_eq!(
            negotiate(&headers(None), &uri("/a2a"), "SendMessage"),
            Ok(WireVersion::V1_0)
        );
    }

    #[test]
    fn header_picks_version_by_major_minor() {
        for (value, expected) in [
            ("1.0", WireVersion::V1_0),
            ("1.0.1", WireVersion::V1_0),
            ("1", WireVersion::V1_0),
            ("0.3", WireVersion::V0_3),
            ("0.3.0", WireVersion::V0_3),
        ] {
            assert_eq!(
                negotiate(&headers(Some(value)), &uri("/a2a"), "SendMessage"),
                Ok(expected),
                "{value}"
            );
        }
    }

    #[test]
    fn header_wins_over_method_style() {
        assert_eq!(
            negotiate(&headers(Some("0.3")), &uri("/a2a"), "SendMessage"),
            Ok(WireVersion::V0_3)
        );
        assert_eq!(
            negotiate(&headers(Some("1.0")), &uri("/a2a"), "message/send"),
            Ok(WireVersion::V1_0)
        );
    }

    #[test]
    fn query_parameter_is_read_when_header_absent() {
        assert_eq!(
            negotiate(&headers(None), &uri("/a2a?A2A-Version=1.0"), "message/send"),
            Ok(WireVersion::V1_0)
        );
    }

    #[test]
    fn unknown_version_is_rejected_with_the_requested_value() {
        assert_eq!(
            negotiate(&headers(Some("0.5")), &uri("/a2a"), "SendMessage"),
            Err("0.5".to_string())
        );
        assert_eq!(
            negotiate(&headers(Some("2.0")), &uri("/a2a"), "SendMessage"),
            Err("2.0".to_string())
        );
    }

    fn sample_task() -> Value {
        json!({
            "id": "t1",
            "contextId": "t1",
            "kind": "task",
            "status": {
                "state": "input-required",
                "message": {
                    "kind": "message",
                    "role": "agent",
                    "messageId": "m1",
                    "parts": [
                        { "kind": "text", "text": "which one?" },
                        { "kind": "data", "data": { "a": 1 } }
                    ]
                }
            },
            "artifacts": [{
                "artifactId": "response",
                "parts": [{ "kind": "text", "text": "hi" }]
            }]
        })
    }

    #[test]
    fn v0_3_rendering_is_identity() {
        assert_eq!(task(WireVersion::V0_3, sample_task()), sample_task());
        assert_eq!(
            send_message_result(WireVersion::V0_3, sample_task()),
            sample_task()
        );
    }

    #[test]
    fn v1_task_drops_kind_and_uses_protojson_enums() {
        let rendered = task(WireVersion::V1_0, sample_task());
        assert!(rendered.get("kind").is_none());
        assert_eq!(rendered["status"]["state"], "TASK_STATE_INPUT_REQUIRED");
        let message = &rendered["status"]["message"];
        assert!(message.get("kind").is_none());
        assert_eq!(message["role"], "ROLE_AGENT");
        assert_eq!(message["parts"][0], json!({ "text": "which one?" }));
        assert_eq!(
            message["parts"][1],
            json!({ "data": { "a": 1 }, "mediaType": "application/json" })
        );
        assert_eq!(
            rendered["artifacts"][0]["parts"][0],
            json!({ "text": "hi" })
        );
    }

    #[test]
    fn v1_task_parses_as_the_reference_rust_sdk_task() {
        let rendered = task(WireVersion::V1_0, sample_task());
        let parsed: a2a::Task = serde_json::from_value(rendered).expect("a2a::Task");
        assert_eq!(parsed.status.state, a2a::TaskState::InputRequired);
    }

    #[test]
    fn v1_send_message_result_wraps_the_task() {
        let rendered = send_message_result(WireVersion::V1_0, sample_task());
        let parsed: a2a::SendMessageResponse =
            serde_json::from_value(rendered).expect("a2a::SendMessageResponse");
        assert!(matches!(parsed, a2a::SendMessageResponse::Task(_)));
    }

    #[test]
    fn v1_stream_frames_are_stream_responses() {
        let frames = [
            json!({ "kind": "task", "id": "t1", "contextId": "t1", "status": { "state": "working" } }),
            json!({
                "kind": "artifact-update", "taskId": "t1", "contextId": "t1",
                "artifact": { "artifactId": "a1", "parts": [{ "kind": "text", "text": "x" }] }
            }),
            json!({
                "kind": "status-update", "taskId": "t1", "contextId": "t1",
                "status": { "state": "completed" }, "final": true
            }),
        ];
        for frame in frames {
            let rendered = stream_frame(WireVersion::V1_0, frame);
            assert!(
                rendered.to_string().find("\"kind\"").is_none(),
                "{rendered}"
            );
            assert!(
                rendered.to_string().find("\"final\"").is_none(),
                "{rendered}"
            );
            serde_json::from_value::<a2a::StreamResponse>(rendered.clone())
                .unwrap_or_else(|err| panic!("{rendered}: {err}"));
        }
    }

    #[test]
    fn v1_file_part_flattens_to_url_and_media_type() {
        let part = v1_part(json!({
            "kind": "file",
            "file": { "uri": "https://example.test/x.png", "mimeType": "image/png", "name": "x.png" }
        }));
        assert_eq!(
            part,
            json!({ "url": "https://example.test/x.png", "mediaType": "image/png", "filename": "x.png" })
        );
    }
}
