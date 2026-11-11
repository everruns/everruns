//! A fake control plane serving `/dp/v1` to the cells (data plane v2 step
//! 6, design.md "Levels", level 2): a ledger that grants spend allowances
//! from each org's balance and takes settlements, a feed store, and knobs
//! that break it the ways the failure suite needs (down, lost replies,
//! short grants).
//!
//! It keeps the protocol's rules the way the real control plane must:
//! - an allowance request is answered once, by `request_id`: a retry gets
//!   the same grant, so a lost reply never grants twice;
//! - money leaves the balance when granted and returns only through a
//!   closed settlement, never by time;
//! - feed records are stored once by `(source, incarnation, index,
//!   ordinal)`, usage once by `attempt_id`, and a session summary never
//!   replaces a newer one.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use everruns_cell::dp::{
    AllowanceGrant, AllowanceRequest, ConfigAck, ConfigUpdate, FeedAck, FeedBatch, Record,
};
use parking_lot::Mutex;

#[derive(Debug, Clone, Default)]
pub struct IssuedGrant {
    pub org: String,
    pub granted: i64,
    pub used: i64,
    pub reserved: i64,
    pub closed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    pub version: u64,
    pub status: String,
    pub turns: u64,
}

#[derive(Default)]
struct Ledger {
    /// Money each org has not been granted yet.
    balances: HashMap<String, i64>,
    grants: BTreeMap<String, IssuedGrant>,
    answers: HashMap<String, AllowanceGrant>,
    seen: HashSet<(String, u64, u64, u32)>,
    usage: HashMap<String, (String, i64)>,
    usage_repeats: usize,
    sessions: HashMap<String, SessionSummary>,
    config_applied: Vec<(String, u64)>,
    org_events: Vec<(String, String)>,
    requests: usize,
    /// Feed records that arrived again under an id already stored.
    duplicates: usize,
}

struct Knobs {
    down: AtomicBool,
    /// Requests to answer as lost, per channel: processed, then a 503.
    /// Decision: one count per channel, so a test that loses replies
    /// knows both the allowance and the feed path saw it, whichever the
    /// cell happened to call first.
    lose_allowance_replies: AtomicUsize,
    lose_feed_replies: AtomicUsize,
    grant_ttl_ms: Mutex<i64>,
    per_call_micros: Mutex<Option<i64>>,
    /// The link's token: required on calls in, sent on calls out.
    link_token: Mutex<Option<String>>,
}

/// A running fake control plane.
#[derive(Clone)]
pub struct FakeCp {
    pub url: String,
    ledger: Arc<Mutex<Ledger>>,
    knobs: Arc<Knobs>,
    /// Signs org tokens; its key is served as the JWKS.
    pub signer: Arc<crate::signer::Signer>,
}

impl FakeCp {
    /// Serve on a free local port.
    pub async fn start() -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let cp = Self {
            url: format!("http://{}", listener.local_addr()?),
            ledger: Default::default(),
            knobs: Arc::new(Knobs {
                down: AtomicBool::new(false),
                lose_allowance_replies: AtomicUsize::new(0),
                lose_feed_replies: AtomicUsize::new(0),
                grant_ttl_ms: Mutex::new(60_000),
                per_call_micros: Mutex::new(Some(100)),
                link_token: Mutex::new(None),
            }),
            signer: Arc::new(crate::signer::Signer::new("cp-key-1")?),
        };
        let app = Router::new()
            .route("/dp/v1/tenants/{org}/allowances", post(allowance))
            .route("/dp/v1/feed", post(feed))
            .route("/.well-known/jwks.json", axum::routing::get(jwks))
            .with_state(cp.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(cp)
    }

    /// Require and send `token` on the link from now on.
    pub fn set_link_token(&self, token: &str) {
        *self.knobs.link_token.lock() = Some(token.to_owned());
    }

    /// Where the cells find the keys of org tokens.
    pub fn jwks_url(&self) -> String {
        format!("{}/.well-known/jwks.json", self.url)
    }

    /// A token for `org` on data plane `dp-local`, valid ten minutes.
    pub fn org_token(&self, org: &str, scope: &str) -> anyhow::Result<String> {
        self.signer.token(org, "dp-local", scope, 600)
    }

    /// Feed records that arrived again (a replay shows here).
    pub fn duplicates(&self) -> usize {
        self.ledger.lock().duplicates
    }

    /// `method` `path` on a data plane gateway or node with the link's
    /// token, as the control plane calls it; the reply's status and body.
    pub async fn call(
        &self,
        base: &str,
        method: reqwest::Method,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<(StatusCode, serde_json::Value)> {
        let http = reqwest::Client::builder().no_proxy().build()?;
        let mut request = http
            .request(method, format!("{base}{path}"))
            .json(body)
            .timeout(std::time::Duration::from_secs(30));
        if let Some(token) = self.knobs.link_token.lock().clone() {
            request = request.bearer_auth(token);
        }
        let reply = request.send().await?;
        let status = StatusCode::from_u16(reply.status().as_u16())?;
        let text = reply.text().await?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        ))
    }

    fn admit(&self, headers: &axum::http::HeaderMap) -> Result<(), (StatusCode, String)> {
        let Some(token) = self.knobs.link_token.lock().clone() else {
            return Ok(());
        };
        let sent = everruns_cell::org_token::bearer(headers);
        if sent == Some(token.as_str()) {
            Ok(())
        } else {
            Err((StatusCode::UNAUTHORIZED, "no link token".into()))
        }
    }

    pub fn set_balance(&self, org: &str, micros: i64) {
        self.ledger.lock().balances.insert(org.to_owned(), micros);
    }

    pub fn balance(&self, org: &str) -> i64 {
        self.ledger.lock().balances.get(org).copied().unwrap_or(0)
    }

    /// Answer nothing (503) while `down`.
    pub fn set_down(&self, down: bool) {
        self.knobs.down.store(down, Ordering::SeqCst);
    }

    /// Process the next `n` allowance requests and the next `n` feed
    /// batches but lose their replies.
    pub fn lose_replies(&self, n: usize) {
        self.knobs.lose_allowance_replies.store(n, Ordering::SeqCst);
        self.knobs.lose_feed_replies.store(n, Ordering::SeqCst);
    }

    /// How long new grants may be spent.
    pub fn set_grant_ttl_ms(&self, ms: i64) {
        *self.knobs.grant_ttl_ms.lock() = ms;
    }

    pub fn grants(&self) -> BTreeMap<String, IssuedGrant> {
        self.ledger.lock().grants.clone()
    }

    /// Allowance requests that reached the ledger (retries included).
    pub fn requests(&self) -> usize {
        self.ledger.lock().requests
    }

    /// Usage records stored, by attempt: `(org, cost)`.
    pub fn usage(&self) -> HashMap<String, (String, i64)> {
        self.ledger.lock().usage.clone()
    }

    /// Usage records that arrived again under another feed id (a replay
    /// across incarnations would show here).
    pub fn usage_repeats(&self) -> usize {
        self.ledger.lock().usage_repeats
    }

    pub fn sessions(&self) -> HashMap<String, SessionSummary> {
        self.ledger.lock().sessions.clone()
    }

    pub fn org_events(&self) -> Vec<(String, String)> {
        self.ledger.lock().org_events.clone()
    }

    pub fn config_applied(&self) -> Vec<(String, u64)> {
        self.ledger.lock().config_applied.clone()
    }

    /// Money granted and not returned: what the org could have spent.
    pub fn outstanding(&self, org: &str) -> i64 {
        self.ledger
            .lock()
            .grants
            .values()
            .filter(|g| g.org == org && !g.closed)
            .map(|g| g.granted)
            .sum()
    }

    fn lost(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }

    /// Push `update` for `org` to a data plane gateway, as the outbox
    /// does: retried until the data plane answers (a directory cell that
    /// is electing a leader answers 5xx for a moment), for up to 30 s.
    pub async fn push_config(
        &self,
        gateway: &str,
        org: &str,
        update: &ConfigUpdate,
    ) -> anyhow::Result<ConfigAck> {
        let http = reqwest::Client::builder().no_proxy().build()?;
        let url = format!(
            "{gateway}/dp/v1/orgs/{org}/config/{}/{}",
            update.epoch, update.version
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let token = self.knobs.link_token.lock().clone();
        loop {
            let mut request = http.put(&url).json(update);
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            let sent = request.send().await;
            let error = match sent {
                Ok(reply) if reply.status().is_success() => return Ok(reply.json().await?),
                Ok(reply) if reply.status().is_client_error() => {
                    anyhow::bail!("{url}: {}", reply.status())
                }
                Ok(reply) => anyhow::anyhow!("{url}: {}", reply.status()),
                Err(error) => error.into(),
            };
            if tokio::time::Instant::now() >= deadline {
                return Err(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

type Reply<T> = Result<Json<T>, (StatusCode, String)>;

fn unavailable() -> (StatusCode, String) {
    (StatusCode::SERVICE_UNAVAILABLE, "control plane down".into())
}

async fn jwks(State(cp): State<FakeCp>) -> Reply<jsonwebtoken::jwk::JwkSet> {
    cp.signer
        .jwks()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn allowance(
    State(cp): State<FakeCp>,
    Path(org): Path<String>,
    headers: axum::http::HeaderMap,
    Json(request): Json<AllowanceRequest>,
) -> Reply<AllowanceGrant> {
    cp.admit(&headers)?;
    if cp.knobs.down.load(Ordering::SeqCst) {
        return Err(unavailable());
    }
    let answer = {
        let mut ledger = cp.ledger.lock();
        ledger.requests += 1;
        // Settlements are cumulative, so applying one again changes nothing.
        for s in &request.settlements {
            let returned = match ledger.grants.get_mut(&s.grant_id) {
                Some(g) if g.org == org && !g.closed => {
                    g.used = g.used.max(s.used_micros);
                    g.reserved = s.reserved_micros;
                    if s.closed {
                        g.closed = true;
                        Some(g.granted - g.used)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(back) = returned {
                *ledger.balances.entry(org.clone()).or_default() += back.max(0);
            }
        }
        if let Some(answer) = ledger.answers.get(&request.request_id) {
            answer.clone()
        } else {
            let balance = ledger.balances.get(&org).copied().unwrap_or(0);
            let amount = request.want_micros.min(balance).max(0);
            let grant_id = format!("grant-{}", ledger.answers.len() + 1);
            let answer = AllowanceGrant {
                grant_id: grant_id.clone(),
                granted_micros: amount,
                spend_until_unix_ms: chrono::Utc::now().timestamp_millis()
                    + *cp.knobs.grant_ttl_ms.lock(),
                reason: (amount == 0 && request.want_micros > 0)
                    .then(|| "budget_exhausted".to_string()),
                per_call_micros: *cp.knobs.per_call_micros.lock(),
            };
            if amount > 0 {
                *ledger.balances.entry(org.clone()).or_default() -= amount;
                ledger.grants.insert(
                    grant_id,
                    IssuedGrant {
                        org: org.clone(),
                        granted: amount,
                        ..Default::default()
                    },
                );
            }
            ledger
                .answers
                .insert(request.request_id.clone(), answer.clone());
            answer
        }
    };
    if FakeCp::lost(&cp.knobs.lose_allowance_replies) {
        return Err(unavailable());
    }
    Ok(Json(answer))
}

async fn feed(
    State(cp): State<FakeCp>,
    headers: axum::http::HeaderMap,
    Json(batch): Json<FeedBatch>,
) -> Reply<FeedAck> {
    cp.admit(&headers)?;
    if cp.knobs.down.load(Ordering::SeqCst) {
        return Err(unavailable());
    }
    let mut through = (0, 0);
    {
        let mut ledger = cp.ledger.lock();
        for record in batch.records {
            through = (record.index, record.ordinal);
            let id = (
                batch.source.clone(),
                batch.incarnation,
                record.index,
                record.ordinal,
            );
            if !ledger.seen.insert(id) {
                ledger.duplicates += 1;
                continue;
            }
            match record.record {
                Record::Usage {
                    attempt_id,
                    cost_micros,
                    ..
                } => match ledger.usage.entry(attempt_id) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        ledger.usage_repeats += 1;
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert((record.org_id.clone(), cost_micros));
                    }
                },
                Record::Session {
                    session_id,
                    session_version,
                    status,
                    turns,
                    ..
                } => {
                    let current = ledger.sessions.entry(session_id).or_default();
                    if session_version >= current.version {
                        *current = SessionSummary {
                            version: session_version,
                            status,
                            turns,
                        };
                    }
                }
                Record::ConfigApplied { version } => {
                    ledger.config_applied.push((record.org_id.clone(), version));
                }
                Record::OrgEvent { kind, .. } => {
                    ledger.org_events.push((record.org_id.clone(), kind));
                }
                Record::SecretRefreshed { .. } | Record::Health { .. } | Record::Unknown => {}
            }
        }
    }
    if FakeCp::lost(&cp.knobs.lose_feed_replies) {
        return Err(unavailable());
    }
    Ok(Json(FeedAck {
        source: batch.source,
        incarnation: batch.incarnation,
        stored_through: through,
    }))
}
