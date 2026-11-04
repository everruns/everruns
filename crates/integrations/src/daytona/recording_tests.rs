use super::*;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SANDBOX: &str = "sbx-1";

fn route(suffix: &str) -> String {
    format!("/toolbox/{SANDBOX}/computeruse{suffix}")
}

fn client(server: &MockServer) -> DaytonaClient {
    DaytonaClient::with_base_urls(
        "key".to_string(),
        server.uri(),
        format!("{}/toolbox", server.uri()),
    )
}

fn recording(id: &str, status: &str, end_time: Option<&str>) -> Value {
    let mut entry = json!({
        "id": id,
        "fileName": format!("{id}.mp4"),
        "filePath": format!("/home/daytona/.recordings/{id}.mp4"),
        "status": status,
        "startTime": "2026-10-26T19:50:00Z",
        "durationSeconds": 12.5,
        "sizeBytes": 3,
    });
    if let Some(end) = end_time {
        entry["endTime"] = json!(end);
    }
    entry
}

async fn list_returns(server: &MockServer, recordings: Value) {
    Mock::given(method("GET"))
        .and(path(route("/recordings")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "recordings": recordings })))
        .mount(server)
        .await;
}

#[test]
fn a_recording_without_an_end_time_is_running() {
    let running: Recording = serde_json::from_value(recording("a", "recording", None)).unwrap();
    assert!(running.is_active());
    let done: Recording =
        serde_json::from_value(recording("b", "completed", Some("2026-10-26T19:51:00Z"))).unwrap();
    assert!(!done.is_active());
    let failed: Recording = serde_json::from_value(recording("c", "failed", None)).unwrap();
    assert!(!failed.is_active());
}

#[test]
fn stored_names_sort_by_start_time_and_keep_the_extension() {
    let mut entry: Recording =
        serde_json::from_value(recording("a", "completed", Some("x"))).unwrap();
    assert_eq!(entry.stored_name(), "desktop-2026-10-26T19-50-00Z.mp4");
    entry.file_name = "../../etc/passwd".to_string();
    entry.start_time = "2026/10/26 19:50".to_string();
    assert_eq!(entry.stored_name(), "desktop-2026102619-50.mp4");
}

#[tokio::test]
async fn starts_a_recording_only_when_none_runs() {
    let server = MockServer::start().await;
    list_returns(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path(route("/recordings/start")))
        .and(body_json(json!({"label": "everruns"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(recording("a", "recording", None)))
        .expect(1)
        .mount(&server)
        .await;
    ensure_started(&client(&server), SANDBOX).await.unwrap();

    let busy = MockServer::start().await;
    list_returns(&busy, json!([recording("a", "recording", None)])).await;
    Mock::given(method("POST"))
        .and(path(route("/recordings/start")))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&busy)
        .await;
    ensure_started(&client(&busy), SANDBOX).await.unwrap();
}

/// Read a collected recording's whole body.
async fn read_body(recording: &mut DesktopRecording) -> Vec<u8> {
    use futures::TryStreamExt;
    recording
        .body
        .take()
        .expect("a body")
        .try_concat()
        .await
        .unwrap()
}

#[tokio::test]
async fn collect_stops_streams_and_deletes_only_when_told() {
    let server = MockServer::start().await;
    list_returns(
        &server,
        json!([
            recording("live", "recording", None),
            recording("broken", "failed", Some("2026-10-26T19:50:01Z")),
        ]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(route("/recordings/stop")))
        .and(body_json(json!({"id": "live"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(recording(
            "live",
            "completed",
            Some("2026-10-26T19:50:12Z"),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/toolbox/{SANDBOX}/files/download")))
        .and(query_param("path", "/home/daytona/.recordings/live.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3]))
        .expect(1)
        .mount(&server)
        .await;
    for id in ["live", "broken"] {
        Mock::given(method("DELETE"))
            .and(path(route(&format!("/recordings/{id}"))))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .named(id)
            .mount(&server)
            .await;
    }

    let mut collected = collect(Arc::new(client(&server)), SANDBOX).await.unwrap();
    assert_eq!(collected.len(), 1);
    let mut live = collected.remove(0);
    assert_eq!(live.file_name, "desktop-2026-10-26T19-50-00Z.mp4");
    assert_eq!(live.duration_seconds, Some(12.5));
    assert_eq!(live.size_bytes, 3);
    assert_eq!(read_body(&mut live).await, vec![1, 2, 3]);

    // Only the failed recording is gone so far; the stored one waits for
    // its owner to finish.
    let deletes = |requests: Vec<wiremock::Request>| {
        requests
            .into_iter()
            .filter(|r| r.method == wiremock::http::Method::DELETE)
            .map(|r| r.url.path().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        deletes(server.received_requests().await.unwrap()),
        [route("/recordings/broken")]
    );
    live.remove.await.unwrap();
    assert_eq!(
        deletes(server.received_requests().await.unwrap()),
        [route("/recordings/broken"), route("/recordings/live")]
    );
}

#[tokio::test]
async fn an_oversized_recording_is_reported_without_its_video() {
    let server = MockServer::start().await;
    let mut big = recording("big", "completed", Some("2026-10-26T19:59:00Z"));
    big["sizeBytes"] = json!(MAX_RECORDING_BYTES + 1);
    list_returns(&server, json!([big])).await;
    Mock::given(method("GET"))
        .and(path(format!("/toolbox/{SANDBOX}/files/download")))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(route("/recordings/big")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let collected = collect(Arc::new(client(&server)), SANDBOX).await.unwrap();
    assert_eq!(collected.len(), 1);
    let big = collected.into_iter().next().unwrap();
    assert!(big.body.is_none());
    assert_eq!(big.size_bytes, MAX_RECORDING_BYTES + 1);
    big.remove.await.unwrap();
}
