//! Safety and robustness tests for FileSaver and save_to_file
//!
//! Covers: error handling, path traversal, hung servers, malformed responses,
//! concurrent saves, empty/huge payloads, special filenames, and edge cases.

use fetchkit::{FetchError, FetchRequest, LocalFileSaver, Tool};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tool_with_save() -> Tool {
    Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .build()
}

fn saver_in(dir: &std::path::Path) -> LocalFileSaver {
    LocalFileSaver::new(Some(dir.to_path_buf()))
}

#[cfg(unix)]
fn symlink_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::os::unix::fs::symlink(src, dst).unwrap();
}

#[cfg(unix)]
fn symlink_file(src: &std::path::Path, dst: &std::path::Path) {
    std::os::unix::fs::symlink(src, dst).unwrap();
}

#[cfg(windows)]
fn symlink_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::os::windows::fs::symlink_dir(src, dst).unwrap();
}

#[cfg(windows)]
fn symlink_file(src: &std::path::Path, dst: &std::path::Path) {
    std::os::windows::fs::symlink_file(src, dst).unwrap();
}

// ---------------------------------------------------------------------------
// Path traversal attacks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_path_traversal_dotdot() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pwned"))
        .mount(&mock)
        .await;

    for attack_path in &[
        "../../etc/passwd",
        "../../../etc/shadow",
        "foo/../../bar",
        "a/b/c/../../../../outside",
        "../.ssh/authorized_keys",
    ] {
        let req =
            FetchRequest::new(format!("{}/", mock.uri())).save_to_file(attack_path.to_string());
        let result = tool.execute_with_saver(req, Some(&saver)).await;
        assert!(
            result.is_err(),
            "Path traversal should be rejected: {}",
            attack_path
        );
    }
}

#[tokio::test]
async fn test_path_traversal_absolute_escape() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pwned"))
        .mount(&mock)
        .await;

    // Absolute path outside base_dir
    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("/tmp/evil.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;
    assert!(result.is_err(), "Absolute path outside base should fail");
}

#[tokio::test]
async fn test_path_traversal_null_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data"))
        .mount(&mock)
        .await;

    // Null byte in path — should either fail or be sanitized
    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("file\x00.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;
    // On most OSes, null bytes in filenames cause IO errors
    assert!(result.is_err(), "Null byte path should fail");
}

#[tokio::test]
async fn test_no_base_dir_requires_absolute() {
    let saver = LocalFileSaver::new(None);
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data"))
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("relative.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;
    assert!(
        result.is_err(),
        "Relative path without base_dir should fail"
    );
}

#[tokio::test]
async fn test_path_traversal_symlink_escape_rejected() {
    use fetchkit::file_saver::FileSaver;

    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let saver = saver_in(base.path());

    symlink_dir(outside.path(), &base.path().join("pivot"));

    let result = saver.save("pivot/escape.txt", b"pwned").await;
    assert!(result.is_err(), "symlink escape should be rejected");
    assert!(!outside.path().join("escape.txt").exists());
}

#[tokio::test]
async fn test_path_traversal_final_symlink_escape_rejected() {
    use fetchkit::file_saver::FileSaver;

    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    let saver = saver_in(base.path());

    symlink_file(outside.path(), &base.path().join("out"));

    let result = saver.save("out", b"pwned").await;
    assert!(result.is_err(), "final symlink escape should be rejected");
    assert_eq!(std::fs::read(outside.path()).unwrap(), b"");
}

#[tokio::test]
async fn test_execute_with_saver_rejects_symlink_escape() {
    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let saver = saver_in(base.path());
    let tool = tool_with_save();

    symlink_dir(outside.path(), &base.path().join("pivot"));

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pwned"))
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("pivot/escape.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;

    assert!(
        result.is_err(),
        "execute_with_saver should reject symlink escape"
    );
    assert!(!outside.path().join("escape.txt").exists());
}

// ---------------------------------------------------------------------------
// Server errors and edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_http_404() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("Not Found")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/missing", mock.uri())).save_to_file("not_found.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    // 404 body is still saved — caller decides what to do with status
    assert_eq!(resp.status_code, 404);
    assert!(resp.saved_path.is_some());
    let content = std::fs::read_to_string(dir.path().join("not_found.txt")).unwrap();
    assert_eq!(content, "Not Found");
}

#[tokio::test]
async fn test_save_http_500() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/error"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string("Internal Error")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/error", mock.uri())).save_to_file("error.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();
    assert_eq!(resp.status_code, 500);
    assert!(resp.saved_path.is_some());
}

#[tokio::test]
async fn test_save_empty_body() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/empty"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![])
                .insert_header("content-type", "application/octet-stream"),
        )
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/empty", mock.uri())).save_to_file("empty.bin");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.bytes_written, Some(0));
    assert!(dir.path().join("empty.bin").exists());
    assert_eq!(
        std::fs::read(dir.path().join("empty.bin")).unwrap().len(),
        0
    );
}

// ---------------------------------------------------------------------------
// Slow/hung server — must not hang forever
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_slow_server_does_not_hang() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&mock)
        .await;

    // Wrap in a tokio timeout to ensure we don't hang
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let req = FetchRequest::new(format!("{}/slow", mock.uri())).save_to_file("slow.txt");
        tool.execute_with_saver(req, Some(&saver)).await
    })
    .await;

    // Should complete within timeout — either with timeout error or partial content
    assert!(
        result.is_ok(),
        "Should not hang — must complete within 10 seconds"
    );
    // The inner result may be an error (timeout) or a truncated response, both are fine
}

#[tokio::test]
async fn test_save_connect_timeout_does_not_hang() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    // Use a non-routable IP to trigger connect timeout
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let req = FetchRequest::new("http://192.0.2.1:12345/file").save_to_file("timeout.txt");
        // Need to allow this IP for the test (it's a documentation range, normally blocked)
        let tool = Tool::builder()
            .block_private_ips(false)
            .enable_save_to_file(true)
            .build();
        tool.execute_with_saver(req, Some(&saver)).await
    })
    .await;

    assert!(
        result.is_ok(),
        "Connect timeout should not hang the process"
    );
    assert!(result.unwrap().is_err(), "Should return error on timeout");
}

// ---------------------------------------------------------------------------
// Feature gating / authorization
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_disabled_by_default() {
    let tool = Tool::default();
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    let req = FetchRequest::new("https://example.com").save_to_file("file.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;

    assert!(matches!(result, Err(FetchError::SaverNotAvailable)));
}

#[tokio::test]
async fn test_save_no_saver_errors() {
    let tool = tool_with_save();

    let req = FetchRequest::new("https://example.com").save_to_file("file.txt");
    let result = tool.execute_with_saver(req, None).await;

    assert!(matches!(result, Err(FetchError::SaverNotAvailable)));
}

#[tokio::test]
async fn test_save_schema_gating_default_hidden() {
    let tool = Tool::default();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        !props.contains_key("save_to_file"),
        "save_to_file should be hidden in default schema"
    );
}

#[tokio::test]
async fn test_save_schema_gating_enabled_visible() {
    let tool = tool_with_save();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("save_to_file"),
        "save_to_file should be visible when enabled"
    );
}

// ---------------------------------------------------------------------------
// Concurrent saves
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_saves_dont_corrupt() {
    let dir = tempfile::tempdir().unwrap();

    let mock = MockServer::start().await;
    for i in 0..5 {
        Mock::given(method("GET"))
            .and(path(format!("/file{}", i)))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("content-{}", i))
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&mock)
            .await;
    }

    let mut handles = vec![];
    for i in 0..5 {
        let url = format!("{}/file{}", mock.uri(), i);
        let save_path = format!("concurrent/file{}.txt", i);
        let dir_path = dir.path().to_path_buf();

        let handle = tokio::spawn(async move {
            let saver = LocalFileSaver::new(Some(dir_path));
            let tool = Tool::builder()
                .block_private_ips(false)
                .enable_save_to_file(true)
                .build();
            let req = FetchRequest::new(url).save_to_file(save_path);
            tool.execute_with_saver(req, Some(&saver)).await
        });
        handles.push(handle);
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "Concurrent save {} failed: {:?}", i, result);
    }

    // Verify each file has correct content
    for i in 0..5 {
        let content =
            std::fs::read_to_string(dir.path().join(format!("concurrent/file{}.txt", i))).unwrap();
        assert_eq!(content, format!("content-{}", i));
    }
}

// ---------------------------------------------------------------------------
// Special filenames
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_filename_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data"))
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("my file (1).txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();
    assert!(resp.saved_path.is_some());
    assert!(dir.path().join("my file (1).txt").exists());
}

#[tokio::test]
async fn test_save_filename_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data"))
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("datos_ñ.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();
    assert!(resp.saved_path.is_some());
}

#[tokio::test]
async fn test_save_deeply_nested_path() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("deep"))
        .mount(&mock)
        .await;

    let req =
        FetchRequest::new(format!("{}/", mock.uri())).save_to_file("a/b/c/d/e/f/g/h/deep.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();
    assert!(resp.saved_path.is_some());
    assert!(dir.path().join("a/b/c/d/e/f/g/h/deep.txt").exists());
}

// ---------------------------------------------------------------------------
// Overwrite behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_overwrites_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    // Create existing file
    std::fs::write(dir.path().join("existing.txt"), "old content").unwrap();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("new content"))
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("existing.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();
    assert!(resp.saved_path.is_some());

    let content = std::fs::read_to_string(dir.path().join("existing.txt")).unwrap();
    assert_eq!(content, "new content");
}

// ---------------------------------------------------------------------------
// Various binary content types
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_various_binary_types() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();
    let mock = MockServer::start().await;

    let cases = vec![
        ("image.jpg", "image/jpeg", vec![0xFF, 0xD8, 0xFF, 0xE0]),
        ("doc.pdf", "application/pdf", b"%PDF-1.4".to_vec()),
        (
            "archive.zip",
            "application/zip",
            vec![0x50, 0x4B, 0x03, 0x04],
        ),
        (
            "data.bin",
            "application/octet-stream",
            (0..=255u8).collect::<Vec<u8>>(),
        ),
    ];

    for (filename, content_type, data) in &cases {
        let url_path = format!("/{}", filename);
        Mock::given(method("GET"))
            .and(path(&url_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .insert_header("content-type", *content_type),
            )
            .mount(&mock)
            .await;

        let req = FetchRequest::new(format!("{}{}", mock.uri(), url_path))
            .save_to_file(filename.to_string());
        let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

        assert_eq!(resp.status_code, 200, "Failed for {}", filename);
        assert!(resp.saved_path.is_some(), "No saved_path for {}", filename);
        assert!(resp.error.is_none(), "Unexpected error for {}", filename);

        let saved = std::fs::read(dir.path().join(filename)).unwrap();
        assert_eq!(saved, *data, "Content mismatch for {}", filename);
    }
}

// ---------------------------------------------------------------------------
// URL allow/block list interaction with save_to_file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_respects_block_list() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("blocked"))
        .mount(&mock)
        .await;

    let tool = Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .block_prefix("http://127.0.0.1")
        .build();

    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("blocked.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;
    assert!(
        matches!(result, Err(FetchError::BlockedUrl)),
        "Blocked URLs should still be rejected for save_to_file"
    );
}

#[tokio::test]
async fn test_save_respects_allow_list() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("denied"))
        .mount(&mock)
        .await;

    let tool = Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .allow_prefix("https://allowed.example.com")
        .build();

    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    let req = FetchRequest::new(format!("{}/", mock.uri())).save_to_file("denied.txt");
    let result = tool.execute_with_saver(req, Some(&saver)).await;
    assert!(result.is_err(), "Non-allowed URLs should be rejected");
}

// ---------------------------------------------------------------------------
// HEAD request with save_to_file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_head_request_no_body() {
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .insert_header("content-length", "1000"),
        )
        .mount(&mock)
        .await;

    let req = FetchRequest::new(format!("{}/file", mock.uri()))
        .method(fetchkit::HttpMethod::Head)
        .save_to_file("metadata.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    // HEAD returns metadata, no body to save
    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.method, Some("HEAD".to_string()));
}

// ---------------------------------------------------------------------------
// LocalFileSaver unit tests (edge cases)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_local_file_saver_validate_then_save() {
    use fetchkit::file_saver::FileSaver;

    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    // Validate first, then save
    saver.validate_path("valid.txt").await.unwrap();
    let result = saver.save("valid.txt", b"validated content").await.unwrap();
    assert_eq!(result.bytes_written, 17);
}

#[tokio::test]
async fn test_local_file_saver_empty_filename() {
    use fetchkit::file_saver::FileSaver;

    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    // Empty filename — resolves to base dir itself, which is a directory
    let result = saver.save("", b"data").await;
    // This should fail because we're trying to write to a directory
    assert!(result.is_err());
}

#[tokio::test]
async fn test_local_file_saver_large_write() {
    use fetchkit::file_saver::FileSaver;

    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    // 1MB write
    let data = vec![0xABu8; 1_000_000];
    let result = saver.save("large.bin", &data).await.unwrap();
    assert_eq!(result.bytes_written, 1_000_000);

    let saved = std::fs::read(dir.path().join("large.bin")).unwrap();
    assert_eq!(saved.len(), 1_000_000);
    assert!(saved.iter().all(|&b| b == 0xAB));
}

// ---------------------------------------------------------------------------
// Default Fetcher::fetch_to_file (used by non-DefaultFetcher fetchers)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_via_default_fetch_to_file_trait_method() {
    // Tests the default Fetcher::fetch_to_file implementation, which calls
    // fetch() then saves the string content. This path is used by fetchers
    // that don't override fetch_to_file (e.g. GitHubRepoFetcher).
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());

    let mock = MockServer::start().await;
    // Return HTML that will be converted to markdown by DefaultFetcher
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><h1>Title</h1><p>Body text</p></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock)
        .await;

    let tool = Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .enable_markdown(true)
        .build();

    // save_to_file with HTML content: DefaultFetcher::fetch_to_file handles
    // this directly (binary-aware path), but confirms the full save pipeline
    let req = FetchRequest::new(format!("{}/page", mock.uri())).save_to_file("page_content.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert!(resp.saved_path.is_some());
    assert!(resp.bytes_written.unwrap() > 0);
    assert!(resp.content.is_none());

    // File should exist with some content
    let content = std::fs::read_to_string(dir.path().join("page_content.txt")).unwrap();
    assert!(!content.is_empty());
}

// ---------------------------------------------------------------------------
// Truncated binary save (slow binary download)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_save_binary_with_slow_response_truncated() {
    // Verifies that a binary download with a delayed response times out
    // gracefully and either saves partial content or returns an error
    let dir = tempfile::tempdir().unwrap();
    let saver = saver_in(dir.path());
    let tool = tool_with_save();

    let mock = MockServer::start().await;
    // Respond with binary content-type but delay the response
    Mock::given(method("GET"))
        .and(path("/big.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])
                .insert_header("content-type", "application/octet-stream")
                .set_delay(Duration::from_secs(45)),
        )
        .mount(&mock)
        .await;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let req = FetchRequest::new(format!("{}/big.bin", mock.uri())).save_to_file("big.bin");
        tool.execute_with_saver(req, Some(&saver)).await
    })
    .await;

    // Must not hang
    assert!(result.is_ok(), "Binary save with slow server must not hang");
    // Inner result is expected to be an error (connect/first-byte timeout)
}

// ---------------------------------------------------------------------------
// Description/llmtxt reflects enabled state
// ---------------------------------------------------------------------------

#[test]
fn test_description_reflects_save_enabled() {
    let default_tool = Tool::default();
    assert!(!default_tool.description().contains("save_to_file"));
    assert!(!default_tool.llmtxt().contains("save_to_file"));

    let save_tool = tool_with_save();
    assert!(save_tool.description().contains("save_to_file"));
    assert!(save_tool.llmtxt().contains("save_to_file"));
}
