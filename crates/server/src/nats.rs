// Shared NATS client connection for the control plane.
//
// Decision: credentials embedded in `NATS_URL` (`nats://user:pass@host:4222`)
// are applied explicitly. async-nats parses the URL userinfo into
// `ServerAddr::username()` / `password()` but never sends it in the CONNECT
// handshake, so a server running with `authorization {}` rejected every
// client and the control plane silently degraded to in-memory event delivery
// and PG NOTIFY on each boot (Sentry EVERRUNS-2 / EVERRUNS-4).
//
// Decision: connection failures are reported with the full error chain.
// `anyhow` `Display` prints only the outermost context, which hid the
// authorization violation behind "Failed to connect to NATS".

use anyhow::{Context, Result};
use async_nats::{Client, ConnectOptions, ServerAddr};

/// Parse `NATS_URL`: one server URL or a comma-separated list of them.
fn parse_server_addrs(url: &str) -> Result<Vec<ServerAddr>> {
    url.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<ServerAddr>()
                .with_context(|| format!("Invalid NATS URL: {part}"))
        })
        .collect()
}

/// Extract `(user, password)` from a NATS URL, percent-decoded.
///
/// The first server that carries a username wins. Returns `None` when no
/// server does. A username without a password yields an empty password,
/// matching NATS token-less user auth.
pub fn credentials_from_url(url: &str) -> Result<Option<(String, String)>> {
    for addr in parse_server_addrs(url)? {
        let Some(user) = addr.username() else {
            continue;
        };
        let user = urlencoding::decode(user)
            .context("Invalid percent-encoding in NATS username")?
            .into_owned();
        let pass = match addr.password() {
            Some(pass) => urlencoding::decode(pass)
                .context("Invalid percent-encoding in NATS password")?
                .into_owned(),
            None => String::new(),
        };
        return Ok(Some((user, pass)));
    }
    Ok(None)
}

/// Connect to NATS, honouring credentials embedded in the URL.
pub async fn connect(url: &str) -> Result<Client> {
    let addrs = parse_server_addrs(url)?;
    if addrs.is_empty() {
        anyhow::bail!("NATS URL is empty");
    }
    let mut options = ConnectOptions::new();
    if let Some((user, pass)) = credentials_from_url(url)? {
        options = options.user_and_password(user, pass);
    }
    options
        .connect(addrs)
        .await
        .context("NATS connection failed")
}

/// Render an error with its full context chain on one line for log fields.
pub fn error_chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_absent_without_userinfo() {
        assert_eq!(credentials_from_url("nats://localhost:4222").unwrap(), None);
    }

    #[test]
    fn credentials_parsed_and_percent_decoded() {
        assert_eq!(
            credentials_from_url("nats://control:p%40ss%2Fw0rd@nats:4222").unwrap(),
            Some(("control".to_string(), "p@ss/w0rd".to_string()))
        );
    }

    #[test]
    fn username_without_password_yields_empty_password() {
        assert_eq!(
            credentials_from_url("nats://token@nats:4222").unwrap(),
            Some(("token".to_string(), String::new()))
        );
    }

    #[test]
    fn credentials_found_in_a_server_list() {
        assert_eq!(
            credentials_from_url("nats://nats1:4222, nats://u:p@nats2:4222").unwrap(),
            Some(("u".to_string(), "p".to_string()))
        );
    }

    #[test]
    fn invalid_url_is_an_error() {
        assert!(credentials_from_url("not a url").is_err());
    }

    #[test]
    fn error_chain_includes_context() {
        let error = anyhow::anyhow!("authorization violation").context("NATS connection failed");
        assert_eq!(
            error_chain(&error),
            "NATS connection failed: authorization violation"
        );
    }
}
