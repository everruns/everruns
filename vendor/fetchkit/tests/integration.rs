//! Integration tests for FetchKit using wiremock

use fetchkit::{
    fetch_with_options, DnsPolicy, FetchError, FetchOptions, FetchRequest, FetcherRegistry,
    HttpMethod, LocalFileSaver, Tool,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::Service;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: create FetchOptions that permit loopback (for wiremock tests)
fn test_options() -> FetchOptions {
    FetchOptions {
        enable_markdown: true,
        enable_text: true,
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    }
}

/// Helper: create a Tool that permits loopback (for wiremock tests)
fn test_tool() -> Tool {
    Tool::builder().block_private_ips(false).build()
}

/// Helper: create a Tool with save_to_file enabled (for wiremock tests)
fn test_tool_with_save() -> Tool {
    Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .build()
}

async fn spawn_malformed_chunked_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\nZZ\r\nboom\r\n0\r\n\r\n",
                )
                .await;
        }
    });

    format!("http://{addr}/")
}

#[tokio::test]
async fn test_simple_get() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Hello, World!")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.content_type, Some("text/plain".to_string()));
    assert!(resp.content.unwrap().contains("Hello, World!"));
    assert_eq!(resp.format, Some("raw".to_string()));
}

#[tokio::test]
async fn test_malformed_chunked_body_returns_error() {
    let req = FetchRequest::new(spawn_malformed_chunked_server().await);
    let result = fetch_with_options(req, test_options()).await;

    assert!(matches!(result, Err(FetchError::RequestError(_))));
}

#[tokio::test]
async fn test_save_to_file_malformed_chunked_body_does_not_create_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let req =
        FetchRequest::new(spawn_malformed_chunked_server().await).save_to_file("malformed.txt");
    let result = test_tool_with_save()
        .execute_with_saver(req, Some(&saver))
        .await;

    assert!(matches!(result, Err(FetchError::RequestError(_))));
    assert!(!dir.path().join("malformed.txt").exists());
}

#[tokio::test]
async fn test_head_request() {
    let mock_server = MockServer::start().await;

    Mock::given(method("HEAD"))
        .and(path("/file.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .insert_header("content-length", "12345")
                .insert_header("last-modified", "Tue, 01 Jan 2024 00:00:00 GMT"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/file.pdf", mock_server.uri())).method(HttpMethod::Head);
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.method, Some("HEAD".to_string()));
    assert_eq!(resp.content_type, Some("application/pdf".to_string()));
    assert_eq!(resp.size, Some(12345));
    assert_eq!(
        resp.last_modified,
        Some("Tue, 01 Jan 2024 00:00:00 GMT".to_string())
    );
    assert!(resp.content.is_none());
}

#[tokio::test]
async fn test_html_to_markdown() {
    let mock_server = MockServer::start().await;

    let html = r#"<!DOCTYPE html>
<html>
<head><title>Test</title></head>
<body>
    <h1>Hello World</h1>
    <p>This is a <strong>test</strong> paragraph.</p>
    <ul>
        <li>Item 1</li>
        <li>Item 2</li>
    </ul>
</body>
</html>"#;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_markdown();
    let resp = tool.execute(req).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.format, Some("markdown".to_string()));

    let content = resp.content.unwrap();
    assert!(content.contains("# Hello World"));
    assert!(content.contains("**test**"));
    assert!(content.contains("- Item 1"));
    assert!(content.contains("- Item 2"));
}

#[tokio::test]
async fn test_html_to_text() {
    let mock_server = MockServer::start().await;

    let html = r#"<!DOCTYPE html>
<html>
<body>
    <h1>Title</h1>
    <p>Paragraph text.</p>
    <script>alert('bad');</script>
</body>
</html>"#;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_text();
    let resp = tool.execute(req).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.format, Some("text".to_string()));

    let content = resp.content.unwrap();
    assert!(content.contains("Title"));
    assert!(content.contains("Paragraph text"));
    assert!(!content.contains("alert")); // Script should be stripped
}

#[tokio::test]
async fn test_binary_content() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]) // PNG magic bytes
                .insert_header("content-type", "image/png")
                .insert_header("content-length", "4"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/image.png", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.content_type, Some("image/png".to_string()));
    assert_eq!(resp.size, Some(4));
    assert!(resp.content.is_none());
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().contains("Binary content"));
}

#[tokio::test]
async fn test_4xx_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("Not Found")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/not-found", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    // 4xx is still a success response (not a tool error)
    assert_eq!(resp.status_code, 404);
    assert!(resp.content.unwrap().contains("Not Found"));
}

#[tokio::test]
async fn test_5xx_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/error"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string("Internal Server Error")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/error", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    // 5xx is still a success response (not a tool error)
    assert_eq!(resp.status_code, 500);
    assert!(resp.content.unwrap().contains("Internal Server Error"));
}

#[tokio::test]
async fn test_content_disposition_filename() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("file content")
                .insert_header("content-type", "text/plain")
                .insert_header("content-disposition", "attachment; filename=\"report.txt\""),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/download", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.filename, Some("report.txt".to_string()));
}

#[tokio::test]
async fn test_filename_from_url() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/path/to/document.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .insert_header("content-length", "100"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/path/to/document.pdf", mock_server.uri()))
        .method(HttpMethod::Head);
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.filename, Some("document.pdf".to_string()));
}

#[tokio::test]
async fn test_size_for_text_content() {
    let mock_server = MockServer::start().await;

    let body = "Hello, this is test content!";

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    // Size should equal bytes read from body
    assert_eq!(resp.size, Some(body.len() as u64));
}

#[tokio::test]
async fn test_text_body_truncated_at_safety_limit() {
    let mock_server = MockServer::start().await;

    let body = "A".repeat(1024);

    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/large", mock_server.uri()));
    let resp = fetch_with_options(
        req,
        FetchOptions {
            enable_markdown: true,
            enable_text: true,
            dns_policy: DnsPolicy::allow_all(),
            max_body_size: Some(128),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.size, Some(128));
    assert_eq!(resp.truncated, Some(true));

    let content = resp.content.unwrap();
    assert!(content.starts_with(&"A".repeat(128)));
    assert!(content.contains("[..content truncated...]"));
}

#[tokio::test]
async fn test_url_prefix_allow_list() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    // Create tool with allow list that doesn't include the mock server
    let tool = Tool::builder()
        .block_private_ips(false)
        .allow_prefix("https://allowed.example.com")
        .build();

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = tool.execute(req).await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("not allowed by policy"));
}

#[tokio::test]
async fn test_url_prefix_block_list() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    // Create tool with block list that includes localhost
    let tool = Tool::builder()
        .block_private_ips(false)
        .block_prefix("http://127.0.0.1")
        .build();

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = tool.execute(req).await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("not allowed by policy"));
}

#[tokio::test]
async fn test_invalid_url_scheme() {
    let req = FetchRequest::new("ftp://example.com/file.txt");
    let result = fetch_with_options(req, test_options()).await;

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("http:// or https://"));
}

#[tokio::test]
async fn test_missing_url() {
    let req = FetchRequest::new("");
    let result = fetch_with_options(req, test_options()).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing"));
}

#[tokio::test]
async fn test_entity_decoding_in_html() {
    let mock_server = MockServer::start().await;

    let html = "<p>Tom &amp; Jerry &lt;3 &gt; others &quot;quoted&quot;</p>";

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_text();
    let resp = tool.execute(req).await.unwrap();

    let content = resp.content.unwrap();
    assert!(content.contains("Tom & Jerry"));
    assert!(content.contains("<3"));
    assert!(content.contains("> others"));
    assert!(content.contains("\"quoted\""));
}

#[tokio::test]
async fn test_non_html_with_conversion_flags() {
    let mock_server = MockServer::start().await;

    let json = r#"{"key": "value"}"#;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(json)
                .insert_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let req = FetchRequest::new(format!("{}/api/data", mock_server.uri())).as_markdown();
    let resp = tool.execute(req).await.unwrap();

    // Non-HTML should return raw even with as_markdown flag
    assert_eq!(resp.format, Some("raw".to_string()));
    assert!(resp.content.unwrap().contains("\"key\""));
}

#[tokio::test]
async fn test_html_detection_by_body() {
    let mock_server = MockServer::start().await;

    // Server returns HTML without proper content-type
    let html = "<!DOCTYPE html><html><body>Hello</body></html>";

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(html)
                .insert_header("content-type", "text/plain"), // Wrong content-type
        )
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_markdown();
    let resp = tool.execute(req).await.unwrap();

    // Should detect HTML by body content and convert
    assert_eq!(resp.format, Some("markdown".to_string()));
}

#[tokio::test]
async fn test_custom_user_agent() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    let tool = Tool::builder()
        .block_private_ips(false)
        .user_agent("CustomBot/1.0")
        .build();

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let resp = tool.execute(req).await.unwrap();

    assert_eq!(resp.status_code, 200);
}

#[tokio::test]
async fn test_excessive_newlines_filtered() {
    let mock_server = MockServer::start().await;

    let body = "Line1\n\n\n\n\n\nLine2";

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    // Should have at most 2 consecutive newlines
    assert!(!resp.content.unwrap().contains("\n\n\n"));
}

// ============================================================================
// Fetcher System Integration Tests
// ============================================================================

#[tokio::test]
async fn test_fetcher_registry_with_defaults() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><h1>Test</h1></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    let registry = FetcherRegistry::with_defaults();
    let options = FetchOptions {
        enable_markdown: true,
        enable_text: true,
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };

    let req = FetchRequest::new(format!("{}/page", mock_server.uri())).as_markdown();
    let resp = registry.fetch(req, options).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.format, Some("markdown".to_string()));
    assert!(resp.content.unwrap().contains("# Test"));
}

#[tokio::test]
async fn test_fetcher_registry_url_validation() {
    let registry = FetcherRegistry::with_defaults();
    let options = FetchOptions {
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };

    // Invalid scheme
    let req = FetchRequest::new("ftp://example.com");
    let result = registry.fetch(req, options.clone()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("http://"));

    // Empty URL handled by fetch_with_options before registry
    let req = FetchRequest::new("");
    let result = fetch_with_options(req, options).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fetcher_registry_allow_block_lists() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    let registry = FetcherRegistry::with_defaults();

    // Block list
    let options = FetchOptions {
        block_prefixes: vec!["http://127.0.0.1".to_string()],
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };
    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = registry.fetch(req, options).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Blocked"));

    // Allow list (not matching)
    let options = FetchOptions {
        allow_prefixes: vec!["https://allowed.com".to_string()],
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };
    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = registry.fetch(req, options).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fetcher_registry_allow_list_rejects_lookalike_host() {
    let registry = FetcherRegistry::with_defaults();
    let options = FetchOptions {
        allow_prefixes: vec!["https://allowed.example.com".to_string()],
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };

    let req = FetchRequest::new("https://allowed.example.com.evil.test/");
    let result = registry.fetch(req, options).await;

    assert!(matches!(result, Err(FetchError::BlockedUrl)));
}

#[tokio::test]
async fn test_github_fetcher_url_matching() {
    // These URLs should NOT match GitHubRepoFetcher (will use DefaultFetcher)
    let mock_server = MockServer::start().await;

    // Mock for non-GitHub URLs
    Mock::given(method("GET"))
        .and(path("/owner/repo/issues"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("issues page")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let req = FetchRequest::new(format!("{}/owner/repo/issues", mock_server.uri()));
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    // Should use default fetcher (format is "raw", not "github_repo")
    assert_eq!(resp.format, Some("raw".to_string()));
    assert!(resp.content.unwrap().contains("issues page"));
}

#[tokio::test]
async fn test_fetch_enables_conversions_by_default() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><p>Hello</p></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    // test_options() has enable_markdown: true (matching fetch() default)
    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_markdown();
    let resp = fetch_with_options(req, test_options()).await.unwrap();

    assert_eq!(resp.format, Some("markdown".to_string()));
}

#[tokio::test]
async fn test_fetch_with_options_respects_disabled_conversion() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><p>Hello</p></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    // Disable markdown conversion
    let options = FetchOptions {
        enable_markdown: false,
        enable_text: false,
        dns_policy: DnsPolicy::allow_all(),
        ..Default::default()
    };

    let req = FetchRequest::new(format!("{}/", mock_server.uri())).as_markdown();
    let resp = fetch_with_options(req, options).await.unwrap();

    // Should be raw because conversion is disabled
    assert_eq!(resp.format, Some("raw".to_string()));
}

// ============================================================================
// Safe-by-default: fetch() and Tool::default() block private IPs
// ============================================================================

#[tokio::test]
async fn test_fetch_blocks_loopback_by_default() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    // fetch() uses default options which now block private IPs
    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = fetchkit::fetch(req).await;
    assert!(matches!(result, Err(fetchkit::FetchError::BlockedUrl)));
}

#[tokio::test]
async fn test_tool_default_blocks_loopback() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    let tool = Tool::default();
    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let result = tool.execute(req).await;
    assert!(matches!(result, Err(fetchkit::FetchError::BlockedUrl)));
}

// ============================================================================
// File Save Integration Tests
// ============================================================================

#[tokio::test]
async fn test_save_to_file_text_content() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/data.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"key": "value"}"#)
                .insert_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let tool = test_tool_with_save();

    let req =
        FetchRequest::new(format!("{}/data.json", mock_server.uri())).save_to_file("output.json");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert!(resp.saved_path.is_some());
    assert_eq!(resp.bytes_written, Some(16));
    // No inline content when saving to file
    assert!(resp.content.is_none());

    // Verify file on disk
    let content = std::fs::read_to_string(dir.path().join("output.json")).unwrap();
    assert_eq!(content, r#"{"key": "value"}"#);
}

#[tokio::test]
async fn test_save_to_file_binary_content() {
    let mock_server = MockServer::start().await;

    let binary_data: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(binary_data.clone())
                .insert_header("content-type", "image/png"),
        )
        .mount(&mock_server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let tool = test_tool_with_save();

    let req =
        FetchRequest::new(format!("{}/image.png", mock_server.uri())).save_to_file("image.png");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert!(resp.saved_path.is_some());
    assert_eq!(resp.bytes_written, Some(8));
    // Binary saved without error (unlike normal fetch which rejects binary)
    assert!(resp.error.is_none());

    // Verify binary content on disk
    let saved = std::fs::read(dir.path().join("image.png")).unwrap();
    assert_eq!(saved, binary_data);
}

#[tokio::test]
async fn test_save_to_file_creates_subdirectories() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("content")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let tool = test_tool_with_save();

    let req =
        FetchRequest::new(format!("{}/file", mock_server.uri())).save_to_file("sub/dir/file.txt");
    let resp = tool.execute_with_saver(req, Some(&saver)).await.unwrap();

    assert!(resp.saved_path.is_some());
    assert!(dir.path().join("sub/dir/file.txt").exists());
}

#[tokio::test]
async fn test_save_to_file_rejects_path_traversal() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let tool = test_tool_with_save();

    let req = FetchRequest::new(format!("{}/", mock_server.uri())).save_to_file("../../etc/passwd");
    let result = tool.execute_with_saver(req, Some(&saver)).await;

    // Path traversal should be rejected before HTTP request
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("save file"));
}

#[tokio::test]
async fn test_save_to_file_without_saver_errors() {
    let tool = test_tool_with_save();

    let req = FetchRequest::new("https://example.com/file").save_to_file("file.txt");
    let result = tool.execute_with_saver(req, None).await;

    assert!(matches!(
        result,
        Err(fetchkit::FetchError::SaverNotAvailable)
    ));
}

#[tokio::test]
async fn test_save_to_file_disabled_by_default() {
    // Default tool does NOT have save_to_file enabled
    let tool = Tool::builder().block_private_ips(false).build();

    let req = FetchRequest::new("https://example.com/file").save_to_file("file.txt");
    let dir = tempfile::tempdir().unwrap();
    let saver = LocalFileSaver::new(Some(dir.path().to_path_buf()));
    let result = tool.execute_with_saver(req, Some(&saver)).await;

    assert!(matches!(
        result,
        Err(fetchkit::FetchError::SaverNotAvailable)
    ));
}

#[tokio::test]
async fn test_save_to_file_schema_gating() {
    // Default: save_to_file not in schema
    let tool = Tool::default();
    let schema = tool.input_schema();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        assert!(!props.contains_key("save_to_file"));
    }

    // Enabled: save_to_file in schema
    let tool = Tool::builder().enable_save_to_file(true).build();
    let schema = tool.input_schema();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        assert!(props.contains_key("save_to_file"));
    }
}

#[tokio::test]
async fn test_execute_with_saver_no_save_falls_through() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Hello")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let tool = test_tool_with_save();

    // No save_to_file — should behave like normal execute
    let req = FetchRequest::new(format!("{}/", mock_server.uri()));
    let resp = tool.execute_with_saver(req, None).await.unwrap();

    assert_eq!(resp.status_code, 200);
    assert!(resp.content.unwrap().contains("Hello"));
    assert!(resp.saved_path.is_none());
    assert!(resp.bytes_written.is_none());
}

#[tokio::test]
async fn test_tool_execution_returns_contract_output() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<h1>Hello</h1>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock_server)
        .await;

    let tool = test_tool();
    let output = tool
        .execution(json!({
            "url": format!("{}/", mock_server.uri()),
            "as_markdown": true
        }))
        .unwrap()
        .execute()
        .await
        .unwrap();

    assert_eq!(output.result["status_code"], 200);
    assert!(output.result["format"].is_string());
    assert!(output.result["content"].as_str().unwrap().contains("Hello"));
    assert_eq!(output.metadata.extra["http_status"], 200);
    assert!(output.images.is_empty());
}

#[tokio::test]
async fn test_tool_service_executes_json_calls() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Hello, Service!")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let mut service = Tool::builder().block_private_ips(false).build_service();
    let result = service
        .call(json!({
            "url": format!("{}/", mock_server.uri())
        }))
        .await
        .unwrap();

    assert_eq!(result["status_code"], 200);
    assert_eq!(result["content"], "Hello, Service!");
}
