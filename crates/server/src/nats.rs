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

use anyhow::{Context, Result};
use async_nats::{Client, ConnectOptions, ServerAddr};

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
        addrs.push(
            bare.parse::<ServerAddr>()
                .with_context(|| format!("Invalid NATS URL: {bare}"))?,
        );
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
    let mut options = ConnectOptions::new();
    if let Some((user, pass)) = credentials {
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

    /// The shape that left production on the in-memory fallback: a generated
    /// password with a bare `/`, which ends the URL authority and made the
    /// whole URL fail to parse.
    #[test]
    fn password_with_an_unencoded_slash_is_accepted() {
        let (addrs, creds) = parse_nats_url("nats://everruns_prod_nats:ODC05x/uceWsdB@nats:4222")
            .expect("a bare slash in the password must not fail the parse");
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].host(), "nats");
        assert_eq!(addrs[0].port(), 4222);
        assert_eq!(
            creds,
            Some((
                "everruns_prod_nats".to_string(),
                "ODC05x/uceWsdB".to_string()
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
    fn credentials_found_in_a_server_list() {
        assert_eq!(
            credentials_from_url("nats://nats1:4222, nats://u:p/w@nats2:4222").unwrap(),
            Some(("u".to_string(), "p/w".to_string()))
        );
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
