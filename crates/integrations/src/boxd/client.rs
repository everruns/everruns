//! A small async client for the parts of boxd's public gRPC API that
//! sandboxes need.
//!
//! Decisions:
//! - Generated from the vendored `proto/boxd/api.proto` (the file boxd
//!   publishes, unmodified) rather than a port of an SDK: the integration
//!   needs about fifteen RPCs and the proto is the API's own contract.
//! - Authentication is two steps. The long-lived API key (`bxd_...`) is
//!   exchanged over HTTPS at the console host for a JWT that lives an hour,
//!   and the JWT rides as a bearer token on every gRPC call. Exchanges are
//!   rate-limited per source IP, and every tool call builds a fresh client, so
//!   JWTs are cached process-wide per key and renewed a few minutes early or
//!   when a call answers `UNAUTHENTICATED`.
//! - Regions have their own endpoints. Account and org calls go to the home
//!   endpoint (`boxd.sh:9443`, Frankfurt); a call on a machine goes to its
//!   region's endpoint, which `ListRegions` names. The region list is cached
//!   per key for a few minutes, because it also says which regions the org
//!   may use.
//! - Errors are strings, like the other sandbox clients, but a missing
//!   machine always reads `not found: ...` so callers can match on it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::Mutex;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

use super::proto as pb;
use crate::grpc_transport as transport;

type Stub = pb::boxd_api_client::BoxdApiClient<Channel>;

/// boxd's home API endpoint (Frankfurt), where account and org calls go.
pub const BOXD_API_URL: &str = "https://boxd.sh:9443";
/// The console host's API-key exchange.
pub const BOXD_AUTH_URL: &str = "https://app.boxd.sh/api/v1/auth/token";

/// Per-stream output cap. Command output beyond it is dropped and reported as
/// truncated, so a runaway command cannot exhaust worker memory.
pub const MAX_EXEC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Bytes per `FsWrite` chunk, well under the 4 MiB message cap.
const WRITE_CHUNK_BYTES: usize = 1024 * 1024;
/// Renew a JWT this long before boxd says it expires.
const JWT_RENEW_MARGIN_SECS: i64 = 5 * 60;
/// How long the region list is trusted.
const REGIONS_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a new machine may take to reach `running`.
const READY_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const READY_POLL: Duration = Duration::from_millis(500);
/// Deadline for unary calls.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Cache keyed by (endpoint, API key).
type KeyedCache<V> = LazyLock<std::sync::Mutex<HashMap<(String, String), V>>>;

/// JWTs and their expiry per (auth URL, API key).
static JWTS: KeyedCache<(String, i64)> = LazyLock::new(Default::default);
/// Region lists per (home endpoint, API key).
static REGIONS: KeyedCache<(Instant, Vec<Region>)> = LazyLock::new(Default::default);

/// One boxd region as the org sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// Short code, such as `fra` or `nyc`.
    pub id: String,
    /// Display name, such as `Frankfurt`.
    pub name: String,
    /// The region's API endpoint as an `https://` URL.
    pub api_endpoint: String,
    /// Whether account and org calls live here.
    pub home: bool,
    /// The org's standing there: `enabled`, `requested` or `available`.
    pub org_state: String,
}

/// What a command produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// Output passed [`MAX_EXEC_OUTPUT_BYTES`] and was cut.
    pub truncated: bool,
    /// The command ran past its timeout and was killed (`exit_code` 124).
    pub timed_out: bool,
}

/// What a create asks boxd for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateMachineParams {
    /// Unique per region.
    pub name: String,
    /// boxd's `<vcpu>x<GiB>` size, or `None` for the org default.
    pub size: Option<String>,
    /// Snapshot to restore instead of the default image.
    pub snapshot: Option<String>,
    pub labels: HashMap<String, String>,
    /// Egress allowlist from birth. Empty is unrestricted.
    pub egress: Vec<String>,
    /// Server-side delete timer.
    pub delete_after_secs: Option<u64>,
}

/// A machine as boxd reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    pub id: String,
    pub name: String,
    /// `pending`, `starting`, `running`, `suspended`, `hibernated`,
    /// `stopped`, `failed`, `migrating`.
    pub status: String,
    pub region: String,
    pub url: Option<String>,
    pub labels: HashMap<String, String>,
    pub egress_allow: Vec<String>,
    pub created_at: i64,
}

impl From<pb::GetVmResponse> for Machine {
    fn from(vm: pb::GetVmResponse) -> Self {
        let url = (!vm.access_domain.is_empty() && !vm.name.is_empty())
            .then(|| format!("https://{}.{}", vm.name, vm.access_domain));
        Self {
            id: vm.vm_id,
            name: vm.name,
            status: vm.status,
            region: vm.region,
            url,
            labels: vm.labels,
            egress_allow: vm.egress_allow,
            created_at: vm.created_at,
        }
    }
}

/// Format a gRPC status. A missing resource always reads `not found: ...`.
fn status_error(context: &str, status: &Status) -> String {
    let message = status.message();
    match status.code() {
        Code::NotFound => format!("not found: {context}: {message}"),
        Code::Unauthenticated => format!(
            "{context}: boxd rejected the credentials ({message}). Check the boxd API key in settings."
        ),
        Code::PermissionDenied => format!("{context}: permission denied by boxd: {message}"),
        Code::ResourceExhausted => {
            format!("{context}: the boxd organization's machine limit is reached ({message})")
        }
        code => format!("{context}: {message} ({code:?})"),
    }
}

/// Whether an error string from this client means the machine is gone.
pub fn is_not_found(error: &str) -> bool {
    error.starts_with("not found:")
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Normalize an endpoint that `ListRegions` may print without a scheme.
fn endpoint_url(raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", raw.trim_end_matches('/'))
    }
}

/// A boxd API client for one API key.
#[derive(Clone)]
pub struct BoxdClient {
    api_key: String,
    auth_url: String,
    home_url: String,
    http: reqwest::Client,
    channels: Arc<Mutex<HashMap<String, Stub>>>,
}

impl std::fmt::Debug for BoxdClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxdClient")
            .field("home_url", &self.home_url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl BoxdClient {
    /// A client for boxd's hosted service.
    pub fn new(api_key: impl Into<String>) -> Result<Self, String> {
        Self::with_endpoints(api_key, BOXD_AUTH_URL, BOXD_API_URL)
    }

    /// A client for other endpoints: a self-hosted boxd, or a test mock.
    pub fn with_endpoints(
        api_key: impl Into<String>,
        auth_url: &str,
        home_url: &str,
    ) -> Result<Self, String> {
        let api_key = api_key.into().trim().to_string();
        if api_key.is_empty() {
            return Err("The boxd API key is empty".to_string());
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        Ok(Self {
            api_key,
            auth_url: auth_url.to_string(),
            home_url: endpoint_url(home_url),
            http,
            channels: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn cache_key(&self, scope: &str) -> (String, String) {
        (scope.to_string(), self.api_key.clone())
    }

    /// A JWT for this key, from the cache or a fresh exchange.
    async fn jwt(&self, force: bool) -> Result<String, String> {
        let key = self.cache_key(&self.auth_url);
        if !force
            && let Ok(cache) = JWTS.lock()
            && let Some((token, expires_at)) = cache.get(&key)
            && *expires_at - JWT_RENEW_MARGIN_SECS > now_secs()
        {
            return Ok(token.clone());
        }
        #[derive(serde::Deserialize)]
        struct Exchange {
            token: String,
            #[serde(default)]
            expires_at: Option<i64>,
        }
        let response = self
            .http
            .post(&self.auth_url)
            .json(&serde_json::json!({ "api_key": self.api_key }))
            .send()
            .await
            .map_err(|e| format!("Failed to reach boxd: {e}"))?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(
                "boxd rejected the API key. Check that it is correct and has not expired."
                    .to_string(),
            );
        }
        if !status.is_success() {
            return Err(format!("boxd token exchange failed (HTTP {status})"));
        }
        let exchange: Exchange = response
            .json()
            .await
            .map_err(|e| format!("Unexpected boxd token response: {e}"))?;
        // boxd documents an hour; trust its own expiry when it sends one.
        let expires_at = exchange.expires_at.unwrap_or_else(|| now_secs() + 3_600);
        if let Ok(mut cache) = JWTS.lock() {
            cache.insert(key, (exchange.token.clone(), expires_at));
        }
        Ok(exchange.token)
    }

    async fn stub(&self, url: &str) -> Result<Stub, String> {
        let mut channels = self.channels.lock().await;
        if let Some(stub) = channels.get(url) {
            return Ok(stub.clone());
        }
        let stub = Stub::new(transport::channel(url)?)
            .max_decoding_message_size(transport::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(transport::MAX_MESSAGE_SIZE);
        channels.insert(url.to_string(), stub.clone());
        Ok(stub)
    }

    async fn request<T>(&self, message: T, force_jwt: bool) -> Result<Request<T>, String> {
        let jwt = self.jwt(force_jwt).await?;
        let mut request = Request::new(message);
        request.set_timeout(CALL_TIMEOUT);
        let value = MetadataValue::try_from(format!("Bearer {jwt}"))
            .map_err(|e| format!("Invalid boxd token: {e}"))?;
        request.metadata_mut().insert("authorization", value);
        Ok(request)
    }

    /// Run one unary call against `url`, renewing the JWT once on
    /// `UNAUTHENTICATED` (a key revoked and recreated, or a clock skew).
    async fn unary<M, R, F, Fut>(
        &self,
        url: &str,
        context: &str,
        message: M,
        call: F,
    ) -> Result<R, String>
    where
        M: Clone,
        F: Fn(Stub, Request<M>) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Response<R>, Status>>,
    {
        let stub = self.stub(url).await?;
        let first = call(stub.clone(), self.request(message.clone(), false).await?).await;
        match first {
            Ok(response) => Ok(response.into_inner()),
            Err(status) if status.code() == Code::Unauthenticated => {
                call(stub, self.request(message, true).await?)
                    .await
                    .map(tonic::Response::into_inner)
                    .map_err(|status| status_error(context, &status))
            }
            Err(status) => Err(status_error(context, &status)),
        }
    }

    // ------------------------------------------------------------------
    // Account and regions
    // ------------------------------------------------------------------

    /// Confirm the key works: exchange it and call `Whoami`.
    pub async fn verify(&self) -> Result<(), String> {
        self.unary(
            &self.home_url,
            "whoami",
            pb::WhoamiRequest {},
            |mut stub, req| async move { stub.whoami(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Every region and where the org stands with it.
    pub async fn regions(&self) -> Result<Vec<Region>, String> {
        let key = self.cache_key(&self.home_url);
        if let Ok(cache) = REGIONS.lock()
            && let Some((at, regions)) = cache.get(&key)
            && at.elapsed() < REGIONS_TTL
        {
            return Ok(regions.clone());
        }
        let response = self
            .unary(
                &self.home_url,
                "list regions",
                pb::ListRegionsRequest::default(),
                |mut stub, req| async move { stub.list_regions(req).await },
            )
            .await?;
        let regions: Vec<Region> = response
            .regions
            .into_iter()
            .map(|region| Region {
                api_endpoint: endpoint_url(&region.api_endpoint),
                id: region.id,
                name: region.name,
                home: region.home,
                org_state: region.org_state,
            })
            .collect();
        if let Ok(mut cache) = REGIONS.lock() {
            cache.insert(key, (Instant::now(), regions.clone()));
        }
        Ok(regions)
    }

    /// The org's default region, when it set one.
    pub async fn default_region(&self) -> Result<Option<String>, String> {
        let response = self
            .unary(
                &self.home_url,
                "read org machine defaults",
                pb::GetOrgMachineDefaultsRequest::default(),
                |mut stub, req| async move { stub.get_org_machine_defaults(req).await },
            )
            .await?;
        Ok(Some(response.region).filter(|region| !region.is_empty()))
    }

    /// The region a machine call should go to: `requested`, else the org
    /// default, else the home region. Errors when the org may not use it.
    pub async fn resolve_region(&self, requested: Option<&str>) -> Result<Region, String> {
        let regions = self.regions().await?;
        let wanted = match requested {
            Some(id) => Some(id.to_string()),
            None => self.default_region().await?,
        };
        let region = match wanted {
            Some(id) => regions
                .iter()
                .find(|region| region.id == id)
                .cloned()
                .ok_or_else(|| {
                    let known: Vec<&str> = regions.iter().map(|r| r.id.as_str()).collect();
                    format!("boxd has no region '{id}' (regions: {})", known.join(", "))
                })?,
            None => regions
                .iter()
                .find(|region| region.home)
                .cloned()
                .ok_or("boxd listed no home region")?,
        };
        // An empty state comes from a server that predates per-org regions.
        if !region.org_state.is_empty() && region.org_state != "enabled" {
            return Err(format!(
                "The boxd organization cannot create machines in {} ({}): the region is '{}'. An org admin can request it with `boxd manage regions`.",
                region.name, region.id, region.org_state
            ));
        }
        Ok(region)
    }

    /// The endpoint for a machine in `region` (empty: the home endpoint).
    async fn region_url(&self, region: &str) -> Result<String, String> {
        if region.is_empty() {
            return Ok(self.home_url.clone());
        }
        self.regions()
            .await?
            .into_iter()
            .find(|r| r.id == region)
            .map(|r| r.api_endpoint)
            .ok_or_else(|| format!("boxd has no region '{region}'"))
    }

    // ------------------------------------------------------------------
    // Machines
    // ------------------------------------------------------------------

    /// Create an isolated, unshared machine in `region` and wait until it
    /// runs. boxd's own idle timers are turned off: they key on network
    /// traffic, so a long CPU-only build would be hibernated mid-turn.
    pub async fn create_machine(
        &self,
        region: &str,
        params: &CreateMachineParams,
    ) -> Result<Machine, String> {
        let url = self.region_url(region).await?;
        let config = pb::VmConfig {
            srf: Some(pb::SrfConfig {
                auto_suspend_timeout_secs: Some(0),
                auto_destroy_timeout_secs: 0,
            }),
            ..Default::default()
        };
        let machine = match &params.snapshot {
            Some(snapshot) => {
                let request = pb::CreateVmFromSnapshotRequest {
                    snapshot: snapshot.clone(),
                    name: params.name.clone(),
                    config: Some(config),
                    isolated: true,
                    labels: params.labels.clone(),
                    spec: (!params.egress.is_empty()).then(|| pb::MachineSpec {
                        version: 1,
                        egress: params.egress.clone(),
                        ..Default::default()
                    }),
                    delete_after_secs: params.delete_after_secs,
                    ..Default::default()
                };
                self.unary(
                    &url,
                    "create boxd machine from snapshot",
                    request,
                    |mut stub, req| async move { stub.create_vm_from_snapshot(req).await },
                )
                .await?
            }
            None => {
                let request = pb::CreateVmRequest {
                    name: params.name.clone(),
                    config: Some(config),
                    spec: Some(pb::MachineSpec {
                        version: 1,
                        size: params.size.clone().unwrap_or_default(),
                        labels: params.labels.clone(),
                        isolated: true,
                        egress: params.egress.clone(),
                        ..Default::default()
                    }),
                    delete_after_secs: params.delete_after_secs,
                    ..Default::default()
                };
                self.unary(
                    &url,
                    "create boxd machine",
                    request,
                    |mut stub, req| async move { stub.create_vm(req).await },
                )
                .await?
            }
        };
        let id = if machine.vm_id.is_empty() {
            machine.name.clone()
        } else {
            machine.vm_id.clone()
        };
        // Hibernation is Everruns' call (the template's idle policy), not boxd's.
        if let Err(err) = self.set_auto_hibernate(region, &id, 0).await {
            tracing::warn!(machine = %id, "Failed to turn off boxd auto-hibernate: {err}");
        }
        self.wait_until_running(region, &id).await
    }

    /// Poll until the machine runs. A failed machine is an error.
    pub async fn wait_until_running(&self, region: &str, id: &str) -> Result<Machine, String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let machine = self.get_machine(region, id).await?;
            match machine.status.as_str() {
                "running" => return Ok(machine),
                "failed" => return Err(format!("boxd machine {id} failed to start")),
                _ if Instant::now() >= deadline => {
                    return Err(format!(
                        "boxd machine {id} did not start within {}s (status {})",
                        READY_TIMEOUT.as_secs(),
                        machine.status
                    ));
                }
                _ => tokio::time::sleep(READY_POLL).await,
            }
        }
    }

    pub async fn get_machine(&self, region: &str, id: &str) -> Result<Machine, String> {
        let url = self.region_url(region).await?;
        let request = pb::GetVmRequest {
            vm_id: id.to_string(),
        };
        self.unary(
            &url,
            &format!("boxd machine {id}"),
            request,
            |mut stub, req| async move { stub.get_vm(req).await },
        )
        .await
        .map(Machine::from)
    }

    /// Machines in `region` carrying every `key=value` in `labels`.
    pub async fn list_machines(
        &self,
        region: &str,
        labels: &[(String, String)],
    ) -> Result<Vec<Machine>, String> {
        let url = self.region_url(region).await?;
        let request = pb::ListVmsRequest {
            label_selector: labels.iter().map(|(k, v)| format!("{k}={v}")).collect(),
            ..Default::default()
        };
        let response = self
            .unary(
                &url,
                "list boxd machines",
                request,
                |mut stub, req| async move { stub.list_vms(req).await },
            )
            .await?;
        Ok(response.vms.into_iter().map(Machine::from).collect())
    }

    pub async fn destroy(&self, region: &str, id: &str) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::DestroyVmRequest {
            vm_id: id.to_string(),
        };
        self.unary(
            &url,
            &format!("delete boxd machine {id}"),
            request,
            |mut stub, req| async move { stub.destroy_vm(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Write memory to disk; billing drops to disk only.
    pub async fn hibernate(&self, region: &str, id: &str) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::HibernateVmRequest {
            vm_id: id.to_string(),
        };
        self.unary(
            &url,
            &format!("hibernate boxd machine {id}"),
            request,
            |mut stub, req| async move { stub.hibernate_vm(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Bring a hibernated or suspended machine back, processes intact.
    pub async fn wake(&self, region: &str, id: &str) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::WakeVmRequest {
            vm_id: id.to_string(),
        };
        self.unary(
            &url,
            &format!("wake boxd machine {id}"),
            request,
            |mut stub, req| async move { stub.wake_vm(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Boot a stopped machine.
    pub async fn start(&self, region: &str, id: &str) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::StartVmRequest {
            vm_id: id.to_string(),
        };
        self.unary(
            &url,
            &format!("start boxd machine {id}"),
            request,
            |mut stub, req| async move { stub.start_vm(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Replace the egress allowlist. Empty is unrestricted, `["none"]` blocks
    /// everything. In force within about a second, without a restart.
    pub async fn set_egress(
        &self,
        region: &str,
        id: &str,
        entries: &[String],
    ) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::SetVmEgressAllowRequest {
            vm: id.to_string(),
            entries: entries.to_vec(),
        };
        self.unary(
            &url,
            "set boxd egress allowlist",
            request,
            |mut stub, req| async move { stub.set_vm_egress_allow(req).await },
        )
        .await
        .map(|_| ())
    }

    pub async fn set_auto_hibernate(
        &self,
        region: &str,
        id: &str,
        secs: u32,
    ) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::SetAutoHibernateTimeoutRequest {
            vm_id: id.to_string(),
            timeout_secs: secs,
        };
        self.unary(
            &url,
            "set boxd auto-hibernate",
            request,
            |mut stub, req| async move { stub.set_auto_hibernate_timeout(req).await },
        )
        .await
        .map(|_| ())
    }

    /// Save the running machine as a named snapshot. Returns its status
    /// (`pending` until the artifact lands).
    pub async fn create_snapshot(
        &self,
        region: &str,
        id: &str,
        name: &str,
    ) -> Result<String, String> {
        let url = self.region_url(region).await?;
        let request = pb::CreateSnapshotRequest {
            vm: id.to_string(),
            name: name.to_string(),
        };
        self.unary(
            &url,
            "create boxd snapshot",
            request,
            |mut stub, req| async move { stub.create_snapshot(req).await },
        )
        .await
        .map(|response| response.status)
    }

    /// A snapshot's status: `pending`, `ready` or `failed`.
    pub async fn snapshot_status(&self, region: &str, name: &str) -> Result<String, String> {
        let url = self.region_url(region).await?;
        let request = pb::GetSnapshotRequest {
            name: name.to_string(),
            ..Default::default()
        };
        self.unary(
            &url,
            &format!("boxd snapshot {name}"),
            request,
            |mut stub, req| async move { stub.get_snapshot(req).await },
        )
        .await
        .map(|response| response.snapshot.map(|s| s.status).unwrap_or_default())
    }

    // ------------------------------------------------------------------
    // Inside a machine
    // ------------------------------------------------------------------

    /// Run a shell command line, feeding `stdin` when given, and collect the
    /// output. Past `timeout` the call is cancelled, which boxd turns into a
    /// SIGTERM then SIGKILL of the command's process group; the output so far
    /// comes back with exit code 124.
    pub async fn exec(
        &self,
        region: &str,
        id: &str,
        command: &str,
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<ExecOutput, String> {
        let url = self.region_url(region).await?;
        let mut chunks = vec![pb::ExecChunk {
            vm_id: id.to_string(),
            command: command.to_string(),
            ..Default::default()
        }];
        if let Some(data) = stdin {
            for piece in data.chunks(WRITE_CHUNK_BYTES) {
                chunks.push(pb::ExecChunk {
                    stdin: true,
                    data: piece.to_vec(),
                    ..Default::default()
                });
            }
        }
        let mut stub = self.stub(&url).await?;
        // A finite request stream half-closes after the last chunk, which is
        // stdin EOF for the command.
        let open = |force: bool| {
            let chunks = chunks.clone();
            async move {
                let jwt = self.jwt(force).await?;
                let mut request = Request::new(futures::stream::iter(chunks));
                let value = MetadataValue::try_from(format!("Bearer {jwt}"))
                    .map_err(|e| format!("Invalid boxd token: {e}"))?;
                request.metadata_mut().insert("authorization", value);
                Ok::<_, String>(request)
            }
        };
        let response = match stub.exec(open(false).await?).await {
            Ok(response) => response,
            Err(status) if status.code() == Code::Unauthenticated => stub
                .exec(open(true).await?)
                .await
                .map_err(|status| status_error("boxd exec", &status))?,
            Err(status) => return Err(status_error("boxd exec", &status)),
        };
        let mut stream = response.into_inner();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut output = ExecOutput::default();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let next = match tokio::time::timeout_at(deadline, stream.next()).await {
                Ok(next) => next,
                Err(_) => {
                    output.timed_out = true;
                    output.exit_code = 124;
                    break;
                }
            };
            match next {
                None => break,
                Some(Err(status)) => return Err(status_error("boxd exec", &status)),
                Some(Ok(chunk)) => {
                    if !chunk.data.is_empty() {
                        let sink = if chunk.is_stderr {
                            &mut stderr
                        } else {
                            &mut stdout
                        };
                        let room = MAX_EXEC_OUTPUT_BYTES.saturating_sub(sink.len());
                        if chunk.data.len() > room {
                            output.truncated = true;
                        }
                        sink.extend_from_slice(&chunk.data[..chunk.data.len().min(room)]);
                    }
                    // The final chunk carries the exit code.
                    output.exit_code = chunk.exit_code;
                }
            }
        }
        // Dropping the stream cancels the call when it timed out.
        drop(stream);
        output.stdout = String::from_utf8_lossy(&stdout).into_owned();
        output.stderr = String::from_utf8_lossy(&stderr).into_owned();
        if output.timed_out {
            if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
                output.stderr.push('\n');
            }
            output.stderr.push_str(&format!(
                "Command timed out after {}s and was killed.\n",
                timeout.as_secs()
            ));
        }
        Ok(output)
    }

    /// Read a file, at most `max_bytes` of it.
    pub async fn read_file(
        &self,
        region: &str,
        id: &str,
        path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        let url = self.region_url(region).await?;
        let request = pb::FsReadRequest {
            vm_id: id.to_string(),
            path: path.as_bytes().to_vec(),
            offset: 0,
            length: None,
        };
        let context = format!("read {path}");
        let mut stream = self
            .unary(&url, &context, request, |mut stub, req| async move {
                stub.fs_read(req).await
            })
            .await?;
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|status| status_error(&context, &status))?;
            if let Some(entry) = &chunk.entry
                && entry.size as usize > max_bytes
            {
                return Err(format!(
                    "{path} is {} bytes; the limit is {max_bytes}",
                    entry.size
                ));
            }
            data.extend_from_slice(&chunk.data);
            if data.len() > max_bytes {
                return Err(format!("{path} is larger than the {max_bytes} byte limit"));
            }
            if chunk.eof {
                break;
            }
        }
        Ok(data)
    }

    /// Replace a file's contents, creating missing parent directories.
    pub async fn write_file(
        &self,
        region: &str,
        id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<u64, String> {
        let url = self.region_url(region).await?;
        let mut chunks = vec![pb::FsWriteChunk {
            vm_id: id.to_string(),
            path: path.as_bytes().to_vec(),
            expected_size: Some(content.len() as u64),
            ..Default::default()
        }];
        for (index, piece) in content.chunks(WRITE_CHUNK_BYTES).enumerate() {
            if index == 0 {
                chunks[0].data = piece.to_vec();
            } else {
                chunks.push(pb::FsWriteChunk {
                    data: piece.to_vec(),
                    ..Default::default()
                });
            }
        }
        let context = format!("write {path}");
        self.unary(&url, &context, chunks, |mut stub, req| async move {
            let (metadata, extensions, chunks) = req.into_parts();
            stub.fs_write(Request::from_parts(
                metadata,
                extensions,
                futures::stream::iter(chunks),
            ))
            .await
        })
        .await
        .map(|response| response.bytes_written)
    }

    /// Create a directory and any missing parents.
    pub async fn mkdir(&self, region: &str, id: &str, path: &str) -> Result<(), String> {
        let url = self.region_url(region).await?;
        let request = pb::FsMkdirRequest {
            vm_id: id.to_string(),
            path: path.as_bytes().to_vec(),
            mode: None,
        };
        self.unary(
            &url,
            &format!("create {path}"),
            request,
            |mut stub, req| async move { stub.fs_mkdir(req).await },
        )
        .await
        .map(|_| ())
    }
}

/// Quote `value` for a POSIX shell.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_get_a_scheme() {
        assert_eq!(endpoint_url("nyc.boxd.sh:9443"), "https://nyc.boxd.sh:9443");
        assert_eq!(
            endpoint_url("https://boxd.sh:9443/"),
            "https://boxd.sh:9443"
        );
        assert_eq!(
            endpoint_url("http://127.0.0.1:5000"),
            "http://127.0.0.1:5000"
        );
    }

    #[test]
    fn not_found_is_recognizable() {
        let error = status_error(
            "boxd machine x",
            &Status::not_found("vm not found (region nyc)"),
        );
        assert!(is_not_found(&error), "{error}");
        assert!(!is_not_found(&status_error("x", &Status::internal("boom"))));
    }

    #[test]
    fn shell_quoting_survives_quotes() {
        assert_eq!(shell_quote("/home/boxd/it's"), "'/home/boxd/it'\\''s'");
    }

    #[test]
    fn debug_redacts_the_key() {
        let client = BoxdClient::new("bxd_secret").unwrap();
        assert!(!format!("{client:?}").contains("bxd_secret"));
        assert!(BoxdClient::new("  ").is_err());
    }
}
