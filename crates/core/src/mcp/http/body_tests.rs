use super::*;
use crate::{EgressResponse, EgressResult, EgressStreamResponse};

struct StreamingOnly;

#[async_trait]
impl EgressService for StreamingOnly {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        panic!("MCP must not buffer an unbounded response through send()")
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert_eq!(request.url, "http://8.8.8.8/mcp");
        assert_eq!(request.timeout_ms, Some(1000));
        Ok(EgressStreamResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: Box::pin(futures::stream::iter([Ok(b"{}".to_vec())])),
        })
    }
}

#[tokio::test]
async fn ordinary_rpc_uses_streaming_egress() {
    let body = http_send_rpc(
        &StreamingOnly,
        "http://8.8.8.8/mcp",
        &HashMap::new(),
        None,
        b"{}".to_vec(),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(body, "{}");
}

fn response(status: u16, chunks: Vec<Vec<u8>>) -> EgressStreamResponse {
    EgressStreamResponse {
        status,
        headers: BTreeMap::new(),
        body: Box::pin(futures::stream::iter(chunks.into_iter().map(Ok))),
    }
}

async fn read(response: EgressStreamResponse, limit: usize) -> Result<RawMcpResponse> {
    body::read(
        response,
        limit,
        false,
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await
}

#[test]
fn response_limit_configuration_requires_positive_bytes() {
    assert_eq!(body::configured_limit(None).unwrap(), 16 * 1024 * 1024);
    assert_eq!(
        body::configured_limit(Some("33554432")).unwrap(),
        32 * 1024 * 1024
    );
    for invalid in ["0", "", "-1", "garbage", "184467440737095516160"] {
        assert!(body::configured_limit(Some(invalid)).is_err());
    }
}

#[tokio::test]
async fn counts_actual_bytes_and_preserves_split_utf8_at_exact_limit() {
    let text = "{\"text\":\"привіт 🌍\"}";
    for length in [None, Some("1"), Some("999999999")] {
        let mut response = response(200, text.as_bytes().iter().map(|b| vec![*b]).collect());
        if let Some(length) = length {
            response
                .headers
                .insert("content-length".into(), length.into());
        }
        assert_eq!(read(response, text.len()).await.unwrap().body, text);
    }
    assert!(
        read(
            response(200, vec![text.as_bytes().to_vec()]),
            text.len() - 1
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("exceeds")
    );
    assert!(read(response(200, vec![vec![0xff]]), 100).await.is_err());
}

#[tokio::test]
async fn large_valid_payload_respects_configured_limit() {
    let text = serde_json::json!({"result": {"content": [{"type": "text", "text": "x".repeat(17 * 1024 * 1024)}]}}).to_string();
    let chunks = || {
        text.as_bytes()
            .chunks(64 * 1024)
            .map(<[u8]>::to_vec)
            .collect()
    };
    assert!(
        read(response(200, chunks()), 16 * 1024 * 1024)
            .await
            .is_err()
    );
    assert_eq!(
        read(response(200, chunks()), 32 * 1024 * 1024)
            .await
            .unwrap()
            .body,
        text
    );
}

struct DropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn endless_success_and_error_streams_are_cancelled_at_the_bound() {
    for status in [200, 401, 403, 500] {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = DropSignal(dropped.clone());
        let response = EgressStreamResponse {
            status,
            headers: BTreeMap::from([("www-authenticate".into(), "Bearer".into())]),
            body: Box::pin(futures::stream::unfold(guard, |guard| async {
                Some((Ok(vec![b'x'; 1024]), guard))
            })),
        };
        let result = read(response, 2048).await;
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        if status == 200 {
            assert!(
                result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("exceeds 2048 bytes")
            );
        } else {
            let result = result.unwrap();
            assert_eq!(result.status, status);
            assert_eq!(result.body.len(), body::MAX_ERROR_BODY_BYTES);
            assert_eq!(result.headers["www-authenticate"], "Bearer");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_body_times_out_without_losing_auth_status() {
    for status in [200, 401, 403, 500] {
        let response = EgressStreamResponse {
            status,
            headers: BTreeMap::new(),
            body: Box::pin(futures::stream::pending()),
        };
        let result = read(response, 100).await;
        if status == 200 {
            assert!(result.err().unwrap().to_string().contains("timed out"));
        } else {
            assert_eq!(result.unwrap().status, status);
        }
    }
}

#[tokio::test]
async fn auth_status_survives_stream_failure_and_invalid_or_truncated_utf8() {
    for status in [401, 403] {
        let mut response = response(
            status,
            vec![
                vec![b'x'; body::MAX_ERROR_BODY_BYTES - 1],
                "€".as_bytes().to_vec(),
            ],
        );
        let result = read(response, 100).await.unwrap();
        assert_eq!(result.status, status);
        assert!(result.body.ends_with('\u{fffd}'));
        response = EgressStreamResponse {
            status,
            headers: BTreeMap::new(),
            body: Box::pin(futures::stream::iter([
                Ok(vec![0xff]),
                Err(crate::EgressError::Transport("broken body".into())),
            ])),
        };
        let result = read(response, 100).await.unwrap();
        assert_eq!(result.status, status);
        assert_eq!(result.body, "\u{fffd}");
    }
}

struct EndlessEgress(u16);

#[async_trait]
impl EgressService for EndlessEgress {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        panic!("unbounded send")
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Ok(EgressStreamResponse {
            status: self.0,
            headers: BTreeMap::new(),
            body: Box::pin(futures::stream::repeat_with(|| Ok(vec![b'x'; 4096]))),
        })
    }
}

#[tokio::test]
async fn ordinary_operations_and_initialization_reject_endless_bodies() {
    use crate::mcp::{McpClient, NoAuthProvider};
    let client = McpClient::new(Arc::new(EndlessEgress(200)), Arc::new(NoAuthProvider));
    let connection = McpConnection::http("large", "http://8.8.8.8/mcp");
    let errors = [
        client
            .call(&connection, "echo", serde_json::json!({}), None)
            .await
            .err()
            .unwrap(),
        client
            .read_resource(&connection, "file:///large")
            .await
            .err()
            .unwrap(),
        client
            .list_resources(&connection, None)
            .await
            .err()
            .unwrap(),
        client
            .list_resource_templates(&connection, None)
            .await
            .err()
            .unwrap(),
        do_handshake(
            &EndlessEgress(200),
            "http://8.8.8.8/mcp",
            &HashMap::new(),
            None,
            "2025-06-18",
            Duration::from_secs(1),
        )
        .await
        .err()
        .unwrap(),
    ];
    for error in errors {
        assert!(
            error.to_string().contains("MCP HTTP response exceeds"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn endless_http_errors_keep_typed_auth_classification() {
    for status in [401, 403, 500] {
        let error = http_send_rpc(
            &EndlessEgress(status),
            "http://8.8.8.8/mcp",
            &HashMap::new(),
            None,
            vec![],
            Duration::from_secs(1),
        )
        .await
        .err()
        .unwrap();
        let error = error.downcast_ref::<McpHttpStatusError>().unwrap();
        assert_eq!(error.status, status);
        assert_eq!(error.is_unauthorized(), status == 401);
        assert_eq!(error.body.len(), body::MAX_ERROR_BODY_BYTES);
    }
}

#[cfg(feature = "direct-egress")]
#[tokio::test]
async fn real_chunked_http_body_is_cancelled_before_eof() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Exercise real socket -> reqwest -> DirectEgressService -> bounded reader.
    // This local fixture calls egress directly; public MCP URLs still pass SSRF validation.
    for status in [200, 401, 403, 500] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    format!("HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            loop {
                let chunk = format!("1000\r\n{}\r\n", "x".repeat(4096));
                if socket.write_all(chunk.as_bytes()).await.is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        });
        let response = crate::host::DirectEgressService::new()
            .send_stream(
                EgressRequest::new("GET", format!("http://{addr}/mcp"), EgressRequestKind::Mcp)
                    .timeout_ms(1000),
            )
            .await
            .unwrap();
        let result = read(response, 8192).await;
        if status == 200 {
            assert!(result.err().unwrap().to_string().contains("exceeds"));
        } else {
            assert_eq!(result.unwrap().status, status);
        }
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("body drop closes the connection")
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn exact_limit_without_eof_still_obeys_deadline() {
    use futures::StreamExt;
    let response = EgressStreamResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: Box::pin(
            futures::stream::iter([Ok(vec![b'x'; 100])]).chain(futures::stream::pending()),
        ),
    };
    assert!(
        read(response, 100)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("timed out")
    );
}

#[tokio::test]
async fn successful_status_does_not_hide_body_failure() {
    let response = EgressStreamResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: Box::pin(futures::stream::iter([Err(crate::EgressError::Transport(
            "broken body".into(),
        ))])),
    };
    assert!(
        read(response, 100)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("broken body")
    );
}
