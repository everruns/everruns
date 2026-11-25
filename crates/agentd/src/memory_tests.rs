//! A session's Memory on a shard: its mounts, served by the control
//! plane's files server.

mod files_server {
    //! Memory through the control plane's files server: the same mount,
    //! each operation an MCP request with a session token, served by the
    //! core handler a control plane runs.

    fn file(
        session: SessionId,
        path: &str,
        content: &str,
    ) -> everruns_core::session_file::SessionFile {
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
                path.starts_with("/memory/user") || path.starts_with("/application/memory/"),
                "{path} is not in this session's Memory"
            );
            let key = (who, path.clone());
            let mut tree = self.0.tree.lock();
            let file = |content: &str| {
                json!(file(
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

    /// The control plane's endpoint, as the egress the shard's calls go out
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
    /// credentials.
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
            session,
            &super::super::SessionMemory {
                mounts: vec![
                    "/memory/user".into(),
                    "/memory/shared".into(),
                    "/application/memory".into(),
                    "/etc".into(),
                ],
                server: Some(URL.into()),
            },
        );
        // A path that is no Memory mount is never mounted.
        assert_eq!(backends.memory_mounts.len(), 3);
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
    async fn application_memory_routes_to_the_control_plane_not_the_workspace() {
        let (files, _, executor) = setup();
        let session = SessionId::new();
        let fs = mounted(&executor, session);
        let path = "/application/memory/handbook/brand.md";
        fs.write_file(session, path, "Use plain words", "text")
            .await
            .unwrap();
        assert_eq!(
            fs.read_file(session, path)
                .await
                .unwrap()
                .unwrap()
                .content
                .as_deref(),
            Some("Use plain words")
        );
        assert_eq!(files.asked.lock()[0].2["path"], path);
        assert_eq!(files.asked.lock()[1].1, "read");
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

    #[tokio::test]
    async fn a_memory_mount_serves_only_its_own_session() {
        let (files, _cp, executor) = setup();
        let session = SessionId::new();
        let fs = mounted(&executor, session);
        let error = fs
            .read_file(SessionId::new(), "/memory/user/notes.md")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("only its own Memory"), "{error}");
        assert!(files.asked.lock().is_empty());
    }

    #[test]
    fn no_files_server_means_no_memory() {
        let backends = super::super::attach(
            everruns_core::host::HostBackends::in_memory(),
            None,
            SessionId::new(),
            &super::super::SessionMemory {
                mounts: vec!["/memory/user".into()],
                server: None,
            },
        );
        assert!(backends.memory_mounts.is_empty());
    }
}
