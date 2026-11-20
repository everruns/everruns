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
    /// Tapped session events: `(org, session, event)`.
    events: Vec<(String, String, serde_json::Value)>,
    /// Task transitions for webhooks: `(org, session, transition, task,
    /// target urls)`.
    task_transitions: Vec<TaskTransitionSeen>,
    /// Secrets the data plane refreshed: (org, secret id, revision).
    refreshed: Vec<(String, String, u64)>,
    /// Orgs whose lookups are answered: their data key and its id, and the
    /// answer per command.
    lookup_orgs: HashMap<String, (everruns_contracts::dataplane_seal::Key, String)>,
    lookup_answers: HashMap<String, everruns_contracts::host_calls::HostCallAnswer>,
    /// Lookups asked: `(org, command, params)`.
    lookups: Vec<(String, String, serde_json::Value)>,
    /// Dreaming's state, kept as the control plane keeps it
    /// (`worker_dreaming_state`): `(org, key)` → the row.
    dreams: std::collections::BTreeMap<(String, String), serde_json::Value>,
    requests: usize,
    /// Feed records that arrived again under an id already stored.
    duplicates: usize,
    /// Per stored feed record: arrival time minus the record's own time,
    /// in ms (the protocol's feed lag).
    feed_lags: Vec<i64>,
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
    /// How long exchanged org tokens live.
    token_ttl_secs: Mutex<i64>,
    /// Org tokens exchanged, and calls the stand-in `/v1` API answered.
    exchanges: AtomicUsize,
    api_calls: AtomicUsize,
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
                token_ttl_secs: Mutex::new(600),
                exchanges: AtomicUsize::new(0),
                api_calls: AtomicUsize::new(0),
            }),
            signer: Arc::new(crate::signer::Signer::new("cp-key-1")?),
        };
        let app = Router::new()
            .route("/dp/v1/tenants/{org}/allowances", post(allowance))
            .route("/dp/v1/tenants/{org}/host-calls/{name}", post(host_call))
            .route("/dp/v1/feed", post(feed))
            .route("/.well-known/jwks.json", axum::routing::get(jwks))
            .route("/v1/auth/token", post(exchange))
            .fallback(api)
            .with_state(cp.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(cp)
    }

    /// Exchanged org tokens live `secs` from now on.
    pub fn set_token_ttl_secs(&self, secs: i64) {
        *self.knobs.token_ttl_secs.lock() = secs;
    }

    /// Org tokens exchanged so far.
    pub fn exchanges(&self) -> usize {
        self.knobs.exchanges.load(Ordering::SeqCst)
    }

    /// Calls the stand-in `/v1` API answered.
    pub fn api_calls(&self) -> usize {
        self.knobs.api_calls.load(Ordering::SeqCst)
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

    /// Answer `org`'s lookups, sealed with its data key `key` (`key_id`).
    pub fn answer_lookups(
        &self,
        org: &str,
        key: everruns_contracts::dataplane_seal::Key,
        key_id: &str,
    ) {
        self.ledger
            .lock()
            .lookup_orgs
            .insert(org.to_owned(), (key, key_id.to_owned()));
    }

    /// Answer the lookup `command` with `answer` from now on (a command
    /// with none is not found, as the control plane answers an unknown
    /// session).
    pub fn set_lookup(
        &self,
        command: &str,
        answer: everruns_contracts::host_calls::HostCallAnswer,
    ) {
        self.ledger
            .lock()
            .lookup_answers
            .insert(command.to_owned(), answer);
    }

    /// Dreaming's state row `key` of `org`, as the cells left it.
    pub fn dream_state(&self, org: &str, key: &str) -> Option<serde_json::Value> {
        self.ledger
            .lock()
            .dreams
            .get(&(org.to_owned(), key.to_owned()))
            .cloned()
    }

    /// Make dreaming's row `key` of `org` due now, as a dream its turn's
    /// timer did not finish leaves it.
    pub fn make_dream_due(&self, org: &str, key: &str) {
        if let Some(row) = self
            .ledger
            .lock()
            .dreams
            .get_mut(&(org.to_owned(), key.to_owned()))
        {
            row["due_at"] = serde_json::json!(0);
            // The job inside carries its own due time, which the runner reads.
            if let Some(mut value) = row["value"]
                .as_str()
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            {
                value["job"]["due_at"] = serde_json::json!(0);
                row["value"] = serde_json::json!(value.to_string());
            }
        }
    }

    /// Lookups asked so far: `(org, command, params)`.
    pub fn lookups(&self) -> Vec<(String, String, serde_json::Value)> {
        self.ledger.lock().lookups.clone()
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

    /// The feed lag at `percentile` (0-100) over every stored record: from
    /// the record's own time to its arrival here. `None` before any record.
    pub fn feed_lag(&self, percentile: usize) -> Option<std::time::Duration> {
        let mut lags = self.ledger.lock().feed_lags.clone();
        if lags.is_empty() {
            return None;
        }
        lags.sort_unstable();
        let at = (lags.len() * percentile.min(100)).div_ceil(100).max(1) - 1;
        Some(std::time::Duration::from_millis(lags[at].max(0) as u64))
    }

    pub fn org_events(&self) -> Vec<(String, String)> {
        self.ledger.lock().org_events.clone()
    }

    /// The task transitions the cells put on the feed for webhooks, in
    /// feed order.
    pub fn task_transitions(&self) -> Vec<TaskTransitionSeen> {
        self.ledger.lock().task_transitions.clone()
    }

    /// The session events the cells tapped onto the feed, in feed order.
    pub fn tapped_events(&self) -> Vec<(String, String, serde_json::Value)> {
        self.ledger.lock().events.clone()
    }

    pub fn refreshed(&self) -> Vec<(String, String, u64)> {
        self.ledger.lock().refreshed.clone()
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

/// `POST /v1/auth/token`, with credentials of the form `Bearer user-<org>`
/// (a member of `<org>`, on this data plane) or `Bearer v1-user` (an org
/// that is not on a data plane).
async fn exchange(
    State(cp): State<FakeCp>,
    headers: axum::http::HeaderMap,
) -> Reply<serde_json::Value> {
    if cp.knobs.down.load(Ordering::SeqCst) {
        return Err(unavailable());
    }
    let credential = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if credential == "v1-user" {
        return Err((StatusCode::CONFLICT, "not on a data plane".into()));
    }
    let Some(org) = credential.strip_prefix("user-") else {
        return Err((StatusCode::UNAUTHORIZED, "sign in".into()));
    };
    let ttl = *cp.knobs.token_ttl_secs.lock();
    let token = cp
        .signer
        .token(org, "dp-local", "sessions:write", ttl)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    cp.knobs.exchanges.fetch_add(1, Ordering::SeqCst);
    Ok(Json(serde_json::json!({
        "token": token,
        "token_type": "Bearer",
        "expires_at_unix": chrono::Utc::now().timestamp() + ttl,
        "org": org,
    })))
}

/// The control plane's own `/v1` API, standing in: it names itself and
/// the call, so a test sees what the edge passed on.
async fn api(
    State(cp): State<FakeCp>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> Reply<serde_json::Value> {
    if cp.knobs.down.load(Ordering::SeqCst) {
        return Err(unavailable());
    }
    cp.knobs.api_calls.fetch_add(1, Ordering::SeqCst);
    Ok(Json(serde_json::json!({
        "served_by": "control-plane",
        "method": method.as_str(),
        "path": uri.to_string(),
        "authorization": headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    })))
}

async fn jwks(State(cp): State<FakeCp>) -> Reply<jsonwebtoken::jwk::JwkSet> {
    cp.signer
        .jwks()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// A host call, answered sealed for the org as the control plane answers
/// it. Answers and the ledger are kept by the call's v1 command.
async fn host_call(
    State(cp): State<FakeCp>,
    Path((org, name)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(request): Json<everruns_contracts::host_calls::HostCallRequest>,
) -> Reply<everruns_cell::dp::SealedSecret> {
    use everruns_contracts::dataplane_seal as seal;
    use everruns_contracts::host_calls::{HostCallAnswer, by_name, host_call_secret};
    cp.admit(&headers)?;
    if cp.knobs.down.load(Ordering::SeqCst) {
        return Err(unavailable());
    }
    let call = by_name(&name).ok_or((StatusCode::NOT_FOUND, "not a host call".to_owned()))?;
    let mut ledger = cp.ledger.lock();
    let (key, key_id) = ledger.lookup_orgs.get(&org).cloned().ok_or((
        StatusCode::NOT_FOUND,
        format!("org {org} is not on a data plane"),
    ))?;
    ledger
        .lookups
        .push((org.clone(), call.command.to_owned(), request.params.clone()));
    let answer = match ledger.lookup_answers.get(call.command).cloned() {
        Some(answer) => answer,
        None if call.command == "worker_dreaming_state" => {
            HostCallAnswer::Value(dreaming_state(&mut ledger.dreams, &org, &request.params))
        }
        None => HostCallAnswer::Error {
            kind: 3,
            message: "Session not found".into(),
        },
    };
    let plain = serde_json::to_vec(&answer)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    seal::seal(
        &key,
        &key_id,
        &org,
        &host_call_secret(&request.call),
        0,
        &plain,
        seal::random_nonce(),
    )
    .map(Json)
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `worker_dreaming_state` as the control plane answers it: a row per org
/// and key, changed by compare-exchange, and the org's due sessions.
fn dreaming_state(
    dreams: &mut std::collections::BTreeMap<(String, String), serde_json::Value>,
    org: &str,
    params: &serde_json::Value,
) -> serde_json::Value {
    let key = (
        org.to_owned(),
        params["key"].as_str().unwrap_or("").to_owned(),
    );
    match params["operation"].as_str() {
        Some("get") => dreams.get(&key).cloned().unwrap_or(serde_json::Value::Null),
        Some("cas") => {
            let revision = dreams.get(&key).map(|row| row["revision"].clone());
            let expected = params.get("expected").filter(|e| !e.is_null()).cloned();
            if revision != expected {
                return serde_json::json!(false);
            }
            let next = expected.and_then(|e| e.as_i64()).unwrap_or(0) + 1;
            dreams.insert(
                key,
                serde_json::json!({
                    "revision": next,
                    "value": params["value"],
                    "session_id": params["session_id"],
                    "due_at": params["due_at"],
                }),
            );
            serde_json::json!(true)
        }
        Some("due_org") => {
            let now = params["now"].as_i64().unwrap_or(i64::MAX);
            let due: Vec<_> = dreams
                .iter()
                .filter(|((o, _), row)| {
                    o == org
                        && !row["session_id"].is_null()
                        && row["due_at"].as_i64().is_some_and(|at| at <= now)
                })
                .map(|(_, row)| serde_json::json!([0, row["session_id"]]))
                .collect();
            serde_json::json!(due)
        }
        _ => serde_json::Value::Null,
    }
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

/// A task transition the feed carried (`Record::TaskTransition`).
#[derive(Debug, Clone)]
pub struct TaskTransitionSeen {
    pub org: String,
    pub session: String,
    pub transition: String,
    pub task: serde_json::Value,
    pub targets: Vec<everruns_contracts::dataplane::PushTarget>,
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
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
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
            if record.at_unix_ms > 0 {
                ledger.feed_lags.push(now_ms - record.at_unix_ms);
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
                Record::SecretRefreshed {
                    secret_id,
                    revision,
                    ..
                } => {
                    ledger
                        .refreshed
                        .push((record.org_id.clone(), secret_id, revision));
                }
                Record::Event { session_id, event } => {
                    ledger
                        .events
                        .push((record.org_id.clone(), session_id, event));
                }
                Record::TaskTransition {
                    session_id,
                    transition,
                    task,
                    targets,
                } => ledger.task_transitions.push(TaskTransitionSeen {
                    org: record.org_id.clone(),
                    session: session_id,
                    transition,
                    task,
                    targets,
                }),
                Record::Health { .. } | Record::Unknown => {}
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
