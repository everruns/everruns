#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Modal tools end to end against an in-process mock of Modal's gRPC API.
//!
//! The mock serves both the control plane (`modal.client.ModalClient`) and the
//! command router (`modal.task_command_router.TaskCommandRouter`) on one
//! loopback port. Its router keeps a tiny in-memory filesystem so file writes
//! (stdin into `cat > path`) and reads (`cat -- path`) round-trip through the
//! same wire calls the real router sees. It also enforces the auth contract:
//! control-plane calls need the token pair and a minted auth token, and the
//! router rejects the first JWT it hands out, so every test exercises a JWT
//! refresh.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::session_services::SessionStorageStore;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_integrations::modal::ModalCapability;
use everruns_integrations::modal::proto::client as pb;
use everruns_integrations::modal::proto::router as rpb;
use everruns_integrations::modal::state::ModalServerUrlOverride;
use futures::Stream;
use serde_json::{Value, json};
use tonic::{Request, Response, Status};

mod common;

use common::{MockLeasedResourceStore, MockStorageStore, call as call_tool, ok, tool_error};

// ============================================================================
// Mock Modal
// ============================================================================

/// A started command: argv, working directory, extra environment.
type ExecRecord = (Vec<String>, Option<String>, HashMap<String, String>);

#[derive(Default)]
struct MockState {
    url: String,
    image_builds: Vec<Vec<String>>,
    sandboxes: HashMap<String, pb::Sandbox>,
    terminated: Vec<String>,
    files: HashMap<String, Vec<u8>>,
    execs: HashMap<String, ExecRecord>,
    stdin: HashMap<String, Vec<u8>>,
    router_access_calls: u32,
}

#[derive(Clone, Default)]
struct MockModal {
    state: Arc<StdMutex<MockState>>,
}

const TOKEN_ID: &str = "ak-test";
const TOKEN_SECRET: &str = "as-test";
const AUTH_TOKEN: &str = "auth-token";
const GOOD_JWT: &str = "jwt-fresh";

fn header<'a, T>(request: &'a Request<T>, name: &str) -> Option<&'a str> {
    request.metadata().get(name).and_then(|v| v.to_str().ok())
}

fn check_token_pair<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    if header(request, "x-modal-token-id") != Some(TOKEN_ID)
        || header(request, "x-modal-token-secret") != Some(TOKEN_SECRET)
    {
        return Err(Status::unauthenticated("invalid token pair"));
    }
    Ok(())
}

fn check_control<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    check_token_pair(request)?;
    if header(request, "x-modal-auth-token") != Some(AUTH_TOKEN) {
        return Err(Status::unauthenticated("missing auth token"));
    }
    Ok(())
}

fn check_router<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    if header(request, "authorization") != Some(&format!("Bearer {GOOD_JWT}")) {
        return Err(Status::unauthenticated("jwt expired"));
    }
    Ok(())
}

fn success() -> pb::GenericResult {
    pb::GenericResult {
        status: pb::generic_result::GenericStatus::Success as i32,
        ..Default::default()
    }
}

type BoxStream<T> = Pin<Box<dyn Stream<Item = std::result::Result<T, Status>> + Send>>;

impl MockModal {
    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }

    /// Run a recorded exec against the in-memory filesystem: returns (stdout, stderr, code).
    fn run(&self, exec_id: &str) -> (Vec<u8>, Vec<u8>, i32) {
        let state = self.lock();
        let Some((argv, workdir, env)) = state.execs.get(exec_id).cloned() else {
            return (vec![], b"unknown exec".to_vec(), 127);
        };
        let stdin = state.stdin.get(exec_id).cloned().unwrap_or_default();
        drop(state);
        match argv
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["cat", "--", path] => match self.lock().files.get(*path) {
                Some(bytes) => (bytes.clone(), vec![], 0),
                None => (
                    vec![],
                    format!("cat: {path}: No such file or directory").into_bytes(),
                    1,
                ),
            },
            ["sh", "-c", script, "sh", path] if script.contains("cat > \"$1\"") => {
                self.lock().files.insert((*path).to_string(), stdin);
                (vec![], vec![], 0)
            }
            ["mkdir", "-p", _] => (vec![], vec![], 0),
            ["sh", "-c", command] => {
                if command.contains("exit 3") {
                    (b"partial\n".to_vec(), b"boom\n".to_vec(), 3)
                } else {
                    let env_line = env
                        .get("GREETING")
                        .map(|g| format!("GREETING={g}\n"))
                        .unwrap_or_default();
                    (
                        format!(
                            "ran: {command}\ncwd: {}\n{env_line}",
                            workdir.unwrap_or_default()
                        )
                        .into_bytes(),
                        vec![],
                        0,
                    )
                }
            }
            other => (vec![], format!("unsupported: {other:?}").into_bytes(), 127),
        }
    }
}

#[tonic::async_trait]
impl pb::modal_client_server::ModalClient for MockModal {
    async fn app_get_or_create(
        &self,
        request: Request<pb::AppGetOrCreateRequest>,
    ) -> std::result::Result<Response<pb::AppGetOrCreateResponse>, Status> {
        check_control(&request)?;
        assert_eq!(request.get_ref().app_name, "everruns-sandboxes");
        Ok(Response::new(pb::AppGetOrCreateResponse {
            app_id: "ap-mock".into(),
        }))
    }

    async fn auth_token_get(
        &self,
        request: Request<pb::AuthTokenGetRequest>,
    ) -> std::result::Result<Response<pb::AuthTokenGetResponse>, Status> {
        check_token_pair(&request)?;
        Ok(Response::new(pb::AuthTokenGetResponse {
            token: AUTH_TOKEN.into(),
        }))
    }

    async fn image_get_or_create(
        &self,
        request: Request<pb::ImageGetOrCreateRequest>,
    ) -> std::result::Result<Response<pb::ImageGetOrCreateResponse>, Status> {
        check_control(&request)?;
        let image = request.into_inner().image.unwrap_or_default();
        self.lock().image_builds.push(image.dockerfile_commands);
        // Not built yet: the client must wait on ImageJoinStreaming.
        Ok(Response::new(pb::ImageGetOrCreateResponse {
            image_id: "im-built".into(),
            result: None,
        }))
    }

    type ImageJoinStreamingStream = BoxStream<pb::ImageJoinStreamingResponse>;

    async fn image_join_streaming(
        &self,
        request: Request<pb::ImageJoinStreamingRequest>,
    ) -> std::result::Result<Response<Self::ImageJoinStreamingStream>, Status> {
        check_control(&request)?;
        let items = vec![
            Ok(pb::ImageJoinStreamingResponse {
                result: None,
                entry_id: "1".into(),
                eof: false,
            }),
            Ok(pb::ImageJoinStreamingResponse {
                result: Some(success()),
                entry_id: "2".into(),
                eof: true,
            }),
        ];
        Ok(Response::new(Box::pin(futures::stream::iter(items))))
    }

    async fn sandbox_create(
        &self,
        request: Request<pb::SandboxCreateRequest>,
    ) -> std::result::Result<Response<pb::SandboxCreateResponse>, Status> {
        check_control(&request)?;
        let request = request.into_inner();
        let mut state = self.lock();
        let id = format!("sb-mock{}", state.sandboxes.len() + 1);
        state
            .sandboxes
            .insert(id.clone(), request.definition.unwrap_or_default());
        Ok(Response::new(pb::SandboxCreateResponse { sandbox_id: id }))
    }

    async fn sandbox_get_task_id(
        &self,
        request: Request<pb::SandboxGetTaskIdRequest>,
    ) -> std::result::Result<Response<pb::SandboxGetTaskIdResponse>, Status> {
        check_control(&request)?;
        let id = request.into_inner().sandbox_id;
        Ok(Response::new(pb::SandboxGetTaskIdResponse {
            task_id: Some(id.replace("sb-", "ta-")),
            task_result: None,
        }))
    }

    async fn sandbox_get_tunnels(
        &self,
        request: Request<pb::SandboxGetTunnelsRequest>,
    ) -> std::result::Result<Response<pb::SandboxGetTunnelsResponse>, Status> {
        check_control(&request)?;
        let id = request.into_inner().sandbox_id;
        let state = self.lock();
        let ports = match state
            .sandboxes
            .get(&id)
            .and_then(|s| s.open_ports_oneof.clone())
        {
            Some(pb::sandbox::OpenPortsOneof::OpenPorts(specs)) => specs.ports,
            None => vec![],
        };
        Ok(Response::new(pb::SandboxGetTunnelsResponse {
            result: None,
            tunnels: ports
                .iter()
                .map(|p| pb::TunnelData {
                    host: format!("{id}-{}.modal.host", p.port),
                    port: 443,
                    unencrypted_host: None,
                    unencrypted_port: None,
                    container_port: p.port,
                })
                .collect(),
        }))
    }

    async fn sandbox_snapshot_fs(
        &self,
        request: Request<pb::SandboxSnapshotFsRequest>,
    ) -> std::result::Result<Response<pb::SandboxSnapshotFsResponse>, Status> {
        check_control(&request)?;
        Ok(Response::new(pb::SandboxSnapshotFsResponse {
            image_id: "im-snapshot".into(),
            result: Some(success()),
        }))
    }

    async fn sandbox_terminate(
        &self,
        request: Request<pb::SandboxTerminateRequest>,
    ) -> std::result::Result<Response<pb::SandboxTerminateResponse>, Status> {
        check_control(&request)?;
        let id = request.into_inner().sandbox_id;
        self.lock().terminated.push(id);
        Ok(Response::new(pb::SandboxTerminateResponse {
            existing_result: None,
        }))
    }

    async fn sandbox_wait(
        &self,
        request: Request<pb::SandboxWaitRequest>,
    ) -> std::result::Result<Response<pb::SandboxWaitResponse>, Status> {
        check_control(&request)?;
        let id = request.into_inner().sandbox_id;
        let finished = self.lock().terminated.contains(&id);
        Ok(Response::new(pb::SandboxWaitResponse {
            result: finished.then(|| pb::GenericResult {
                status: pb::generic_result::GenericStatus::Terminated as i32,
                ..Default::default()
            }),
        }))
    }

    async fn task_get_command_router_access(
        &self,
        request: Request<pb::TaskGetCommandRouterAccessRequest>,
    ) -> std::result::Result<Response<pb::TaskGetCommandRouterAccessResponse>, Status> {
        check_control(&request)?;
        let task_id = request.into_inner().task_id;
        let mut state = self.lock();
        if state.terminated.contains(&task_id.replace("ta-", "sb-")) {
            return Err(Status::not_found("task has finished"));
        }
        state.router_access_calls += 1;
        // The first JWT is already expired, as after a long idle period.
        let jwt = if state.router_access_calls == 1 {
            "jwt-stale"
        } else {
            GOOD_JWT
        };
        Ok(Response::new(pb::TaskGetCommandRouterAccessResponse {
            jwt: jwt.into(),
            url: state.url.clone(),
        }))
    }
}

#[tonic::async_trait]
impl rpb::task_command_router_server::TaskCommandRouter for MockModal {
    async fn task_exec_start(
        &self,
        request: Request<rpb::TaskExecStartRequest>,
    ) -> std::result::Result<Response<rpb::TaskExecStartResponse>, Status> {
        check_router(&request)?;
        let request = request.into_inner();
        assert_eq!(
            request.stdout_config,
            rpb::TaskExecStdoutConfig::Pipe as i32
        );
        assert_eq!(
            request.stderr_config,
            rpb::TaskExecStderrConfig::Pipe as i32
        );
        assert!(request.timeout_secs.is_some());
        self.lock().execs.insert(
            request.exec_id,
            (request.command_args, request.workdir, request.env),
        );
        Ok(Response::new(rpb::TaskExecStartResponse {}))
    }

    async fn task_exec_stdin_write(
        &self,
        request: Request<rpb::TaskExecStdinWriteRequest>,
    ) -> std::result::Result<Response<rpb::TaskExecStdinWriteResponse>, Status> {
        check_router(&request)?;
        let request = request.into_inner();
        let mut state = self.lock();
        let buffer = state.stdin.entry(request.exec_id).or_default();
        assert_eq!(
            buffer.len() as u64,
            request.offset,
            "stdin offsets must be contiguous"
        );
        buffer.extend_from_slice(&request.data);
        Ok(Response::new(rpb::TaskExecStdinWriteResponse {}))
    }

    type TaskExecStdioReadStream = BoxStream<rpb::TaskExecStdioReadResponse>;

    async fn task_exec_stdio_read(
        &self,
        request: Request<rpb::TaskExecStdioReadRequest>,
    ) -> std::result::Result<Response<Self::TaskExecStdioReadStream>, Status> {
        check_router(&request)?;
        let request = request.into_inner();
        // Give stdin writes (which race with this read) a moment to land.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (stdout, stderr, _) = self.run(&request.exec_id);
        let data = if request.file_descriptor == rpb::TaskExecStdioFileDescriptor::Stdout as i32 {
            stdout
        } else {
            stderr
        };
        let data = data[(request.offset as usize).min(data.len())..].to_vec();
        // Split into two chunks to exercise reassembly.
        let mid = data.len() / 2;
        let chunks = vec![data[..mid].to_vec(), data[mid..].to_vec()];
        Ok(Response::new(Box::pin(futures::stream::iter(
            chunks
                .into_iter()
                .filter(|c| !c.is_empty())
                .map(|data| Ok(rpb::TaskExecStdioReadResponse { data })),
        ))))
    }

    async fn task_exec_wait(
        &self,
        request: Request<rpb::TaskExecWaitRequest>,
    ) -> std::result::Result<Response<rpb::TaskExecWaitResponse>, Status> {
        check_router(&request)?;
        let (_, _, code) = self.run(&request.into_inner().exec_id);
        Ok(Response::new(rpb::TaskExecWaitResponse {
            exit_status: Some(rpb::task_exec_wait_response::ExitStatus::Code(code)),
        }))
    }
}

async fn start_mock() -> (MockModal, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let mock = MockModal::default();
    mock.lock().url = url.clone();
    let server = tonic::transport::Server::builder()
        .add_service(pb::modal_client_server::ModalClientServer::new(
            mock.clone(),
        ))
        .add_service(rpb::task_command_router_server::TaskCommandRouterServer::new(mock.clone()))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
    tokio::spawn(server);
    (mock, url)
}

async fn call(h: &Fixture, name: &str, args: Value) -> ToolExecutionResult {
    call_tool(&h.context, name, args).await
}

struct Fixture {
    mock: MockModal,
    storage: Arc<MockStorageStore>,
    leases: Arc<MockLeasedResourceStore>,
    context: ToolContext,
}

async fn harness_with_token(token: Option<&str>) -> Fixture {
    let (mock, url) = start_mock().await;
    let (context, storage, leases) = common::context(token);
    let context = context.with_extension(Arc::new(ModalServerUrlOverride(url)));
    Fixture {
        mock,
        storage,
        leases,
        context,
    }
}

async fn harness() -> Fixture {
    harness_with_token(Some(&format!("{TOKEN_ID}:{TOKEN_SECRET}"))).await
}

async fn create(h: &Fixture, args: Value) -> String {
    let created = ok(call(h, "modal_create_sandbox", args).await);
    created["sandbox_id"].as_str().unwrap().to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn full_lifecycle_runs_through_the_wire() {
    let h = harness().await;

    let created = ok(call(
        &h,
        "modal_create_sandbox",
        json!({"title": "demo", "expose_ports": [8080, 8080, 3000], "cpu": 2, "memory_mb": 4096}),
    )
    .await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();
    assert_eq!(created["runtime"], "vm");
    assert_eq!(created["workspace_path"], "/workspace");
    assert_eq!(created["tunnels"].as_array().unwrap().len(), 2);

    {
        let state = h.mock.lock();
        // The default image adds git and curl on top of the base image.
        assert_eq!(state.image_builds.len(), 1);
        assert_eq!(state.image_builds[0][0], "FROM python:3.13-slim");
        assert!(state.image_builds[0][1].contains("git curl"));
        let definition = &state.sandboxes[&sandbox_id];
        assert_eq!(definition.runtime.as_deref(), Some("vm"));
        assert_eq!(definition.image_id, "im-built");
        assert_eq!(definition.timeout_secs, 3600);
        assert_eq!(definition.entrypoint_args, ["sleep", "infinity"]);
        let resources = definition.resources.unwrap();
        assert_eq!((resources.milli_cpu, resources.memory_mb), (2000, 4096));
    }
    let lease = h.leases.resources.lock().await[0].clone();
    assert_eq!(
        (lease.provider.as_str(), lease.resource_type.as_str()),
        ("modal", "sandbox")
    );
    assert_eq!(lease.external_id, sandbox_id);
    assert_eq!(lease.display_name.as_deref(), Some("demo"));

    let exec = ok(call(
        &h,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "echo hi", "cwd": "src", "env": {"GREETING": "hello"}}),
    )
    .await);
    let stdout = exec["stdout"].as_str().unwrap();
    assert!(stdout.contains("ran: echo hi"), "{stdout}");
    assert!(stdout.contains("cwd: /workspace/src"), "{stdout}");
    assert!(stdout.contains("GREETING=hello"), "{stdout}");
    assert_eq!(exec["exit_code"], 0);

    let failed = ok(call(
        &h,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "exit 3"}),
    )
    .await);
    assert_eq!(failed["exit_code"], 3);
    assert_eq!(failed["success"], false);
    assert!(failed["stderr"].as_str().unwrap().contains("boom"));

    // 3 MiB crosses the 1 MiB stdin chunk size, so offsets must line up.
    let big = "x".repeat(3 * 1024 * 1024 + 7);
    let written = ok(call(
        &h,
        "modal_write_file",
        json!({"sandbox_id": sandbox_id, "path": "data/big.txt", "content": big}),
    )
    .await);
    assert_eq!(written["path"], "/workspace/data/big.txt");
    assert_eq!(
        h.mock.lock().files["/workspace/data/big.txt"].len(),
        big.len()
    );

    ok(call(
        &h,
        "modal_write_file",
        json!({"sandbox_id": sandbox_id, "path": "/tmp/notes.txt", "content": "one\ntwo\nthree\n"}),
    )
    .await);
    let read = ok(call(
        &h,
        "modal_read_file",
        json!({"sandbox_id": sandbox_id, "path": "/tmp/notes.txt", "offset": 1, "limit": 1}),
    )
    .await);
    let rendered = read.to_string();
    assert!(rendered.contains("two"), "{rendered}");
    assert!(!rendered.contains("three"), "{rendered}");
    assert_eq!(read["sandbox_id"], sandbox_id);

    let missing = tool_error(
        call(
            &h,
            "modal_read_file",
            json!({"sandbox_id": sandbox_id, "path": "/nope"}),
        )
        .await,
    );
    assert!(missing.contains("No such file"), "{missing}");

    let listed = ok(call(&h, "modal_list_sandboxes", json!({})).await);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["sandboxes"][0]["exposed_ports"], json!([3000, 8080]));

    let snapshot = ok(call(
        &h,
        "modal_snapshot_sandbox",
        json!({"sandbox_id": sandbox_id}),
    )
    .await);
    assert_eq!(snapshot["snapshot_image_id"], "im-snapshot");

    let tunnels = ok(call(&h, "modal_tunnel_urls", json!({"sandbox_id": sandbox_id})).await);
    assert_eq!(
        tunnels["tunnels"][1]["url"],
        format!("https://{sandbox_id}-8080.modal.host")
    );

    let status = ok(call(
        &h,
        "modal_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "status"}),
    )
    .await);
    assert_eq!(status["status"], "running");

    ok(call(
        &h,
        "modal_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "terminate"}),
    )
    .await);
    assert_eq!(h.mock.lock().terminated, std::slice::from_ref(&sandbox_id));
    assert!(h.storage.secrets.lock().await.is_empty());
    assert_eq!(
        h.leases.resources.lock().await[0].status,
        LeasedResourceStatus::Released
    );
    let gone = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    // The released lease is what refuses it, before any call reaches Modal.
    assert!(gone.contains("not created by this session"), "{gone}");
}

#[tokio::test]
async fn custom_images_and_snapshots_boot_what_was_asked() {
    let h = harness().await;
    create(
        &h,
        json!({"image": "node:22", "setup_commands": ["RUN npm i -g pnpm"], "runtime": "gvisor"}),
    )
    .await;
    create(
        &h,
        json!({"snapshot_image_id": "im-snapshot", "timeout_seconds": 600}),
    )
    .await;

    let state = h.mock.lock();
    assert_eq!(
        state.image_builds,
        [vec![
            "FROM node:22".to_string(),
            "RUN npm i -g pnpm".to_string()
        ]]
    );
    let gvisor = &state.sandboxes["sb-mock1"];
    assert_eq!(gvisor.runtime.as_deref(), Some("gvisor"));
    let restored = &state.sandboxes["sb-mock2"];
    assert_eq!(restored.image_id, "im-snapshot");
    assert_eq!(restored.timeout_secs, 600);
}

#[tokio::test]
async fn a_finished_sandbox_is_reported_as_such() {
    let h = harness().await;
    let sandbox_id = create(&h, json!({})).await;
    // Modal ended it (timeout, idle timeout, or outside termination).
    h.mock.lock().terminated.push(sandbox_id.clone());
    let message = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    assert!(message.contains("no longer running"), "{message}");
    assert!(message.contains("terminated"), "{message}");
}

#[tokio::test]
async fn a_missing_connection_asks_for_one() {
    let h = harness_with_token(None).await;
    match call(&h, "modal_create_sandbox", json!({})).await {
        ToolExecutionResult::ConnectionRequired { provider, .. } => assert_eq!(provider, "modal"),
        other => panic!("expected ConnectionRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn rejected_credentials_say_so() {
    let h = harness_with_token(Some("ak-test:as-wrong")).await;
    let message = tool_error(call(&h, "modal_create_sandbox", json!({})).await);
    assert!(message.contains("rejected the credentials"), "{message}");
    let malformed = harness_with_token(Some("only-one-part")).await;
    let message = tool_error(call(&malformed, "modal_create_sandbox", json!({})).await);
    assert!(message.contains("not a valid token pair"), "{message}");
}

#[tokio::test]
async fn forged_state_without_a_lease_is_refused() {
    let h = harness().await;
    // A real sandbox gives the session lease tracking...
    create(&h, json!({})).await;
    // ...then a state record for someone else's sandbox is planted directly.
    let forged = json!({
        "sandbox_id": "sb-victim", "task_id": "ta-victim", "app_id": "ap-x",
        "runtime": "vm", "image": "x", "workspace_path": "/workspace",
        "started_at": "2026-10-05T00:00:00Z", "timeout_seconds": 60
    });
    h.storage
        .set_secret(
            h.context.session_id,
            "modal_sandbox:sb-victim",
            &forged.to_string(),
        )
        .await
        .unwrap();
    let result = call(
        &h,
        "modal_exec",
        json!({"sandbox_id": "sb-victim", "command": "id"}),
    )
    .await;
    assert!(
        !matches!(result, ToolExecutionResult::Success(_)),
        "forged sandbox was used: {result:?}"
    );
    assert!(
        h.mock
            .lock()
            .execs
            .values()
            .all(|(argv, _, _)| argv != &["sh", "-c", "id"])
    );
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_any_call() {
    let h = harness().await;
    for (args, needle) in [
        (json!({"runtime": "firecracker"}), "runtime"),
        (json!({"timeout_seconds": 10}), "timeout_seconds"),
        (json!({"cpu": 1000}), "cpu"),
        (json!({"expose_ports": [0]}), "expose_ports"),
        (json!({"image": "bad image"}), "image"),
        (
            json!({"snapshot_image_id": "im-x", "image": "node:22"}),
            "cannot be combined",
        ),
        (json!({"snapshot_image_id": "sb-x"}), "snapshot_image_id"),
        (
            json!({"setup_commands": ["RUN a\nRUN b"]}),
            "setup_commands",
        ),
    ] {
        let message = tool_error(call(&h, "modal_create_sandbox", args.clone()).await);
        assert!(message.contains(needle), "{args}: {message}");
    }
    let message = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": "../etc", "command": "ls"}),
        )
        .await,
    );
    assert!(message.contains("Invalid Modal sandbox ID"), "{message}");
    let message = tool_error(call(&h, "modal_exec", json!({"sandbox_id": "sb-1"})).await);
    assert!(
        message.contains("Missing required parameter: command"),
        "{message}"
    );
    let message = tool_error(
        call(
            &h,
            "modal_manage_sandbox",
            json!({"sandbox_id": "sb-1", "action": "pause"}),
        )
        .await,
    );
    assert!(message.contains("Invalid action"), "{message}");
    assert!(h.mock.lock().sandboxes.is_empty());
}

#[tokio::test]
async fn tools_without_context_refuse() {
    for tool in ModalCapability.tools() {
        match tool.execute(json!({})).await {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("requires context"), "{message}")
            }
            other => panic!("{}: expected ToolError, got {other:?}", tool.name()),
        }
    }
}
