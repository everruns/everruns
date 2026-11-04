#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! In-process mock of boxd, shared by the boxd test binaries.
//!
//! One loopback port serves both halves of boxd's API the way the client
//! reaches them: the HTTP key exchange (`POST /auth/token`) and the gRPC
//! service (`boxd.api.v1.BoxdApi`, cut down to the methods the client calls).
//! It enforces the auth contract: every gRPC call needs a JWT the exchange
//! minted, and the first JWT it hands out is revoked, so every test exercises
//! a JWT refresh.
//!
//! Machines keep a tiny in-memory filesystem, and `Exec` understands a few
//! commands, enough to check what reaches the machine and how output, exit
//! codes, stdin and timeouts come back.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use everruns_integrations::boxd::mock_proto as pb;
use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

pub const API_KEY: &str = "bxd_test_key";

#[derive(Default)]
pub struct MockState {
    pub url: String,
    pub machines: HashMap<String, pb::GetVmResponse>,
    pub create_requests: Vec<pb::CreateVmRequest>,
    pub snapshot_creates: Vec<pb::CreateVmFromSnapshotRequest>,
    pub destroyed: Vec<String>,
    pub egress_calls: Vec<(String, Vec<String>)>,
    pub auto_hibernate_calls: Vec<(String, u32)>,
    pub snapshots: HashMap<String, String>,
    /// Machine id -> path -> bytes.
    pub files: HashMap<String, BTreeMap<String, Vec<u8>>>,
    pub dirs: HashMap<String, HashSet<String>>,
    /// Commands each machine ran.
    pub commands: HashMap<String, Vec<String>>,
    pub jwts_issued: u32,
    pub valid_jwts: HashSet<String>,
    pub next_id: u32,
}

#[derive(Clone, Default)]
pub struct MockBoxd {
    pub state: Arc<StdMutex<MockState>>,
}

impl MockBoxd {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }

    fn authorize<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default()
            .to_string();
        if self.lock().valid_jwts.contains(&token) {
            Ok(())
        } else {
            Err(Status::unauthenticated("token expired"))
        }
    }

    fn machine(&self, id: &str) -> Result<pb::GetVmResponse, Status> {
        self.lock()
            .machines
            .get(id)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("machine {id}")))
    }

    fn set_status(&self, id: &str, status: &str) -> Result<(), Status> {
        let mut state = self.lock();
        let machine = state
            .machines
            .get_mut(id)
            .ok_or_else(|| Status::not_found(format!("machine {id}")))?;
        machine.status = status.to_string();
        Ok(())
    }

    fn new_machine(
        &self,
        name: &str,
        labels: HashMap<String, String>,
        egress: Vec<String>,
        isolated: bool,
    ) -> pb::GetVmResponse {
        let mut state = self.lock();
        state.next_id += 1;
        let id = format!("vm_{}", state.next_id);
        let name = if name.is_empty() {
            format!("auto-{}", state.next_id)
        } else {
            name.to_string()
        };
        let machine = pb::GetVmResponse {
            vm_id: id.clone(),
            name,
            // Reaches `running` on the next GetVm, so creation has to wait.
            status: "starting".into(),
            access_domain: "boxd.test".into(),
            region: "fra".into(),
            created_at: chrono::Utc::now().timestamp(),
            labels,
            egress_allow: egress,
            isolated,
            vcpu: 2,
            memory_bytes: 8 << 30,
            ..Default::default()
        };
        state.machines.insert(id, machine.clone());
        machine
    }

    /// Set a machine's status from a test, e.g. to simulate boxd losing it.
    pub fn force_status(&self, id: &str, status: &str) {
        self.set_status(id, status).unwrap();
    }

    pub fn file(&self, id: &str, path: &str) -> Option<Vec<u8>> {
        self.lock().files.get(id)?.get(path).cloned()
    }

    pub fn put_file(&self, id: &str, path: &str, data: &[u8]) {
        self.lock()
            .files
            .entry(id.to_string())
            .or_default()
            .insert(path.to_string(), data.to_vec());
    }
}

fn spec_labels(
    spec: &Option<pb::MachineSpec>,
    fallback: &HashMap<String, String>,
) -> HashMap<String, String> {
    spec.as_ref()
        .map(|spec| spec.labels.clone())
        .filter(|labels| !labels.is_empty())
        .unwrap_or_else(|| fallback.clone())
}

fn exec_chunk(data: &[u8], is_stderr: bool) -> pb::ExecChunk {
    pb::ExecChunk {
        data: data.to_vec(),
        is_stderr,
        ..Default::default()
    }
}

type ExecStream = Pin<Box<dyn Stream<Item = Result<pb::ExecChunk, Status>> + Send>>;
type ReadStream = Pin<Box<dyn Stream<Item = Result<pb::FsReadChunk, Status>> + Send>>;

#[tonic::async_trait]
impl pb::boxd_api_server::BoxdApi for MockBoxd {
    type ExecStream = ExecStream;
    type FsReadStream = ReadStream;

    async fn whoami(
        &self,
        request: Request<pb::WhoamiRequest>,
    ) -> Result<Response<pb::WhoamiResponse>, Status> {
        self.authorize(&request)?;
        Ok(Response::new(pb::WhoamiResponse {
            user_id: "user_1".into(),
            ..Default::default()
        }))
    }

    async fn list_regions(
        &self,
        request: Request<pb::ListRegionsRequest>,
    ) -> Result<Response<pb::ListRegionsResponse>, Status> {
        self.authorize(&request)?;
        let url = self.lock().url.clone();
        let region = |id: &str, home: bool, org_state: &str| pb::Region {
            id: id.into(),
            name: id.to_uppercase(),
            api_endpoint: url.clone(),
            home,
            org_state: org_state.into(),
            ..Default::default()
        };
        Ok(Response::new(pb::ListRegionsResponse {
            regions: vec![
                region("fra", true, "enabled"),
                region("nyc", false, "enabled"),
                region("sgp", false, "available"),
            ],
            this_region: "fra".into(),
            ..Default::default()
        }))
    }

    async fn get_org_machine_defaults(
        &self,
        request: Request<pb::GetOrgMachineDefaultsRequest>,
    ) -> Result<Response<pb::GetOrgMachineDefaultsResponse>, Status> {
        self.authorize(&request)?;
        Ok(Response::new(pb::GetOrgMachineDefaultsResponse {
            region: "fra".into(),
            ..Default::default()
        }))
    }

    async fn create_vm(
        &self,
        request: Request<pb::CreateVmRequest>,
    ) -> Result<Response<pb::CreateVmResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        let egress = request
            .spec
            .as_ref()
            .map(|spec| spec.egress.clone())
            .unwrap_or_default();
        if egress.iter().any(|entry| entry.contains(':')) {
            return Err(Status::invalid_argument("IPv6 egress is not supported"));
        }
        let labels = spec_labels(&request.spec, &request.labels);
        let isolated = request.isolated || request.spec.as_ref().is_some_and(|s| s.isolated);
        let machine = self.new_machine(&request.name, labels, egress, isolated);
        self.lock().create_requests.push(request);
        Ok(Response::new(pb::CreateVmResponse {
            vm_id: machine.vm_id.clone(),
            name: machine.name.clone(),
            status: machine.status.clone(),
            machine: Some(machine),
            ..Default::default()
        }))
    }

    async fn create_vm_from_snapshot(
        &self,
        request: Request<pb::CreateVmFromSnapshotRequest>,
    ) -> Result<Response<pb::CreateVmResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        if !self.lock().snapshots.contains_key(&request.snapshot) {
            return Err(Status::not_found(format!("snapshot {}", request.snapshot)));
        }
        let egress = request
            .spec
            .as_ref()
            .map(|spec| spec.egress.clone())
            .unwrap_or_default();
        let labels = spec_labels(&request.spec, &request.labels);
        let machine = self.new_machine(&request.name, labels, egress, request.isolated);
        self.lock().snapshot_creates.push(request);
        Ok(Response::new(pb::CreateVmResponse {
            vm_id: machine.vm_id.clone(),
            name: machine.name.clone(),
            status: machine.status.clone(),
            machine: Some(machine),
            ..Default::default()
        }))
    }

    async fn get_vm(
        &self,
        request: Request<pb::GetVmRequest>,
    ) -> Result<Response<pb::GetVmResponse>, Status> {
        self.authorize(&request)?;
        let id = request.into_inner().vm_id;
        let mut state = self.lock();
        let machine = state
            .machines
            .get_mut(&id)
            .ok_or_else(|| Status::not_found(format!("machine {id}")))?;
        let current = machine.clone();
        if matches!(machine.status.as_str(), "starting" | "waking") {
            machine.status = "running".into();
        }
        Ok(Response::new(current))
    }

    async fn list_vms(
        &self,
        request: Request<pb::ListVmsRequest>,
    ) -> Result<Response<pb::ListVmsResponse>, Status> {
        self.authorize(&request)?;
        let selector = request.into_inner().label_selector;
        let vms = self
            .lock()
            .machines
            .values()
            .filter(|machine| {
                selector.iter().all(|pair| {
                    let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                    machine.labels.get(k).map(String::as_str) == Some(v)
                })
            })
            .cloned()
            .collect();
        Ok(Response::new(pb::ListVmsResponse {
            vms,
            ..Default::default()
        }))
    }

    async fn destroy_vm(
        &self,
        request: Request<pb::DestroyVmRequest>,
    ) -> Result<Response<pb::DestroyVmResponse>, Status> {
        self.authorize(&request)?;
        let id = request.into_inner().vm_id;
        let mut state = self.lock();
        if state.machines.remove(&id).is_none() {
            return Err(Status::not_found(format!("machine {id}")));
        }
        state.destroyed.push(id);
        Ok(Response::new(pb::DestroyVmResponse {}))
    }

    async fn hibernate_vm(
        &self,
        request: Request<pb::HibernateVmRequest>,
    ) -> Result<Response<pb::HibernateVmResponse>, Status> {
        self.authorize(&request)?;
        self.set_status(&request.into_inner().vm_id, "hibernated")?;
        Ok(Response::new(pb::HibernateVmResponse {}))
    }

    async fn wake_vm(
        &self,
        request: Request<pb::WakeVmRequest>,
    ) -> Result<Response<pb::WakeVmResponse>, Status> {
        self.authorize(&request)?;
        let id = request.into_inner().vm_id;
        if self.machine(&id)?.status == "stopped" {
            return Err(Status::failed_precondition("machine is stopped; start it"));
        }
        self.set_status(&id, "waking")?;
        Ok(Response::new(pb::WakeVmResponse {}))
    }

    async fn start_vm(
        &self,
        request: Request<pb::StartVmRequest>,
    ) -> Result<Response<pb::StartVmResponse>, Status> {
        self.authorize(&request)?;
        self.set_status(&request.into_inner().vm_id, "starting")?;
        Ok(Response::new(pb::StartVmResponse {}))
    }

    async fn set_vm_egress_allow(
        &self,
        request: Request<pb::SetVmEgressAllowRequest>,
    ) -> Result<Response<pb::SetVmEgressAllowResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        let mut state = self.lock();
        let machine = state
            .machines
            .get_mut(&request.vm)
            .ok_or_else(|| Status::not_found(format!("machine {}", request.vm)))?;
        machine.egress_allow = request.entries.clone();
        state
            .egress_calls
            .push((request.vm.clone(), request.entries.clone()));
        Ok(Response::new(pb::SetVmEgressAllowResponse {
            entries: request.entries,
        }))
    }

    async fn set_auto_hibernate_timeout(
        &self,
        request: Request<pb::SetAutoHibernateTimeoutRequest>,
    ) -> Result<Response<pb::SetAutoHibernateTimeoutResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        self.machine(&request.vm_id)?;
        self.lock()
            .auto_hibernate_calls
            .push((request.vm_id, request.timeout_secs));
        Ok(Response::new(pb::SetAutoHibernateTimeoutResponse {}))
    }

    async fn create_snapshot(
        &self,
        request: Request<pb::CreateSnapshotRequest>,
    ) -> Result<Response<pb::CreateSnapshotResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        if self.machine(&request.vm)?.status != "running" {
            return Err(Status::failed_precondition("machine must be running"));
        }
        self.lock()
            .snapshots
            .insert(request.name.clone(), request.vm.clone());
        Ok(Response::new(pb::CreateSnapshotResponse {
            snapshot_id: format!("snap_{}", request.name),
            name: request.name,
            version: 1,
            status: "pending".into(),
            ..Default::default()
        }))
    }

    async fn get_snapshot(
        &self,
        request: Request<pb::GetSnapshotRequest>,
    ) -> Result<Response<pb::GetSnapshotResponse>, Status> {
        self.authorize(&request)?;
        let name = request.into_inner().name;
        if !self.lock().snapshots.contains_key(&name) {
            return Err(Status::not_found(format!("snapshot {name}")));
        }
        Ok(Response::new(pb::GetSnapshotResponse {
            snapshot: Some(pb::SnapshotInfo {
                snapshot_id: format!("snap_{name}"),
                name,
                version: 1,
                status: "ready".into(),
                ..Default::default()
            }),
        }))
    }

    async fn exec(
        &self,
        request: Request<Streaming<pb::ExecChunk>>,
    ) -> Result<Response<Self::ExecStream>, Status> {
        self.authorize(&request)?;
        let mut incoming = request.into_inner();
        let first = incoming
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty exec stream"))??;
        let id = first.vm_id.clone();
        if self.machine(&id)?.status != "running" {
            return Err(Status::failed_precondition("machine is not running"));
        }
        let command = first.command.clone();
        let mut stdin = Vec::new();
        while let Some(chunk) = incoming.next().await {
            let chunk = chunk?;
            if chunk.stdin {
                stdin.extend_from_slice(&chunk.data);
            }
        }
        self.lock()
            .commands
            .entry(id.clone())
            .or_default()
            .push(command.clone());

        // A tiny interpreter over the command line the client sent.
        if command.contains("sleep") {
            // Never answers: the client's deadline has to cancel the call.
            let stream = futures::stream::pending();
            return Ok(Response::new(Box::pin(stream)));
        }
        let mut out = vec![exec_chunk(format!("ran: {command}\n").as_bytes(), false)];
        if command.contains("cat") && !stdin.is_empty() {
            out.push(exec_chunk(&stdin, false));
        }
        if command.contains(">&2") {
            out.push(exec_chunk(b"oops\n", true));
        }
        if command.contains("yes") {
            // More output than the client keeps.
            for _ in 0..9 {
                out.push(exec_chunk(&vec![b'y'; 1 << 20], false));
            }
        }
        let exit_code = command
            .split("exit ")
            .nth(1)
            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        out.push(pb::ExecChunk {
            exit_code,
            ..Default::default()
        });
        Ok(Response::new(Box::pin(futures::stream::iter(
            out.into_iter().map(Ok),
        ))))
    }

    async fn fs_read(
        &self,
        request: Request<pb::FsReadRequest>,
    ) -> Result<Response<Self::FsReadStream>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        let path = String::from_utf8(request.path).unwrap();
        let data = self
            .file(&request.vm_id, &path)
            .ok_or_else(|| Status::not_found(format!("{path}: no such file")))?;
        let entry = pb::FsEntry {
            path: path.clone().into_bytes(),
            size: data.len() as u64,
            r#type: pb::FsEntryType::File as i32,
            ..Default::default()
        };
        let mut chunks: Vec<pb::FsReadChunk> = data
            .chunks(64 * 1024)
            .map(|piece| pb::FsReadChunk {
                data: piece.to_vec(),
                ..Default::default()
            })
            .collect();
        if chunks.is_empty() {
            chunks.push(pb::FsReadChunk::default());
        }
        chunks[0].entry = Some(entry);
        let last = chunks.len() - 1;
        chunks[last].eof = true;
        chunks[last].bytes_read = data.len() as u64;
        Ok(Response::new(Box::pin(futures::stream::iter(
            chunks.into_iter().map(Ok),
        ))))
    }

    async fn fs_write(
        &self,
        request: Request<Streaming<pb::FsWriteChunk>>,
    ) -> Result<Response<pb::FsWriteResponse>, Status> {
        self.authorize(&request)?;
        let mut incoming = request.into_inner();
        let first = incoming
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty write stream"))??;
        let id = first.vm_id.clone();
        self.machine(&id)?;
        let path = String::from_utf8(first.path.clone()).unwrap();
        let mut data = first.data.clone();
        while let Some(chunk) = incoming.next().await {
            data.extend_from_slice(&chunk?.data);
        }
        if let Some(expected) = first.expected_size
            && expected != data.len() as u64
        {
            return Err(Status::data_loss("short write"));
        }
        let written = data.len() as u64;
        self.put_file(&id, &path, &data);
        Ok(Response::new(pb::FsWriteResponse {
            bytes_written: written,
            ..Default::default()
        }))
    }

    async fn fs_mkdir(
        &self,
        request: Request<pb::FsMkdirRequest>,
    ) -> Result<Response<pb::FsMkdirResponse>, Status> {
        self.authorize(&request)?;
        let request = request.into_inner();
        self.machine(&request.vm_id)?;
        self.lock()
            .dirs
            .entry(request.vm_id)
            .or_default()
            .insert(String::from_utf8(request.path).unwrap());
        Ok(Response::new(pb::FsMkdirResponse {}))
    }
}

/// `POST /auth/token`: the key exchange. The first JWT is revoked straight
/// away, so the client's refresh-on-UNAUTHENTICATED path always runs.
async fn exchange(
    axum::extract::State(mock): axum::extract::State<MockBoxd>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if body["api_key"] != API_KEY {
        return (axum::http::StatusCode::UNAUTHORIZED, "invalid api key").into_response();
    }
    let mut state = mock.lock();
    state.jwts_issued += 1;
    let token = format!("jwt-{}", state.jwts_issued);
    if state.jwts_issued > 1 {
        state.valid_jwts.insert(token.clone());
    }
    axum::Json(serde_json::json!({
        "token": token,
        "expires_at": chrono::Utc::now().timestamp() + 3_600,
    }))
    .into_response()
}

/// Start the mock on a loopback port; its URL serves both the key exchange
/// (`<url>/auth/token`) and the gRPC API.
pub async fn start_mock() -> (MockBoxd, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let mock = MockBoxd::default();
    mock.lock().url = url.clone();
    let grpc = tonic::service::Routes::new(pb::boxd_api_server::BoxdApiServer::new(mock.clone()))
        .into_axum_router();
    let app = grpc.merge(
        axum::Router::new()
            .route("/auth/token", axum::routing::post(exchange))
            .with_state(mock.clone()),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (mock, url)
}
