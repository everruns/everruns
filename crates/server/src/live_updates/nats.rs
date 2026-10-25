// Shared NATS client connection for the control plane.
//
// Decision: credentials embedded in `NATS_URL` (`nats://user:pass@host:4222`)
// are applied explicitly. async-nats parses the URL userinfo into
// `ServerAddr::username()` / `password()` but never sends it in the CONNECT
// handshake, so a server running with `authorization {}` rejected every
// client and the control plane silently degraded to in-memory event delivery
// and PG NOTIFY on each boot (Sentry EVERRUNS-2 / EVERRUNS-4).
//
// Decision: credentials are split off the raw string before the URL is
// parsed, rather than read back out of a parsed `ServerAddr`. Generated
// passwords routinely contain characters that are reserved in a URL
// authority — a single `/` ends it, so `nats://u:pa/ss@host:4222` parses as
// host `u:pa` and fails with "invalid port number", and the deployment falls
// back with no usable diagnosis. Splitting first accepts those passwords
// unencoded, which is what an operator pasting a generated secret expects;
// percent-encoded passwords keep working because the value is decoded when it
// is still a valid encoding.
//
// Decision: connection failures are reported with the full error chain.
// `anyhow` `Display` prints only the outermost context, which hid the real
// cause behind "Failed to connect to NATS".
//
// Decision: the first connection is retried a few times with backoff before
// giving up. Callers fall back to in-memory event delivery and PG NOTIFY and
// never retry NATS for the life of the process, so one refused connect while
// NATS restarts alongside the control plane pinned the instance to the
// fallback until its next restart (Sentry EVERRUNS-4). The retry is bounded
// rather than async-nats' `retry_on_initial_connect`, which would hide a
// misconfigured `NATS_URL` behind a client that never connects.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::{Client, ConnectOptions, ServerAddr};
use tracing::warn;

/// Waits between initial connection attempts: five attempts over about 7.5s.
const INITIAL_CONNECT_BACKOFF: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

/// Username and password parsed out of a NATS URL's userinfo.
type Credentials = (String, String);

/// Split one server URL into `(url_without_userinfo, credentials)`.
///
/// The authority ends at the last `@`, so a password may itself contain `@`
/// or `/`. The returned URL carries no userinfo, so it always parses even
/// when the password does not survive URL syntax.
fn split_credentials(part: &str) -> (String, Option<Credentials>) {
    let Some((scheme, rest)) = part.split_once("://") else {
        return (part.to_string(), None);
    };
    let Some((userinfo, host)) = rest.rsplit_once('@') else {
        return (part.to_string(), None);
    };
    if userinfo.is_empty() {
        return (format!("{scheme}://{host}"), None);
    }
    let (user, pass) = match userinfo.split_once(':') {
        Some((user, pass)) => (user, pass),
        None => (userinfo, ""),
    };
    (
        format!("{scheme}://{host}"),
        Some((decode(user), decode(pass))),
    )
}

/// Remove userinfo from server URLs before they cross an observability boundary.
///
/// This intentionally also handles strings that are not valid URLs: parsing errors
/// must not turn malformed configuration into a credential disclosure.
pub fn redact_server_urls(url: &str) -> String {
    url.split(',')
        .map(str::trim)
        .map(|part| {
            let Some((prefix, host)) = part.rsplit_once('@') else {
                return part.to_string();
            };
            match prefix.rsplit_once("://") {
                Some((scheme, _userinfo)) => format!("{scheme}://[redacted]@{host}"),
                None => format!("[redacted]@{host}"),
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Percent-decode when the value is valid percent-encoding, else take it
/// literally. A generated password containing a bare `%` is not an encoding
/// mistake to reject — it is a password.
fn decode(raw: &str) -> String {
    urlencoding::decode(raw)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| raw.to_string())
}

/// Parse `NATS_URL`: one server URL or a comma-separated list of them.
///
/// Returns the addresses with userinfo stripped, plus the first credentials
/// found. A username without a password yields an empty password, matching
/// NATS token-less user auth.
fn parse_nats_url(url: &str) -> Result<(Vec<ServerAddr>, Option<Credentials>)> {
    let mut addrs = Vec::new();
    let mut credentials = None;
    for part in url.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (bare, found) = split_credentials(part);
        addrs.push(bare.parse::<ServerAddr>().context("Invalid NATS URL")?);
        if credentials.is_none() {
            credentials = found;
        }
    }
    Ok((addrs, credentials))
}

/// Extract `(user, password)` from a NATS URL.
pub fn credentials_from_url(url: &str) -> Result<Option<Credentials>> {
    Ok(parse_nats_url(url)?.1)
}

/// Connect to NATS, honouring credentials embedded in the URL.
pub async fn connect(url: &str) -> Result<Client> {
    let (addrs, credentials) = parse_nats_url(url)?;
    if addrs.is_empty() {
        anyhow::bail!("NATS URL is empty");
    }
    let target = redact_server_urls(url);
    retry_initial_connect(&INITIAL_CONNECT_BACKOFF, &target, || {
        let mut options = ConnectOptions::new();
        if let Some((user, pass)) = credentials.clone() {
            options = options.user_and_password(user, pass);
        }
        let addrs = addrs.clone();
        async move {
            options
                .connect(addrs)
                .await
                .context("NATS connection failed")
        }
    })
    .await
}

/// Run `attempt` once, then again after each `backoff` delay while it fails.
/// Returns the first success or the last error.
async fn retry_initial_connect<T, F, Fut>(
    backoff: &[Duration],
    target: &str,
    mut attempt: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delays = backoff.iter();
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) => match delays.next() {
                Some(delay) => {
                    warn!(
                        target_url = %target,
                        error = %error_chain(&error),
                        retry_in_ms = delay.as_millis() as u64,
                        "NATS connection attempt failed, retrying"
                    );
                    tokio::time::sleep(*delay).await;
                }
                None => return Err(error),
            },
        }
    }
}

/// Render an error with its full context chain on one line for log fields.
pub fn error_chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    const NO_WAIT: [Duration; 4] = [Duration::ZERO; 4];

    /// Regression test for Sentry EVERRUNS-4: a connect refused while NATS is
    /// still starting must be retried instead of pinning the fallback.
    #[tokio::test]
    async fn initial_connect_retries_until_nats_accepts() {
        let attempts = AtomicUsize::new(0);
        let result = retry_initial_connect(&NO_WAIT, "nats://nats:4222", || {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    anyhow::bail!("connection refused")
                }
                Ok(n)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 2);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn initial_connect_gives_up_after_the_backoff_schedule() {
        let attempts = AtomicUsize::new(0);
        let result: Result<()> = retry_initial_connect(&NO_WAIT, "nats://nats:4222", || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { anyhow::bail!("connection refused") }
        })
        .await;
        assert_eq!(result.unwrap_err().to_string(), "connection refused");
        assert_eq!(attempts.load(Ordering::SeqCst), NO_WAIT.len() + 1);
    }

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

    /// Regression test for a password with a bare `/`, which ends the URL
    /// authority when parsed as a standard URL.
    #[test]
    fn password_with_an_unencoded_slash_is_accepted() {
        let (addrs, creds) = parse_nats_url("nats://test_user:test-password/with-slash@nats:4222")
            .expect("a bare slash in the password must not fail the parse");
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].host(), "nats");
        assert_eq!(addrs[0].port(), 4222);
        assert_eq!(
            creds,
            Some((
                "test_user".to_string(),
                "test-password/with-slash".to_string()
            ))
        );
    }

    #[test]
    fn password_may_contain_reserved_characters() {
        for (password, label) in [
            ("pa@ss", "at sign"),
            ("pa/ss", "slash"),
            ("pa:ss", "colon"),
            ("pa?ss#frag", "query and fragment markers"),
            ("100%pure", "bare percent"),
        ] {
            assert_eq!(
                credentials_from_url(&format!("nats://u:{password}@nats:4222")).unwrap(),
                Some(("u".to_string(), password.to_string())),
                "{label} must survive"
            );
        }
    }

    #[test]
    fn username_without_password_yields_empty_password() {
        assert_eq!(
            credentials_from_url("nats://token@nats:4222").unwrap(),
            Some(("token".to_string(), String::new()))
        );
    }

    #[test]
    fn server_urls_are_redacted_for_logs() {
        for (url, expected) in [
            (
                "nats://control:secret@nats:4222",
                "nats://[redacted]@nats:4222",
            ),
            (
                "nats://control:p%40ss%2Fword@nats:4222",
                "nats://[redacted]@nats:4222",
            ),
            ("control:secret@nats:4222", "[redacted]@nats:4222"),
            (
                "nats://first:secret@nats1:4222, nats://nats2:4222,nats://second:p/w@nats3:4222",
                "nats://[redacted]@nats1:4222,nats://nats2:4222,nats://[redacted]@nats3:4222",
            ),
        ] {
            let redacted = redact_server_urls(url);
            assert_eq!(redacted, expected);
            assert!(!redacted.contains("secret"));
            assert!(!redacted.contains("p%40ss"));
            assert!(!redacted.contains("p/w"));
        }
    }

    /// Every server in the list must still be dialled, and none of them may
    /// carry userinfo into the address the client connects to.
    #[test]
    fn every_server_is_kept_and_userinfo_is_stripped() {
        let (addrs, creds) = parse_nats_url("nats://u:p/w@nats1:4222,nats://nats2:4223").unwrap();
        assert_eq!(addrs.len(), 2);
        assert_eq!((addrs[0].host(), addrs[0].port()), ("nats1", 4222));
        assert_eq!((addrs[1].host(), addrs[1].port()), ("nats2", 4223));
        assert!(
            addrs.iter().all(|a| a.username().is_none()),
            "userinfo must not reach the dialled address"
        );
        assert_eq!(creds, Some(("u".to_string(), "p/w".to_string())));
    }

    #[test]
    fn invalid_url_error_does_not_include_credentials() {
        let secret = "never-log-this";
        let error = credentials_from_url(&format!("nats://user:{secret}@invalid host"))
            .expect_err("malformed URL must fail");
        let rendered = error_chain(&error);
        assert!(rendered.starts_with("Invalid NATS URL:"));
        assert!(!rendered.contains(secret));
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
