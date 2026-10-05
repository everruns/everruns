//! A small async client for the parts of Modal's API that sandboxes need.
//!
//! Decision: hand-written over a trimmed copy of Modal's protos rather than a
//! port of an SDK. The integration needs about a dozen RPCs; a full port would
//! carry hundreds and their churn.
//!
//! Two endpoints are involved:
//! - the control plane (`api.modal.com`, service `modal.client.ModalClient`)
//!   creates apps, images and sandboxes, and authenticates with the token
//!   pair plus a short-lived `x-modal-auth-token` minted by `AuthTokenGet`;
//! - the per-task command router (URL and bearer JWT from
//!   `TaskGetCommandRouterAccess`) runs commands inside a sandbox. Every
//!   current Modal SDK execs through it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use futures::StreamExt;
use tokio::sync::Mutex;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Code, Request, Status};

use super::proto::client as pb;
use super::proto::router as rpb;
use super::transport;

/// Default Modal control-plane endpoint.
pub const MODAL_API_URL: &str = "https://api.modal.com";

/// Image builder version the requests pin, the same default the Modal SDKs use.
const IMAGE_BUILDER_VERSION: &str = "2024.10";

/// `x-modal-client-type` the requests report: the value Modal assigns to the
/// Go SDK (libmodal), whose wire behaviour this client follows.
const CLIENT_TYPE: &str = "9";
/// `x-modal-client-version`: the Python SDK version whose behaviour the
/// libmodal SDKs (and this client) emulate.
const CLIENT_VERSION: &str = "1.0.0";

/// Per-stream output cap. Command output beyond it is dropped and reported as
/// truncated, so a runaway command cannot exhaust worker memory.
pub const MAX_EXEC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Bytes sent per stdin write.
const STDIN_CHUNK_BYTES: usize = 1024 * 1024;

/// How long one `SandboxGetTaskId` long-poll may wait server side.
const TASK_ID_POLL_SECS: f32 = 50.0;
/// How long sandbox scheduling may take before creation gives up.
const SCHEDULING_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long one `TaskExecWait` call may block before it is re-issued.
const EXEC_WAIT_CALL: Duration = Duration::from_secs(55);
/// Slack added to a command's own timeout for the client-side deadline.
const EXEC_DEADLINE_SLACK: Duration = Duration::from_secs(30);
/// Retries for transient errors on unary router calls.
const ROUTER_RETRIES: u32 = 8;

/// A Modal token pair (`modal token new` prints both halves).
#[derive(Clone)]
pub struct ModalCredentials {
    /// Token ID, `ak-...`.
    pub token_id: String,
    /// Token secret, `as-...`.
    pub token_secret: String,
}

impl std::fmt::Debug for ModalCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModalCredentials")
            .field("token_id", &self.token_id)
            .field("token_secret", &"<redacted>")
            .finish()
    }
}

impl ModalCredentials {
    /// Parse the single stored connection credential.
    ///
    /// Accepts `token_id:token_secret`, the two halves separated by
    /// whitespace, or the `--token-id ... --token-secret ...` flags that
    /// `modal token new` and the dashboard print.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let cleaned = raw
            .replace("--token-id", " ")
            .replace("--token-secret", " ")
            .replace(['=', ','], " ");
        let parts: Vec<&str> = cleaned
            .split(|c: char| c == ':' || c.is_whitespace())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        let token_id = parts.iter().find(|p| p.starts_with("ak-"));
        let token_secret = parts.iter().find(|p| p.starts_with("as-"));
        match (token_id, token_secret, parts.as_slice()) {
            (Some(id), Some(secret), _) => Ok(Self {
                token_id: (*id).to_string(),
                token_secret: (*secret).to_string(),
            }),
            // Tokens issued by self-hosted or future Modal deployments may not
            // carry the usual prefixes; accept an unlabelled pair in order.
            (None, None, [id, secret]) => Ok(Self {
                token_id: (*id).to_string(),
                token_secret: (*secret).to_string(),
            }),
            _ => Err(
                "Expected a Modal token pair as `<token-id>:<token-secret>` (ak-...:as-...)."
                    .to_string(),
            ),
        }
    }

    /// The single-string form stored in a connection: `token_id:token_secret`.
    pub fn to_connection_string(&self) -> String {
        format!("{}:{}", self.token_id, self.token_secret)
    }
}

/// Parameters for [`ModalClient::create_sandbox`].
#[derive(Debug, Clone, Default)]
pub struct CreateSandboxParams {
    /// Image to boot (from [`ModalClient::build_image`] or a filesystem snapshot).
    pub image_id: String,
    /// `"vm"` for a full VM, `"gvisor"` (or `None`) for the gVisor runtime.
    pub runtime: Option<String>,
    /// Maximum lifetime in seconds.
    pub timeout_secs: u32,
    /// Terminate after this many idle seconds.
    pub idle_timeout_secs: Option<u32>,
    /// Default working directory.
    pub workdir: Option<String>,
    /// Requested CPU in milli-cores.
    pub milli_cpu: Option<u32>,
    /// Requested memory in MiB.
    pub memory_mb: Option<u32>,
    /// Container ports to expose through encrypted tunnels.
    pub encrypted_ports: Vec<u32>,
    /// Tags recorded on the sandbox (visible in the Modal dashboard).
    pub tags: Vec<(String, String)>,
}

/// Result of [`ModalClient::exec`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    /// Captured stdout (lossy UTF-8).
    pub stdout: String,
    /// Captured stderr (lossy UTF-8).
    pub stderr: String,
    /// Exit code; `128 + signal` when the process was killed by a signal.
    pub exit_code: i32,
    /// Whether either stream exceeded [`MAX_EXEC_OUTPUT_BYTES`].
    pub truncated: bool,
}

/// A public tunnel to a sandbox port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelInfo {
    /// Port inside the sandbox.
    pub container_port: u32,
    /// `https://` URL that reaches it.
    pub url: String,
}

/// Coarse sandbox lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxStatus {
    /// Still running.
    Running,
    /// Finished; carries Modal's terminal status name and any exception text.
    Finished {
        /// e.g. `success`, `terminated`, `timeout`, `idle_timeout`.
        status: String,
        /// Failure detail, when Modal reports one.
        exception: Option<String>,
    },
}

#[derive(Clone)]
struct AuthToken {
    token: String,
    /// Unix seconds; refreshed a few minutes before.
    expires_at: i64,
}

/// Client for Modal's control plane and command router.
#[derive(Clone)]
pub struct ModalClient {
    credentials: ModalCredentials,
    stub: pb::modal_client_client::ModalClientClient<tonic::transport::Channel>,
    auth: Arc<Mutex<Option<AuthToken>>>,
    /// Command-router stubs per task, so repeated calls reuse one HTTP/2 connection.
    routers: Arc<Mutex<HashMap<String, Router>>>,
}

#[derive(Clone)]
struct Router {
    url: String,
    jwt: String,
    stub: rpb::task_command_router_client::TaskCommandRouterClient<tonic::transport::Channel>,
}

impl std::fmt::Debug for ModalClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModalClient")
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

fn status_error(context: &str, status: &Status) -> String {
    match status.code() {
        Code::Unauthenticated | Code::PermissionDenied => format!(
            "{context}: Modal rejected the credentials ({}). Check the token pair in your Modal connection.",
            status.message()
        ),
        Code::NotFound => format!("{context}: not found ({})", status.message()),
        Code::ResourceExhausted => format!(
            "{context}: Modal capacity or quota exhausted ({})",
            status.message()
        ),
        _ => format!("{context}: {} ({:?})", status.message(), status.code()),
    }
}

fn is_transient(code: Code) -> bool {
    matches!(
        code,
        Code::DeadlineExceeded
            | Code::Unavailable
            | Code::Internal
            | Code::Unknown
            | Code::Cancelled
    )
}

/// Read the `exp` claim of a JWT without verifying it (the server does that).
fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp")?.as_i64()
}

fn generic_status_name(status: i32) -> String {
    pb::generic_result::GenericStatus::try_from(status)
        .map(|s| {
            s.as_str_name()
                .trim_start_matches("GENERIC_STATUS_")
                .to_ascii_lowercase()
        })
        .unwrap_or_else(|_| format!("status_{status}"))
}

impl ModalClient {
    /// Client for the production Modal API.
    pub fn new(credentials: ModalCredentials) -> Result<Self, String> {
        Self::with_server_url(credentials, MODAL_API_URL)
    }

    /// Client for a different control-plane URL (tests, self-hosted proxies).
    pub fn with_server_url(credentials: ModalCredentials, url: &str) -> Result<Self, String> {
        let channel = transport::channel(url)?;
        let stub = pb::modal_client_client::ModalClientClient::new(channel)
            .max_decoding_message_size(transport::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(transport::MAX_MESSAGE_SIZE);
        Ok(Self {
            credentials,
            stub,
            auth: Arc::new(Mutex::new(None)),
            routers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn base_metadata(&self, metadata: &mut MetadataMap) -> Result<(), String> {
        let value = |v: &str| {
            MetadataValue::try_from(v)
                .map_err(|_| "Modal token contains invalid characters".to_string())
        };
        metadata.insert("x-modal-client-type", value(CLIENT_TYPE)?);
        metadata.insert("x-modal-client-version", value(CLIENT_VERSION)?);
        metadata.insert(
            "x-modal-libmodal-version",
            value(concat!("everruns/", env!("CARGO_PKG_VERSION")))?,
        );
        metadata.insert("x-modal-token-id", value(&self.credentials.token_id)?);
        metadata.insert(
            "x-modal-token-secret",
            value(&self.credentials.token_secret)?,
        );
        Ok(())
    }

    async fn auth_token(&self) -> Result<String, String> {
        let mut guard = self.auth.lock().await;
        let now = chrono::Utc::now().timestamp();
        if let Some(token) = guard.as_ref()
            && token.expires_at - now > 5 * 60
        {
            return Ok(token.token.clone());
        }
        let mut request = Request::new(pb::AuthTokenGetRequest {});
        self.base_metadata(request.metadata_mut())?;
        let response = self
            .stub
            .clone()
            .auth_token_get(request)
            .await
            .map_err(|s| status_error("Modal authentication failed", &s))?
            .into_inner();
        if response.token.is_empty() {
            return Err("Modal authentication failed: empty auth token".to_string());
        }
        let expires_at = jwt_expiry(&response.token).unwrap_or(now + 20 * 60);
        *guard = Some(AuthToken {
            token: response.token.clone(),
            expires_at,
        });
        Ok(response.token)
    }

    async fn request<T>(&self, message: T) -> Result<Request<T>, String> {
        let token = self.auth_token().await?;
        let mut request = Request::new(message);
        self.base_metadata(request.metadata_mut())?;
        request.metadata_mut().insert(
            "x-modal-auth-token",
            MetadataValue::try_from(token.as_str())
                .map_err(|_| "Modal auth token contains invalid characters".to_string())?,
        );
        request.metadata_mut().insert(
            "x-idempotency-key",
            MetadataValue::try_from(uuid::Uuid::new_v4().to_string().as_str())
                .map_err(|e| e.to_string())?,
        );
        Ok(request)
    }

    /// Validate the credentials by minting an auth token.
    pub async fn verify_credentials(&self) -> Result<(), String> {
        self.auth_token().await.map(|_| ())
    }

    /// Look up `name` in the default environment, creating it when missing.
    pub async fn get_or_create_app(&self, name: &str) -> Result<String, String> {
        let request = self
            .request(pb::AppGetOrCreateRequest {
                app_name: name.to_string(),
                environment_name: String::new(),
                object_creation_type: pb::ObjectCreationType::CreateIfMissing as i32,
            })
            .await?;
        let response = self
            .stub
            .clone()
            .app_get_or_create(request)
            .await
            .map_err(|s| status_error("Failed to look up Modal app", &s))?
            .into_inner();
        Ok(response.app_id)
    }

    /// Build (or reuse from Modal's cache) an image from a registry tag plus
    /// extra Dockerfile commands, returning its image ID.
    pub async fn build_image(
        &self,
        app_id: &str,
        registry_tag: &str,
        dockerfile_commands: &[String],
    ) -> Result<String, String> {
        if registry_tag.trim().is_empty() || registry_tag.contains(char::is_whitespace) {
            return Err(format!("Invalid image tag '{registry_tag}'"));
        }
        let mut commands = vec![format!("FROM {registry_tag}")];
        for command in dockerfile_commands {
            if command.contains('\n') {
                return Err("Dockerfile commands must be single lines".to_string());
            }
            commands.push(command.clone());
        }
        let request = self
            .request(pb::ImageGetOrCreateRequest {
                image: Some(pb::Image {
                    base_images: vec![],
                    dockerfile_commands: commands,
                }),
                app_id: app_id.to_string(),
                force_build: false,
                builder_version: IMAGE_BUILDER_VERSION.to_string(),
            })
            .await?;
        let response = self
            .stub
            .clone()
            .image_get_or_create(request)
            .await
            .map_err(|s| status_error("Failed to create Modal image", &s))?
            .into_inner();
        let image_id = response.image_id;
        let mut result = response
            .result
            .filter(|r| r.status != pb::generic_result::GenericStatus::Unspecified as i32);

        let mut last_entry_id = String::new();
        let deadline = Instant::now() + Duration::from_secs(30 * 60);
        while result.is_none() {
            if Instant::now() > deadline {
                return Err(format!(
                    "Modal image {image_id} did not finish building in 30 minutes"
                ));
            }
            let request = self
                .request(pb::ImageJoinStreamingRequest {
                    image_id: image_id.clone(),
                    timeout: 55.0,
                    last_entry_id: last_entry_id.clone(),
                    include_logs_for_finished: false,
                })
                .await?;
            let mut stream = match self.stub.clone().image_join_streaming(request).await {
                Ok(stream) => stream.into_inner(),
                Err(status) if is_transient(status.code()) => continue,
                Err(status) => {
                    return Err(status_error(
                        "Failed to wait for Modal image build",
                        &status,
                    ));
                }
            };
            while let Some(item) = stream.next().await {
                let item = match item {
                    Ok(item) => item,
                    Err(status) if is_transient(status.code()) => break,
                    Err(status) => {
                        return Err(status_error(
                            "Failed to wait for Modal image build",
                            &status,
                        ));
                    }
                };
                if !item.entry_id.is_empty() {
                    last_entry_id = item.entry_id;
                }
                if let Some(r) = item.result
                    && r.status != pb::generic_result::GenericStatus::Unspecified as i32
                {
                    result = Some(r);
                    break;
                }
                if item.eof {
                    break;
                }
            }
        }

        let result = result.unwrap_or_default();
        if result.status == pb::generic_result::GenericStatus::Success as i32 {
            Ok(image_id)
        } else {
            Err(format!(
                "Modal image build for {registry_tag} ended with status {}: {}",
                generic_status_name(result.status),
                result.exception
            ))
        }
    }

    /// Create a sandbox and wait until it is scheduled. Returns `(sandbox_id, task_id)`.
    pub async fn create_sandbox(
        &self,
        app_id: &str,
        params: &CreateSandboxParams,
    ) -> Result<(String, String), String> {
        let resources =
            (params.milli_cpu.is_some() || params.memory_mb.is_some()).then(|| pb::Resources {
                memory_mb: params.memory_mb.unwrap_or(0),
                milli_cpu: params.milli_cpu.unwrap_or(0),
            });
        let open_ports = (!params.encrypted_ports.is_empty()).then(|| {
            pb::sandbox::OpenPortsOneof::OpenPorts(pb::PortSpecs {
                ports: params
                    .encrypted_ports
                    .iter()
                    .map(|port| pb::PortSpec {
                        port: *port,
                        unencrypted: false,
                    })
                    .collect(),
            })
        });
        let definition = pb::Sandbox {
            // A long-lived entrypoint keeps the sandbox up between execs;
            // its lifetime is bounded by timeout_secs/idle_timeout_secs.
            entrypoint_args: vec!["sleep".to_string(), "infinity".to_string()],
            image_id: params.image_id.clone(),
            resources,
            timeout_secs: params.timeout_secs,
            workdir: params.workdir.clone(),
            open_ports_oneof: open_ports,
            runtime: params.runtime.clone(),
            name: None,
            idle_timeout_secs: params.idle_timeout_secs,
        };
        let request = self
            .request(pb::SandboxCreateRequest {
                app_id: app_id.to_string(),
                definition: Some(definition),
                tags: params
                    .tags
                    .iter()
                    .map(|(k, v)| pb::SandboxTag {
                        tag_name: k.clone(),
                        tag_value: v.clone(),
                    })
                    .collect(),
            })
            .await?;
        let sandbox_id = self
            .stub
            .clone()
            .sandbox_create(request)
            .await
            .map_err(|s| status_error("Failed to create Modal sandbox", &s))?
            .into_inner()
            .sandbox_id;

        match self.task_id(&sandbox_id, SCHEDULING_TIMEOUT).await {
            Ok(task_id) => Ok((sandbox_id, task_id)),
            Err(err) => {
                // Nobody will ever learn this ID, so do not leave it running.
                let _ = self.terminate(&sandbox_id).await;
                Err(err)
            }
        }
    }

    /// The task (container) ID running `sandbox_id`, waiting up to `wait` for scheduling.
    pub async fn task_id(&self, sandbox_id: &str, wait: Duration) -> Result<String, String> {
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "Modal sandbox {sandbox_id} was not scheduled in time (insufficient capacity?)"
                ));
            }
            let request = self
                .request(pb::SandboxGetTaskIdRequest {
                    sandbox_id: sandbox_id.to_string(),
                    timeout: Some(remaining.as_secs_f32().min(TASK_ID_POLL_SECS)),
                    wait_until_ready: false,
                })
                .await?;
            let response = match self.stub.clone().sandbox_get_task_id(request).await {
                Ok(response) => response.into_inner(),
                Err(status) if is_transient(status.code()) => continue,
                Err(status) => return Err(status_error("Failed to locate Modal sandbox", &status)),
            };
            if let Some(task_id) = response.task_id.filter(|t| !t.is_empty()) {
                return Ok(task_id);
            }
            if let Some(result) = response.task_result
                && result.status != pb::generic_result::GenericStatus::Unspecified as i32
            {
                return Err(format!(
                    "Modal sandbox {sandbox_id} is no longer running ({}){}",
                    generic_status_name(result.status),
                    if result.exception.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", result.exception)
                    }
                ));
            }
        }
    }

    /// Whether the sandbox is still running.
    pub async fn sandbox_status(&self, sandbox_id: &str) -> Result<SandboxStatus, String> {
        let request = self
            .request(pb::SandboxWaitRequest {
                sandbox_id: sandbox_id.to_string(),
                timeout: 0.0,
            })
            .await?;
        let response = self
            .stub
            .clone()
            .sandbox_wait(request)
            .await
            .map_err(|s| status_error("Failed to read Modal sandbox status", &s))?
            .into_inner();
        Ok(match response.result {
            Some(result)
                if result.status != pb::generic_result::GenericStatus::Unspecified as i32 =>
            {
                SandboxStatus::Finished {
                    status: generic_status_name(result.status),
                    exception: (!result.exception.is_empty()).then_some(result.exception),
                }
            }
            _ => SandboxStatus::Running,
        })
    }

    /// Terminate a sandbox. Terminating one that already finished succeeds.
    pub async fn terminate(&self, sandbox_id: &str) -> Result<(), String> {
        let request = self
            .request(pb::SandboxTerminateRequest {
                sandbox_id: sandbox_id.to_string(),
            })
            .await?;
        self.stub
            .clone()
            .sandbox_terminate(request)
            .await
            .map_err(|s| status_error("Failed to terminate Modal sandbox", &s))?;
        Ok(())
    }

    /// Public URLs for the sandbox's exposed ports.
    pub async fn tunnels(&self, sandbox_id: &str) -> Result<Vec<TunnelInfo>, String> {
        let request = self
            .request(pb::SandboxGetTunnelsRequest {
                sandbox_id: sandbox_id.to_string(),
                timeout: 50.0,
            })
            .await?;
        let response = self
            .stub
            .clone()
            .sandbox_get_tunnels(request)
            .await
            .map_err(|s| status_error("Failed to read Modal sandbox tunnels", &s))?
            .into_inner();
        if let Some(result) = response.result
            && result.status == pb::generic_result::GenericStatus::Timeout as i32
        {
            return Err("Timed out waiting for Modal sandbox tunnels".to_string());
        }
        Ok(response
            .tunnels
            .into_iter()
            .map(|t| TunnelInfo {
                container_port: t.container_port,
                url: if t.port == 443 {
                    format!("https://{}", t.host)
                } else {
                    format!("https://{}:{}", t.host, t.port)
                },
            })
            .collect())
    }

    /// Snapshot the sandbox filesystem into a new image; returns the image ID.
    pub async fn snapshot_filesystem(&self, sandbox_id: &str) -> Result<String, String> {
        let request = self
            .request(pb::SandboxSnapshotFsRequest {
                sandbox_id: sandbox_id.to_string(),
                timeout: 55.0,
            })
            .await?;
        let response = self
            .stub
            .clone()
            .sandbox_snapshot_fs(request)
            .await
            .map_err(|s| status_error("Failed to snapshot Modal sandbox", &s))?
            .into_inner();
        match response.result {
            Some(result) if result.status != pb::generic_result::GenericStatus::Success as i32 => {
                Err(format!(
                    "Modal snapshot ended with status {}: {}",
                    generic_status_name(result.status),
                    result.exception
                ))
            }
            _ if response.image_id.is_empty() => {
                Err("Modal snapshot returned no image ID".to_string())
            }
            _ => Ok(response.image_id),
        }
    }

    async fn router(&self, task_id: &str, refresh: bool) -> Result<Router, String> {
        let mut routers = self.routers.lock().await;
        if !refresh && let Some(router) = routers.get(task_id) {
            return Ok(router.clone());
        }
        let request = self
            .request(pb::TaskGetCommandRouterAccessRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        let access = self
            .stub
            .clone()
            .task_get_command_router_access(request)
            .await
            .map_err(|s| status_error("Failed to reach Modal sandbox", &s))?
            .into_inner();
        // THREAT[TM-AGENT-020]: the JWT authorises commands in this task; only
        // ever send it over TLS to the router the control plane named.
        let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
            .iter()
            .any(|prefix| access.url.starts_with(prefix));
        if !access.url.starts_with("https://") && !loopback {
            return Err(format!(
                "Modal command router URL must be https: {}",
                access.url
            ));
        }
        let stub = match routers.get(task_id).filter(|r| r.url == access.url) {
            Some(existing) => existing.stub.clone(),
            None => rpb::task_command_router_client::TaskCommandRouterClient::new(
                transport::channel(&access.url)?,
            )
            .max_decoding_message_size(transport::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(transport::MAX_MESSAGE_SIZE),
        };
        let router = Router {
            url: access.url,
            jwt: access.jwt,
            stub,
        };
        routers.insert(task_id.to_string(), router.clone());
        Ok(router)
    }

    fn router_request<T>(
        router: &Router,
        message: T,
        timeout: Option<Duration>,
    ) -> Result<Request<T>, String> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {}", router.jwt).as_str())
                .map_err(|_| "Modal router token contains invalid characters".to_string())?,
        );
        if let Some(timeout) = timeout {
            request.set_timeout(timeout);
        }
        Ok(request)
    }

    /// Run a unary router call with JWT refresh on `Unauthenticated` and
    /// bounded retries on transient errors.
    async fn router_call<R, F, Fut>(
        &self,
        task_id: &str,
        context: &str,
        mut call: F,
    ) -> Result<R, String>
    where
        F: FnMut(Router) -> Fut,
        Fut: std::future::Future<Output = Result<R, Status>>,
    {
        let mut router = self.router(task_id, false).await?;
        let mut refreshed = false;
        let mut delay = Duration::from_millis(50);
        let mut attempt = 0;
        loop {
            match call(router.clone()).await {
                Ok(value) => return Ok(value),
                Err(status) if status.code() == Code::Unauthenticated && !refreshed => {
                    refreshed = true;
                    router = self.router(task_id, true).await?;
                }
                Err(status) if is_transient(status.code()) && attempt < ROUTER_RETRIES => {
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
                Err(status) => return Err(status_error(context, &status)),
            }
        }
    }

    /// Run `command` (argv, no shell) in the sandbox's task and collect its output.
    ///
    /// `stdin`, when given, is written in full and then closed; otherwise
    /// stdin is closed immediately so commands that read it do not hang.
    pub async fn exec(
        &self,
        task_id: &str,
        command: &[String],
        workdir: Option<&str>,
        env: &HashMap<String, String>,
        timeout: Duration,
        stdin: Option<&[u8]>,
    ) -> Result<ExecOutput, String> {
        if command.is_empty() {
            return Err("Command must not be empty".to_string());
        }
        let exec_id = uuid::Uuid::new_v4().to_string();
        let timeout_secs = u32::try_from(timeout.as_secs().max(1)).unwrap_or(u32::MAX);
        let start = rpb::TaskExecStartRequest {
            task_id: task_id.to_string(),
            exec_id: exec_id.clone(),
            command_args: command.to_vec(),
            stdout_config: rpb::TaskExecStdoutConfig::Pipe as i32,
            stderr_config: rpb::TaskExecStderrConfig::Pipe as i32,
            timeout_secs: Some(timeout_secs),
            workdir: workdir.map(str::to_string),
            env: env.clone(),
        };
        self.router_call(
            task_id,
            "Failed to start command in Modal sandbox",
            |router| {
                let start = start.clone();
                async move {
                    let request =
                        Self::router_request(&router, start, Some(Duration::from_secs(60)))
                            .map_err(Status::invalid_argument)?;
                    router.stub.clone().task_exec_start(request).await
                }
            },
        )
        .await?;

        let deadline = Instant::now() + timeout + EXEC_DEADLINE_SLACK;

        // Write stdin while the output streams drain, so a command that
        // echoes a large input cannot deadlock on a full pipe.
        let stdin_task = {
            let this = self.clone();
            let task_id = task_id.to_string();
            let exec_id = exec_id.clone();
            let data = stdin.map(<[u8]>::to_vec).unwrap_or_default();
            async move { this.write_stdin(&task_id, &exec_id, &data).await }
        };
        let stdout_task = self.read_stdio(
            task_id,
            &exec_id,
            rpb::TaskExecStdioFileDescriptor::Stdout,
            deadline,
        );
        let stderr_task = self.read_stdio(
            task_id,
            &exec_id,
            rpb::TaskExecStdioFileDescriptor::Stderr,
            deadline,
        );
        let (stdin_result, stdout, stderr) = tokio::join!(stdin_task, stdout_task, stderr_task);
        stdin_result?;
        let (stdout, stdout_truncated) = stdout?;
        let (stderr, stderr_truncated) = stderr?;

        let exit_code = self.wait_exec(task_id, &exec_id, deadline).await?;
        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            exit_code,
            truncated: stdout_truncated || stderr_truncated,
        })
    }

    async fn write_stdin(&self, task_id: &str, exec_id: &str, data: &[u8]) -> Result<(), String> {
        let mut offset = 0usize;
        loop {
            let end = (offset + STDIN_CHUNK_BYTES).min(data.len());
            let eof = end == data.len();
            let message = rpb::TaskExecStdinWriteRequest {
                task_id: task_id.to_string(),
                exec_id: exec_id.to_string(),
                offset: offset as u64,
                data: data[offset..end].to_vec(),
                eof,
            };
            self.router_call(
                task_id,
                "Failed to write command input in Modal sandbox",
                |router| {
                    let message = message.clone();
                    async move {
                        let request =
                            Self::router_request(&router, message, Some(Duration::from_secs(60)))
                                .map_err(Status::invalid_argument)?;
                        router.stub.clone().task_exec_stdin_write(request).await
                    }
                },
            )
            .await?;
            if eof {
                return Ok(());
            }
            offset = end;
        }
    }

    /// Drain one output stream, resuming from the byte offset after a
    /// transient disconnect. Returns the captured bytes and whether they were capped.
    async fn read_stdio(
        &self,
        task_id: &str,
        exec_id: &str,
        fd: rpb::TaskExecStdioFileDescriptor,
        deadline: Instant,
    ) -> Result<(Vec<u8>, bool), String> {
        let mut buffer = Vec::new();
        let mut truncated = false;
        let mut offset: u64 = 0;
        let mut retries = 0u32;
        let mut refreshed = false;
        let mut refresh = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Timed out reading command output from Modal sandbox".to_string());
            }
            let router = self.router(task_id, std::mem::take(&mut refresh)).await?;
            let request = Self::router_request(
                &router,
                rpb::TaskExecStdioReadRequest {
                    task_id: task_id.to_string(),
                    exec_id: exec_id.to_string(),
                    offset,
                    file_descriptor: fd as i32,
                },
                Some(remaining),
            )?;
            let failure = match router.stub.clone().task_exec_stdio_read(request).await {
                Ok(stream) => {
                    let mut stream = stream.into_inner();
                    let mut failure = None;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(chunk) => {
                                retries = 0;
                                offset += chunk.data.len() as u64;
                                let room = MAX_EXEC_OUTPUT_BYTES.saturating_sub(buffer.len());
                                if chunk.data.len() > room {
                                    truncated = true;
                                }
                                buffer.extend_from_slice(&chunk.data[..chunk.data.len().min(room)]);
                            }
                            Err(status) => {
                                failure = Some(status);
                                break;
                            }
                        }
                    }
                    match failure {
                        None => return Ok((buffer, truncated)),
                        Some(status) => status,
                    }
                }
                Err(status) => status,
            };
            if failure.code() == Code::Unauthenticated && !refreshed {
                refreshed = true;
                refresh = true;
                continue;
            }
            if is_transient(failure.code()) && retries < ROUTER_RETRIES {
                retries += 1;
                tokio::time::sleep(Duration::from_millis(50 * u64::from(retries))).await;
                continue;
            }
            return Err(status_error(
                "Failed to read command output from Modal sandbox",
                &failure,
            ));
        }
    }

    async fn wait_exec(
        &self,
        task_id: &str,
        exec_id: &str,
        deadline: Instant,
    ) -> Result<i32, String> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Timed out waiting for command to finish in Modal sandbox".to_string());
            }
            let message = rpb::TaskExecWaitRequest {
                task_id: task_id.to_string(),
                exec_id: exec_id.to_string(),
            };
            let call_timeout = remaining.min(EXEC_WAIT_CALL);
            let router = self.router(task_id, false).await?;
            let request = Self::router_request(&router, message, Some(call_timeout))?;
            match router.stub.clone().task_exec_wait(request).await {
                Ok(response) => {
                    return Ok(match response.into_inner().exit_status {
                        Some(rpb::task_exec_wait_response::ExitStatus::Code(code)) => code,
                        Some(rpb::task_exec_wait_response::ExitStatus::Signal(signal)) => {
                            128 + signal
                        }
                        None => -1,
                    });
                }
                Err(status) if status.code() == Code::Unauthenticated => {
                    self.router(task_id, true).await?;
                }
                Err(status) if is_transient(status.code()) => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(status) => {
                    return Err(status_error(
                        "Failed to wait for command in Modal sandbox",
                        &status,
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_pairs_in_the_forms_modal_prints() {
        for raw in [
            "ak-abc:as-def",
            " ak-abc : as-def ",
            "ak-abc as-def",
            "as-def\nak-abc",
            "--token-id ak-abc --token-secret as-def",
            "modal token set --token-id=ak-abc --token-secret=as-def",
        ] {
            let creds = ModalCredentials::parse(raw).unwrap();
            assert_eq!(creds.token_id, "ak-abc", "{raw}");
            assert_eq!(creds.token_secret, "as-def", "{raw}");
        }
        let unlabelled = ModalCredentials::parse("id123:secret456").unwrap();
        assert_eq!(unlabelled.token_id, "id123");
        assert_eq!(unlabelled.token_secret, "secret456");
    }

    #[test]
    fn rejects_incomplete_token_pairs() {
        for raw in ["", "ak-abc", "as-def", "a:b:c", "ak-abc:"] {
            assert!(ModalCredentials::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn connection_string_round_trips() {
        let creds = ModalCredentials::parse("ak-1:as-2").unwrap();
        let again = ModalCredentials::parse(&creds.to_connection_string()).unwrap();
        assert_eq!(again.token_id, "ak-1");
        assert_eq!(again.token_secret, "as-2");
    }

    #[test]
    fn debug_redacts_the_secret() {
        let creds = ModalCredentials::parse("ak-1:as-topsecret").unwrap();
        assert!(!format!("{creds:?}").contains("topsecret"));
    }

    #[test]
    fn reads_jwt_expiry() {
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1700000000}"#);
        assert_eq!(jwt_expiry(&format!("h.{payload}.s")), Some(1_700_000_000));
        assert_eq!(jwt_expiry("not-a-jwt"), None);
    }

    #[test]
    fn names_generic_statuses() {
        assert_eq!(generic_status_name(1), "success");
        assert_eq!(generic_status_name(7), "idle_timeout");
        assert_eq!(generic_status_name(99), "status_99");
    }
}
