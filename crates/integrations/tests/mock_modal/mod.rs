#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! In-process mock of Modal's gRPC API, shared by the Modal test binaries.
//!
//! It serves both the control plane (`modal.client.ModalClient`) and the
//! command router (`modal.task_command_router.TaskCommandRouter`) on one
//! loopback port. Its router keeps a tiny in-memory filesystem so file writes
//! (stdin into `cat > "$1"`) and reads (`cat --`, or `base64`) round-trip
//! through the same wire calls the real router sees. It also enforces the auth
//! contract: control-plane calls need the token pair and a minted auth token,
//! and the router rejects the first JWT it hands out, so every test exercises
//! a JWT refresh.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use base64::Engine as _;
use everruns_integrations::modal::proto::client as pb;
use everruns_integrations::modal::proto::router as rpb;
use futures::Stream;
use tonic::{Request, Response, Status};

// ============================================================================
// Mock Modal
// ============================================================================

/// A started command: argv, working directory, extra environment.
pub type ExecRecord = (Vec<String>, Option<String>, HashMap<String, String>);

#[derive(Default)]
pub struct MockState {
    pub url: String,
    pub image_builds: Vec<Vec<String>>,
    pub sandboxes: HashMap<String, pb::Sandbox>,
    pub terminated: Vec<String>,
    pub files: HashMap<String, Vec<u8>>,
    pub execs: HashMap<String, ExecRecord>,
    pub stdin: HashMap<String, Vec<u8>>,
    pub router_access_calls: u32,
}

#[derive(Clone, Default)]
pub struct MockModal {
    pub state: Arc<StdMutex<MockState>>,
}

pub const TOKEN_ID: &str = "ak-test";
pub const TOKEN_SECRET: &str = "as-test";
pub const AUTH_TOKEN: &str = "auth-token";
pub const GOOD_JWT: &str = "jwt-fresh";

pub fn header<'a, T>(request: &'a Request<T>, name: &str) -> Option<&'a str> {
    request.metadata().get(name).and_then(|v| v.to_str().ok())
}

pub fn check_token_pair<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    if header(request, "x-modal-token-id") != Some(TOKEN_ID)
        || header(request, "x-modal-token-secret") != Some(TOKEN_SECRET)
    {
        return Err(Status::unauthenticated("invalid token pair"));
    }
    Ok(())
}

pub fn check_control<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    check_token_pair(request)?;
    if header(request, "x-modal-auth-token") != Some(AUTH_TOKEN) {
        return Err(Status::unauthenticated("missing auth token"));
    }
    Ok(())
}

pub fn check_router<T>(request: &Request<T>) -> std::result::Result<(), Status> {
    if header(request, "authorization") != Some(&format!("Bearer {GOOD_JWT}")) {
        return Err(Status::unauthenticated("jwt expired"));
    }
    Ok(())
}

pub fn success() -> pb::GenericResult {
    pb::GenericResult {
        status: pb::generic_result::GenericStatus::Success as i32,
        ..Default::default()
    }
}

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = std::result::Result<T, Status>> + Send>>;

impl MockModal {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }

    /// Run a recorded exec against the in-memory filesystem: returns (stdout, stderr, code).
    pub fn run(&self, exec_id: &str) -> (Vec<u8>, Vec<u8>, i32) {
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
            ["sh", "-c", script, "sh", path] if script.contains("base64") => {
                match self.lock().files.get(*path) {
                    Some(bytes) => (
                        base64::engine::general_purpose::STANDARD
                            .encode(bytes)
                            .into_bytes(),
                        vec![],
                        0,
                    ),
                    None => (
                        vec![],
                        format!("sh: {path}: No such file or directory").into_bytes(),
                        1,
                    ),
                }
            }
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

pub async fn start_mock() -> (MockModal, String) {
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
