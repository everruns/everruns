//! Socket Mode shares the host HTTP client's proxy and TLS settings.
//! Keep ticket-bearing URLs out of diagnostics and validate the HTTP upgrade
//! before treating bytes as authenticated provider frames.
use reqwest::{
    Client, ClientBuilder,
    header::{
        CONNECTION, HeaderMap, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_EXTENSIONS, SEC_WEBSOCKET_KEY,
        SEC_WEBSOCKET_PROTOCOL, SEC_WEBSOCKET_VERSION, UPGRADE,
    },
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        handshake::{client::generate_key, derive_accept_key},
        protocol::{Role, WebSocketConfig},
    },
};

use super::socket_mode::SocketError;

fn client_builder() -> ClientBuilder {
    Client::builder()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(20))
}

pub(super) async fn connect(url: &str) -> Result<WebSocketStream<reqwest::Upgraded>, SocketError> {
    let client = client_builder()
        .build()
        .map_err(|_| SocketError::Transport("Slack socket HTTP client unavailable".into()))?;
    connect_with_client(&client, url).await
}

fn connection_upgrade(headers: &HeaderMap) -> bool {
    headers.get_all(CONNECTION).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
    })
}

async fn connect_with_client(
    client: &Client,
    url: &str,
) -> Result<WebSocketStream<reqwest::Upgraded>, SocketError> {
    let mut endpoint = reqwest::Url::parse(url)
        .map_err(|_| SocketError::Transport("Invalid Slack socket URL".into()))?;
    if endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(SocketError::Transport("Invalid Slack socket URL".into()));
    }
    let scheme = match endpoint.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err(SocketError::Transport("Invalid Slack socket scheme".into())),
    };
    endpoint
        .set_scheme(scheme)
        .map_err(|_| SocketError::Transport("Invalid Slack socket URL".into()))?;
    let key = generate_key();
    let response = client
        .get(endpoint)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_VERSION, "13")
        .header(SEC_WEBSOCKET_KEY, &key)
        .send()
        .await
        .map_err(|_| SocketError::Transport("Slack socket upgrade unavailable".into()))?;
    let headers = response.headers();
    if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS
        || !connection_upgrade(headers)
        || !headers
            .get(UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"))
        || headers.get_all(SEC_WEBSOCKET_ACCEPT).iter().count() != 1
        || headers
            .get(SEC_WEBSOCKET_ACCEPT)
            .and_then(|value| value.to_str().ok())
            != Some(derive_accept_key(key.as_bytes()).as_str())
        || headers.contains_key(SEC_WEBSOCKET_EXTENSIONS)
        || headers.contains_key(SEC_WEBSOCKET_PROTOCOL)
    {
        return Err(SocketError::Transport(
            "Invalid Slack socket upgrade response".into(),
        ));
    }
    let upgraded = response
        .upgrade()
        .await
        .map_err(|_| SocketError::Transport("Slack socket upgrade failed".into()))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(channelkit::MAX_WEBHOOK_BYTES))
        .max_frame_size(Some(channelkit::MAX_WEBHOOK_BYTES));
    Ok(WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::{Duration, timeout},
    };
    use tokio_tungstenite::tungstenite::{
        Message,
        handshake::server::{Callback, ErrorResponse, Request, Response},
    };

    struct VerifyProxyTarget;
    impl Callback for VerifyProxyTarget {
        fn on_request(
            self,
            request: &Request,
            response: Response,
        ) -> Result<Response, ErrorResponse> {
            assert_eq!(request.uri().host(), Some("provider.invalid"));
            assert_eq!(request.uri().path(), "/link");
            assert!(request.headers().get("authorization").is_none());
            Ok(response)
        }
    }

    async fn http_response(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() <= 8192);
            }
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("ws://{address}/link?ticket=synthetic-private-ticket")
    }

    #[tokio::test]
    async fn an_explicit_http_proxy_carries_the_upgrade_and_masked_frames() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = proxy.local_addr().unwrap();
        let worker = tokio::spawn(async move {
            let (socket, _) = proxy.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(socket, VerifyProxyTarget)
                .await
                .unwrap();
            socket.send(Message::Text("hello".into())).await.unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap().into_text().unwrap(),
                "ack"
            );
        });
        let client = client_builder()
            .no_proxy()
            .proxy(reqwest::Proxy::http(format!("http://{address}")).unwrap())
            .build()
            .unwrap();
        let connected = tokio::spawn(async move {
            connect_with_client(
                &client,
                "ws://provider.invalid/link?ticket=synthetic-private-ticket",
            )
            .await
        });
        let mut socket = timeout(Duration::from_secs(5), connected)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap(),
            "hello"
        );
        socket.send(Message::Text("ack".into())).await.unwrap();
        timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_bad_accept_key_is_rejected_without_exposing_the_ticket() {
        let url = http_response(
            "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid\r\n\r\n".into(),
        )
        .await;
        let client = client_builder().no_proxy().build().unwrap();
        let error = timeout(Duration::from_secs(5), connect_with_client(&client, &url))
            .await
            .unwrap()
            .expect_err("the server's handshake key must match");
        assert_eq!(error.to_string(), "Invalid Slack socket upgrade response");
        assert!(!error.to_string().contains("synthetic-private-ticket"));
    }

    #[tokio::test]
    async fn url_credentials_and_fragments_are_rejected_before_connecting() {
        let client = client_builder().no_proxy().build().unwrap();
        for url in [
            "ws://provider.invalid/link#synthetic-private-ticket",
            "ws://synthetic-private-ticket@provider.invalid/link",
            "wss://user:synthetic-private-ticket@provider.invalid/link",
        ] {
            let error = connect_with_client(&client, url).await.err().unwrap();
            assert_eq!(error.to_string(), "Invalid Slack socket URL");
        }
    }

    #[tokio::test]
    async fn redirects_do_not_forward_the_ticket_to_another_endpoint() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = target.local_addr().unwrap();
        let reached = Arc::new(AtomicBool::new(false));
        let target_reached = reached.clone();
        let worker = tokio::spawn(async move {
            let (mut socket, _) = target.accept().await.unwrap();
            target_reached.store(true, Ordering::SeqCst);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let url = http_response(format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{address}/other\r\nContent-Length: 0\r\n\r\n"
        ))
        .await;
        let client = client_builder().no_proxy().build().unwrap();
        assert!(
            timeout(Duration::from_secs(5), connect_with_client(&client, &url))
                .await
                .unwrap()
                .is_err()
        );
        assert!(!reached.load(Ordering::SeqCst));
        worker.abort();
    }

    #[tokio::test]
    async fn oversized_provider_frames_are_rejected_before_dispatch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let worker = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let _ = socket
                .send(Message::Binary(
                    vec![0; channelkit::MAX_WEBHOOK_BYTES + 1].into(),
                ))
                .await;
        });
        let client = client_builder().no_proxy().build().unwrap();
        let mut socket = timeout(
            Duration::from_secs(5),
            connect_with_client(&client, &format!("ws://{address}/link")),
        )
        .await
        .unwrap()
        .unwrap();
        let error = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .expect_err("the provider frame must respect the webhook budget");
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Capacity(_)
        ));
        drop(socket);
        worker.abort();
    }
}
