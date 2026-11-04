//! Run environment-driven clients in a child process: never mutate test-process env.
use anyhow::Context;
use everruns_contracts::http_transport::HttpClient;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, path},
};

#[tokio::test]
async fn configured_proxy_is_reachable_but_private_urls_are_not_forwarded() -> anyhow::Result<()> {
    let proxy = MockServer::start().await;
    let direct = MockServer::builder()
        .listener(std::net::TcpListener::bind("127.0.0.2:0")?)
        .start()
        .await;
    Mock::given(path("/direct"))
        .respond_with(ResponseTemplate::new(200).set_body_string("direct"))
        .expect(1)
        .mount(&direct)
        .await;
    Mock::given(path("/provider"))
        .and(header("Authorization", "Bearer synthetic-provider-key"))
        .and(header("Proxy-Authorization", "Basic dXNlcjpwYXNz"))
        .respond_with(ResponseTemplate::new(200).set_body_string("proxied"))
        .expect(1)
        .mount(&proxy)
        .await;
    // wiremock expects URL-form request targets and cannot parse CONNECT.
    let tunnel_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let tunnel_url = format!(
        "http://user:pass@localhost:{}",
        tunnel_listener.local_addr()?.port()
    );
    let tunnel = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = tunnel_listener.accept().await?;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 8192);
            request.push(socket.read_u8().await?);
        }
        socket
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await?;
        Ok::<_, anyhow::Error>(String::from_utf8(request)?)
    });
    Mock::given(path("/redirect"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "http://169.254.169.254/latest/meta-data/"),
        )
        .expect(1)
        .mount(&proxy)
        .await;
    // The hostname matters: the old guard mistakes this proxy for the destination.
    let proxy_url = proxy.uri().replace("127.0.0.1", "user:pass@localhost");
    let output = tokio::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "proxy_probe", "--nocapture"])
        .env("EVERRUNS_TEST_PROXY_PROBE", "1")
        .env(
            "EVERRUNS_TEST_DIRECT_URL",
            format!("{}/direct", direct.uri()),
        )
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("HTTPS_PROXY", &tunnel_url)
        .env("https_proxy", &tunnel_url)
        .env("NO_PROXY", "127.0.0.2")
        .env("no_proxy", "127.0.0.2")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("REQUEST_METHOD")
        .output()
        .await?;
    assert!(
        output.status.success(),
        "probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let connect = tokio::time::timeout(std::time::Duration::from_secs(5), tunnel).await???;
    assert!(connect.starts_with("CONNECT provider.invalid:443 HTTP/1.1\r\n"));
    assert!(
        connect
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic dxnlcjpwyxnz\r\n")
    );
    assert!(!connect.contains("synthetic-provider-key"));
    assert_eq!(
        proxy
            .received_requests()
            .await
            .context("proxy request recording")?
            .len(),
        2
    );
    let direct_requests = direct
        .received_requests()
        .await
        .context("direct request recording")?;
    assert_eq!(direct_requests.len(), 1);
    assert!(
        !direct_requests[0]
            .headers
            .contains_key("proxy-authorization")
    );
    proxy.verify().await;
    Ok(())
}

#[tokio::test]
async fn proxy_probe() -> anyhow::Result<()> {
    if std::env::var_os("EVERRUNS_TEST_PROXY_PROBE").is_none() {
        return Ok(());
    }
    let client = HttpClient::default();
    let response = client
        .get("http://provider.invalid/provider")
        .header("Authorization", "Bearer synthetic-provider-key")
        .send()
        .await
        .context("public destination through an operator-configured proxy")?;
    assert_eq!(response.text().await?, "proxied");
    for url in [
        "http://127.0.0.1/private",
        "http://10.0.0.1/private",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]/private",
        "http://localhost/private",
    ] {
        assert!(client.get(url).send().await.is_err(), "accepted {url}");
    }
    let response = client
        .get("http://provider.invalid/redirect")
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 302);
    // The proxy rejects the tunnel deliberately: reaching CONNECT proves HTTPS
    // routing without making an external connection or bypassing TLS validation.
    assert!(
        client
            .get("https://provider.invalid/provider")
            .send()
            .await
            .is_err()
    );
    let direct = client
        .get(&std::env::var("EVERRUNS_TEST_DIRECT_URL")?)
        .send()
        .await?;
    assert_eq!(direct.text().await?, "direct");
    Ok(())
}
