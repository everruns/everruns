// Container health probe for `everruns-server --health-check`.
//
// Decision: the runtime image is distroless (no shell, curl, or wget), so a
// Compose or Kubernetes exec healthcheck has nothing to call except the server
// binary itself. This probe uses only std so the check stays a few
// milliseconds and never starts the async runtime, telemetry, or database.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Probe `GET /health` on the server's own listen address.
///
/// A wildcard bind address (`0.0.0.0` or `::`) is probed on loopback.
/// Returns `Ok` only for an HTTP 200 response.
pub fn probe(addr: &str, timeout: Duration) -> Result<(), String> {
    let target = resolve(addr)?;
    let mut stream = TcpStream::connect_timeout(&target, timeout)
        .map_err(|e| format!("connect {target}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|_| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| format!("set timeout: {e}"))?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|e| format!("write request: {e}"))?;

    // Only the status line matters; 64 bytes is enough to hold it.
    let mut buf = [0u8; 64];
    let n = stream
        .read(&mut buf)
        .map_err(|e| format!("read response: {e}"))?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let status = head.lines().next().unwrap_or_default();
    match status.split_whitespace().nth(1) {
        Some("200") => Ok(()),
        _ => Err(format!("unhealthy response: {status:?}")),
    }
}

fn resolve(addr: &str) -> Result<SocketAddr, String> {
    let mut target = addr
        .to_socket_addrs()
        .map_err(|e| format!("invalid address {addr:?}: {e}"))?
        .next()
        .ok_or_else(|| format!("address {addr:?} resolved to nothing"))?;
    if target.ip().is_unspecified() {
        target.set_ip(match target.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(response: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut req = [0u8; 256];
            let n = conn.read(&mut req).unwrap();
            assert!(String::from_utf8_lossy(&req[..n]).starts_with("GET /health "));
            conn.write_all(response.as_bytes()).unwrap();
        });
        addr
    }

    #[test]
    fn ok_on_200() {
        let addr = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
        assert!(probe(&addr.to_string(), Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn fails_on_non_200() {
        let addr = serve_once("HTTP/1.1 503 Service Unavailable\r\n\r\n");
        let err = probe(&addr.to_string(), Duration::from_secs(2)).unwrap_err();
        assert!(err.contains("503"), "{err}");
    }

    #[test]
    fn fails_when_nothing_listens() {
        // Bind then drop to get a port that is very likely closed.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(probe(&format!("127.0.0.1:{port}"), Duration::from_secs(2)).is_err());
    }

    #[test]
    fn wildcard_bind_probes_loopback() {
        assert_eq!(
            resolve("0.0.0.0:9000").unwrap(),
            "127.0.0.1:9000".parse().unwrap()
        );
        assert_eq!(resolve("[::]:9000").unwrap(), "[::1]:9000".parse().unwrap());
    }
}
