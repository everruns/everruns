//! Where a pushed secret's value comes from (agentd design, "Secrets"): the
//! value itself (stored), the executor's environment (env), or a resolver
//! the executor asks at the turn (resolve). The sealed record holds the
//! value, or only where to find it.
//!
//! Decisions:
//! - One record shape for all three: a stored value is sealed as is (what
//!   the hosted control plane pushes today), and the other two seal a
//!   marker no real value starts with (a NUL byte), so nothing that reads
//!   secrets needs to know which it got. Only the executor that runs the
//!   turn turns a marker into a value, in memory.
//! - Environment values are off unless the install allows them
//!   (`--env-secrets`): one model key in the environment would otherwise
//!   serve every session that names its provider.
//! - A resolver is a secret source like Vault, not a host: agentd asks it
//!   for a value (`POST {resolver}/resolve {name, ref, group}` answering
//!   `{value, expires_in}`), never for an action. Values are kept in memory
//!   until they expire and never written down. A breaker (5 misses in a
//!   row, then 10 s open) keeps a slow resolver from holding every turn.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// What starts a source marker: no stored secret starts with a NUL.
const MARKER: &[u8] = b"\0agentd-source\0";

/// Whether this process lets secrets come from its environment.
static ENV_ALLOWED: AtomicBool = AtomicBool::new(false);

/// Let secrets come from this process's environment (`--env-secrets`).
pub fn allow_env(allowed: bool) {
    ENV_ALLOWED.store(allowed, Ordering::SeqCst);
}

/// A resolved value's key: resolver URL, reference, group.
type CacheKey = (String, String, String);

/// A secret whose value a resolver gives.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resolve {
    /// What the resolver knows the secret by.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The resolver's base URL; the install's (`AGENTD_RESOLVER_URL`) when
    /// unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
enum Source {
    Env { var: String },
    Resolve(Resolve),
}

/// What to seal for a pushed secret.
pub fn record(body: &crate::install::SecretBody) -> Result<Vec<u8>, String> {
    let source = match (&body.value, &body.env, &body.resolve) {
        (Some(value), None, None) => {
            if value.as_bytes().starts_with(b"\0") {
                return Err("a secret's value does not start with a NUL byte".into());
            }
            return Ok(value.clone().into_bytes());
        }
        (None, Some(var), None) if !var.is_empty() => Source::Env { var: var.clone() },
        (None, None, Some(resolve)) if !resolve.reference.is_empty() => {
            Source::Resolve(resolve.clone())
        }
        _ => return Err("give exactly one of `value`, `env` or `resolve`".into()),
    };
    let mut out = MARKER.to_vec();
    out.extend(serde_json::to_vec(&source).map_err(|e| e.to_string())?);
    Ok(out)
}

/// The value of an opened secret: itself, or what its source gives.
pub async fn open(plain: Vec<u8>, group: &str, name: &str) -> anyhow::Result<Vec<u8>> {
    let Some(rest) = plain.strip_prefix(MARKER) else {
        return Ok(plain);
    };
    match serde_json::from_slice::<Source>(rest)? {
        Source::Env { var } => {
            anyhow::ensure!(
                ENV_ALLOWED.load(Ordering::SeqCst),
                "secret {name} comes from the environment, which this install does not allow (--env-secrets)"
            );
            std::env::var(&var)
                .map(String::into_bytes)
                .map_err(|_| anyhow::anyhow!("secret {name}: {var} is not set on this executor"))
        }
        Source::Resolve(resolve) => RESOLVER.get(&resolve, group, name).await,
    }
}

/// Misses in a row that open the breaker, and for how long.
const BREAK_AFTER: u32 = 5;
const BREAK_FOR: Duration = Duration::from_secs(10);
/// How long one resolver call may take.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a value with no `expires_in` is kept.
const DEFAULT_KEEP: Duration = Duration::from_secs(60);

static RESOLVER: LazyLock<Resolver> = LazyLock::new(Resolver::default);

#[derive(Default)]
struct Resolver {
    http: reqwest::Client,
    /// (url, ref, group) → value and when it expires.
    cache: Mutex<HashMap<CacheKey, (Vec<u8>, Instant)>>,
    /// url → misses in a row, and when the breaker opened.
    breaker: Mutex<HashMap<String, (u32, Option<Instant>)>>,
}

#[derive(Deserialize)]
struct Answer {
    value: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl Resolver {
    async fn get(&self, resolve: &Resolve, group: &str, name: &str) -> anyhow::Result<Vec<u8>> {
        let url = resolve
            .url
            .clone()
            .or_else(|| std::env::var("AGENTD_RESOLVER_URL").ok())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow::anyhow!("secret {name}: no resolver (AGENTD_RESOLVER_URL)"))?;
        let key = (url.clone(), resolve.reference.clone(), group.to_owned());
        if let Some((value, until)) = self.cache.lock().get(&key)
            && Instant::now() < *until
        {
            return Ok(value.clone());
        }
        if let Some((_, Some(opened))) = self.breaker.lock().get(&url)
            && opened.elapsed() < BREAK_FOR
        {
            anyhow::bail!("secret {name}: the resolver at {url} is failing; trying again shortly");
        }
        match self.call(&url, resolve, group, name).await {
            Ok(answer) => {
                self.breaker.lock().remove(&url);
                let keep = answer.expires_in.map_or(DEFAULT_KEEP, Duration::from_secs);
                let value = answer.value.into_bytes();
                self.cache
                    .lock()
                    .insert(key, (value.clone(), Instant::now() + keep));
                Ok(value)
            }
            Err(error) => {
                let mut breaker = self.breaker.lock();
                let entry = breaker.entry(url).or_insert((0, None));
                entry.0 += 1;
                if entry.0 >= BREAK_AFTER {
                    *entry = (0, Some(Instant::now()));
                }
                Err(error)
            }
        }
    }

    async fn call(
        &self,
        url: &str,
        resolve: &Resolve,
        group: &str,
        name: &str,
    ) -> anyhow::Result<Answer> {
        let mut request = self
            .http
            .post(format!("{}/resolve", url.trim_end_matches('/')))
            .timeout(RESOLVE_TIMEOUT)
            .json(&serde_json::json!({
                "name": name,
                "ref": resolve.reference,
                "group": group,
            }));
        if let Ok(token) = std::env::var("AGENTD_RESOLVER_TOKEN")
            && !token.is_empty()
        {
            request = request.bearer_auth(token);
        }
        let answer = request
            .send()
            .await?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("secret {name}: the resolver answered {e}"))?
            .json()
            .await?;
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::SecretBody;

    #[tokio::test]
    async fn a_stored_value_opens_as_itself() {
        let body = SecretBody {
            value: Some("sk-1".into()),
            ..SecretBody::default()
        };
        let plain = record(&body).unwrap();
        assert_eq!(plain, b"sk-1");
        assert_eq!(open(plain, "g", "k").await.unwrap(), b"sk-1");
    }

    #[tokio::test]
    async fn an_env_value_needs_the_install_to_allow_it() {
        let body = SecretBody {
            env: Some("AGENTD_TEST_SECRET_SOURCE".into()),
            ..SecretBody::default()
        };
        let plain = record(&body).unwrap();
        assert!(plain.starts_with(MARKER));
        allow_env(false);
        let refused = open(plain.clone(), "g", "k").await.unwrap_err();
        assert!(refused.to_string().contains("--env-secrets"), "{refused}");
        // SAFETY: the variable is this test's own.
        unsafe { std::env::set_var("AGENTD_TEST_SECRET_SOURCE", "from-env") };
        allow_env(true);
        assert_eq!(open(plain, "g", "k").await.unwrap(), b"from-env");
        allow_env(false);
    }

    #[test]
    fn exactly_one_source() {
        let both = SecretBody {
            value: Some("a".into()),
            env: Some("B".into()),
            ..SecretBody::default()
        };
        assert!(record(&both).is_err());
        assert!(record(&SecretBody::default()).is_err());
    }

    #[tokio::test]
    async fn a_failing_resolver_opens_the_breaker() {
        let resolve = Resolve {
            reference: "r".into(),
            // Nothing listens here.
            url: Some("http://127.0.0.1:9".into()),
        };
        let resolver = Resolver::default();
        for _ in 0..BREAK_AFTER {
            let error = resolver.get(&resolve, "g", "k").await.unwrap_err();
            assert!(!error.to_string().contains("failing"), "{error}");
        }
        let error = resolver.get(&resolve, "g", "k").await.unwrap_err();
        assert!(error.to_string().contains("failing"), "{error}");
    }
}
