//! The worker's checks on its own setup, reported to the control plane so the
//! system doctor can show what only the worker can see.
//!
//! Decisions:
//! - Only what differs per worker is checked here: the worker's own
//!   environment and what it can reach. Everything the server can judge
//!   alone stays in the server's doctor.
//! - Cheap and bounded: a few environment reads and one TCP connect with a
//!   timeout, sent at registration and every [`REPORT_INTERVAL`], never on
//!   the heartbeat path.
//! - No secrets: summaries name settings and hosts, never values.

use serde::Serialize;
use std::time::Duration;

use crate::host::{
    EGRESS_PROXY_SECRET_ENV, EGRESS_PROXY_URL_ENV, MIN_PROXY_SECRET_LEN, SANDBOX_PROXY_ADDRESS_ENV,
    SANDBOX_PROXY_TLS_ENV, TenantEgressProxy,
};

/// How often a worker sends a fresh report.
pub const REPORT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Upper bound for one reachability probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Check id the server compares with its own NATS setting.
pub const NATS_CHECK: &str = "nats";

/// One finding. `status` is `ok`, `info`, `warning` or `problem`, as in the
/// system doctor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelfCheck {
    pub id: &'static str,
    pub title: &'static str,
    pub status: &'static str,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl SelfCheck {
    fn new(id: &'static str, title: &'static str, status: &'static str, summary: String) -> Self {
        Self {
            id,
            title,
            status,
            summary,
            fix: None,
        }
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

/// The settings the checks judge, read from the environment once per report.
#[derive(Debug, Default, Clone)]
pub struct SelfCheckEnv {
    pub egress_proxy_url: Option<String>,
    pub egress_proxy_secret_len: usize,
    pub sandbox_proxy_address: Option<String>,
    pub sandbox_proxy_tls: Option<String>,
    pub nats: bool,
}

impl SelfCheckEnv {
    pub fn from_env() -> Self {
        let set = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            egress_proxy_url: set(EGRESS_PROXY_URL_ENV),
            egress_proxy_secret_len: set(EGRESS_PROXY_SECRET_ENV).map_or(0, |secret| secret.len()),
            sandbox_proxy_address: set(SANDBOX_PROXY_ADDRESS_ENV),
            sandbox_proxy_tls: set(SANDBOX_PROXY_TLS_ENV),
            // Same test as the event emitter's: the worker assumes the server
            // delivers live updates through NATS exactly when it has NATS_URL.
            nats: std::env::var("NATS_URL").is_ok(),
        }
    }
}

/// Run every check once.
pub async fn run(env: &SelfCheckEnv) -> Vec<SelfCheck> {
    vec![egress_proxy(env).await, sandbox_proxy(env), nats(env)]
}

async fn egress_proxy(env: &SelfCheckEnv) -> SelfCheck {
    const ID: &str = "egress_proxy";
    const TITLE: &str = "Egress proxy from this worker";
    let Some(url) = env.egress_proxy_url.as_deref() else {
        return SelfCheck::new(
            ID,
            TITLE,
            "info",
            "Not used: tools and integrations call the internet directly from this worker."
                .to_string(),
        );
    };
    // Validation only needs the length; a dummy of that length keeps the
    // secret itself out of this module.
    let secret = vec![0u8; env.egress_proxy_secret_len];
    if let Err(error) = TenantEgressProxy::new(url, secret) {
        return SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("The egress proxy setting is unusable: {error}."),
        )
        .fix(format!(
            "Set {EGRESS_PROXY_URL_ENV} to http(s)://host:port and {EGRESS_PROXY_SECRET_ENV} \
                 to the proxy's secret (at least {MIN_PROXY_SECRET_LEN} bytes). Until then every \
                 tool call that reaches the internet fails."
        ));
    }
    let Some(target) = proxy_target(url) else {
        return SelfCheck::new(ID, TITLE, "ok", "Configured.".to_string());
    };
    match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(&target)).await {
        Ok(Ok(_)) => SelfCheck::new(ID, TITLE, "ok", format!("Reachable at {target}.")),
        Ok(Err(error)) => SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("Cannot reach the egress proxy at {target}: {error}."),
        )
        .fix("Check that the egress proxy runs and that this worker's network can reach it."),
        Err(_) => SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("The egress proxy at {target} did not answer within 3 seconds."),
        )
        .fix("Check that the egress proxy runs and that this worker's network can reach it."),
    }
}

/// `host:port` of a proxy URL, with the scheme's default port.
fn proxy_target(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

fn sandbox_proxy(env: &SelfCheckEnv) -> SelfCheck {
    const ID: &str = "sandbox_proxy";
    const TITLE: &str = "Sandbox egress from this worker";
    let Some(address) = env.sandbox_proxy_address.as_deref() else {
        return SelfCheck::new(
            ID,
            TITLE,
            "info",
            "Not used: sandboxes reach the internet through their provider.".to_string(),
        );
    };
    let fix = format!(
        "Set {SANDBOX_PROXY_ADDRESS_ENV} to the proxy's public host:port, \
         {SANDBOX_PROXY_TLS_ENV} to true or false, and {EGRESS_PROXY_SECRET_ENV} \
         (at least {MIN_PROXY_SECRET_LEN} bytes). Until then sandboxes that should \
         use the proxy do not start."
    );
    let valid_address = address.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty() && !host.contains('/') && port.parse::<u16>().is_ok_and(|port| port != 0)
    });
    let tls = match env
        .sandbox_proxy_tls
        .as_deref()
        .map(str::to_ascii_lowercase)
    {
        None => Some(true),
        Some(value) => match value.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
    };
    if !valid_address {
        return SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("{SANDBOX_PROXY_ADDRESS_ENV} is not host:port."),
        )
        .fix(fix);
    }
    let Some(tls) = tls else {
        return SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("{SANDBOX_PROXY_TLS_ENV} must be true or false."),
        )
        .fix(fix);
    };
    if env.egress_proxy_secret_len < MIN_PROXY_SECRET_LEN {
        return SelfCheck::new(
            ID,
            TITLE,
            "problem",
            format!("{EGRESS_PROXY_SECRET_ENV} is missing or too short."),
        )
        .fix(fix);
    }
    let transport = if tls { "over TLS" } else { "without TLS" };
    SelfCheck::new(
        ID,
        TITLE,
        "ok",
        format!("Sandboxes tunnel through {address} {transport}."),
    )
}

fn nats(env: &SelfCheckEnv) -> SelfCheck {
    const TITLE: &str = "Live updates from this worker";
    if env.nats {
        SelfCheck::new(
            NATS_CHECK,
            TITLE,
            "ok",
            "NATS_URL is set: streamed output goes out through NATS.".to_string(),
        )
    } else {
        SelfCheck::new(
            NATS_CHECK,
            TITLE,
            "info",
            "NATS_URL is not set: streamed output is saved through the server.".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(checks: &[SelfCheck], id: &str) -> &'static str {
        checks
            .iter()
            .find(|check| check.id == id)
            .expect("check present")
            .status
    }

    #[tokio::test]
    async fn an_unconfigured_worker_reports_notes_only() {
        let checks = run(&SelfCheckEnv::default()).await;
        assert!(checks.iter().all(|check| check.status == "info"));
    }

    #[tokio::test]
    async fn a_reachable_egress_proxy_is_ok_and_an_unreachable_one_a_problem() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut env = SelfCheckEnv {
            egress_proxy_url: Some(format!("http://127.0.0.1:{port}")),
            egress_proxy_secret_len: 32,
            ..Default::default()
        };
        assert_eq!(egress_proxy(&env).await.status, "ok");

        drop(listener);
        let unreachable = egress_proxy(&env).await;
        assert_eq!(unreachable.status, "problem");
        assert!(unreachable.fix.is_some());

        env.egress_proxy_secret_len = 8;
        let short = egress_proxy(&env).await;
        assert_eq!(short.status, "problem");
        assert!(short.summary.contains("unusable"));
    }

    #[test]
    fn sandbox_proxy_settings_are_validated() {
        let mut env = SelfCheckEnv {
            sandbox_proxy_address: Some("egress.example.com:1080".to_string()),
            egress_proxy_secret_len: 32,
            ..Default::default()
        };
        assert_eq!(sandbox_proxy(&env).status, "ok");
        env.sandbox_proxy_tls = Some("maybe".to_string());
        assert_eq!(sandbox_proxy(&env).status, "problem");
        env.sandbox_proxy_tls = Some("false".to_string());
        assert!(sandbox_proxy(&env).summary.contains("without TLS"));
        env.sandbox_proxy_address = Some("no-port".to_string());
        assert_eq!(sandbox_proxy(&env).status, "problem");
    }

    #[tokio::test]
    async fn nats_use_is_reported_for_the_server_to_compare() {
        let env = SelfCheckEnv {
            nats: true,
            ..Default::default()
        };
        assert_eq!(status(&run(&env).await, NATS_CHECK), "ok");
    }

    #[test]
    fn an_ok_report_omits_the_fix() {
        let check = SelfCheck::new("x", "X", "ok", "fine".to_string());
        let json = serde_json::to_value(&check).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json.get("fix").is_none());
    }
}
