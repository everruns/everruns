use super::*;
use fetchkit::FetchError;

use self::typed_id::SessionId;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Create a WebFetchTool with permissive DNS policy for wiremock tests
/// (wiremock binds to 127.0.0.1 which is blocked by default).
fn tool_for_wiremock() -> WebFetchTool {
    let builder = fetchkit::Tool::builder()
        .enable_save_to_file(true)
        .block_private_ips(false);
    let fetchkit_tool = builder.build();
    let description = fetchkit_tool.description().to_string();
    WebFetchTool {
        builder,
        fetchkit_tool,
        enable_save_to_file: true,
        description,
        system_policy: None,
    }
}

#[tokio::test]
async fn system_allowlist_blocks_with_clear_system_policy_error() {
    use everruns_contracts::runtime::{SystemAllowlist, SystemEgressPolicy};

    let mut tool = tool_for_wiremock();
    tool.system_policy = Some(Arc::new(SystemEgressPolicy::allowlist_only(
        SystemAllowlist::from_toml("[groups.test]\nallowed = [\"allowed.example.com\"]\n")
            .map(Arc::new)
            .unwrap(),
    )));

    let result = tool
        .execute(serde_json::json!({ "url": "https://blocked.example.com/path" }))
        .await;

    let message = match result {
        ToolExecutionResult::ToolError(message) => message,
        other => panic!("blocked URL should be a tool error, got: {other:?}"),
    };
    assert!(
        message.contains("blocked by system policy"),
        "error should name the system policy, got: {message}"
    );
    assert!(
        message.contains("blocked.example.com"),
        "error should include the URL, got: {message}"
    );
}

#[tokio::test]
async fn legacy_path_system_policy_error_wins_when_both_policies_deny() {
    use everruns_contracts::runtime::{SystemAllowlist, SystemEgressPolicy};

    let mut tool = tool_for_wiremock();
    tool.system_policy = Some(Arc::new(SystemEgressPolicy::allowlist_only(
        SystemAllowlist::from_toml("[groups.test]\nallowed = [\"allowed.example.com\"]\n")
            .map(Arc::new)
            .unwrap(),
    )));
    // No egress service → legacy path; ACL denies the URL too.
    let mut context = ToolContext::new(SessionId::new());
    context.network_access = Some(self::network_access::NetworkAccessList::allow_only([
        "allowed.example.com",
    ]));

    let result = tool
        .execute_with_context(
            serde_json::json!({ "url": "https://blocked.example.com/x" }),
            &context,
        )
        .await;

    assert!(
        matches!(
            &result,
            ToolExecutionResult::ToolError(msg) if msg.contains("blocked by system policy")
        ),
        "operator-level system policy error should take precedence, got: {result:?}"
    );
}

/// In-memory SessionFileSystem for testing file downloads
struct MockFileStore {
    files: tokio::sync::Mutex<std::collections::HashMap<(SessionId, String), (String, String)>>,
    directories: tokio::sync::Mutex<std::collections::HashSet<(SessionId, String)>>,
}

impl MockFileStore {
    fn new() -> Self {
        Self {
            files: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            directories: tokio::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    async fn add_directory(&self, session_id: SessionId, path: &str) {
        self.directories
            .lock()
            .await
            .insert((session_id, path.to_string()));
    }

    async fn get_file(&self, session_id: SessionId, path: &str) -> Option<(String, String)> {
        self.files
            .lock()
            .await
            .get(&(session_id, path.to_string()))
            .cloned()
    }
}

#[async_trait]
impl SessionFileSystem for MockFileStore {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> self::error::Result<Option<self::session_file::SessionFile>> {
        let guard = self.files.lock().await;
        if let Some((content, encoding)) = guard.get(&(session_id, path.to_string())) {
            Ok(Some(self::session_file::SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                content: Some(content.clone()),
                encoding: encoding.clone(),
                size_bytes: content.len() as i64,
                is_directory: false,
                is_readonly: false,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        } else {
            Ok(None)
        }
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> self::error::Result<self::session_file::SessionFile> {
        self.files.lock().await.insert(
            (session_id, path.to_string()),
            (content.to_string(), encoding.to_string()),
        );
        Ok(self::session_file::SessionFile {
            id: uuid::Uuid::new_v4(),
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            content: Some(content.to_string()),
            encoding: encoding.to_string(),
            size_bytes: content.len() as i64,
            is_directory: false,
            is_readonly: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }

    async fn delete_file(
        &self,
        _session_id: SessionId,
        _path: &str,
        _recursive: bool,
    ) -> self::error::Result<bool> {
        Ok(false)
    }

    async fn list_directory(
        &self,
        _session_id: SessionId,
        _path: &str,
    ) -> self::error::Result<Vec<self::session_file::FileInfo>> {
        Ok(vec![])
    }

    async fn stat_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> self::error::Result<Option<self::session_file::FileStat>> {
        if self
            .directories
            .lock()
            .await
            .contains(&(session_id, path.to_string()))
        {
            return Ok(Some(self::session_file::FileStat {
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }));
        }
        Ok(None)
    }

    async fn grep_files(
        &self,
        _session_id: SessionId,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> self::error::Result<Vec<self::session_file::GrepMatch>> {
        Ok(vec![])
    }

    async fn create_directory(
        &self,
        _session_id: SessionId,
        _path: &str,
    ) -> self::error::Result<self::session_file::FileInfo> {
        unimplemented!()
    }
}

#[tokio::test]
async fn crawl_with_network_policy_requires_egress_service() {
    let tool = WebFetchTool::default();
    let mut context = ToolContext::new(SessionId::new());
    context.network_access = Some(self::network_access::NetworkAccessList::allow_only([
        "https://example.com/api/",
    ]));

    let result = tool
        .execute_with_context(
            serde_json::json!({
                "url": "https://example.com/api/index.html",
                "crawl": true
            }),
            &context,
        )
        .await;

    assert!(matches!(
        result,
        ToolExecutionResult::ToolError(message)
            if message == "Crawl requires an egress service when network policy is active"
    ));
}

#[tokio::test]
async fn test_blank_save_to_file_fetches_inline_without_file_store() {
    let tool = WebFetchTool::new(false, None);
    let mut context = ToolContext::new(SessionId::new());
    context.egress_service = Some(Arc::new(CannedEgress::default()));

    let result = tool
        .execute_with_context(
            serde_json::json!({
                "url": "http://93.184.216.34/file.txt",
                "save_to_file": "  \n "
            }),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("blank save_to_file should fetch inline: {result:?}");
    };
    assert_eq!(value["content"], "pong from egress");
    assert!(value.get("saved_path").is_none() || value["saved_path"].is_null());
}

#[test]
fn test_save_error_remains_distinct_from_http_errors() {
    let result = fetch_error::map_error(FetchError::SaveError(
        "Path not allowed: Destination is an existing directory: /downloads".to_string(),
    ));
    assert!(matches!(
        result,
        ToolExecutionResult::ToolError(message)
            if message == "Failed to save file: Path not allowed: Destination is an existing directory: /downloads"
    ));
}

#[tokio::test]
async fn test_session_file_saver_rejects_workspace_roots_and_directories() {
    let file_store = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    file_store.add_directory(session_id, "/downloads").await;
    let saver = SessionFileSaver {
        file_store: file_store.clone(),
        session_id,
    };

    for path in ["", "   ", "/", "/workspace", "/workspace/"] {
        let error = saver.validate_path(path).await.unwrap_err();
        assert!(matches!(
            saver.save(path, b"must not write").await,
            Err(FileSaveError::PathNotAllowed(_))
        ));
        assert!(
            matches!(error, FileSaveError::PathNotAllowed(_)),
            "root destination {path:?} should be a path error: {error}"
        );
    }

    assert!(matches!(
        saver.save("/downloads", b"must not write").await,
        Err(FileSaveError::PathNotAllowed(_))
    ));
    assert!(file_store.files.lock().await.is_empty());
    let error = saver.validate_path("/downloads").await.unwrap_err();
    assert!(
        matches!(error, FileSaveError::PathNotAllowed(ref message) if message.contains("directory")),
        "directory destination should be a precise path error: {error}"
    );
}

#[tokio::test]
async fn test_session_file_saver_resolves_and_saves_valid_path() {
    let file_store = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let saver = SessionFileSaver {
        file_store: file_store.clone(),
        session_id,
    };

    saver.validate_path("downloads/file.txt").await.unwrap();
    let saved = saver
        .save("downloads/file.txt", b"downloaded")
        .await
        .unwrap();

    assert_eq!(saved.path, "/downloads/file.txt");
    assert_eq!(saved.bytes_written, 10);
    assert_eq!(
        file_store.get_file(session_id, "/downloads/file.txt").await,
        Some(("downloaded".to_string(), "text".to_string()))
    );
}

#[tokio::test]
async fn test_save_to_file_without_context_strips_save() {
    // When execute() is called (no context), save_to_file should be ignored
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("hello")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let tool = tool_for_wiremock();
    let result = tool
        .execute(serde_json::json!({
            "url": format!("{}/file.txt", mock_server.uri()),
            "save_to_file": "/downloads/file.txt"
        }))
        .await;

    // Should succeed with inline content (save_to_file stripped)
    if let ToolExecutionResult::Success(value) = result {
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["content"], "hello");
        assert!(value.get("saved_path").is_none() || value["saved_path"].is_null());
    } else {
        panic!("Expected successful response, got: {:?}", result);
    }
}

// ========================================================================
// Egress-backed path (knowledge/operations/egress.md migration step 3)
//
// URLs use public IP literals so `validate_url_dns_pinned` passes without
// DNS; the egress mock never performs real network I/O.
// ========================================================================

/// Canned egress service: always returns 200 text/plain "pong from egress".
#[derive(Default)]
struct CannedEgress {
    requests: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl self::egress::EgressService for CannedEgress {
    async fn send(
        &self,
        request: self::egress::EgressRequest,
    ) -> self::egress::EgressResult<self::egress::EgressResponse> {
        self.requests.lock().unwrap().push(request.url);
        Ok(self::egress::EgressResponse {
            status: 200,
            headers: [("content-type".to_string(), "text/plain".to_string())]
                .into_iter()
                .collect(),
            body: b"pong from egress".to_vec(),
        })
    }

    async fn send_stream(
        &self,
        request: self::egress::EgressRequest,
    ) -> self::egress::EgressResult<self::egress::EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(self::egress::EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
        })
    }
}

struct RedirectingEgress {
    requests: std::sync::Mutex<Vec<String>>,
}

impl RedirectingEgress {
    fn requested_urls(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl self::egress::EgressService for RedirectingEgress {
    async fn send(
        &self,
        request: self::egress::EgressRequest,
    ) -> self::egress::EgressResult<self::egress::EgressResponse> {
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request.url);
        // The final host returns 200; only the initial host issues the
        // cross-host redirect. Returning 200 on the final URL keeps the test
        // deterministic — if the same-host-only policy ever regressed, the
        // second hop would terminate here instead of self-redirecting.
        if url == "http://93.184.216.35/final" {
            return Ok(self::egress::EgressResponse {
                status: 200,
                headers: [("content-type".to_string(), "text/plain".to_string())]
                    .into_iter()
                    .collect(),
                body: b"final".to_vec(),
            });
        }
        Ok(self::egress::EgressResponse {
            status: 302,
            headers: [(
                "location".to_string(),
                "http://93.184.216.35/final".to_string(),
            )]
            .into_iter()
            .collect(),
            body: Vec::new(),
        })
    }

    async fn send_stream(
        &self,
        request: self::egress::EgressRequest,
    ) -> self::egress::EgressResult<self::egress::EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(self::egress::EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
        })
    }
}

#[tokio::test]
async fn test_execute_with_context_routes_through_egress() {
    let tool = WebFetchTool::default();
    let mut context = ToolContext::new(SessionId::new());
    let egress = Arc::new(CannedEgress::default());
    context.egress_service = Some(egress.clone());

    let result = tool
        .execute_with_context(
            serde_json::json!({ "url": "http://93.184.216.34/ping" }),
            &context,
        )
        .await;

    if let ToolExecutionResult::Success(value) = result {
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["content"], "pong from egress");
    } else {
        panic!("Expected successful egress-path response, got: {result:?}");
    }
    assert_eq!(
        *egress.requests.lock().unwrap(),
        vec!["http://93.184.216.34/ping".to_string()]
    );
}

#[tokio::test]
async fn test_egress_path_system_allowlist_blocks_cross_host_redirect_before_second_hop() {
    use everruns_contracts::runtime::{SystemAllowlist, SystemEgressPolicy};

    let tool = WebFetchTool {
        system_policy: Some(Arc::new(SystemEgressPolicy::allowlist_only(
            SystemAllowlist::from_toml("[groups.test]\nallowed = [\"93.184.216.34\"]\n")
                .map(Arc::new)
                .unwrap(),
        ))),
        ..Default::default()
    };
    let egress = Arc::new(RedirectingEgress {
        requests: std::sync::Mutex::new(Vec::new()),
    });
    let mut context = ToolContext::new(SessionId::new());
    context.egress_service = Some(egress.clone());

    let result = tool
        .execute_with_context(
            serde_json::json!({ "url": "http://93.184.216.34/start" }),
            &context,
        )
        .await;

    assert!(
        matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("blocked")),
        "expected cross-host redirect denial, got: {result:?}"
    );
    assert_eq!(
        egress.requested_urls(),
        vec!["http://93.184.216.34/start"],
        "redirect target must be rejected before a second egress hop can resolve it"
    );
}

#[tokio::test]
async fn test_egress_path_denies_url_outside_network_access_list() {
    let tool = WebFetchTool::default();
    let mut context = ToolContext::new(SessionId::new());
    let egress = Arc::new(CannedEgress::default());
    context.egress_service = Some(egress.clone());
    context.network_access = Some(self::network_access::NetworkAccessList::allow_only([
        "allowed.example.com",
    ]));

    let result = tool
        .execute_with_context(
            serde_json::json!({ "url": "http://93.184.216.34/ping" }),
            &context,
        )
        .await;

    assert!(
        matches!(
            &result,
            ToolExecutionResult::ToolError(msg) if msg.contains("blocked by network access policy")
        ),
        "expected network access denial, got: {result:?}"
    );
    assert_eq!(*egress.requests.lock().unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn test_egress_path_blocks_private_address_before_sending() {
    let tool = WebFetchTool::default();
    let mut context = ToolContext::new(SessionId::new());
    let egress = Arc::new(CannedEgress::default());
    context.egress_service = Some(egress.clone());

    let result = tool
        .execute_with_context(
            serde_json::json!({ "url": "http://169.254.169.254/latest/meta-data/" }),
            &context,
        )
        .await;

    assert!(
        matches!(
            &result,
            ToolExecutionResult::ToolError(msg) if msg.contains("blocked")
        ),
        "expected SSRF block on egress path, got: {result:?}"
    );
    assert_eq!(*egress.requests.lock().unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn test_egress_path_save_to_file_writes_session_file() {
    let tool = WebFetchTool::new(true, None);
    let file_store = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let mut context = ToolContext::with_file_store(session_id, file_store.clone());
    let egress = Arc::new(CannedEgress::default());
    context.egress_service = Some(egress.clone());

    let result = tool
        .execute_with_context(
            serde_json::json!({
                "url": "http://93.184.216.34/file.txt",
                "save_to_file": "/downloads/file.txt"
            }),
            &context,
        )
        .await;

    if let ToolExecutionResult::Success(value) = result {
        assert_eq!(value["saved_path"], "/downloads/file.txt");
        assert_eq!(value["bytes_written"], 16);
        assert!(value.get("content").is_none());
        let (content, encoding) = file_store
            .get_file(session_id, "/downloads/file.txt")
            .await
            .expect("file should have been written via egress path");
        assert_eq!(encoding, "text");
        assert_eq!(content, "pong from egress");
    } else {
        panic!("Expected successful response, got: {result:?}");
    }
    assert_eq!(
        *egress.requests.lock().unwrap(),
        vec!["http://93.184.216.34/file.txt".to_string()]
    );
}
fn successful(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected a successful HTTP response, got {other:?}"),
    }
}

#[tokio::test]
async fn http_conversions_preserve_complete_content_and_response_metadata() {
    let server = MockServer::builder().start().await;
    let html = "<html><body><h1>Title</h1><p>Body</p></body></html>";
    let cases = [
        (
            "/",
            html,
            "text/html; charset=utf-8",
            serde_json::json!({}),
            "raw",
            html,
        ),
        (
            "/nested/document.html",
            html,
            "text/html",
            serde_json::json!({"as_text":true}),
            "text",
            "Title\n\nBody",
        ),
        (
            "/repository/readme",
            html,
            "text/html",
            serde_json::json!({"as_markdown":true}),
            "markdown",
            "# Title\n\nBody",
        ),
        (
            "/data.json",
            "{\"key\":\"value\"}",
            "application/json",
            serde_json::json!({}),
            "raw",
            "{\"key\":\"value\"}",
        ),
        (
            "/newlines",
            "line1\n\n\n\n\nline2",
            "text/plain",
            serde_json::json!({}),
            "raw",
            "line1\n\nline2",
        ),
    ];
    let tool = tool_for_wiremock();
    for (route, body, content_type, mut arguments, format, content) in cases {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(body, content_type)
                    .insert_header("etag", "\"fixture-v1\"")
                    .insert_header("last-modified", "Wed, 15 Jul 2026 12:00:00 GMT"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}{route}", server.uri());
        arguments["url"] = serde_json::json!(url);
        let value = successful(tool.execute(arguments).await);
        assert_eq!(value["url"], url);
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["content_type"], content_type);
        assert_eq!(value["size"], body.len());
        assert_eq!(value["format"], format);
        assert_eq!(value["content"], content, "route {route}");
        assert_eq!(value["etag"], "\"fixture-v1\"");
        assert_eq!(value["last_modified"], "Wed, 15 Jul 2026 12:00:00 GMT");
        for absent in [
            "truncated",
            "method",
            "error",
            "saved_path",
            "bytes_written",
        ] {
            assert!(value.get(absent).is_none(), "unexpected {absent}: {value}");
        }
    }
}

#[tokio::test]
async fn head_and_binary_responses_preserve_metadata_without_inline_content() {
    let server = MockServer::builder().start().await;
    let tool = tool_for_wiremock();
    for (verb, route, content_type, size) in [
        ("HEAD", "/head", "text/html", 5000),
        ("GET", "/image.png", "image/png", 4),
    ] {
        let response = ResponseTemplate::new(200)
            .insert_header("content-type", content_type)
            .insert_header("content-length", size.to_string())
            .insert_header("etag", "\"v2\"");
        let response = if verb == "GET" {
            response.set_body_bytes(vec![0x89, 0x50, 0x4e, 0x47])
        } else {
            response
        };
        Mock::given(method(verb))
            .and(path(route))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}{route}", server.uri());
        let value = successful(
            tool.execute(serde_json::json!({"url":url,"method":verb}))
                .await,
        );
        assert_eq!(value["url"], url);
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["content_type"], content_type);
        assert_eq!(value["size"], size);
        assert_eq!(value["etag"], "\"v2\"");
        for absent in ["content", "format", "saved_path", "bytes_written"] {
            assert!(value.get(absent).is_none(), "unexpected {absent}: {value}");
        }
        if verb == "HEAD" {
            assert_eq!(value["method"], "HEAD");
            assert!(value.get("error").is_none());
        } else {
            assert!(value.get("method").is_none());
            assert_eq!(
                value["error"],
                "Binary content is not supported. Only textual content (HTML, text, JSON, etc.) can be fetched."
            );
        }
    }
}

#[tokio::test]
async fn http_error_statuses_preserve_the_response_body() {
    let server = MockServer::builder().start().await;
    let tool = tool_for_wiremock();
    for (status, body) in [(404, "Not Found"), (500, "Internal Server Error")] {
        let route = format!("/status/{status}");
        Mock::given(method("GET"))
            .and(path(&route))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(body)
                    .insert_header("content-type", "text/plain"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}{route}", server.uri());
        let value = successful(tool.execute(serde_json::json!({"url":url})).await);
        assert_eq!(value["url"], url);
        assert_eq!(value["status_code"], status);
        assert_eq!(value["content"], body);
        assert_eq!(value["size"], body.len());
        assert_eq!(value["format"], "raw");
    }
}

struct FailedTransport {
    kind: u8,
    requests: std::sync::Mutex<Vec<String>>,
}
#[async_trait]
impl fetchkit::HttpTransport for FailedTransport {
    async fn execute(
        &self,
        request: fetchkit::TransportRequest,
    ) -> Result<fetchkit::TransportResponse, fetchkit::TransportError> {
        self.requests.lock().unwrap().push(request.url.to_string());
        Err(match self.kind {
            0 => fetchkit::TransportError::Timeout,
            1 => fetchkit::TransportError::Connect,
            2 => fetchkit::TransportError::Request("connection reset".to_string()),
            _ => fetchkit::TransportError::Other("transport unavailable".to_string()),
        })
    }
}

#[tokio::test]
async fn transport_failures_are_errors_on_both_execution_paths() {
    // Exercise the complete tool/FetchKit/error mapping without DNS or wall-clock waits.
    let url = "http://93.184.216.34/failure";
    for (kind, expected) in [
        (
            0,
            "Request timed out: server did not respond within 1 second",
        ),
        (1, "Request failed: failed to connect to server"),
        (2, "Request failed: connection reset"),
        (3, "Request failed: transport unavailable"),
    ] {
        let transport = Arc::new(FailedTransport {
            kind,
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let builder = fetchkit::Tool::builder().transport(transport.clone());
        let tool = WebFetchTool {
            fetchkit_tool: builder.build(),
            builder,
            description: String::new(),
            enable_save_to_file: false,
            system_policy: None,
        };
        for with_context in [false, true] {
            let args = serde_json::json!({"url":url});
            let result = if with_context {
                tool.execute_with_context(args, &ToolContext::new(SessionId::new()))
                    .await
            } else {
                tool.execute(args).await
            };
            assert!(
                matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
                "kind {kind}, context {with_context}: {result:?}"
            );
        }
        assert_eq!(
            *transport.requests.lock().unwrap(),
            vec![url.to_string(), url.to_string()]
        );
    }
}
#[tokio::test]
async fn invalid_arguments_and_schemes_fail_before_network_io() {
    let server = MockServer::builder().start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("unexpected request"))
        .expect(0)
        .mount(&server)
        .await;
    let tool = tool_for_wiremock();
    let mut cases = vec![
        (serde_json::json!({}), "Missing required parameter: url"),
        (
            serde_json::json!({"url":false}),
            "Missing required parameter: url",
        ),
        (
            serde_json::json!({"url":"not-a-valid-url"}),
            "Invalid URL: must start with http:// or https://",
        ),
    ];
    for url in [
        "file:///etc/passwd",
        "ftp://example.com/file.txt",
        "gopher://internal-server/",
    ] {
        cases.push((
            serde_json::json!({"url":url}),
            "Invalid URL: must start with http:// or https://",
        ));
    }
    for method in [
        serde_json::json!("TRACE"),
        serde_json::json!(""),
        serde_json::json!(true),
        serde_json::json!(7),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        cases.push((
            serde_json::json!({"url":server.uri(),"method":method}),
            request::INVALID_METHOD,
        ));
    }
    for (args, expected) in cases {
        for with_context in [false, true] {
            let result = if with_context {
                tool.execute_with_context(args.clone(), &ToolContext::new(SessionId::new()))
                    .await
            } else {
                tool.execute(args.clone()).await
            };
            assert!(
                matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
                "arguments {args}, context {with_context}: {result:?}"
            );
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn request_normalization_preserves_all_supported_fields() {
    let input = serde_json::json!({
        "url":"https://example.com/docs?a=1","method":"hEaD","as_markdown":true,"as_text":false,
        "save_to_file":"  /downloads/file.txt \n", "content_focus":"agent","crawl":true,"max_pages":3,
        "if_none_match":"\"abc123\"","if_modified_since":"Wed, 15 Jul 2026 12:00:00 GMT"
    });
    let expected = serde_json::json!({
        "url":"https://example.com/docs?a=1","method":"HEAD","as_markdown":true,"as_text":false,
        "save_to_file":"/downloads/file.txt", "content_focus":"agent","crawl":true,"max_pages":3,
        "if_none_match":"\"abc123\"","if_modified_since":"Wed, 15 Jul 2026 12:00:00 GMT"
    });
    assert_eq!(
        serde_json::to_value(WebFetchTool::parse_request(&input).unwrap()).unwrap(),
        expected
    );
    for arguments in [
        serde_json::json!({"url":"https://example.com"}),
        serde_json::json!({"url":"https://example.com","method":null}),
        serde_json::json!({"url":"https://example.com","method":"get","save_to_file":" \n\t "}),
    ] {
        assert_eq!(
            serde_json::to_value(WebFetchTool::parse_request(&arguments).unwrap()).unwrap(),
            serde_json::json!({"url":"https://example.com","method":"GET"})
        );
    }
}

#[tokio::test]
async fn private_address_families_are_blocked_on_both_execution_paths() {
    // Literal addresses do not require external DNS. A policy failure must
    // never be accepted as a connection failure or successful empty response.
    let tool = WebFetchTool::default();
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://127.0.0.1:1/",
        "http://10.0.0.1:1/",
        "http://172.16.0.1:1/",
        "http://192.168.0.1:1/",
        "http://[::1]:1/",
        "http://0.0.0.0:1/",
    ] {
        for with_context in [false, true] {
            let args = serde_json::json!({"url":url});
            let result = if with_context {
                tool.execute_with_context(args, &ToolContext::new(SessionId::new()))
                    .await
            } else {
                tool.execute(args).await
            };
            assert!(
                matches!(&result,ToolExecutionResult::ToolError(message) if message=="URL is blocked by policy"),
                "{url}, context {with_context}: {result:?}"
            );
        }
    }
}
#[tokio::test]
async fn downloads_store_exact_bytes_only_in_the_requested_session() {
    let server = MockServer::builder().start().await;
    let tool = tool_for_wiremock();
    let store = Arc::new(MockFileStore::new());
    let session = SessionId::new();
    let other_session = SessionId::new();
    let context = ToolContext::with_file_store(session, store.clone());
    for (route, content_type, bytes, encoding) in [
        (
            "/data.json",
            "application/json",
            b"{\"key\":\"value\"}".to_vec(),
            "text",
        ),
        (
            "/image.png",
            "image/png",
            vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe],
            "base64",
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(bytes.clone(), content_type))
            .expect(1)
            .mount(&server)
            .await;
        let destination = format!("/downloads{route}");
        let url = format!("{}{route}", server.uri());
        let value = successful(
            tool.execute_with_context(
                serde_json::json!({"url":url,"save_to_file":destination}),
                &context,
            )
            .await,
        );
        assert_eq!(value["url"], url);
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["saved_path"], destination);
        assert_eq!(value["bytes_written"], bytes.len());
        assert!(value.get("content").is_none());
        assert!(value.get("error").is_none());
        let (stored, stored_encoding) = store
            .get_file(session, &destination)
            .await
            .expect("saved file");
        assert_eq!(stored_encoding, encoding);
        let actual = if encoding == "base64" {
            base64::engine::general_purpose::STANDARD
                .decode(stored)
                .unwrap()
        } else {
            stored.into_bytes()
        };
        assert_eq!(actual, bytes);
        assert!(store.get_file(other_session, &destination).await.is_none());
    }
    assert_eq!(store.files.lock().await.len(), 2);
}

#[tokio::test]
async fn download_gates_reject_before_network_or_file_writes() {
    let server = MockServer::builder().start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("must not fetch"))
        .expect(0)
        .mount(&server)
        .await;
    let store = Arc::new(MockFileStore::new());
    let session = SessionId::new();
    for (enabled, with_store, expected) in [
        (false, true, "File download is disabled for this capability"),
        (true, false, "File system not available in this context"),
    ] {
        let mut tool = tool_for_wiremock();
        tool.enable_save_to_file = enabled;
        let context = if with_store {
            ToolContext::with_file_store(session, store.clone())
        } else {
            ToolContext::new(session)
        };
        let result = tool
            .execute_with_context(
                serde_json::json!({"url":server.uri(),"save_to_file":"/downloads/file.txt"}),
                &context,
            )
            .await;
        assert!(
            matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
            "{result:?}"
        );
    }
    store.add_directory(session, "/downloads").await;
    let context = ToolContext::with_file_store(session, store.clone());
    for (destination, diagnostic) in [
        ("/", "workspace root"),
        ("/workspace", "workspace root"),
        ("/downloads", "existing directory"),
    ] {
        let result = tool_for_wiremock()
            .execute_with_context(
                serde_json::json!({"url":server.uri(),"save_to_file":destination}),
                &context,
            )
            .await;
        assert!(
            matches!(&result, ToolExecutionResult::ToolError(message) if message.starts_with("Failed to save file:") && message.contains(diagnostic)),
            "{destination}: {result:?}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(store.files.lock().await.is_empty());
}
#[test]
fn bot_auth_jwk_matches_the_known_ed25519_vector_and_fetchkit_identity() {
    // RFC 8032 section 7.1, test 1; thumbprint uses RFC 7638 canonical JWK.
    let seed = "nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A";
    let key = derive_bot_auth_public_key(seed).unwrap();
    assert_eq!(
        key.jwk,
        serde_json::json!({"kty":"OKP","crv":"Ed25519","x":"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"})
    );
    assert_eq!(key.key_id, "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
    assert_eq!(
        key.key_id,
        BotAuthConfig::from_base64_seed(seed).unwrap().keyid()
    );
    for invalid in [
        "!!!invalid!!!".to_string(),
        "tooshort".to_string(),
        format!("{seed}="),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 31]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 33]),
    ] {
        assert!(derive_bot_auth_public_key(&invalid).is_none(), "{invalid}");
    }
}

#[tokio::test]
async fn capability_configuration_aligns_schema_prompt_and_download_behavior() {
    let capability = WebFetchCapability::new(None);
    assert_eq!(capability.id(), "web_fetch");
    assert_eq!(capability.risk_level(), RiskLevel::High);
    assert!(capability.system_prompt_addition().is_none());
    assert!(
        capability
            .system_prompt_preview()
            .unwrap()
            .contains("save_to_file")
    );
    let config_schema = capability.config_schema().unwrap();
    assert_eq!(
        config_schema["properties"]["enable_file_download"]["type"],
        "boolean"
    );
    assert_eq!(
        config_schema["properties"]["enable_file_download"]["default"],
        false
    );
    for invalid in [
        serde_json::json!([]),
        serde_json::json!(true),
        serde_json::json!("true"),
        serde_json::json!({"enable_file_download":null}),
        serde_json::json!({"enable_file_download":"true"}),
    ] {
        assert!(capability.validate_config(&invalid).is_err(), "{invalid}");
    }
    for (config, enabled) in [
        (serde_json::Value::Null, false),
        (serde_json::json!({}), false),
        (serde_json::json!({"enable_file_download":false}), false),
        (serde_json::json!({"enable_file_download":true}), true),
    ] {
        capability.validate_config(&config).unwrap();
        let tools = capability.tools_with_config(&config);
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.name(), "web_fetch");
        assert!(tool.requires_context());
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], serde_json::json!(["url"]));
        assert_eq!(
            schema["properties"]["method"],
            serde_json::json!({"type":"string","enum":["GET","HEAD","POST","PUT","PATCH","DELETE","OPTIONS"],"default":"GET","description":request::extend_schema(serde_json::json!({"properties":{}}))["properties"]["method"]["description"]})
        );
        for (name, kind) in [
            ("url", "string"),
            ("as_markdown", "boolean"),
            ("as_text", "boolean"),
            ("content_focus", "string"),
            ("crawl", "boolean"),
            ("max_pages", "integer"),
        ] {
            assert_eq!(schema["properties"][name]["type"], kind, "{name}");
        }
        assert_eq!(schema["properties"].get("save_to_file").is_some(), enabled);
        assert!(schema["properties"].get("render").is_none());
        if !enabled {
            let defaults = capability.tools();
            assert_eq!(defaults.len(), 1);
            assert_eq!(defaults[0].parameters_schema(), schema);
        }
        let session = SessionId::new();
        let prompt = capability
            .system_prompt_contribution_with_config(
                &SystemPromptContext::without_file_store(session),
                &config,
            )
            .await
            .unwrap();
        assert_eq!(prompt.contains("save_to_file"), enabled);
        assert!(prompt.len() <= if enabled { 350 } else { 250 });
        assert!(prompt.contains("GET/HEAD"));
        assert!(prompt.contains("not a search engine"));
        let store = Arc::new(MockFileStore::new());
        let mut context = ToolContext::with_file_store(session, store.clone());
        context.egress_service = Some(Arc::new(CannedEgress::default()));
        let result=tool.execute_with_context(serde_json::json!({"url":"http://93.184.216.34/config","save_to_file":"/config.txt"}),&context).await;
        if enabled {
            assert_eq!(successful(result)["saved_path"], "/config.txt");
            assert_eq!(
                store.get_file(session, "/config.txt").await,
                Some(("pong from egress".to_string(), "text".to_string()))
            );
        } else {
            assert!(
                matches!(result,ToolExecutionResult::ToolError(message) if message=="File download is disabled for this capability")
            );
            assert!(store.files.lock().await.is_empty());
        }
    }
}
