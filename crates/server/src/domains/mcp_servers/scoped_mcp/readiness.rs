//! Opt-in Platform discovery handoff. Credentials are resolved before every
//! lookup; only the HTTP fetch outlives a turn, never the permission decision.
use super::*;
use everruns_core::mcp::{HttpToolsList, MCP_DISCOVERY_DEADLINE, MCP_DISCOVERY_MAX_BYTES};
use std::time::Instant;
use tokio::sync::{Notify, Semaphore};

const MAX_JOBS: usize = 16;
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 128 * 1024 * 1024;
type Key = [u8; 32];

#[derive(Debug, thiserror::Error)]
#[error("MCP discovery is still pending")]
pub(super) struct Pending;
#[derive(Debug, thiserror::Error)]
#[error("MCP authentication rejected")]
pub(super) struct AuthenticationRequired;

struct Entry {
    started: Instant,
    notify: Arc<Notify>,
    completion: Arc<std::sync::OnceLock<Completion>>,
    bytes: usize,
}

struct Completion {
    finished: Instant,
    result: Result<HttpToolsList, bool>,
}

impl Entry {
    fn age(&self) -> Duration {
        self.completion
            .get()
            .map_or(self.started, |done| done.finished)
            .elapsed()
    }
}

fn harvest(completion: &Completion) -> HttpToolsList {
    let elapsed = completion.finished.elapsed();
    let mut result = completion.result.as_ref().expect("successful").clone();
    if let Some(hints) = &mut result.cache_hints {
        match hints.ttl.checked_sub(elapsed).filter(|ttl| !ttl.is_zero()) {
            Some(remaining) => hints.ttl = remaining,
            None => result.cache_hints = None,
        }
    }
    result
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
static SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(MAX_JOBS)));

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Snapshot {
    pub config: String,
    pub viewer: Option<Uuid>,
    pub status: String,
}

pub(crate) fn config_digest(server: &ScopedMcpServer) -> String {
    let mut value = serde_json::to_value(server).expect("MCP config is serializable");
    value.sort_all_objects();
    hex::encode(Sha256::digest(
        serde_json::to_vec(&value).expect("JSON is serializable"),
    ))
}

#[cfg(test)]
tokio::task_local! {
    static TEST_READINESS_BUDGET: Duration;
}

pub(crate) struct Readiness {
    org_id: i64,
    session: SessionId,
    deadline: tokio::time::Instant,
    egress: Arc<dyn EgressService>,
    pub(crate) snapshots: Mutex<HashMap<String, Snapshot>>,
    pub(crate) pending: Mutex<Vec<String>>,
    pub(crate) authentication_required: Mutex<Vec<String>>,
}

impl Readiness {
    pub(crate) fn from_env(
        org_id: i64,
        session: SessionId,
        egress: Arc<dyn EgressService>,
    ) -> Option<Self> {
        #[cfg(test)]
        if let Ok(budget) = TEST_READINESS_BUDGET.try_with(|budget| *budget) {
            return Some(Self::new(org_id, session, egress, budget));
        }
        let millis = std::env::var("EVERRUNS_MCP_DISCOVERY_READINESS_MS")
            .ok()?
            .parse::<u64>()
            .ok()?;
        Some(Self::new(
            org_id,
            session,
            egress,
            Duration::from_millis(millis),
        ))
    }

    /// Exercise the real worker loading path without mutating process-wide env.
    #[cfg(test)]
    pub(crate) async fn with_test_budget<F: std::future::Future>(
        budget: Duration,
        work: F,
    ) -> F::Output {
        TEST_READINESS_BUDGET.scope(budget, work).await
    }

    pub(crate) fn new(
        org_id: i64,
        session: SessionId,
        egress: Arc<dyn EgressService>,
        budget: Duration,
    ) -> Self {
        Self {
            org_id,
            session,
            deadline: tokio::time::Instant::now() + budget.min(MCP_DISCOVERY_DEADLINE),
            egress,
            snapshots: Mutex::new(HashMap::new()),
            pending: Mutex::new(Vec::new()),
            authentication_required: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn observe(&self, name: &str, server: &ScopedMcpServer, viewer: Option<Uuid>) {
        self.snapshots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                name.to_owned(),
                Snapshot {
                    config: config_digest(server),
                    viewer,
                    status: "ready".into(),
                },
            );
    }

    pub(super) fn authentication_required(&self, name: &str) -> anyhow::Error {
        self.authentication_required
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name.to_owned());
        AuthenticationRequired.into()
    }

    fn pending(&self, name: &str) -> anyhow::Error {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name.to_owned());
        Pending.into()
    }
}

// Hash the entire materialized config, actual credential, and resolved catalog
// identity. Neither credentials nor configs are retained in coordinator keys.
fn key(
    context: &Readiness,
    name: &str,
    server: &ScopedMcpServer,
    identity: &str,
    token: Option<&str>,
) -> Result<Key> {
    let mut value = serde_json::to_value((
        context.org_id,
        context.session,
        name,
        server,
        identity,
        token,
    ))?;
    value.sort_all_objects();
    Ok(Sha256::digest(serde_json::to_vec(&value)?).into())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn fetch(
    context: Option<&Readiness>,
    name: &str,
    server: &ScopedMcpServer,
    identity: &str,
    egress: &dyn EgressService,
    url: &str,
    token: Option<&str>,
    headers: &HashMap<String, String>,
) -> Result<HttpToolsList> {
    let Some(context) = context else {
        return fetch_mcp_tools_with_cache_hints(egress, url, token, headers).await;
    };
    let key = key(context, name, server, identity, token)?;
    let (notify, completion) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .entries
            .retain(|_, entry| entry.age() < MCP_DISCOVERY_DEADLINE + Duration::from_secs(60));
        if let Some(entry) = state.entries.get(&key) {
            match entry.completion.get().map(|done| &done.result) {
                Some(Ok(_)) => {
                    let tools = harvest(entry.completion.get().expect("completed"));
                    if tools.cache_hints.is_none() {
                        // Even an expired TTL may be delivered once as a
                        // delayed response. It is not fresh cache data and
                        // carries no hints that could write a new DB TTL.
                        // Existing waiters retain their completion generation.
                        state.entries.remove(&key);
                    }
                    return Ok(tools);
                }
                Some(Err(auth)) if *auth || entry.age() < Duration::from_secs(5) => {
                    return Err(if *auth {
                        context.authentication_required(name)
                    } else {
                        anyhow!("MCP discovery failed")
                    });
                }
                Some(Err(_)) => {
                    state.entries.remove(&key);
                }
                None => {}
            }
        }
        if let Some(entry) = state.entries.get(&key) {
            (entry.notify.clone(), entry.completion.clone())
        } else {
            let Ok(permit) = SLOTS.clone().try_acquire_owned() else {
                return Err(context.pending(name));
            };
            if state.entries.len() >= MAX_ENTRIES {
                return Err(context.pending(name));
            }
            let notify = Arc::new(Notify::new());
            let completion = Arc::new(std::sync::OnceLock::new());
            state.entries.insert(
                key,
                Entry {
                    started: Instant::now(),
                    notify: notify.clone(),
                    completion: completion.clone(),
                    bytes: 0,
                },
            );
            let egress = context.egress.clone();
            let url = url.to_owned();
            let token = token.map(str::to_owned);
            let headers = headers.clone();
            let task_completion = completion.clone();
            let task_notify = notify.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let result = tokio::time::timeout(
                    MCP_DISCOVERY_DEADLINE,
                    fetch_mcp_tools_with_cache_hints(
                        egress.as_ref(),
                        &url,
                        token.as_deref(),
                        &headers,
                    ),
                )
                .await;
                let mut result = match result {
                    Ok(Ok(result)) => Ok(result),
                    Ok(Err(error)) => Err(error
                        .downcast_ref::<everruns_core::mcp::McpHttpStatusError>()
                        .is_some_and(|e| matches!(e.status, 401 | 403))),
                    Err(_) => Err(false),
                };
                let bytes = result
                    .as_ref()
                    .ok()
                    .and_then(|r| serde_json::to_vec(&(&r.tools, &r.server_info)).ok())
                    .map_or(0, |b| b.len());
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let retained: usize = state.entries.values().map(|e| e.bytes).sum();
                if bytes > MCP_DISCOVERY_MAX_BYTES || retained.saturating_add(bytes) > MAX_BYTES {
                    result = Err(false);
                }
                if let Some(entry) = state.entries.get_mut(&key)
                    && Arc::ptr_eq(&entry.completion, &task_completion)
                {
                    entry.bytes = if result.is_ok() { bytes } else { 0 };
                } else {
                    // An evicted generation must neither overwrite its
                    // replacement nor retain an unaccounted catalog.
                    result = Err(false);
                }
                let _ = task_completion.set(Completion {
                    finished: Instant::now(),
                    result,
                });
                task_notify.notify_waiters();
            });
            (notify, completion)
        }
    };
    // Register before inspecting completion, so notification cannot race the wait.
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    if completion.get().is_none()
        && tokio::time::timeout_at(context.deadline, notified)
            .await
            .is_err()
    {
        return Err(context.pending(name));
    }
    match completion.get() {
        Some(done) => match &done.result {
            Ok(_) => {
                let tools = harvest(done);
                if tools.cache_hints.is_none() {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if state
                        .entries
                        .get(&key)
                        .is_some_and(|entry| Arc::ptr_eq(&entry.completion, &completion))
                    {
                        state.entries.remove(&key);
                    }
                }
                Ok(tools)
            }
            Err(auth) => Err(if *auth {
                context.authentication_required(name)
            } else {
                anyhow!("MCP discovery failed")
            }),
        },
        None => Err(context.pending(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::CatalogPreviewEgress;
    use super::*;

    struct SlowEgress(CatalogPreviewEgress);
    #[async_trait::async_trait]
    impl EgressService for SlowEgress {
        async fn send(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.0.send(request).await
        }
        async fn send_stream(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(everruns_core::EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter(vec![Ok(response.body)])),
            })
        }
    }

    #[tokio::test]
    async fn pending_discovery_is_harvested_without_restarting_http() {
        let egress = Arc::new(SlowEgress(CatalogPreviewEgress::default()));
        let session = SessionId::new();
        let context = Readiness::new(1, session, egress.clone(), Duration::ZERO);
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/mcp".into(),
            ..Default::default()
        };
        let result = fetch(
            Some(&context),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await;
        assert!(result.unwrap_err().is::<Pending>());
        let next = Readiness::new(1, session, egress.clone(), Duration::from_secs(2));
        let result = fetch(
            Some(&next),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await
        .unwrap();
        assert!(!result.tools.is_empty());
        assert_eq!(egress.0.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(next.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_waiters_share_the_completed_catalog() {
        let egress = Arc::new(SlowEgress(CatalogPreviewEgress::default()));
        let context = Readiness::new(1, SessionId::new(), egress.clone(), Duration::from_secs(2));
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/concurrent".into(),
            ..Default::default()
        };
        let lookup = || {
            fetch(
                Some(&context),
                "docs",
                &server,
                "inline",
                egress.as_ref(),
                &server.url,
                None,
                &server.headers,
            )
        };
        let (first, second) = tokio::join!(lookup(), lookup());
        assert_eq!(
            serde_json::to_value(first.unwrap().tools).unwrap(),
            serde_json::to_value(second.unwrap().tools).unwrap()
        );
        assert_eq!(egress.0.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_hint_result_is_one_handoff_not_a_successive_turn_cache() {
        let egress = Arc::new(SlowEgress(CatalogPreviewEgress::default()));
        let session = SessionId::new();
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/no-hints".into(),
            ..Default::default()
        };
        let initial = Readiness::new(1, session, egress.clone(), Duration::ZERO);
        assert!(
            fetch(
                Some(&initial),
                "docs",
                &server,
                "inline",
                egress.as_ref(),
                &server.url,
                None,
                &server.headers
            )
            .await
            .unwrap_err()
            .is::<Pending>()
        );
        let handoff = Readiness::new(1, session, egress.clone(), Duration::from_secs(2));
        let result = fetch(
            Some(&handoff),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await
        .unwrap();
        assert!(result.cache_hints.is_none());
        assert_eq!(egress.0.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let next_turn = Readiness::new(1, session, egress.clone(), Duration::from_secs(2));
        fetch(
            Some(&next_turn),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await
        .unwrap();
        assert_eq!(egress.0.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn expired_short_ttl_is_delivered_once_without_restarting_freshness() {
        struct ShortTtlEgress(SlowEgress);
        #[async_trait::async_trait]
        impl EgressService for ShortTtlEgress {
            async fn send(
                &self,
                request: everruns_core::EgressRequest,
            ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
                let listing = serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["method"]
                    == "tools/list";
                let mut response = self.0.send(request).await?;
                if listing {
                    let mut body: serde_json::Value =
                        serde_json::from_slice(&response.body).unwrap();
                    body["result"]["ttlMs"] = serde_json::json!(10);
                    body["result"]["cacheScope"] = serde_json::json!("private");
                    response.body = serde_json::to_vec(&body).unwrap();
                }
                Ok(response)
            }
            async fn send_stream(
                &self,
                request: everruns_core::EgressRequest,
            ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
                let response = self.send(request).await?;
                Ok(everruns_core::EgressStreamResponse {
                    status: response.status,
                    headers: response.headers,
                    body: Box::pin(futures::stream::iter(vec![Ok(response.body)])),
                })
            }
        }
        let egress = Arc::new(ShortTtlEgress(SlowEgress(CatalogPreviewEgress::default())));
        let session = SessionId::new();
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/short-ttl".into(),
            ..Default::default()
        };
        let initial = Readiness::new(1, session, egress.clone(), Duration::from_millis(1));
        assert!(
            fetch(
                Some(&initial),
                "docs",
                &server,
                "inline",
                egress.as_ref(),
                &server.url,
                None,
                &server.headers
            )
            .await
            .unwrap_err()
            .is::<Pending>()
        );
        let key = key(&initial, "docs", &server, "inline", None).unwrap();
        let completion = STATE.lock().unwrap().entries[&key].completion.clone();
        tokio::time::timeout(Duration::from_secs(2), async {
            while completion.get().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            completion
                .get()
                .unwrap()
                .result
                .as_ref()
                .unwrap()
                .cache_hints
                .is_some()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        let handoff = Readiness::new(1, session, egress.clone(), Duration::ZERO);
        let tools = fetch(
            Some(&handoff),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await
        .unwrap();
        assert!(!tools.tools.is_empty());
        assert!(
            tools.cache_hints.is_none(),
            "expired response must not write a new DB TTL"
        );
        assert_eq!(
            egress.0.0.calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let next = Readiness::new(1, session, egress.clone(), Duration::from_secs(2));
        fetch(
            Some(&next),
            "docs",
            &server,
            "inline",
            egress.as_ref(),
            &server.url,
            None,
            &server.headers,
        )
        .await
        .unwrap();
        assert_eq!(
            egress.0.0.calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    }

    #[tokio::test]
    async fn evicted_job_cannot_overwrite_replacement_generation() {
        let egress = Arc::new(SlowEgress(CatalogPreviewEgress::default()));
        let context = Readiness::new(1, SessionId::new(), egress.clone(), Duration::ZERO);
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/generation".into(),
            ..Default::default()
        };
        assert!(
            fetch(
                Some(&context),
                "docs",
                &server,
                "inline",
                egress.as_ref(),
                &server.url,
                None,
                &server.headers
            )
            .await
            .unwrap_err()
            .is::<Pending>()
        );
        let key = key(&context, "docs", &server, "inline", None).unwrap();
        let replacement = Arc::new(std::sync::OnceLock::new());
        let old = {
            let mut state = STATE.lock().unwrap();
            let old = state.entries.remove(&key).unwrap().completion;
            state.entries.insert(
                key,
                Entry {
                    started: Instant::now(),
                    notify: Arc::new(Notify::new()),
                    completion: replacement.clone(),
                    bytes: 0,
                },
            );
            old
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while old.get().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(old.get().unwrap().result.is_err());
        assert!(replacement.get().is_none());
        STATE.lock().unwrap().entries.remove(&key);
    }

    #[tokio::test]
    async fn authentication_failure_is_not_pending_and_holds_retries() {
        struct Denied(std::sync::atomic::AtomicUsize);
        #[async_trait::async_trait]
        impl EgressService for Denied {
            async fn send(
                &self,
                _request: everruns_core::EgressRequest,
            ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(everruns_core::EgressResponse {
                    status: 401,
                    headers: Default::default(),
                    body: Vec::new(),
                })
            }
            async fn send_stream(
                &self,
                request: everruns_core::EgressRequest,
            ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
                let response = self.send(request).await?;
                Ok(everruns_core::EgressStreamResponse {
                    status: response.status,
                    headers: response.headers,
                    body: Box::pin(futures::stream::iter(vec![Ok(response.body)])),
                })
            }
        }
        let egress = Arc::new(Denied(std::sync::atomic::AtomicUsize::new(0)));
        let context = Readiness::new(1, SessionId::new(), egress.clone(), Duration::from_secs(2));
        let server = ScopedMcpServer {
            url: "http://8.8.8.8/denied".into(),
            ..Default::default()
        };
        for _ in 0..2 {
            let error = fetch(
                Some(&context),
                "docs",
                &server,
                "inline",
                egress.as_ref(),
                &server.url,
                None,
                &server.headers,
            )
            .await
            .unwrap_err();
            assert!(error.is::<AuthenticationRequired>());
        }
        assert!(context.pending.lock().unwrap().is_empty());
        assert!(!context.authentication_required.lock().unwrap().is_empty());
        assert_eq!(egress.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn handoff_does_not_restart_upstream_cache_ttl() {
        let entry = |age| Completion {
            finished: Instant::now() - age,
            result: Ok(HttpToolsList {
                tools: Vec::new(),
                server_info: None,
                cache_hints: Some(CacheHints {
                    ttl: Duration::from_secs(10),
                    scope: CacheScope::Private,
                }),
            }),
        };
        let fresh = harvest(&entry(Duration::from_secs(3)));
        assert!(fresh.cache_hints.unwrap().ttl <= Duration::from_secs(7));
        let expired = harvest(&entry(Duration::from_secs(11)));
        assert!(expired.cache_hints.is_none());
    }

    #[test]
    fn handoff_keys_isolate_authority_and_configuration() {
        let egress = Arc::new(CatalogPreviewEgress::default());
        let session = SessionId::new();
        let context = Readiness::new(1, session, egress.clone(), Duration::ZERO);
        let server = ScopedMcpServer::default();
        let original = key(&context, "docs", &server, "user-a", Some("token-a")).unwrap();
        assert_ne!(
            original,
            key(&context, "docs", &server, "user-b", Some("token-a")).unwrap()
        );
        assert_ne!(
            original,
            key(&context, "docs", &server, "user-a", Some("token-b")).unwrap()
        );
        let other_org = Readiness::new(2, session, egress.clone(), Duration::ZERO);
        assert_ne!(
            original,
            key(&other_org, "docs", &server, "user-a", Some("token-a")).unwrap()
        );
        let other_session = Readiness::new(1, SessionId::new(), egress, Duration::ZERO);
        assert_ne!(
            original,
            key(&other_session, "docs", &server, "user-a", Some("token-a")).unwrap()
        );
        let changed = ScopedMcpServer {
            url: "https://example.com/mcp".into(),
            ..server
        };
        assert_ne!(
            original,
            key(&context, "docs", &changed, "user-a", Some("token-a")).unwrap()
        );
    }
}
