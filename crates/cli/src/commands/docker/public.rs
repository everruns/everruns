//! A public address for a local instance: `--public-url` and `--tunnel`.
//!
//! Decision: the tunnel runs as one more Compose service in front of Caddy,
//! chosen by a Compose profile, not as a program on the person's machine. It
//! then starts, stops and restarts with the stack, and nothing is left running
//! after `everruns docker stop`. The address is always passed to the server as
//! `PUBLIC_APP_URL`, which Slack Request URLs, OAuth callbacks and links use.
//!
//! Decision: a public address on an instance without sign-in is refused. With
//! `--auth none` every visitor is admin, which is fine on 127.0.0.1 and
//! dangerous on the internet.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

use super::settings::{self, Settings};

/// How long to wait for a tunnel to report its address.
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(90);
/// How long to wait for the public address to answer.
const REACH_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TunnelChoice {
    /// No tunnel (an address from --public-url, or none)
    None,
    /// Tailscale Funnel: free, stable *.ts.net address, needs TS_AUTHKEY
    Tailscale,
    /// Cloudflare Tunnel: with --public-url a named tunnel (CLOUDFLARE_TUNNEL_TOKEN),
    /// without it a quick tunnel whose random address changes on every restart
    Cloudflare,
    /// ngrok: your static domain as --public-url, needs NGROK_AUTHTOKEN
    Ngrok,
}

impl TunnelChoice {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tailscale => "tailscale",
            Self::Cloudflare => "cloudflare",
            Self::Ngrok => "ngrok",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::value_variants()
            .iter()
            .copied()
            .find(|t| t.name() == name)
    }

    /// The secret the tunnel needs, as read from the shell and saved in `.env`.
    fn token_env(self, has_url: bool) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Tailscale => Some(settings::TS_AUTHKEY),
            Self::Cloudflare if has_url => Some(settings::CLOUDFLARE_TUNNEL_TOKEN),
            Self::Cloudflare => None,
            Self::Ngrok => Some(settings::NGROK_AUTHTOKEN),
        }
    }

    fn token_label(self) -> &'static str {
        match self {
            Self::Tailscale => "Tailscale auth key (tskey-auth-...)",
            Self::Cloudflare => "Cloudflare tunnel token",
            Self::Ngrok => "ngrok authtoken",
            Self::None => "",
        }
    }

    fn token_help(self) -> &'static str {
        match self {
            Self::Tailscale => "create one at https://login.tailscale.com/admin/settings/keys",
            Self::Cloudflare => {
                "create a tunnel in the Cloudflare dashboard (Networks > Tunnels), \
                 point its public hostname at http://caddy:9300 and copy its token"
            }
            Self::Ngrok => "copy it from https://dashboard.ngrok.com/get-started/your-authtoken",
            Self::None => "",
        }
    }
}

/// The Compose service and profile that run a tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TunnelService {
    pub profile: &'static str,
    pub service: &'static str,
    /// The tunnel picks the address; `up` reads it back.
    pub discovers_url: bool,
}

pub const TUNNEL_SERVICES: [TunnelService; 4] = [
    TunnelService {
        profile: "tailscale",
        service: "tailscale",
        discovers_url: true,
    },
    TunnelService {
        profile: "cloudflare",
        service: "cloudflared",
        discovers_url: false,
    },
    TunnelService {
        profile: "cloudflare-quick",
        service: "cloudflared-quick",
        discovers_url: true,
    },
    TunnelService {
        profile: "ngrok",
        service: "ngrok",
        discovers_url: false,
    },
];

/// Which service runs `tunnel`; a Cloudflare tunnel without an address of its
/// own is the quick kind.
pub fn service_for(tunnel: TunnelChoice, has_url: bool) -> Option<TunnelService> {
    let profile = match tunnel {
        TunnelChoice::None => return None,
        TunnelChoice::Tailscale => "tailscale",
        TunnelChoice::Cloudflare if has_url => "cloudflare",
        TunnelChoice::Cloudflare => "cloudflare-quick",
        TunnelChoice::Ngrok => "ngrok",
    };
    TUNNEL_SERVICES.into_iter().find(|t| t.profile == profile)
}

/// What `up` decided about the public address.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub tunnel: TunnelChoice,
    /// The address known before starting (given or saved). Tunnels that pick
    /// their own address fill it in after they start.
    pub url: Option<String>,
    pub service: Option<TunnelService>,
}

/// A public origin: `https://host[:port]`, no path, query or credentials.
/// Plain `http` is accepted for a LAN address, with a warning from the caller.
pub fn normalize_url(raw: &str) -> Result<String> {
    let raw = raw.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(raw).with_context(|| {
        format!("--public-url '{raw}' is not a URL like https://agents.example.com")
    })?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none_or(str::is_empty)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("--public-url must be an origin only, like https://agents.example.com (no path)");
    }
    Ok(parsed.origin().ascii_serialization())
}

/// Decide the tunnel and address from the flags and what was saved before.
/// `--public-url ''` and `--tunnel none` clear what was saved.
pub fn plan(
    s: &Settings,
    tunnel_flag: Option<TunnelChoice>,
    url_flag: Option<&str>,
    no_sign_in: bool,
) -> Result<Plan> {
    let tunnel = tunnel_flag
        .or_else(|| s.value(settings::TUNNEL).and_then(TunnelChoice::from_name))
        .unwrap_or(TunnelChoice::None);
    let given = match url_flag {
        Some("") => None,
        Some(raw) => Some(normalize_url(raw)?),
        None => None,
    };
    let saved = s.value(settings::PUBLIC_URL).map(str::to_string);
    let saved_tunnel = s.value(settings::TUNNEL).and_then(TunnelChoice::from_name);
    // An address a tunnel picked for itself is not carried to another kind.
    let reusable = saved.clone().filter(|u| {
        saved_tunnel != Some(TunnelChoice::Tailscale) && !u.ends_with(".trycloudflare.com")
    });

    let url = match tunnel {
        // The tunnel names the address; one given by hand would be ignored.
        TunnelChoice::Tailscale => {
            if given.is_some() {
                bail!(
                    "Tailscale picks the address itself (https://<name>.<tailnet>.ts.net); \
                     drop --public-url, or set the name with TS_HOSTNAME"
                );
            }
            saved.filter(|_| saved_tunnel == Some(TunnelChoice::Tailscale))
        }
        TunnelChoice::Cloudflare => match (given, url_flag) {
            (Some(url), _) => Some(url),
            (None, Some(_)) => None,
            // Keep a named tunnel's address; a quick tunnel's is read again.
            (None, None) => reusable,
        },
        TunnelChoice::Ngrok => {
            let url = match (given, url_flag) {
                (Some(url), _) => Some(url),
                (None, Some(_)) => None,
                (None, None) => reusable,
            };
            if url.is_none() {
                bail!(
                    "--tunnel ngrok needs your ngrok static domain: \
                     --public-url https://<your-name>.ngrok-free.app"
                );
            }
            url
        }
        TunnelChoice::None => match (given, url_flag) {
            (Some(url), _) => Some(url),
            (None, Some(_)) => None,
            // Moving off a tunnel drops the address it gave.
            (None, None) if tunnel_flag == Some(TunnelChoice::None) => None,
            (None, None) => saved,
        },
    };

    if no_sign_in && (tunnel != TunnelChoice::None || url.is_some()) {
        bail!(
            "a public address needs sign-in: with --auth none anyone who reaches it is admin. \
             Use --auth single (or accounts), or drop --public-url/--tunnel"
        );
    }
    let service = service_for(tunnel, url.is_some());
    Ok(Plan {
        tunnel,
        url,
        service,
    })
}

/// Save the plan in `.env`: the tunnel, its Compose profile and the address.
/// Profiles a person added themselves (`scale`, `metrics`) are kept.
pub fn apply(s: &mut Settings, plan: &Plan) {
    s.set(settings::TUNNEL, plan.tunnel.name());
    s.set(settings::PUBLIC_URL, plan.url.as_deref().unwrap_or(""));
    let mut profiles: Vec<String> = s
        .value(settings::COMPOSE_PROFILES)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !TUNNEL_SERVICES.iter().any(|t| t.profile == *p))
        .map(str::to_string)
        .collect();
    if let Some(service) = plan.service {
        profiles.push(service.profile.to_string());
    }
    s.set(settings::COMPOSE_PROFILES, &profiles.join(","));
}

/// Make sure the tunnel's secret is saved: from the shell, then a prompt.
pub fn ensure_token(s: &mut Settings, plan: &Plan, can_ask: bool) -> Result<()> {
    let Some(var) = plan.tunnel.token_env(plan.url.is_some()) else {
        return Ok(());
    };
    if let Some(value) = std::env::var(var).ok().filter(|v| !v.trim().is_empty()) {
        s.set(var, value.trim());
        return Ok(());
    }
    if s.value(var).is_some() {
        return Ok(());
    }
    if !can_ask {
        bail!(
            "--tunnel {} needs {var} in the environment: {}",
            plan.tunnel.name(),
            plan.tunnel.token_help()
        );
    }
    eprintln!(
        "{}: {}",
        plan.tunnel.token_label(),
        plan.tunnel.token_help()
    );
    let value = dialoguer::Password::new()
        .with_prompt(plan.tunnel.token_label())
        .interact()
        .context("Failed to read the token")?;
    s.set(var, value.trim());
    Ok(())
}

/// The `*.ts.net` name from `tailscale status --json`, once logged in.
pub fn tailscale_url(status_json: &str) -> Option<String> {
    let status: serde_json::Value = serde_json::from_str(status_json).ok()?;
    if status["BackendState"].as_str() != Some("Running") {
        return None;
    }
    let name = status["Self"]["DNSName"].as_str()?.trim_end_matches('.');
    (!name.is_empty()).then(|| format!("https://{name}"))
}

/// The last `https://*.trycloudflare.com` address in cloudflared's log.
pub fn quick_tunnel_url(logs: &str) -> Option<String> {
    logs.split(|c: char| c.is_whitespace() || c == '|' || c == '"')
        .rfind(|w| {
            w.starts_with("https://")
                && w.ends_with(".trycloudflare.com")
                && *w != "https://api.trycloudflare.com"
        })
        .map(str::to_string)
}

/// Wait for a tunnel that picks its own address to report it.
pub async fn discover_url(compose: impl Fn() -> Command, service: TunnelService) -> Result<String> {
    let started = Instant::now();
    let mut told = false;
    loop {
        let found = match service.service {
            "tailscale" => compose()
                .args(["exec", "-T", "tailscale", "tailscale", "status", "--json"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .and_then(|o| tailscale_url(&String::from_utf8_lossy(&o.stdout))),
            _ => compose()
                .args(["logs", "--no-log-prefix", service.service])
                .output()
                .ok()
                .and_then(|o| {
                    let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.push_str(&String::from_utf8_lossy(&o.stderr));
                    quick_tunnel_url(&text)
                }),
        };
        if let Some(url) = found {
            return Ok(url);
        }
        if started.elapsed() > DISCOVER_TIMEOUT {
            let hint = if service.service == "tailscale" {
                "check the auth key (`everruns docker logs tailscale`); \
                 a used one-off key needs replacing in .env as TS_AUTHKEY"
            } else {
                "see `everruns docker logs cloudflared-quick`"
            };
            bail!(
                "the {} tunnel did not report its address; {hint}",
                service.profile
            );
        }
        if !told && started.elapsed() > Duration::from_secs(3) {
            eprintln!(
                "Waiting for the {} tunnel to report its address...",
                service.profile
            );
            told = true;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Whether the public address answers `/health`. A new Tailscale or Cloudflare
/// name can take a minute to resolve, so this retries before giving up.
pub async fn reachable(url: &str) -> bool {
    let Ok(http) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return false;
    };
    let started = Instant::now();
    loop {
        if let Ok(resp) = http.get(format!("{url}/health")).send().await
            && resp.status().is_success()
        {
            return true;
        }
        if started.elapsed() > REACH_TIMEOUT {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// What to check when the address does not answer.
pub fn unreachable_hint(tunnel: TunnelChoice) -> &'static str {
    match tunnel {
        TunnelChoice::Tailscale => {
            "Funnel must be allowed for this machine in your tailnet policy \
             (https://login.tailscale.com/admin/acls, nodeAttrs \"funnel\") and HTTPS \
             certificates turned on (https://login.tailscale.com/admin/dns)"
        }
        TunnelChoice::Cloudflare => {
            "check that the tunnel's public hostname points at http://caddy:9300 \
             (`everruns docker logs cloudflared`)"
        }
        TunnelChoice::Ngrok => "see `everruns docker logs ngrok`",
        TunnelChoice::None => "check your reverse proxy or DNS points at this machine's port",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(pairs: &[(&str, &str)]) -> Settings {
        let mut s = Settings::default();
        for (k, v) in pairs {
            s.set(k, v);
        }
        s
    }

    #[test]
    fn urls_are_reduced_to_their_origin() {
        assert_eq!(
            normalize_url("https://Agents.Example.com/").unwrap(),
            "https://agents.example.com"
        );
        assert_eq!(
            normalize_url("http://192.168.1.5:9300").unwrap(),
            "http://192.168.1.5:9300"
        );
        for bad in [
            "agents.example.com",
            "ftp://x.example.com",
            "https://x.example.com/app",
            "https://u:p@x.example.com",
            "https://x.example.com/?a=1",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_sign_in_never_gets_a_public_address() {
        let s = Settings::default();
        assert!(plan(&s, Some(TunnelChoice::Tailscale), None, true).is_err());
        assert!(plan(&s, None, Some("https://a.example.com"), true).is_err());
        assert!(plan(&s, None, None, true).is_ok());
        // A saved address counts too.
        let s = saved(&[(settings::PUBLIC_URL, "https://a.example.com")]);
        assert!(plan(&s, None, None, true).is_err());
    }

    #[test]
    fn a_given_url_is_kept_and_can_be_cleared() {
        let p = plan(
            &Settings::default(),
            None,
            Some("https://a.example.com/"),
            false,
        )
        .unwrap();
        assert_eq!(p.url.as_deref(), Some("https://a.example.com"));
        assert_eq!(p.service, None);

        let s = saved(&[(settings::PUBLIC_URL, "https://a.example.com")]);
        assert_eq!(
            plan(&s, None, None, false).unwrap().url.as_deref(),
            Some("https://a.example.com")
        );
        assert_eq!(plan(&s, None, Some(""), false).unwrap().url, None);
    }

    #[test]
    fn cloudflare_is_named_with_an_address_and_quick_without() {
        let named = plan(
            &Settings::default(),
            Some(TunnelChoice::Cloudflare),
            Some("https://agents.example.com"),
            false,
        )
        .unwrap();
        assert_eq!(named.service.unwrap().service, "cloudflared");
        let quick = plan(
            &Settings::default(),
            Some(TunnelChoice::Cloudflare),
            None,
            false,
        )
        .unwrap();
        assert_eq!(quick.service.unwrap().service, "cloudflared-quick");
        assert!(quick.service.unwrap().discovers_url);

        // A quick tunnel's old address is not reused: it changes on restart.
        let s = saved(&[
            (settings::TUNNEL, "cloudflare"),
            (settings::PUBLIC_URL, "https://old-words.trycloudflare.com"),
        ]);
        let again = plan(&s, None, None, false).unwrap();
        assert_eq!(again.url, None);
        assert_eq!(again.service.unwrap().service, "cloudflared-quick");
    }

    #[test]
    fn tailscale_names_its_own_address() {
        assert!(
            plan(
                &Settings::default(),
                Some(TunnelChoice::Tailscale),
                Some("https://a.example.com"),
                false
            )
            .is_err()
        );
        // A hand-set address from before is not carried into Tailscale.
        let s = saved(&[(settings::PUBLIC_URL, "https://a.example.com")]);
        let p = plan(&s, Some(TunnelChoice::Tailscale), None, false).unwrap();
        assert_eq!(p.url, None);
        let s = saved(&[
            (settings::TUNNEL, "tailscale"),
            (settings::PUBLIC_URL, "https://everruns.tail1.ts.net"),
        ]);
        assert_eq!(
            plan(&s, None, None, false).unwrap().url.as_deref(),
            Some("https://everruns.tail1.ts.net")
        );
    }

    #[test]
    fn ngrok_needs_its_static_domain() {
        assert!(plan(&Settings::default(), Some(TunnelChoice::Ngrok), None, false).is_err());
        let p = plan(
            &Settings::default(),
            Some(TunnelChoice::Ngrok),
            Some("https://me.ngrok-free.app"),
            false,
        )
        .unwrap();
        assert_eq!(p.service.unwrap().profile, "ngrok");
    }

    #[test]
    fn turning_the_tunnel_off_drops_its_address() {
        let s = saved(&[
            (settings::TUNNEL, "tailscale"),
            (settings::PUBLIC_URL, "https://everruns.tail1.ts.net"),
        ]);
        let p = plan(&s, Some(TunnelChoice::None), None, false).unwrap();
        assert_eq!(
            (p.tunnel, p.url, p.service),
            (TunnelChoice::None, None, None)
        );
    }

    #[test]
    fn apply_swaps_only_the_tunnel_profile() {
        let mut s = saved(&[(settings::COMPOSE_PROFILES, "scale,ngrok,metrics")]);
        let p = plan(
            &Settings::default(),
            Some(TunnelChoice::Tailscale),
            None,
            false,
        )
        .unwrap();
        apply(&mut s, &p);
        assert_eq!(
            s.value(settings::COMPOSE_PROFILES),
            Some("scale,metrics,tailscale")
        );
        assert_eq!(s.value(settings::TUNNEL), Some("tailscale"));

        let off = plan(&s, Some(TunnelChoice::None), None, false).unwrap();
        apply(&mut s, &off);
        assert_eq!(s.value(settings::COMPOSE_PROFILES), Some("scale,metrics"));
        assert_eq!(s.value(settings::PUBLIC_URL), None);
    }

    #[test]
    fn every_tunnel_service_is_in_the_compose_file() {
        for t in TUNNEL_SERVICES {
            assert!(
                settings::COMPOSE_FILE
                    .contains(&format!("  {}:\n    profiles: [{}]", t.service, t.profile)),
                "compose file lacks service {} in profile {}",
                t.service,
                t.profile
            );
        }
        assert!(settings::COMPOSE_FILE.contains("PUBLIC_APP_URL: ${PUBLIC_APP_URL:-}"));
    }

    #[test]
    fn tailscale_address_comes_from_status_once_running() {
        let running =
            r#"{"BackendState":"Running","Self":{"DNSName":"everruns.tail1234.ts.net."}}"#;
        assert_eq!(
            tailscale_url(running).as_deref(),
            Some("https://everruns.tail1234.ts.net")
        );
        let login = r#"{"BackendState":"NeedsLogin","Self":{"DNSName":""}}"#;
        assert_eq!(tailscale_url(login), None);
        assert_eq!(tailscale_url("not json"), None);
    }

    #[test]
    fn quick_tunnel_address_is_read_from_the_log() {
        let log = "2026-11-08T21:00:00Z INF Requesting new quick Tunnel on trycloudflare.com...\n\
            2026-11-08T21:00:01Z INF |  https://brave-lucky-words-here.trycloudflare.com                    |\n";
        assert_eq!(
            quick_tunnel_url(log).as_deref(),
            Some("https://brave-lucky-words-here.trycloudflare.com")
        );
        assert_eq!(quick_tunnel_url("nothing yet"), None);
    }

    #[test]
    fn tokens_come_from_the_shell_first() {
        // SAFETY: no other test reads or writes NGROK_AUTHTOKEN.
        unsafe { std::env::set_var(settings::NGROK_AUTHTOKEN, " tok-1 ") };
        let mut s = Settings::default();
        let p = plan(
            &s,
            Some(TunnelChoice::Ngrok),
            Some("https://me.ngrok-free.app"),
            false,
        )
        .unwrap();
        ensure_token(&mut s, &p, false).unwrap();
        unsafe { std::env::remove_var(settings::NGROK_AUTHTOKEN) };
        assert_eq!(s.value(settings::NGROK_AUTHTOKEN), Some("tok-1"));
        // Saved: no prompt needed.
        ensure_token(&mut s, &p, false).unwrap();
    }

    #[test]
    fn a_missing_token_without_a_terminal_names_the_variable() {
        let mut s = Settings::default();
        let p = plan(
            &s,
            Some(TunnelChoice::Cloudflare),
            Some("https://agents.example.com"),
            false,
        )
        .unwrap();
        let err = ensure_token(&mut s, &p, false).unwrap_err().to_string();
        assert!(err.contains("CLOUDFLARE_TUNNEL_TOKEN"), "{err}");
        // The quick tunnel needs no token.
        let quick = plan(&s, Some(TunnelChoice::Cloudflare), None, false).unwrap();
        ensure_token(&mut s, &quick, false).unwrap();
    }
}
