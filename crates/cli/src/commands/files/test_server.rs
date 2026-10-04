// Minimal in-process stand-in for the session filesystem API, for tests that
// must observe exactly which bytes the CLI uploads.

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Records every upload (POST/PUT) request body; listings are always empty.
pub struct FsApiStub {
    pub url: String,
    uploads: Arc<Mutex<Vec<(String, String)>>>,
}

impl FsApiStub {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let uploads = Arc::new(Mutex::new(Vec::new()));
        let recorded = uploads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // Read headers, then Content-Length bytes of body.
                    let (head_end, content_len) = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            break (pos + 4, len);
                        }
                    };
                    while buf.len() < head_end + content_len {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let request_line = head.lines().next().unwrap_or_default().to_string();
                    let mut parts = request_line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_string();
                    let path = parts.next().unwrap_or_default().to_string();
                    let body = if method == "GET" {
                        r#"{"data":[]}"#.to_string()
                    } else {
                        let sent = String::from_utf8_lossy(&buf[head_end..]).to_string();
                        recorded.lock().unwrap().push((path, sent));
                        "{}".to_string()
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { url, uploads }
    }

    /// (request path, raw JSON body) for every upload received so far.
    pub fn uploads(&self) -> Vec<(String, String)> {
        self.uploads.lock().unwrap().clone()
    }
}
