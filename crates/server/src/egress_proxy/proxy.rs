// The tenant egress proxy itself: an HTTP/1.1 forward proxy.
//
// Decision: hand-rolled on tokio + httparse rather than hyper. The proxy needs
// exactly two shapes (CONNECT and an absolute-form plain HTTP request), exact
// byte counts in each direction, and a raw TCP splice after the request head;
// a full HTTP stack would hide the bytes and add a client it does not need.
//
// Decision: one request per client connection. Plain HTTP is forwarded with
// `Connection: close`, so a client cannot reuse an admitted connection to send
// a second, uncounted request (or one to a different host). CONNECT is one
// tunnel to one validated address by construction.
//
// Decision: what is logged is the decision, never the content: org, session,
// host, port, method, status, bytes and duration. No path, no query string, no
// header value, no body.
//
// Order of checks, cheapest and most decisive first: parse, authenticate
// (407), port policy (403), org limits and suspension (403), rate, host,
// concurrency and byte caps (429), then resolve and refuse non-public
// addresses (403), then connect (502).

use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use super::address::{self, AddressPolicy, ResolveError};
use super::limiter::Limiter;
use super::limits::LimitsCache;
use super::signals::Refusal;
use super::usage::UsageAggregator;
use everruns_core::host::{ProxyTokenError, verify_proxy_authorization};

/// Largest request head the proxy reads.
const MAX_HEAD: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct ProxyOptions {
    pub secret: Vec<u8>,
    pub address_policy: AddressPolicy,
    /// Source address for outbound connections, when the host has a
    /// dedicated tenant egress address.
    pub bind_addr: Option<IpAddr>,
    pub head_timeout: Duration,
    pub connect_timeout: Duration,
    /// Longest a tunnel or forwarded exchange may stay open.
    pub max_duration: Duration,
}

impl ProxyOptions {
    pub fn new(secret: Vec<u8>) -> Self {
        Self {
            secret,
            address_policy: AddressPolicy::PublicOnly,
            bind_addr: None,
            head_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(10),
            max_duration: Duration::from_secs(3_600),
        }
    }
}

pub struct Proxy {
    pub options: ProxyOptions,
    pub limits: LimitsCache,
    pub limiter: Limiter,
    pub usage: Arc<UsageAggregator>,
}

impl Proxy {
    /// Accept connections until the listener fails.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    // Usually EMFILE; back off instead of spinning.
                    tracing::warn!(error = %err, "egress proxy accept failed");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let proxy = self.clone();
            tokio::spawn(async move {
                proxy.handle(stream, peer).await;
            });
        }
    }

    async fn handle(&self, mut client: TcpStream, peer: SocketAddr) {
        let started = Instant::now();
        let _ = client.set_nodelay(true);
        let head =
            match tokio::time::timeout(self.options.head_timeout, read_head(&mut client)).await {
                Ok(Ok(head)) => head,
                Ok(Err(HeadError::Io)) | Err(_) => return,
                Ok(Err(HeadError::TooLarge | HeadError::Malformed)) => {
                    let _ = reply(&mut client, 400, "malformed proxy request").await;
                    return;
                }
            };
        let request = match parse_request(&head.bytes) {
            Ok(request) => request,
            Err(message) => {
                let _ = reply(&mut client, 400, message).await;
                return;
            }
        };

        let now = unix_now();
        let claims = match verify_proxy_authorization(
            request.proxy_authorization.as_deref(),
            &self.options.secret,
            now,
        ) {
            Ok(claims) => claims,
            Err(err) => {
                tracing::info!(
                    peer = %peer.ip(),
                    host = %request.host,
                    port = request.port,
                    decision = "unauthenticated",
                    reason = %err,
                    "egress request refused"
                );
                let _ = reply_407(&mut client, err).await;
                return;
            }
        };
        let org_key = claims.org.clone().unwrap_or_else(|| "-".to_string());
        let session = claims.session.as_deref();
        let log = LogContext {
            org: &org_key,
            session: session.unwrap_or("-"),
            host: &request.host,
            port: request.port,
            method: request.method_for_log(),
        };

        let signal = |refusal: Refusal| {
            if let Some(org) = &claims.org {
                self.usage.signals().refused(org, refusal, now);
            }
        };

        if !address::port_allowed(request.port) {
            signal(Refusal::Port);
            log.refused("port_blocked", 403);
            let _ = reply(&mut client, 403, "egress to this port is not allowed").await;
            return;
        }

        let lookup = match &claims.org {
            Some(org) => self.limits.get(org).await,
            None => super::limits::Lookup {
                limits: self.limits.fallback().clone(),
                refreshed: false,
            },
        };
        if lookup.refreshed {
            self.limiter.limits_refreshed(&org_key);
        }
        if lookup.limits.suspended {
            log.refused("suspended", 403);
            let _ = reply(
                &mut client,
                403,
                "egress is suspended for this organization; contact support",
            )
            .await;
            return;
        }

        let admission = match self
            .limiter
            .admit(&org_key, &request.host, &lookup.limits, now)
        {
            Ok(admission) => admission,
            Err(refusal) => {
                signal(Refusal::Rate);
                log.refused(refusal.as_str(), 429);
                let _ = reply(&mut client, 429, refusal.reason()).await;
                return;
            }
        };
        // Every admitted request counts, even one that later fails to
        // connect: it consumed a slot and shows the attempt on the meter.
        if let Some(org) = &claims.org {
            self.usage.record(org, session, now, 0, 0, 1);
            self.usage.signals().host(org, &request.host, now);
        }

        let addrs = match address::resolve(&request.host, request.port, self.options.address_policy)
            .await
        {
            Ok(addrs) => addrs,
            Err(err) => {
                let (decision, status, message) = match err {
                    ResolveError::PortBlocked => {
                        signal(Refusal::Port);
                        ("port_blocked", 403, "egress to this port is not allowed")
                    }
                    ResolveError::NotPublic => {
                        signal(Refusal::Private);
                        (
                            "non_public_address",
                            403,
                            "destination resolves to a non-public address",
                        )
                    }
                    ResolveError::NoAddress => {
                        ("dns_failure", 502, "destination host does not resolve")
                    }
                };
                log.refused(decision, status);
                let _ = reply(&mut client, status, message).await;
                return;
            }
        };

        let mut upstream = match self.connect(&addrs).await {
            Some(upstream) => upstream,
            None => {
                log.refused("connect_failed", 502);
                let _ = reply(&mut client, 502, "could not connect to destination").await;
                return;
            }
        };

        let mut egress: u64 = 0;
        let setup = if request.is_connect {
            async {
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await?;
                if !head.rest.is_empty() {
                    upstream.write_all(&head.rest).await?;
                }
                Ok::<u64, std::io::Error>(head.rest.len() as u64)
            }
            .await
        } else {
            let forwarded = request.forwarded_head();
            async {
                upstream.write_all(&forwarded).await?;
                upstream.write_all(&head.rest).await?;
                Ok::<u64, std::io::Error>((forwarded.len() + head.rest.len()) as u64)
            }
            .await
        };
        match setup {
            Ok(sent) => egress += sent,
            Err(_) => {
                log.refused("upstream_write_failed", 502);
                return;
            }
        }

        let counters = Arc::new(Counters::default());
        let mut counted = CountingStream {
            inner: client,
            counters: counters.clone(),
        };
        let outcome = tokio::time::timeout(
            self.options.max_duration,
            tokio::io::copy_bidirectional(&mut counted, &mut upstream),
        )
        .await;
        let timed_out = outcome.is_err();
        egress += counters.read.load(Ordering::Relaxed);
        let ingress = counters.written.load(Ordering::Relaxed);
        drop(admission);

        let end = unix_now();
        self.limiter.record_bytes(&org_key, egress + ingress, end);
        if let Some(org) = &claims.org {
            self.usage.record(org, session, end, egress, ingress, 0);
        }
        tracing::info!(
            org = log.org,
            session = log.session,
            host = log.host,
            port = log.port,
            method = log.method,
            decision = if timed_out {
                "allowed_timed_out"
            } else {
                "allowed"
            },
            status = if request.is_connect { 200 } else { 0 },
            egress_bytes = egress,
            ingress_bytes = ingress,
            duration_ms = started.elapsed().as_millis() as u64,
            "egress request"
        );
    }

    /// Connect to the first validated address that answers. Never the name.
    async fn connect(&self, addrs: &[SocketAddr]) -> Option<TcpStream> {
        for addr in addrs {
            let socket = match addr {
                SocketAddr::V4(_) => TcpSocket::new_v4(),
                SocketAddr::V6(_) => TcpSocket::new_v6(),
            };
            let Ok(socket) = socket else { continue };
            if let Some(bind) = self.options.bind_addr {
                if bind.is_ipv4() != addr.is_ipv4() {
                    continue;
                }
                if socket.bind(SocketAddr::new(bind, 0)).is_err() {
                    continue;
                }
            }
            if let Ok(Ok(stream)) =
                tokio::time::timeout(self.options.connect_timeout, socket.connect(*addr)).await
            {
                let _ = stream.set_nodelay(true);
                return Some(stream);
            }
        }
        None
    }
}

struct LogContext<'a> {
    org: &'a str,
    session: &'a str,
    host: &'a str,
    port: u16,
    method: &'a str,
}

impl LogContext<'_> {
    fn refused(&self, decision: &str, status: u16) {
        tracing::info!(
            org = self.org,
            session = self.session,
            host = self.host,
            port = self.port,
            method = self.method,
            decision,
            status,
            "egress request refused"
        );
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// --- request head -----------------------------------------------------------

struct Head {
    /// The head, through the blank line.
    bytes: Vec<u8>,
    /// Anything the client sent after the head (body or TLS bytes).
    rest: Vec<u8>,
}

enum HeadError {
    Io,
    TooLarge,
    Malformed,
}

async fn read_head(stream: &mut TcpStream) -> Result<Head, HeadError> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|_| HeadError::Io)?;
        if n == 0 {
            return Err(HeadError::Io);
        }
        let searched_from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find(&buf[searched_from..], b"\r\n\r\n") {
            let end = searched_from + pos + 4;
            let rest = buf.split_off(end);
            return Ok(Head { bytes: buf, rest });
        }
        if buf.len() > MAX_HEAD {
            return Err(HeadError::TooLarge);
        }
        if buf.len() > 8 && !buf.iter().take(16).all(|b| b.is_ascii()) {
            return Err(HeadError::Malformed);
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[derive(Debug)]
struct ProxyRequest {
    is_connect: bool,
    method: String,
    host: String,
    port: u16,
    /// Origin-form target for plain HTTP (`/path?query`); empty for CONNECT.
    path: String,
    proxy_authorization: Option<String>,
    /// Header lines to forward, already filtered.
    headers: Vec<(String, Vec<u8>)>,
}

/// Headers that belong to this hop, never forwarded upstream.
const HOP_BY_HOP: &[&str] = &[
    "proxy-authorization",
    "proxy-connection",
    "proxy-authenticate",
    "connection",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    "host",
];

impl ProxyRequest {
    fn method_for_log(&self) -> &str {
        &self.method
    }

    /// The request head as the origin server should see it: origin-form
    /// target, our Host, no proxy credentials, and `Connection: close`.
    fn forwarded_head(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512);
        out.extend_from_slice(self.method.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.path.as_bytes());
        out.extend_from_slice(b" HTTP/1.1\r\nHost: ");
        out.extend_from_slice(self.authority().as_bytes());
        out.extend_from_slice(b"\r\n");
        for (name, value) in &self.headers {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == 80 {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

fn parse_request(head: &[u8]) -> Result<ProxyRequest, &'static str> {
    let mut headers = [httparse::EMPTY_HEADER; 96];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err("malformed proxy request"),
    }
    let method = req.method.ok_or("malformed proxy request")?.to_string();
    let target = req.path.ok_or("malformed proxy request")?;
    let proxy_authorization = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("proxy-authorization"))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .map(str::to_string);

    let is_connect = method.eq_ignore_ascii_case("CONNECT");
    let (host, port, path) = if is_connect {
        let (host, port) =
            split_authority(target, None).ok_or("CONNECT target must be host:port")?;
        (host, port, String::new())
    } else {
        let rest = target
            .strip_prefix("http://")
            .or_else(|| target.strip_prefix("HTTP://"))
            .ok_or("only absolute http:// targets are proxied; use CONNECT for https")?;
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(split);
        let (host, port) =
            split_authority(authority, Some(80)).ok_or("malformed request target")?;
        let path = match path.split('#').next().unwrap_or("") {
            "" => "/".to_string(),
            p if p.starts_with('?') => format!("/{p}"),
            p => p.to_string(),
        };
        (host, port, path)
    };

    // Lowercase, and reject anything that is not a plain hostname or address.
    let host = host.to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'));
    if !valid {
        return Err("malformed destination host");
    }

    let headers = req
        .headers
        .iter()
        .filter(|h| {
            !HOP_BY_HOP
                .iter()
                .any(|hop| h.name.eq_ignore_ascii_case(hop))
        })
        .map(|h| (h.name.to_string(), h.value.to_vec()))
        .collect();

    Ok(ProxyRequest {
        is_connect,
        method,
        host,
        port,
        path,
        proxy_authorization,
        headers,
    })
}

/// `host:port`, `[v6]:port`, or (with a default port) `host` / `[v6]`.
/// Userinfo (`user@host`) is refused: it has no place in a proxy target and
/// is a classic way to disguise the real destination.
fn split_authority(authority: &str, default_port: Option<u16>) -> Option<(String, u16)> {
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None if after.is_empty() => default_port?,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host.to_string(), port.parse().ok()?)),
        Some(_) => None,
        None => Some((authority.to_string(), default_port?)),
    }
}

// --- replies ----------------------------------------------------------------

async fn reply(stream: &mut TcpStream, status: u16, message: &str) -> std::io::Result<()> {
    write_reply(stream, status, message, &[]).await
}

async fn reply_407(stream: &mut TcpStream, err: ProxyTokenError) -> std::io::Result<()> {
    write_reply(
        stream,
        407,
        &format!("egress proxy: {err}"),
        &[("Proxy-Authenticate", "Bearer realm=\"everruns-egress\"")],
    )
    .await
}

async fn write_reply(
    stream: &mut TcpStream,
    status: u16,
    message: &str,
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    let reason = match status {
        400 => "Bad Request",
        403 => "Forbidden",
        407 => "Proxy Authentication Required",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let body = format!("{message}\n");
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if status == 429 {
        head.push_str("Retry-After: 60\r\n");
    }
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

// --- byte counting ----------------------------------------------------------

#[derive(Default)]
struct Counters {
    /// Read from the client: egress.
    read: AtomicU64,
    /// Written to the client: ingress.
    written: AtomicU64,
}

/// Counts bytes as they pass, so the totals survive an error or a timeout
/// that would discard `copy_bidirectional`'s own return value.
struct CountingStream<S> {
    inner: S,
    counters: Arc<Counters>,
}

impl<S: AsyncRead + Unpin> AsyncRead for CountingStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let n = (buf.filled().len() - before) as u64;
            self.counters.read.fetch_add(n, Ordering::Relaxed);
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CountingStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &poll {
            self.counters
                .written
                .fetch_add(*n as u64, Ordering::Relaxed);
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// --- health -----------------------------------------------------------------

/// A tiny HTTP listener for probes: `GET /healthz` answers 200, anything else
/// 404. Separate from the proxy port so a probe never needs a token and a
/// tenant can never reach anything but the proxy.
pub async fn serve_health(listener: TcpListener) -> std::io::Result<()> {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await
            else {
                return;
            };
            let ok =
                buf[..n].starts_with(b"GET /healthz ") || buf[..n].starts_with(b"HEAD /healthz ");
            let response: &[u8] = if ok {
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nConnection: close\r\n\r\nok\n"
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            };
            let _ = stream.write_all(response).await;
            let _ = stream.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_targets_parse() {
        let req = parse_request(
            b"CONNECT API.Example.com:443 HTTP/1.1\r\nHost: api.example.com:443\r\nProxy-Authorization: Bearer x\r\n\r\n",
        )
        .expect("parses");
        assert!(req.is_connect);
        assert_eq!((req.host.as_str(), req.port), ("api.example.com", 443));
        assert_eq!(req.proxy_authorization.as_deref(), Some("Bearer x"));

        let req = parse_request(b"CONNECT [2606:4700::1]:8443 HTTP/1.1\r\n\r\n").expect("v6");
        assert_eq!((req.host.as_str(), req.port), ("2606:4700::1", 8443));

        for bad in [
            &b"CONNECT example.com HTTP/1.1\r\n\r\n"[..],
            b"CONNECT user@example.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT exa mple.com:443 HTTP/1.1\r\n\r\n",
            b"GET /relative HTTP/1.1\r\nHost: example.com\r\n\r\n",
            b"GET https://example.com/ HTTP/1.1\r\n\r\n",
            b"GET http://user:pw@example.com/ HTTP/1.1\r\n\r\n",
        ] {
            assert!(
                parse_request(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn plain_http_is_rewritten_to_origin_form_without_proxy_credentials() {
        let head = b"POST http://Example.com:8080/a/b?q=1#frag HTTP/1.1\r\nHost: evil.internal\r\nProxy-Authorization: Bearer secret\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\nContent-Length: 3\r\nX-Custom: yes\r\n\r\n";
        let req = parse_request(head).expect("parses");
        assert!(!req.is_connect);
        assert_eq!((req.host.as_str(), req.port), ("example.com", 8080));
        let forwarded = String::from_utf8(req.forwarded_head()).expect("utf8");
        assert!(forwarded.starts_with("POST /a/b?q=1 HTTP/1.1\r\nHost: example.com:8080\r\n"));
        assert!(forwarded.contains("Content-Length: 3\r\n"));
        assert!(forwarded.contains("X-Custom: yes\r\n"));
        assert!(forwarded.ends_with("Connection: close\r\n\r\n"));
        let lower = forwarded.to_ascii_lowercase();
        assert!(!lower.contains("proxy-authorization"));
        assert!(!lower.contains("secret"));
        assert!(!lower.contains("keep-alive"));
        assert!(!lower.contains("evil.internal"));

        let req = parse_request(b"GET http://example.com HTTP/1.1\r\n\r\n").expect("parses");
        assert_eq!((req.path.as_str(), req.port), ("/", 80));
        let req = parse_request(b"GET http://example.com?x=1 HTTP/1.1\r\n\r\n").expect("parses");
        assert_eq!(req.path, "/?x=1");
    }
}
