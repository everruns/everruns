use fetchkit::{FetchError, FetchRequest, Tool};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[tokio::test]
async fn test_proxy_env_is_ignored_by_default_but_can_be_enabled() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("proxied?")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&mock_server)
        .await;

    let _http_proxy = EnvGuard::set("HTTP_PROXY", "http://127.0.0.1:9");
    let _https_proxy = EnvGuard::set("HTTPS_PROXY", "http://127.0.0.1:9");
    let _no_proxy = EnvGuard::set("NO_PROXY", "");

    let request = FetchRequest::new(format!("{}/", mock_server.uri()));

    let direct_tool = Tool::builder().block_private_ips(false).build();
    let direct_response = direct_tool.execute(request.clone()).await.unwrap();
    assert_eq!(direct_response.status_code, 200);
    assert_eq!(direct_response.content.as_deref(), Some("proxied?"));

    let proxied_tool = Tool::builder()
        .block_private_ips(false)
        .respect_proxy_env(true)
        .build();
    let proxied_result = proxied_tool.execute(request).await;
    assert!(matches!(
        proxied_result,
        Err(FetchError::FirstByteTimeout) | Err(FetchError::ConnectError(_))
    ));
}
