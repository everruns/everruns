use super::*;
use crate::daytona::api::management::types::CreateSandbox;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn missing_cwd_prevents_every_line_of_the_command() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let marker = root.path().join("must-not-run");
    for command in [
        format!("true\nprintf unsafe > '{}'", marker.display()),
        format!("true )\nprintf unsafe > '{}'\n( true", marker.display()),
    ] {
        let generated = format_exec_command(&command, Some(missing.to_str().unwrap()));
        let wrapper = generated.strip_prefix(SHELL_PROFILE_PREAMBLE).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(wrapper)
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(
            !marker.exists(),
            "a later line ran despite the rejected cwd"
        );
        assert!(!output.status.success());
    }
}

#[test]
fn trailing_comment_does_not_hide_the_shell_wrapper() {
    let root = tempfile::tempdir().unwrap();
    for cwd in [None, root.path().to_str()] {
        let generated = format_exec_command("printf 'ok\\n' # comment", cwd);
        let wrapper = generated.strip_prefix(SHELL_PROFILE_PREAMBLE).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(wrapper)
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"ok\n");
    }
}

#[tokio::test]
async fn test_client_create_sandbox() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::body_partial_json(
            json!({"name":"Test"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_test123",
            "name": "Test Sandbox",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .create_sandbox(CreateSandbox {
            name: Some("Test".to_string()),
            ..CreateSandbox::default()
        })
        .await;
    assert!(result.is_ok());
    let info = result.unwrap();
    assert_eq!(info.id, "sb_test123");
    assert_eq!(info.state, "started");
    assert_eq!(info.name, Some("Test Sandbox".to_string()));
}

#[tokio::test]
async fn test_client_get_sandbox() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_test"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_test",
            "name": "Test",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.get_sandbox("sb_test").await;
    assert!(result.is_ok());
    let info = result.unwrap();
    assert_eq!(info.state, "started");
    assert_eq!(info.id, "sb_test");
    assert_eq!(info.name.as_deref(), Some("Test"));
}

#[tokio::test]
async fn test_client_creates_volume() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/volumes"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::body_json(
            json!({"name":"everruns-recovery"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vol_recovery",
            "name": "everruns-recovery",
            "state": "pending_create"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let volume = client.create_volume("everruns-recovery").await.unwrap();

    assert_eq!(volume.id, "vol_recovery");
    assert_eq!(volume.name, "everruns-recovery");
    assert_eq!(volume.state, "pending_create");
}

#[tokio::test]
async fn test_client_gets_volume_by_encoded_name() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/volumes/by-name/everruns%20recovery"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vol_recovery",
            "name": "everruns recovery",
            "state": "ready"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let volume = client
        .get_volume_by_name("everruns recovery")
        .await
        .unwrap();

    assert_eq!(volume.id, "vol_recovery");
    assert_eq!(volume.state, "ready");
}

#[tokio::test]
async fn test_client_stop_sandbox() {
    for response in ["", "{}"] {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/stop"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(response))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.stop_sandbox("sb_test").await;
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_client_delete_sandbox() {
    let mock_server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_test"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.delete_sandbox("sb_test").await;
    assert!(result.is_ok());
}

/// Set up mock responses for the session-based exec flow.
async fn setup_session_mocks(
    mock_server: &MockServer,
    sandbox_id: &str,
    exit_code: i64,
    output: &str,
) {
    // 1. Create session → 201 (or 409 if exists)
    Mock::given(method("POST"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(path(format!("/{sandbox_id}/process/session")))
        .and(wiremock::matchers::body_json(
            json!({"sessionId":"everruns-exec"}),
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(mock_server)
        .await;

    // 2. Async exec → 202 with cmdId
    Mock::given(method("POST"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/exec"
        )))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "cmdId": "cmd_001"
        })))
        .mount(mock_server)
        .await;

    // 3. Logs → output bytes with stdout marker prefix
    let mut log_bytes = vec![0x01, 0x01, 0x01];
    log_bytes.extend_from_slice(output.as_bytes());
    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(mock_server)
        .await;

    // 4. Command status → exitCode
    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_001",
            "command": "test",
            "exitCode": exit_code
        })))
        .mount(mock_server)
        .await;
}

#[tokio::test(start_paused = true)]
async fn test_client_exec_detects_dead_session() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let mock_server = MockServer::builder().start().await;
    let exec_call_count = std::sync::Arc::new(AtomicU32::new(0));

    // 1. Create session → 201
    Mock::given(method("POST"))
        .and(path("/sb_dead/process/session"))
        .and(wiremock::matchers::body_json(
            json!({"sessionId":"everruns-exec"}),
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(2)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/sb_dead/process/session"))
        .and(wiremock::matchers::body_json(
            json!({"sessionId":"everruns-heartbeat"}),
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 2. Exec calls to the main session → 202 with unique cmdId.
    let counter = exec_call_count.clone();
    Mock::given(method("POST"))
        .and(path("/sb_dead/process/session/everruns-exec/exec"))
        .respond_with(move |_: &wiremock::Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(202).set_body_json(json!({
                "cmdId": format!("cmd_{n}")
            }))
        })
        .mount(&mock_server)
        .await;

    // 2b. Heartbeat session exec calls → 202 with unique cmdId.
    //     The heartbeat uses a separate session (everruns-heartbeat).
    //     In a dead-sandbox scenario, this session also can't complete.
    let hb_counter = std::sync::Arc::new(AtomicU32::new(100));
    let hb_counter_clone = hb_counter.clone();
    Mock::given(method("POST"))
        .and(path("/sb_dead/process/session/everruns-heartbeat/exec"))
        .respond_with(move |_: &wiremock::Request| {
            let n = hb_counter_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(202).set_body_json(json!({
                "cmdId": format!("hb_{n}")
            }))
        })
        .mount(&mock_server)
        .await;

    // 3. All command logs → 200 with empty body (dead shell produces nothing)
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"/sb_dead/process/session/everruns-exec/command/cmd_\d+/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
        .mount(&mock_server)
        .await;

    // 4. All command status → 200 with no exitCode (dead shell never reports)
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"/sb_dead/process/session/everruns-exec/command/cmd_\d+$",
        ))
        .respond_with(|req: &wiremock::Request| {
            let path = req.url.path();
            let cmd_id = path.rsplit('/').next().unwrap_or("unknown");
            ResponseTemplate::new(200).set_body_json(json!({
                "id": cmd_id,
                "command": "exit 1"
            }))
        })
        .mount(&mock_server)
        .await;

    // 4b. Heartbeat command status → 200 with no exitCode (sandbox is dead)
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"/sb_dead/process/session/everruns-heartbeat/command/hb_\d+$",
        ))
        .respond_with(|req: &wiremock::Request| {
            let path = req.url.path();
            let cmd_id = path.rsplit('/').next().unwrap_or("unknown");
            ResponseTemplate::new(200).set_body_json(json!({
                "id": cmd_id,
                "command": "true"
            }))
        })
        .mount(&mock_server)
        .await;

    // 5. DELETE session → 204
    Mock::given(method("DELETE"))
        .and(path("/sb_dead/process/session/everruns-exec"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let mut client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    // Idle-pool eviction is unrelated to polling and would auto-advance a paused clock during HTTP I/O.
    client.http = reqwest::Client::builder()
        .pool_idle_timeout(None)
        .build()
        .unwrap();
    // Timeout must exceed stale detection window + heartbeat so the
    // dead-session path fires before the exec timeout path.
    let detection_window_ms = (crate::daytona::SESSION_STALE_THRESHOLD as u64)
        * crate::daytona::EXEC_POLL_INTERVAL.as_millis() as u64
        + crate::daytona::SESSION_HEARTBEAT_TIMEOUT_MS
        + 5_000; // buffer
    let start = tokio::time::Instant::now();
    let result = client
        .exec("sb_dead", "exit 1", None, Some(detection_window_ms), |_| {})
        .await;
    assert_eq!(start.elapsed(), Duration::from_secs(45));
    assert!(result.is_err(), "Should detect dead session");
    let err = result.unwrap_err();
    assert!(
        err.contains("Session shell terminated unexpectedly"),
        "Error should mention session termination, got: {err}"
    );

    // Verify the heartbeat was attempted on the dedicated heartbeat session
    // (separate from the exec session). exec_call_count only tracks the
    // exec session; hb_counter tracks the heartbeat session.
    assert_eq!(
        exec_call_count.load(Ordering::SeqCst),
        1,
        "Expected exactly 1 exec call (the original command)"
    );
    assert_eq!(hb_counter.load(Ordering::SeqCst), 101);
    let requests = mock_server.received_requests().await.unwrap();
    for (suffix, expected) in [
        ("/everruns-exec/command/cmd_0", 30),
        ("/everruns-heartbeat/command/hb_100", 15),
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path().ends_with(suffix))
                .count(),
            expected,
            "{suffix}"
        );
    }
}

#[tokio::test]
async fn test_client_file_list() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sb_test/files"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::query_param("path", "/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "main.py", "isDir": false, "size": 42},
            {"name": "src", "isDir": true, "size": 0}
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.file_list("sb_test", "/sandbox").await;
    assert!(result.is_ok());
    let entries = result.unwrap();
    assert_eq!(
        entries,
        vec![
            json!({"name":"main.py","isDir":false,"size":42}),
            json!({"name":"src","isDir":true,"size":0})
        ]
    );
}

#[tokio::test]
async fn test_client_404_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandbox/nonexistent"))
        .respond_with(
            ResponseTemplate::new(404).set_body_string("{\"error\":\"Sandbox not found\"}"),
        )
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.get_sandbox("nonexistent").await;
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Daytona API error (404 Not Found): {\"error\":\"Sandbox not found\"}"
    );
}

#[tokio::test]
async fn test_client_500_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_test/start"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.start_sandbox("sb_test").await;
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Daytona API error (500 Internal Server Error): Internal Server Error"
    );
}

#[tokio::test]
async fn test_client_malformed_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .create_sandbox(CreateSandbox {
            name: Some("Test".to_string()),
            ..CreateSandbox::default()
        })
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .starts_with("Invalid JSON from Daytona:")
    );
}

#[tokio::test]
async fn test_client_start_sandbox() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_test/start"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.start_sandbox("sb_test").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_set_autostop() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_test/autostop/5"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.set_autostop("sb_test", 5).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_file_download() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sb_test/files/download"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::query_param(
            "path",
            "/my project/file &?#.bin",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0, 255, 128, 10, 65]))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .file_download("sb_test", "/my project/file &?#.bin")
        .await;
    assert!(result.is_ok());
    let bytes = result.unwrap();
    assert_eq!(bytes, vec![0, 255, 128, 10, 65]);
}

#[tokio::test]
async fn test_client_file_delete() {
    let mock_server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/sb_test/files"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::query_param("path", "/old &?#.txt"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.file_delete("sb_test", "/old &?#.txt").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_create_folder() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sb_test/files/folder"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::query_param("path", "/src &dir"))
        .and(wiremock::matchers::query_param("mode", "0750"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.create_folder("sb_test", "/src &dir", "0750").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_file_upload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sb_test/files/upload-v2"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer test_key",
        ))
        .and(wiremock::matchers::query_param(
            "path",
            "/sandbox/test &data.bin",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .file_upload("sb_test", "/sandbox/test &data.bin", &[0, 255, 128, 10, 65])
        .await;
    result.unwrap();
    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(
        request
            .body
            .windows(5)
            .any(|bytes| bytes == [0, 255, 128, 10, 65])
    );
    let envelope = String::from_utf8_lossy(&request.body);
    assert!(envelope.contains("name=\"file\"; filename=\"test &data.bin\""));
    assert!(envelope.contains("application/octet-stream"));
}

#[tokio::test(start_paused = true)]
async fn readiness_timeout_preserves_backoff_without_wall_clock_waiting() {
    let start = tokio::time::Instant::now();
    let mut polls = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(63),
        poll_sandbox_ready(|| {
            polls.push(start.elapsed().as_secs());
            std::future::ready(Ok("creating".to_owned()))
        }),
    )
    .await
    .expect("readiness loop must enforce its own timeout");
    assert_eq!(
        result,
        Err("Sandbox did not become ready within 60s".into())
    );
    assert_eq!(
        polls,
        vec![0, 2, 6, 10, 14, 18, 22, 26, 30, 34, 38, 42, 46, 50, 54, 58]
    );
    assert_eq!(start.elapsed(), Duration::from_secs(62));
}

#[tokio::test(start_paused = true)]
async fn readiness_retries_transient_errors_and_stops_when_started() {
    let start = tokio::time::Instant::now();
    let mut polls = Vec::new();
    let mut states = std::collections::VecDeque::from([
        Ok("creating".to_owned()),
        Err("temporary transport failure".to_owned()),
        Ok("started".to_owned()),
    ]);
    let result = poll_sandbox_ready(|| {
        polls.push(start.elapsed().as_secs());
        std::future::ready(states.pop_front().expect("must stop polling after started"))
    })
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(polls, vec![0, 2, 6]);
}

#[tokio::test(start_paused = true)]
async fn readiness_terminal_states_do_not_sleep_or_retry() {
    for (state, expected) in [
        ("started", Ok(())),
        (
            "error",
            Err("Sandbox entered error state: error".to_owned()),
        ),
        (
            "build_failed",
            Err("Sandbox entered error state: build_failed".to_owned()),
        ),
    ] {
        let start = tokio::time::Instant::now();
        let mut count = 0;
        let result = poll_sandbox_ready(|| {
            count += 1;
            assert_eq!(count, 1, "terminal state must stop polling");
            std::future::ready(Ok(state.to_owned()))
        })
        .await;
        assert_eq!(result, expected);
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}

#[tokio::test]
async fn test_client_wait_for_ready_error_state() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_test",
            "state": "error"
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.wait_for_ready("sb_test").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("error state"));
}

#[tokio::test]
async fn test_client_file_download_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sb_test/files/download"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.file_download("sb_test", "/missing.txt").await;
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Sandbox API error (404 Not Found): Not Found"
    );
}

#[tokio::test]
async fn test_client_file_upload_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sb_test/files/upload-v2"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Disk full"))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.file_upload("sb_test", "/test.txt", b"content").await;
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "File upload error (500 Internal Server Error): Disk full"
    );
}

#[tokio::test]
async fn test_exec_streaming_collects_output_and_exit_code() {
    use std::sync::{Arc, Mutex};

    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_test", 0, "hello streaming\n").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let chunks: Arc<Mutex<Vec<ExecOutputChunk>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = chunks.clone();

    let result = client
        .exec(
            "sb_test",
            "echo hello streaming",
            None,
            Some(30_000),
            |chunk| {
                chunks_clone.lock().unwrap().push(chunk);
            },
        )
        .await;

    assert!(result.is_ok(), "exec_streaming failed: {:?}", result.err());
    let exec_result = result.unwrap();
    assert_eq!(exec_result.exit_code, 0);
    assert_eq!(exec_result.stdout, "hello streaming\n");
    assert_eq!(exec_result.stderr, "");
    assert_eq!(exec_result.result, "hello streaming\n");

    assert_eq!(
        *chunks.lock().unwrap(),
        vec![ExecOutputChunk {
            stream: ExecStream::Stdout,
            text: "hello streaming\n".into()
        }]
    );
}

#[tokio::test]
async fn test_ensure_session_handles_existing() {
    let mock_server = MockServer::start().await;

    // Return 409 Conflict (session already exists)
    Mock::given(method("POST"))
        .and(path("/sb_test/process/session"))
        .and(wiremock::matchers::body_json(
            json!({"sessionId":"everruns-exec"}),
        ))
        .respond_with(ResponseTemplate::new(409))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client.ensure_session("sb_test").await;
    assert!(result.is_ok(), "409 should be treated as success");
}

#[tokio::test]
async fn test_exec_timeout_resets_session_and_next_command_succeeds() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let mock_server = MockServer::start().await;
    let exec_call_count = std::sync::Arc::new(AtomicU32::new(0));

    Mock::given(method("POST"))
        .and(path("/sb_timeout/process/session"))
        .and(wiremock::matchers::body_json(
            json!({"sessionId":"everruns-exec"}),
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(3)
        .mount(&mock_server)
        .await;

    let counter = exec_call_count.clone();
    Mock::given(method("POST"))
        .and(path("/sb_timeout/process/session/everruns-exec/exec"))
        .respond_with(move |_: &wiremock::Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let cmd_id = if n == 0 { "cmd_timeout" } else { "cmd_ok" };
            ResponseTemplate::new(202).set_body_json(json!({ "cmdId": cmd_id }))
        })
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_timeout/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_timeout",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_timeout",
            "command": "sleep 999"
        })))
        .mount(&mock_server)
        .await;

    let mut ok_log_bytes = vec![0x01, 0x01, 0x01];
    ok_log_bytes.extend_from_slice(b"still works\n");
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_ok/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(ok_log_bytes))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_ok",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_ok",
            "command": "echo still works",
            "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/sb_timeout/process/session/everruns-exec"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let timeout_err = client
        .exec("sb_timeout", "sleep 999", None, Some(1_200), |_| {})
        .await
        .unwrap_err();
    assert!(timeout_err.contains("Command timed out after"));
    assert!(timeout_err.contains("automatically reset"));

    let result = client
        .exec("sb_timeout", "echo still works", None, Some(5_000), |_| {})
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "still works\n");
    assert_eq!(result.stderr, "");
    assert_eq!(result.result, "still works\n");
    assert_eq!(exec_call_count.load(Ordering::SeqCst), 2);
}

#[test]
fn test_strip_stream_markers_mixed_streams() {
    let mut raw = Vec::new();
    raw.extend_from_slice(&[0x01, 0x01, 0x01]);
    raw.extend_from_slice(b"stdout line\n");
    raw.extend_from_slice(&[0x02, 0x02, 0x02]);
    raw.extend_from_slice(b"stderr line\n");
    raw.extend_from_slice(&[0x01, 0x01, 0x01]);
    raw.extend_from_slice(b"more stdout\n");
    let parsed = split_stream_markers(&raw);
    assert_eq!(parsed.stdout, "stdout line\nmore stdout\n");
    assert_eq!(parsed.stderr, "stderr line\n");
    assert_eq!(parsed.combined, "stdout line\nstderr line\nmore stdout\n");
}

#[test]
fn test_strip_stream_markers_no_markers() {
    let parsed = split_stream_markers(b"plain text\n");
    assert_eq!(parsed.stdout, "plain text\n");
    assert_eq!(parsed.stderr, "");
    assert_eq!(parsed.combined, "plain text\n");
}

#[test]
fn test_strip_stream_markers_empty() {
    let parsed = split_stream_markers(b"");
    assert_eq!(parsed.stdout, "");
    assert_eq!(parsed.stderr, "");
    assert_eq!(parsed.combined, "");
}

#[test]
fn test_parse_exec_output_delta_preserves_streams() {
    let mut raw = Vec::new();
    raw.extend_from_slice(&STDOUT_MARKER);
    raw.extend_from_slice(b"stdout line\n");
    raw.extend_from_slice(&STDERR_MARKER);
    raw.extend_from_slice(b"stderr line\n");

    let mut parser = ExecDeltaParserState::new(ExecStream::Stdout);
    let parsed = parse_exec_output_delta(&raw, &mut parser);
    assert_eq!(
        parsed.chunks,
        vec![
            ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: "stdout line\n".to_string(),
            },
            ExecOutputChunk {
                stream: ExecStream::Stderr,
                text: "stderr line\n".to_string(),
            },
        ]
    );
    assert_eq!(parser.current_stream, ExecStream::Stderr);
    assert!(parser.pending_marker_prefix.is_empty());
}

#[test]
fn test_parse_exec_output_delta_handles_split_marker_across_polls() {
    let mut parser = ExecDeltaParserState::new(ExecStream::Stdout);

    let mut first = Vec::new();
    first.extend_from_slice(&STDOUT_MARKER);
    first.extend_from_slice(b"stdout line\n");
    first.extend_from_slice(&STDERR_MARKER[..2]);

    let parsed_first = parse_exec_output_delta(&first, &mut parser);
    assert_eq!(
        parsed_first.chunks,
        vec![ExecOutputChunk {
            stream: ExecStream::Stdout,
            text: "stdout line\n".to_string(),
        }]
    );
    assert_eq!(parser.current_stream, ExecStream::Stdout);
    assert_eq!(parser.pending_marker_prefix, STDERR_MARKER[..2].to_vec());

    let mut second = Vec::new();
    second.extend_from_slice(&STDERR_MARKER[2..]);
    second.extend_from_slice(b"stderr line\n");

    let parsed_second = parse_exec_output_delta(&second, &mut parser);
    assert_eq!(
        parsed_second.chunks,
        vec![ExecOutputChunk {
            stream: ExecStream::Stderr,
            text: "stderr line\n".to_string(),
        }]
    );
    assert_eq!(parser.current_stream, ExecStream::Stderr);
    assert!(parser.pending_marker_prefix.is_empty());
}
#[test]
fn streaming_output_preserves_utf8_at_every_poll_boundary() {
    let raw = "\x01\x01\x01hé🙂\x02\x02\x02érror界\x01\x01\x01fin".as_bytes();
    for cut in 0..=raw.len() {
        let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
        let mut chunks = parse_exec_output_delta(&raw[..cut], &mut state).chunks;
        chunks.extend(parse_exec_output_delta(&raw[cut..], &mut state).chunks);
        chunks.extend(finish_exec_output_delta(&mut state).chunks);
        let stdout: String = chunks
            .iter()
            .filter(|c| c.stream == ExecStream::Stdout)
            .map(|c| c.text.as_str())
            .collect();
        let stderr: String = chunks
            .iter()
            .filter(|c| c.stream == ExecStream::Stderr)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(stdout, "hé🙂fin", "poll split {cut}");
        assert_eq!(stderr, "érror界", "poll split {cut}");
    }
}

#[test]
fn streaming_output_flushes_incomplete_text_once_on_finish_or_stream_change() {
    for suffix in [b"".as_slice(), b"\x01", b"\x02\x02"] {
        let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
        let mut raw = b"ok\xff\xe2\x82".to_vec();
        raw.extend_from_slice(suffix);
        let first = parse_exec_output_delta(&raw, &mut state);
        assert_eq!(
            first.chunks,
            vec![ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: "ok�".into()
            }]
        );
        let last = finish_exec_output_delta(&mut state);
        assert_eq!(
            last.chunks,
            vec![ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: format!("�{}", String::from_utf8_lossy(suffix))
            }]
        );
        assert!(finish_exec_output_delta(&mut state).chunks.is_empty());
    }
    let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
    assert!(
        parse_exec_output_delta(b"\xe2", &mut state)
            .chunks
            .is_empty()
    );
    let next = parse_exec_output_delta(b"\x02\x02\x02err", &mut state);
    assert_eq!(
        next.chunks,
        vec![
            ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: "�".into()
            },
            ExecOutputChunk {
                stream: ExecStream::Stderr,
                text: "err".into()
            },
        ]
    );
}
#[test]
fn exec_shell_loads_profiles_silently_and_keeps_cwd_literal_and_exit_local() {
    let home = tempfile::tempdir().unwrap();
    for (path, variable, value) in [
        (".profile", "DAYTONA_TEST_PROFILE", "profile"),
        (".cargo/env", "DAYTONA_TEST_CARGO", "cargo"),
        (".nvm/nvm.sh", "DAYTONA_TEST_NVM", "nvm"),
    ] {
        let path = home.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("export {variable}={value}; echo noise; echo error >&2\n"),
        )
        .unwrap();
    }
    std::fs::write(home.path().join(".bashrc"), "echo must-not-source-bashrc\n").unwrap();
    let cwd = home.path().join("project's $(printf substituted) files");
    std::fs::create_dir(&cwd).unwrap();
    let command = "printf '%s|%s|%s\\n' \"$DAYTONA_TEST_PROFILE\" \"$DAYTONA_TEST_CARGO\" \"$DAYTONA_TEST_NVM\"; pwd -P; exit 7";
    for requested_cwd in [None, Some(cwd.to_str().unwrap())] {
        let generated = format_exec_command(command, requested_cwd);
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{generated}; printf 'session-alive\\n'"))
            .env("HOME", home.path())
            .env_remove("DAYTONA_TEST_PROFILE")
            .env_remove("DAYTONA_TEST_CARGO")
            .env_remove("DAYTONA_TEST_NVM")
            .current_dir(home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "cwd={requested_cwd:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected_cwd = if requested_cwd.is_some() {
            &cwd
        } else {
            &home.path().to_path_buf()
        };
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "profile|cargo|nvm\n{}\nsession-alive\n",
                expected_cwd.canonicalize().unwrap().display()
            )
        );
    }
}
#[tokio::test]
async fn exec_preserves_status_output_and_command_options() {
    for (command, cwd, timeout, exit_code, output, suffix) in [
        (
            "echo hello",
            None,
            None,
            0,
            "hello\n",
            "( eval 'echo hello' )",
        ),
        (
            "ls",
            Some("/my project's files"),
            Some(5000),
            1,
            "failed\n",
            "( cd '/my project'\"'\"'s files' && eval 'ls' )",
        ),
        (
            "nonexistent",
            None,
            Some(5000),
            127,
            "command not found",
            "( eval 'nonexistent' )",
        ),
    ] {
        let server = MockServer::start().await;
        setup_session_mocks(&server, "sb_test", exit_code, output).await;
        let client = DaytonaClient::with_base_urls("test_key".into(), server.uri(), server.uri());
        let result = client
            .exec("sb_test", command, cwd, timeout, |_| {})
            .await
            .unwrap();
        assert_eq!(i64::from(result.exit_code), exit_code);
        assert_eq!(result.stdout, output);
        assert_eq!(result.stderr, "");
        assert_eq!(result.result, output);
        let requests = server.received_requests().await.unwrap();
        let exec: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path().ends_with("/exec"))
            .collect();
        assert_eq!(exec.len(), 1);
        // Profile behavior has an independent real-shell oracle below.
        assert_eq!(
            exec[0].body_json::<Value>().unwrap(),
            json!({
                "command": format!("{SHELL_PROFILE_PREAMBLE}{suffix}"), "runAsync": true
            })
        );
    }
}

#[tokio::test]
async fn test_filesystem_snapshot_lifecycle_uses_native_management_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandbox/builder/snapshot"))
        .and(wiremock::matchers::body_json(
            json!({"name":"baked-tools","includeMemory":false}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"builder","state":"snapshotting"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/snapshots/snapshot-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"snapshot-1","name":"baked-tools","state":"active"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/snapshots/snapshot-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let client =
        DaytonaClient::with_base_urls("host-only-token".into(), server.uri(), server.uri());
    client
        .create_sandbox_snapshot("builder", "baked-tools")
        .await
        .unwrap();
    let info = client.get_snapshot("snapshot-1").await.unwrap();
    assert_eq!(info.state, "active");
    client.delete_snapshot("snapshot-1").await.unwrap();
}

#[tokio::test]
async fn test_builder_output_limit_is_enforced_before_collecting_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/builder/logs"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 256 * 1024]))
        .mount(&server)
        .await;
    let client = DaytonaClient::with_base_urls("test-key".into(), server.uri(), server.uri())
        .with_download_limit(64 * 1024);
    assert!(
        client
            .toolbox_download("builder", "/logs")
            .await
            .unwrap_err()
            .contains("byte limit")
    );
}
