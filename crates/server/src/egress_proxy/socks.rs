// SOCKS5 listener for sandbox providers that tunnel a sandbox's traffic to us
// (E2B "bring your own proxy").
//
// Decision: only what such a provider needs. Username/password auth (RFC
// 1929) and CONNECT; no unauthenticated method, no BIND, no UDP ASSOCIATE.
// The password is a sandbox credential (`s1.<org>.<session>.<exp>.<sig>`),
// never a request token, and the username is ignored: the signed claims are
// the identity.
//
// Decision: after authentication the request goes through the same
// `Proxy::open` and `Proxy::relay` as the HTTP listener: port policy, org
// limits and suspension, caps, the non-public refusal and byte counting. A
// refusal maps to the closest SOCKS5 reply code, and the log line carries
// the same decision names, with method `SOCKS`.
//
// Decision: plain SOCKS5 on the socket. The provider reaches it over TLS
// through whatever terminates TLS in front of the listener; the listener
// itself never sees the public internet in a hosted deployment.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::proxy::{LogContext, Opened, Proxy, Refused, unix_now};
use everruns_core::host::verify_sandbox_proxy_credential;

const VERSION: u8 = 0x05;
const AUTH_VERSION: u8 = 0x01;
const METHOD_USERNAME_PASSWORD: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xff;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

/// SOCKS5 reply codes (RFC 1928 §6).
mod reply {
    pub const SUCCEEDED: u8 = 0x00;
    pub const GENERAL_FAILURE: u8 = 0x01;
    pub const NOT_ALLOWED: u8 = 0x02;
    pub const HOST_UNREACHABLE: u8 = 0x04;
    pub const CONNECTION_REFUSED: u8 = 0x05;
    pub const COMMAND_NOT_SUPPORTED: u8 = 0x07;
    pub const ADDRESS_TYPE_NOT_SUPPORTED: u8 = 0x08;
}

impl Proxy {
    /// Accept SOCKS5 connections until the listener fails.
    pub async fn serve_socks(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    tracing::warn!(error = %err, "egress proxy SOCKS accept failed");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let proxy = self.clone();
            tokio::spawn(async move {
                proxy.handle_socks(stream, peer).await;
            });
        }
    }

    async fn handle_socks(&self, mut client: TcpStream, peer: SocketAddr) {
        let started = Instant::now();
        let _ = client.set_nodelay(true);
        let handshake = tokio::time::timeout(
            self.options.head_timeout,
            handshake(&mut client, &self.options.secret, peer),
        )
        .await;
        let Ok(Some((claims, host, port))) = handshake else {
            return;
        };

        let now = unix_now();
        let Opened {
            upstream,
            admission,
            org_key,
        } = match self.open(&claims, &host, port, "SOCKS", now).await {
            Ok(opened) => opened,
            Err(refused) => {
                let _ = send_reply(&mut client, reply_code(refused)).await;
                return;
            }
        };
        let log = LogContext {
            org: &org_key,
            session: claims.session.as_deref().unwrap_or("-"),
            host: &host,
            port,
            method: "SOCKS",
        };
        if send_reply(&mut client, reply::SUCCEEDED).await.is_err() {
            log.refused("client_write_failed", 0);
            return;
        }
        self.relay(client, upstream, admission, &claims, &log, 0, 0, started)
            .await;
    }
}

/// Method negotiation, authentication and the request. Answers protocol
/// and auth failures itself; returns the claims and destination to open.
async fn handshake(
    client: &mut TcpStream,
    secret: &[u8],
    peer: SocketAddr,
) -> Option<(everruns_core::host::ProxyClaims, String, u16)> {
    // Greeting: VER NMETHODS METHODS...
    let [version, count] = read_array(client).await?;
    if version != VERSION || count == 0 {
        return None;
    }
    let methods = read_vec(client, count as usize).await?;
    if !methods.contains(&METHOD_USERNAME_PASSWORD) {
        let _ = client.write_all(&[VERSION, METHOD_NONE_ACCEPTABLE]).await;
        refused_unauthenticated(peer, "no username/password method offered");
        return None;
    }
    client
        .write_all(&[VERSION, METHOD_USERNAME_PASSWORD])
        .await
        .ok()?;

    // RFC 1929: VER ULEN UNAME PLEN PASSWD
    let [auth_version, username_len] = read_array(client).await?;
    if auth_version != AUTH_VERSION {
        return None;
    }
    let _username = read_vec(client, username_len as usize).await?;
    let [password_len] = read_array(client).await?;
    let password = read_vec(client, password_len as usize).await?;
    let claims = match std::str::from_utf8(&password)
        .ok()
        .map(|password| verify_sandbox_proxy_credential(password, secret, unix_now()))
    {
        Some(Ok(claims)) => claims,
        failure => {
            let _ = client.write_all(&[AUTH_VERSION, 0x01]).await;
            let reason = match failure {
                Some(Err(err)) => err.to_string(),
                _ => "malformed sandbox credential".to_string(),
            };
            refused_unauthenticated(peer, &reason);
            return None;
        }
    };
    client.write_all(&[AUTH_VERSION, 0x00]).await.ok()?;

    // Request: VER CMD RSV ATYP DST.ADDR DST.PORT
    let [version, command, _reserved, address_type] = read_array(client).await?;
    if version != VERSION {
        return None;
    }
    let host = match address_type {
        ATYP_IPV4 => Ipv4Addr::from(read_array::<4>(client).await?).to_string(),
        ATYP_IPV6 => Ipv6Addr::from(read_array::<16>(client).await?).to_string(),
        ATYP_DOMAIN => {
            let [len] = read_array(client).await?;
            let name = read_vec(client, len as usize).await?;
            match valid_domain(&name) {
                Some(name) => name,
                None => {
                    let _ = send_reply(client, reply::GENERAL_FAILURE).await;
                    return None;
                }
            }
        }
        _ => {
            let _ = send_reply(client, reply::ADDRESS_TYPE_NOT_SUPPORTED).await;
            return None;
        }
    };
    let port = u16::from_be_bytes(read_array(client).await?);
    if command != CMD_CONNECT {
        let _ = send_reply(client, reply::COMMAND_NOT_SUPPORTED).await;
        return None;
    }
    Some((claims, host, port))
}

fn refused_unauthenticated(peer: SocketAddr, reason: &str) {
    tracing::info!(
        peer = %peer.ip(),
        method = "SOCKS",
        decision = "unauthenticated",
        reason,
        "egress request refused"
    );
}

/// A lowercase hostname in the alphabet the HTTP listener accepts.
fn valid_domain(name: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(name).ok()?.to_ascii_lowercase();
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    valid.then_some(name)
}

fn reply_code(refused: Refused) -> u8 {
    match refused.decision {
        "dns_failure" => reply::HOST_UNREACHABLE,
        "connect_failed" => reply::CONNECTION_REFUSED,
        _ if matches!(refused.status, 403 | 429) => reply::NOT_ALLOWED,
        _ => reply::GENERAL_FAILURE,
    }
}

/// A reply with an unspecified bound address: providers do not use it, and
/// it keeps the proxy's own outbound address out of the tunnel.
async fn send_reply(client: &mut TcpStream, code: u8) -> std::io::Result<()> {
    client
        .write_all(&[VERSION, code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await?;
    if code != reply::SUCCEEDED {
        client.shutdown().await?;
    }
    Ok(())
}

async fn read_array<const N: usize>(client: &mut TcpStream) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    client.read_exact(&mut buf).await.ok()?;
    Some(buf)
}

async fn read_vec(client: &mut TcpStream, len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    client.read_exact(&mut buf).await.ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_map_to_socks_reply_codes() {
        let refused = |decision, status| Refused {
            decision,
            status,
            message: "",
        };
        assert_eq!(
            reply_code(refused("non_public_address", 403)),
            reply::NOT_ALLOWED
        );
        assert_eq!(reply_code(refused("suspended", 403)), reply::NOT_ALLOWED);
        assert_eq!(reply_code(refused("org_rate", 429)), reply::NOT_ALLOWED);
        assert_eq!(
            reply_code(refused("dns_failure", 502)),
            reply::HOST_UNREACHABLE
        );
        assert_eq!(
            reply_code(refused("connect_failed", 502)),
            reply::CONNECTION_REFUSED
        );
    }

    #[test]
    fn domains_are_lowercased_and_restricted() {
        assert_eq!(
            valid_domain(b"API.Example.com").as_deref(),
            Some("api.example.com")
        );
        for bad in [
            &b""[..],
            b"exa mple.com",
            b"a/b",
            b"user@host",
            &[0xff, 0x41],
        ] {
            assert_eq!(valid_domain(bad), None, "{bad:?}");
        }
    }
}
