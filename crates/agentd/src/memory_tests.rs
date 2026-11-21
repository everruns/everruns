use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use everruns_contracts::dataplane::Connection;
use everruns_contracts::host_calls::{HostCallAnswer, MEMORY_FILE};
use everruns_contracts::runtime::mount_fs::MountFs;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::InMemorySessionFileStore;
use everruns_core::session_file::{FileInfo, SessionFile};
use everruns_core::session_files::SessionFileSystem;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::providers::OrgMaterial;

/// A control plane whose Memory is a map of paths to contents; every call
/// is recorded.
#[derive(Default)]
struct ControlPlane {
    files: Mutex<BTreeMap<String, String>>,
    asked: Mutex<Vec<Value>>,
}

fn file(session: SessionId, path: &str, content: &str) -> SessionFile {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4(),
        "session_id": session.uuid(),
        "path": path,
        "name": path.rsplit('/').next().unwrap_or_default(),
        "content": content,
        "encoding": "text",
        "is_directory": false,
        "is_readonly": false,
        "size_bytes": content.len(),
        "created_at": "2026-11-19T00:00:00Z",
        "updated_at": "2026-11-19T00:00:00Z",
    }))
    .unwrap()
}

fn info(session: SessionId, path: &str) -> FileInfo {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4(),
        "session_id": session.uuid(),
        "path": path,
        "name": path.rsplit('/').next().unwrap_or_default(),
        "is_directory": false,
        "is_readonly": false,
        "size_bytes": 1,
        "created_at": "2026-11-19T00:00:00Z",
        "updated_at": "2026-11-19T00:00:00Z",
    }))
    .unwrap()
}

#[async_trait::async_trait]
impl OrgMaterial for ControlPlane {
    fn tenant(&self, _session: SessionId) -> Option<String> {
        Some("org-a".into())
    }

    async fn connection(&self, _org: &str, _id: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }

    async fn secret(&self, _org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }

    fn looks_up(&self) -> bool {
        true
    }

    fn declared_host_calls(
        &self,
        _session: SessionId,
    ) -> Option<std::collections::BTreeSet<&'static str>> {
        Some([MEMORY_FILE].into())
    }

    async fn lookup(
        &self,
        _org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<HostCallAnswer> {
        assert_eq!(command, MEMORY_FILE);
        self.asked.lock().push(params.clone());
        let session = SessionId::parse(params["session_id"].as_str().unwrap()).unwrap();
        let path = params["path"].as_str().unwrap().to_owned();
        let mut files = self.files.lock();
        let ok = match params["op"].as_str().unwrap() {
            "write" => {
                let content = params["content"].as_str().unwrap().to_owned();
                files.insert(path.clone(), content.clone());
                json!(file(session, &path, &content))
            }
            "read" => match files.get(&path) {
                Some(content) => json!(file(session, &path, content)),
                None => Value::Null,
            },
            "list" => json!(
                files
                    .keys()
                    .filter(|p| p.starts_with(&format!("{path}/")))
                    .map(|p| info(session, p))
                    .collect::<Vec<_>>()
            ),
            "delete" => json!(files.remove(&path).is_some()),
            other => {
                return Ok(HostCallAnswer::Value(
                    json!({ "error": format!("no {other}") }),
                ));
            }
        };
        Ok(HostCallAnswer::Value(json!({ "ok": ok })))
    }
}

fn mounted(cp: &Arc<ControlPlane>, session: SessionId) -> MountFs {
    let material: Arc<dyn OrgMaterial> = cp.clone();
    let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
    // Kept alive by the test's `cp`.
    std::mem::forget(material);
    let lookups = crate::lookups::org_lookups(Some(&weak), session);
    let backends = super::attach(
        everruns_core::host::HostBackends::in_memory(),
        Some(&weak),
        lookups,
        session,
        &super::Memory {
            mounts: vec![
                "/memory/user".into(),
                "/memory/shared".into(),
                "/etc".into(),
            ],
            server: None,
        },
    );
    // A path that is no Memory mount is never mounted.
    assert_eq!(backends.memory_mounts.len(), 2);
    let mut fs = MountFs::new(Arc::new(InMemorySessionFileStore::new()));
    for (point, backend, scope) in backends.memory_mounts {
        fs = fs.with_memory_mount(point, backend, scope);
    }
    fs
}

#[tokio::test]
async fn memory_files_live_in_the_control_plane_under_their_mount() {
    let cp = Arc::new(ControlPlane::default());
    let session = SessionId::new();
    let fs = mounted(&cp, session);

    let written = fs
        .write_file(session, "/memory/user/notes.md", "likes tea", "text")
        .await
        .unwrap();
    assert_eq!(written.path, "/memory/user/notes.md");
    // The control plane got its own path, and keeps the file.
    assert_eq!(cp.asked.lock()[0]["path"], json!("/memory/user/notes.md"));
    assert_eq!(cp.asked.lock()[0]["session_id"], json!(session.to_string()));
    assert_eq!(
        cp.files
            .lock()
            .get("/memory/user/notes.md")
            .map(String::as_str),
        Some("likes tea")
    );

    // Another session's write shows at once: nothing is cached.
    cp.files
        .lock()
        .insert("/memory/shared/team.md".into(), "ship on Fridays".into());
    let read = fs
        .read_file(session, "/memory/shared/team.md")
        .await
        .unwrap()
        .expect("written by another session");
    assert_eq!(read.content.as_deref(), Some("ship on Fridays"));
    assert_eq!(read.path, "/memory/shared/team.md");

    let listed = fs.list_directory(session, "/memory/user").await.unwrap();
    let paths: Vec<_> = listed.iter().map(|info| info.path.as_str()).collect();
    assert_eq!(paths, vec!["/memory/user/notes.md"]);

    // The workspace stays the session's own: no call for it.
    let asked = cp.asked.lock().len();
    fs.write_file(session, "/workspace/draft.md", "x", "text")
        .await
        .unwrap();
    assert!(fs.read_file(session, "/memory/agent/x.md").await.is_ok());
    assert_eq!(cp.asked.lock().len(), asked);

    // The control plane's refusal is the tool's error.
    let error = fs
        .create_directory(session, "/memory/user/dir")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no mkdir"), "{error}");
}

#[tokio::test]
async fn a_memory_mount_serves_only_its_own_session() {
    let cp = Arc::new(ControlPlane::default());
    let session = SessionId::new();
    let fs = mounted(&cp, session);
    let error = fs
        .read_file(SessionId::new(), "/memory/user/notes.md")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("only its own Memory"), "{error}");
    assert!(cp.asked.lock().is_empty());
}

mod files_server {
    //! Memory through the control plane's files server: the same mount,
    //! each operation an MCP request with a session token, served by the
    //! core handler a control plane runs.

    use std::collections::BTreeMap;
    use std::sync::{Arc, OnceLock, Weak};

    use anyhow::Result;
    use async_trait::async_trait;
    use everruns_contracts::dataplane::Connection;
    use everruns_contracts::dataplane_seal::Key;
    use everruns_contracts::runtime::capabilities::injected::{
        InjectedCapabilityDefinition, MEMORY_FILES, injected_capability_id,
    };
    use everruns_contracts::runtime::mount_fs::MountFs;
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::host::InMemorySessionFileStore;
    use everruns_core::mcp::injected_server::{
        InjectedCapabilityService, VerifiedSession, handle_injected_request,
    };
    use everruns_core::mcp::session_token::{SessionTokenSigner, SessionTokenVerifier};
    use everruns_core::session_files::SessionFileSystem;
    use everruns_core::tool_context::ToolContext;
    use everruns_core::tools::Tool;
    use everruns_core::{
        EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    };
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    use crate::mcp::OrgMcp;
    use crate::providers::OrgMaterial;

    // A public address literal: the client checks it without DNS, and the
    // egress below answers without connecting.
    const URL: &str = "https://93.184.215.14/api/v1/injected/memory_files/mcp";
    const KEY: Key = [9u8; 32];
    const ORG: &str = "org-a";

    /// The control plane's Memory: (session, path) to content, and every
    /// operation it was asked for.
    #[derive(Default)]
    struct Files {
        tree: Mutex<BTreeMap<(String, String), String>>,
        asked: Mutex<Vec<(String, String, Value)>>,
    }

    fn definition() -> InjectedCapabilityDefinition {
        serde_json::from_value(json!({
            "name": MEMORY_FILES,
            "description": "Memory",
            "server": { "url": URL },
        }))
        .unwrap()
    }

    struct Service(Arc<Files>, InjectedCapabilityDefinition);

    #[async_trait]
    impl InjectedCapabilityService for Service {
        fn definition(&self) -> &InjectedCapabilityDefinition {
            &self.1
        }
        async fn tools(&self, _: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
            Ok(Vec::new())
        }
        async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
            Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
        }
        fn serves_files(&self) -> bool {
            true
        }
        async fn files(
            &self,
            session: &VerifiedSession,
            op: &'static str,
            params: Value,
        ) -> Result<Value> {
            let who = format!(
                "{}@{}",
                session.session_id,
                session.claims.grp.clone().unwrap()
            );
            self.0
                .asked
                .lock()
                .push((who.clone(), op.to_string(), params.clone()));
            let path = params["path"].as_str().unwrap().to_string();
            anyhow::ensure!(
                path.starts_with("/memory/user"),
                "{path} is not in this session's Memory"
            );
            let key = (who, path.clone());
            let mut tree = self.0.tree.lock();
            let file = |content: &str| {
                json!(super::file(
                    SessionId::from_uuid(session.session_id),
                    &path,
                    content
                ))
            };
            Ok(match op {
                "read" => tree.get(&key).map(|c| file(c)).unwrap_or(Value::Null),
                "write" => {
                    let content = params["content"].as_str().unwrap().to_string();
                    let out = file(&content);
                    tree.insert(key, content);
                    out
                }
                "write_if_matches" => {
                    if tree.get(&key).map(String::as_str) != params["expected_content"].as_str() {
                        return Ok(Value::Null);
                    }
                    let content = params["content"].as_str().unwrap().to_string();
                    let out = file(&content);
                    tree.insert(key, content);
                    out
                }
                other => anyhow::bail!("no {other}"),
            })
        }
    }

    /// The control plane's endpoint, as the egress the cell's calls go out
    /// through: a token that does not verify is a 401.
    struct ControlPlane {
        service: Service,
        verifier: SessionTokenVerifier,
        bearers: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl EgressService for ControlPlane {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            assert_eq!(request.url, URL);
            let bearer = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .and_then(|(_, value)| value.strip_prefix("Bearer "))
                .unwrap_or_default()
                .to_string();
            self.bearers.lock().push(bearer.clone());
            let refused = EgressResponse {
                status: 401,
                headers: BTreeMap::new(),
                body: Vec::new(),
            };
            let Ok(claims) =
                self.verifier
                    .verify(&bearer, URL, &injected_capability_id(MEMORY_FILES))
            else {
                return Ok(refused);
            };
            let session = VerifiedSession::from_claims(claims).unwrap();
            let message: Value = serde_json::from_slice(&request.body).unwrap();
            Ok(
                match handle_injected_request(&self.service, &session, message).await {
                    Some(reply) => EgressResponse {
                        status: 200,
                        headers: [("content-type".to_string(), "application/json".to_string())]
                            .into(),
                        body: serde_json::to_vec(&reply).unwrap(),
                    },
                    None => EgressResponse {
                        status: 202,
                        headers: BTreeMap::new(),
                        body: Vec::new(),
                    },
                },
            )
        }

        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter([Ok(response.body)])),
            })
        }
    }

    /// An executor: its sessions' org, its data plane key, its MCP
    /// credentials. Makes no host calls.
    struct Executor {
        sessions: Mutex<std::collections::HashMap<SessionId, String>>,
        mcp: OnceLock<Arc<OrgMcp>>,
    }

    #[async_trait]
    impl OrgMaterial for Executor {
        fn tenant(&self, session: SessionId) -> Option<String> {
            self.sessions.lock().get(&session).cloned()
        }
        async fn connection(&self, _: &str, _: &str) -> anyhow::Result<Option<Connection>> {
            Ok(None)
        }
        async fn secret(&self, _: &str, _: &str) -> anyhow::Result<Vec<u8>> {
            anyhow::bail!("no secrets")
        }
        fn data_plane_key(&self) -> Option<Key> {
            Some(KEY)
        }
        fn mcp(&self) -> Option<Arc<OrgMcp>> {
            self.mcp.get().cloned()
        }
        async fn lookup(
            &self,
            _: &str,
            command: &str,
            _: Value,
        ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
            panic!("Memory made host call {command}")
        }
    }

    fn setup() -> (Arc<Files>, Arc<ControlPlane>, Arc<Executor>) {
        let files = Arc::new(Files::default());
        let signer = SessionTokenSigner::from_pkcs8(
            everruns_contracts::dataplane_seal::SESSION_TOKEN_ISSUER,
            &everruns_contracts::dataplane_seal::session_token_pkcs8(&KEY),
        )
        .unwrap();
        let cp = Arc::new(ControlPlane {
            service: Service(files.clone(), definition()),
            verifier: SessionTokenVerifier::from_jwks(&signer.jwks(), vec![signer.issuer().into()])
                .unwrap(),
            bearers: Mutex::default(),
        });
        let executor = Arc::new(Executor {
            sessions: Mutex::default(),
            mcp: OnceLock::new(),
        });
        let mcp = Arc::new(OrgMcp::new(cp.clone()));
        let material: Arc<dyn OrgMaterial> = executor.clone();
        mcp.bind(Arc::downgrade(&material));
        let _ = executor.mcp.set(mcp);
        (files, cp, executor)
    }

    fn mounted(executor: &Arc<Executor>, session: SessionId) -> MountFs {
        executor.sessions.lock().insert(session, ORG.into());
        let material: Arc<dyn OrgMaterial> = executor.clone();
        let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
        let backends = super::super::attach(
            everruns_core::host::HostBackends::in_memory(),
            Some(&weak),
            None,
            session,
            &super::super::Memory {
                mounts: vec!["/memory/user".into(), "/memory/shared".into()],
                server: Some(URL.into()),
            },
        );
        assert_eq!(backends.memory_mounts.len(), 2);
        let mut fs = MountFs::new(Arc::new(InMemorySessionFileStore::new()));
        for (point, backend, scope) in backends.memory_mounts {
            fs = fs.with_memory_mount(point, backend, scope);
        }
        fs
    }

    #[tokio::test]
    async fn memory_reaches_the_control_planes_files_server_with_a_session_token() {
        let (files, cp, executor) = setup();
        let session = SessionId::new();
        let fs = mounted(&executor, session);

        let written = fs
            .write_file(session, "/memory/user/notes.md", "likes tea", "text")
            .await
            .unwrap();
        assert_eq!(written.path, "/memory/user/notes.md");
        let read = fs
            .read_file(session, "/memory/user/notes.md")
            .await
            .unwrap()
            .expect("written");
        assert_eq!(read.content.as_deref(), Some("likes tea"));
        // A compare-and-set reaches the server as one.
        let swapped = fs
            .write_file_if_content_matches(
                session,
                "/memory/user/notes.md",
                "likes tea",
                "text",
                "likes coffee",
                "text",
            )
            .await
            .unwrap();
        assert!(swapped.is_some());
        let stale = fs
            .write_file_if_content_matches(
                session,
                "/memory/user/notes.md",
                "likes tea",
                "text",
                "likes water",
                "text",
            )
            .await
            .unwrap();
        assert!(stale.is_none());

        // The server saw its own paths, for this session in its org.
        let asked = files.asked.lock().clone();
        let who = format!("{}@{ORG}", session.uuid());
        assert!(asked.iter().all(|(by, _, _)| *by == who), "{asked:?}");
        assert_eq!(asked[0].1, "write");
        assert_eq!(asked[0].2["path"], "/memory/user/notes.md");
        assert!(cp.bearers.lock().iter().all(|bearer| !bearer.is_empty()));

        // The server's refusal is the tool's error.
        let error = fs
            .read_file(session, "/memory/shared/team.md")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("not in this session's Memory"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_session_this_node_does_not_run_gets_no_token_and_no_answer() {
        let (files, _cp, executor) = setup();
        let session = SessionId::new();
        let fs = mounted(&executor, session);
        // The session left this node: no org, so no token is signed.
        executor.sessions.lock().clear();
        assert!(
            fs.read_file(session, "/memory/user/notes.md")
                .await
                .is_err()
        );
        assert!(files.asked.lock().is_empty());
    }
}
