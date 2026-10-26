//! Deny feeds: public threat-intelligence domain lists, refreshed in the
//! background and enforced at the egress boundary like the curated deny list.
//!
//! The curated deny list (`system_denylist.toml`) names a few dozen capture
//! services by hand, and domain reputation asks a filtering resolver about one
//! host at a time before an open read. Neither covers the long tail of
//! phishing kits, malware droppers and C2 hosts that churn daily, which an
//! open-egress org can post data to. A feed is that long tail as a list.
//!
//! Decisions:
//! - Opt in with `EVERRUNS_EGRESS_DENY_FEEDS`: a comma list of preset names or
//!   `https://` URLs. Off by default; a self-hosted operator chooses whom to
//!   trust.
//! - An entry blocks the domain and every subdomain, the semantics of DNS
//!   blocklists. Requests the curated allowlist (or an org extension) admits
//!   are exempt: a feed is a third party's judgement, so a host the operator
//!   curated wins over it. Everything else, open grant included, is refused.
//! - Matching is a set lookup per host label, so it adds nothing measurable
//!   per request; the list lives once per process.
//! - The fetch is a host-owned call to a URL the operator configured, not
//!   tenant egress, so it uses its own client and never the tenant proxy.
//! - A failed refresh keeps the last good list, and the process starts empty
//!   (fails open) until the first fetch lands: the curated deny list,
//!   reputation, SSRF checks and per-org caps still apply meanwhile, and a
//!   feed outage must not become an egress outage.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use futures::StreamExt;

/// Comma-separated feeds: preset names (see [`preset_url`]) or `https://` URLs.
pub const EGRESS_DENY_FEEDS_ENV: &str = "EVERRUNS_EGRESS_DENY_FEEDS";

/// HaGeZi Threat Intelligence Feeds, mini edition: about 200k malware,
/// phishing, scam and C2 domains aggregated from public sources (URLhaus,
/// OpenPhish, Phishing Database and others), updated several times a day.
const HAGEZI_TIF_MINI: &str = "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/tif.mini-onlydomains.txt";

const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest feed body accepted; the HaGeZi mini list is about 4 MiB.
const MAX_FEED_BYTES: usize = 32 * 1024 * 1024;
/// Most entries kept across all feeds, about 100 MB of memory at the cap.
const MAX_ENTRIES: usize = 1_000_000;

/// The URL a preset name stands for.
fn preset_url(name: &str) -> Option<&'static str> {
    match name {
        "hagezi-tif-mini" => Some(HAGEZI_TIF_MINI),
        _ => None,
    }
}

/// The merged domain set of every configured feed.
pub struct DenyFeeds {
    sources: Vec<String>,
    domains: RwLock<Arc<HashSet<Box<str>>>>,
    started: AtomicBool,
    client: reqwest::Client,
}

impl std::fmt::Debug for DenyFeeds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DenyFeeds")
            .field("sources", &self.sources)
            .field("domains", &self.len())
            .finish_non_exhaustive()
    }
}

impl DenyFeeds {
    /// Feeds fetched from `sources` (URLs) once [`DenyFeeds::start`] runs.
    pub fn new(sources: Vec<String>) -> Self {
        Self {
            sources,
            domains: RwLock::new(Arc::new(HashSet::new())),
            started: AtomicBool::new(false),
            client: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()
                .expect("build deny feed HTTP client"),
        }
    }

    /// A fixed list with no refresh, for tests and embedders that bring their
    /// own source.
    pub fn from_domains<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let feeds = Self::new(Vec::new());
        feeds.replace(
            domains
                .into_iter()
                .filter_map(|line| parse_line(line.as_ref()))
                .collect(),
        );
        feeds
    }

    /// The feeds named by [`EGRESS_DENY_FEEDS_ENV`], if any. Unknown preset
    /// names and non-HTTPS URLs are logged and skipped.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var(EGRESS_DENY_FEEDS_ENV).ok()?;
        let sources = parse_sources(&value);
        (!sources.is_empty()).then(|| Self::new(sources))
    }

    /// Number of domains currently loaded.
    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    /// Whether no domain is loaded (before the first fetch, or none listed).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `host` or one of its parent domains is listed.
    pub fn is_listed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        let domains = self.snapshot();
        if domains.is_empty() {
            return false;
        }
        let mut candidate = host.as_str();
        loop {
            if domains.contains(candidate) {
                return true;
            }
            match candidate.split_once('.') {
                Some((_, parent)) if parent.contains('.') => candidate = parent,
                // Stop before a bare TLD: a feed listing `com` must not
                // block the internet.
                _ => return false,
            }
        }
    }

    /// Spawn the refresh loop on the current Tokio runtime, once. Called on
    /// the request path, so the first request after start-up kicks it off
    /// and later calls cost one atomic load.
    pub fn start(self: &Arc<Self>) {
        if self.sources.is_empty() || self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.started.store(false, Ordering::Release);
            return;
        };
        let feeds = Arc::clone(self);
        runtime.spawn(async move {
            loop {
                let wait = if feeds.refresh().await {
                    REFRESH_INTERVAL
                } else {
                    RETRY_INTERVAL
                };
                tokio::time::sleep(wait).await;
            }
        });
    }

    /// Fetch every feed and swap in the merged set. A feed that fails keeps
    /// the refresh from replacing the current list, so one broken source
    /// never empties the others. Returns whether the list was replaced.
    pub async fn refresh(&self) -> bool {
        let mut merged = HashSet::new();
        for source in &self.sources {
            match self.fetch(source, &mut merged).await {
                Ok(count) => tracing::debug!(source = %source, count, "deny feed fetched"),
                Err(error) => {
                    tracing::warn!(
                        source = %source,
                        error = %error,
                        kept = self.len(),
                        "deny feed refresh failed; keeping the last list"
                    );
                    return false;
                }
            }
        }
        let count = merged.len();
        self.replace(merged);
        tracing::info!(
            feeds = self.sources.len(),
            domains = count,
            "deny feeds refreshed"
        );
        true
    }

    async fn fetch(&self, source: &str, into: &mut HashSet<Box<str>>) -> Result<usize, String> {
        let response = self
            .client
            .get(source)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("status {}", response.status()));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            if body.len() + chunk.len() > MAX_FEED_BYTES {
                return Err(format!("feed larger than {MAX_FEED_BYTES} bytes"));
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body);
        let before = into.len();
        for domain in text.lines().filter_map(parse_line) {
            if into.len() >= MAX_ENTRIES {
                tracing::warn!(source = %source, max = MAX_ENTRIES, "deny feeds truncated at the entry cap");
                break;
            }
            into.insert(domain);
        }
        let added = into.len() - before;
        if added == 0 {
            // An empty feed is far more likely a broken download (an HTML
            // error page, a moved file) than a world without threats.
            return Err("feed has no domains".to_string());
        }
        Ok(added)
    }

    fn snapshot(&self) -> Arc<HashSet<Box<str>>> {
        Arc::clone(&self.domains.read().unwrap_or_else(|p| p.into_inner()))
    }

    fn replace(&self, domains: HashSet<Box<str>>) {
        *self.domains.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(domains);
    }
}

/// Process-wide feeds from the environment, shared by every runtime egress
/// service so the list is fetched and held once per process.
pub(crate) fn runtime_deny_feeds() -> Option<Arc<DenyFeeds>> {
    static FEEDS: OnceLock<Option<Arc<DenyFeeds>>> = OnceLock::new();
    FEEDS
        .get_or_init(|| DenyFeeds::from_env().map(Arc::new))
        .clone()
}

fn parse_sources(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| {
            if let Some(url) = preset_url(item) {
                return Some(url.to_string());
            }
            if item.starts_with("https://") && reqwest::Url::parse(item).is_ok() {
                return Some(item.to_string());
            }
            tracing::error!(
                feed = item,
                "unrecognized {EGRESS_DENY_FEEDS_ENV} entry (not a preset or an https URL); skipped"
            );
            None
        })
        .collect()
}

/// One domain from a feed line. Accepts plain domain lists, hosts files
/// (`0.0.0.0 domain`), adblock rules (`||domain^`) and URLs; skips comments,
/// IP addresses and anything that is not a dotted hostname.
fn parse_line(line: &str) -> Option<Box<str>> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
        return None;
    }
    let token = line.split_whitespace().last()?;
    let token = token
        .strip_prefix("||")
        .map(|rest| rest.trim_end_matches('^'))
        .unwrap_or(token);
    let token = match token.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next()?,
        None => token,
    };
    let host = token
        .rsplit_once('@')
        .map_or(token, |(_, host)| host)
        .split(':')
        .next()?
        .trim_start_matches("*.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let valid = host.contains('.')
        && host.len() <= 253
        && host.parse::<std::net::IpAddr>().is_err()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        && !host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    valid.then(|| host.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_common_feed_formats() {
        let parsed: Vec<_> = [
            "# comment",
            "",
            "evil.example",
            "0.0.0.0 hosts.example # trailing",
            "||adblock.example^",
            "https://user@url.example:8443/path?q=1",
            "*.wild.example",
            "Upper.Example.",
            "203.0.113.7",
            "localhost",
            "not a host!",
            "! adblock comment",
        ]
        .into_iter()
        .filter_map(parse_line)
        .collect();
        let parsed: Vec<&str> = parsed.iter().map(AsRef::as_ref).collect();
        assert_eq!(
            parsed,
            [
                "evil.example",
                "hosts.example",
                "adblock.example",
                "url.example",
                "wild.example",
                "upper.example",
            ]
        );
    }

    #[test]
    fn matches_listed_domains_and_their_subdomains_only() {
        let feeds = DenyFeeds::from_domains(["evil.example", "phish.pages.dev", "com"]);
        assert!(feeds.is_listed("evil.example"));
        assert!(feeds.is_listed("A.B.Evil.Example."));
        assert!(feeds.is_listed("login.phish.pages.dev"));
        assert!(!feeds.is_listed("notevil.example"));
        assert!(!feeds.is_listed("other.pages.dev"));
        // A bare TLD entry is never parsed, and never matched.
        assert!(!feeds.is_listed("docs.rs.com"));
        assert!(!feeds.is_listed(""));
    }

    #[test]
    fn sources_accept_presets_and_https_urls_only() {
        assert_eq!(
            parse_sources(
                " hagezi-tif-mini , https://feeds.example/list.txt, http://plain.example/x, nope,"
            ),
            [HAGEZI_TIF_MINI, "https://feeds.example/list.txt"]
        );
    }

    #[tokio::test]
    async fn refresh_merges_feeds_and_keeps_the_last_list_on_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("# a\none.example\n"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/b.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0.0.0.0 two.example\n"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/b.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>moved</html>"))
            .mount(&server)
            .await;

        let feeds = DenyFeeds::new(vec![
            format!("{}/a.txt", server.uri()),
            format!("{}/b.txt", server.uri()),
        ]);
        assert!(feeds.is_empty());
        assert!(feeds.refresh().await);
        assert!(feeds.is_listed("one.example"));
        assert!(feeds.is_listed("x.two.example"));

        // The second fetch of b.txt is an error page with no domains: the
        // refresh fails and both feeds' domains stay.
        assert!(!feeds.refresh().await);
        assert_eq!(feeds.len(), 2);
        assert!(feeds.is_listed("two.example"));
    }
}
