//! The workspace API end to end: the gateway's `/fs` routes over a
//! one-node shard served on a real socket.

use super::*;

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{Method, Request, header};
use everruns_contracts::typed_id::WorkspaceId;
use tower::ServiceExt as _;

use crate::cluster::{Topology, TopologySource};
use crate::gateway::Gateway;
use crate::node::NodeConfig;
use crate::store::Store;

struct Shard {
    app: Router,
    gateway: Arc<Gateway>,
    base: String,
    session: SessionId,
    _node: Arc<Node>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

async fn shard() -> Shard {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let data = tempfile::tempdir().unwrap();
    let cfg = NodeConfig {
        shard: 0,
        node_id: 1,
        members: BTreeMap::from([(1, addr.clone())]),
        data_dir: data.path().to_path_buf(),
        store: Store::open("memory://").unwrap(),
        memory_budget: 1 << 30,
        memory_limit: 0,
        raft_port_offset: 0,
        unload_idle: None,
        claim_ttl: Duration::from_secs(10),
        max_turns_per_node: 64,
        pressure_turns: 1024,
        snapshot_every: 100_000,
        keep_log: 256,
        executors: Default::default(),
        control_plane: None,
    };
    let node = Node::start(
        cfg,
        Arc::new(
            crate::StandardHost::new()
                .provider("llmsim", |_| Some(everruns::Model::simulated("done."))),
        ),
    )
    .await
    .unwrap();
    let served = crate::api::router(node.clone()).merge(router(node.clone()));
    tokio::spawn(async move { axum::serve(listener, served).await });
    node.bootstrap().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !node.is_leader() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let topology = tempfile::tempdir().unwrap();
    let path = topology.path().join("topology.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&Topology {
            directory_shard: 0,
            shards: BTreeMap::from([(0, BTreeMap::from([(1, addr)]))]),
        })
        .unwrap(),
    )
    .unwrap();
    let gateway = Gateway::new(TopologySource { path });
    let session = SessionId::new();
    gateway
        .create(
            json!({
                "name": "files",
                "instructions": "Keep files.",
                "model": { "provider": "llmsim", "model": "files" },
            }),
            Some(0),
            Some(session),
            None,
        )
        .await
        .unwrap();
    let workspace = WorkspaceId::from_uuid(session.uuid());
    Shard {
        app: crate::gateway::router(gateway.clone()),
        gateway,
        base: format!("/v1/workspaces/{workspace}/fs"),
        session,
        _node: node,
        _dirs: (data, topology),
    }
}

impl Shard {
    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMapAndBody) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, HeaderMapAndBody { headers, body })
    }

    async fn json(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base));
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let (status, reply) = self.send(request.body(body).unwrap()).await;
        (
            status,
            serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
        )
    }

    async fn upload(&self, path: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(Method::PUT)
            .uri(format!("{}/_/upload/{path}", self.base))
            .body(Body::from(bytes))
            .unwrap();
        let (status, reply) = self.send(request).await;
        (
            status,
            serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
        )
    }

    async fn download(&self, path: &str, range: Option<&str>) -> (StatusCode, HeaderMapAndBody) {
        let mut request = Request::builder().uri(format!("{}/_/download/{path}", self.base));
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }
}

struct HeaderMapAndBody {
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

fn names(listing: &Value) -> Vec<String> {
    listing["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn files_round_trip_through_the_gateway() {
    let shard = shard().await;

    // A binary upload is stored base64 and downloads byte for byte.
    let png: Vec<u8> = (0..=255u8).cycle().take(40_000).collect();
    let (status, uploaded) = shard.upload("img/logo.png", png.clone()).await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    assert_eq!(uploaded["size_bytes"], 40_000);
    let (status, whole) = shard.download("img/logo.png", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(whole.body, png);
    assert_eq!(whole.headers[header::CONTENT_TYPE], "image/png");
    assert!(
        whole.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    let (status, part) = shard.download("img/logo.png", Some("bytes=10-19")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, png[10..20]);
    assert_eq!(part.headers[header::CONTENT_RANGE], "bytes 10-19/40000");

    // Create, then a second create of the same path conflicts.
    let note = json!({ "content": "hello\nworld\n" });
    let (status, created) = shard
        .json(Method::POST, "/notes/a.md", Some(note.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["content"], "hello\nworld\n");
    let (status, _) = shard.json(Method::POST, "/notes/a.md", Some(note)).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Listings: the root, a directory, and everything.
    let (_, root) = shard.json(Method::GET, "", None).await;
    assert_eq!(names(&root), vec!["/img", "/notes"]);
    let (_, notes) = shard.json(Method::GET, "/notes", None).await;
    assert_eq!(names(&notes), vec!["/notes/a.md"]);
    let (_, all) = shard.json(Method::GET, "?recursive=true", None).await;
    assert_eq!(names(&all).len(), 4);

    // An update based on stale content conflicts; a current one applies.
    let stale = json!({ "content": "x", "expected_content": "old" });
    let (status, _) = shard.json(Method::PUT, "/notes/a.md", Some(stale)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let current = json!({ "content": "hello again\n", "expected_content": "hello\nworld\n" });
    let (status, updated) = shard.json(Method::PUT, "/notes/a.md", Some(current)).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["content"], "hello again\n");

    // Grep, stat, move, copy.
    let (_, found) = shard
        .json(Method::POST, "/_/grep", Some(json!({ "pattern": "again" })))
        .await;
    assert_eq!(found["data"][0]["path"], "/notes/a.md");
    assert_eq!(found["data"][0]["matches"][0]["line_number"], 1);
    let (status, moved) = shard
        .json(
            Method::POST,
            "/_/move",
            Some(json!({ "src_path": "/notes/a.md", "dst_path": "/notes/b.md" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let (status, _) = shard
        .json(
            Method::POST,
            "/_/stat",
            Some(json!({ "path": "/notes/a.md" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, copied) = shard
        .json(
            Method::POST,
            "/_/copy",
            Some(json!({ "src_path": "/img/logo.png", "dst_path": "/img/copy.png" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{copied}");
    let (_, copy) = shard.download("img/copy.png", None).await;
    assert_eq!(copy.body, png);

    // A read-only file refuses changes and deletion until unmarked.
    let mark = json!({ "is_readonly": true });
    let (status, _) = shard.json(Method::PUT, "/notes/b.md", Some(mark)).await;
    assert_eq!(status, StatusCode::OK);
    let write = json!({ "content": "no" });
    let (status, _) = shard.json(Method::PUT, "/notes/b.md", Some(write)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = shard.json(Method::DELETE, "/notes/b.md", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let unmark = json!({ "is_readonly": false });
    shard.json(Method::PUT, "/notes/b.md", Some(unmark)).await;
    let (status, deleted) = shard.json(Method::DELETE, "/notes/b.md", None).await;
    assert_eq!(
        (status, deleted),
        (StatusCode::OK, json!({ "deleted": true }))
    );

    // `_` is reserved for actions; a directory needs recursive to go.
    let (status, _) = shard
        .json(Method::POST, "/_/x.md", Some(json!({ "content": "x" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = shard.json(Method::DELETE, "/img", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = shard
        .json(Method::DELETE, "/img?recursive=true", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, root) = shard.json(Method::GET, "", None).await;
    assert_eq!(names(&root), vec!["/notes"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_alias_and_the_preview_sandbox() {
    let shard = shard().await;
    let html = "<h1>report</h1>";
    let (status, _) = shard.upload("out/report.html", html.into()).await;
    assert_eq!(status, StatusCode::OK);
    // The older `/v1/sessions/{id}/fs` path reaches the same workspace.
    let request = Request::builder()
        .uri(format!("/v1/sessions/{}/fs/out/report.html", shard.session))
        .body(Body::empty())
        .unwrap();
    let (status, reply) = shard.send(request).await;
    assert_eq!(status, StatusCode::OK);
    let file: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(file["content"], html);
    assert_eq!(file["encoding"], "text");
    let request = Request::builder()
        .uri(format!("{}/_/preview/out/report.html", shard.base))
        .body(Body::empty())
        .unwrap();
    let (status, preview) = shard.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview.body, html.as_bytes());
    let csp = preview.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("sandbox allow-scripts"));
    assert!(!csp.contains("allow-same-origin"));
    let request = Request::builder()
        .uri(format!("{}/_/preview/out/notes.md", shard.base))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        shard.send(request).await.0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    // A missing file is a 404, not a missing session.
    let (status, _) = shard.json(Method::GET, "/nope.md", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn starter_files_are_written_only_where_absent() {
    let shard = shard().await;
    shard
        .json(
            Method::POST,
            "/brief.md",
            Some(json!({ "content": "changed" })),
        )
        .await;
    let seed = |path: &str, content: &str, readonly: bool| InitialFile {
        path: path.into(),
        content: content.into(),
        encoding: "text".into(),
        is_readonly: readonly,
    };
    let op = FsOp::Seed {
        files: vec![
            seed("/brief.md", "original", false),
            seed("/workspace/rules.md", "be kind", true),
        ],
    };
    let reply = shard
        .gateway
        .session_command(shard.session, "fs", &serde_json::to_value(op).unwrap())
        .await
        .unwrap();
    assert_eq!(reply["value"]["seeded"], 2, "{reply}");
    let (_, brief) = shard.json(Method::GET, "/brief.md", None).await;
    assert_eq!(brief["content"], "changed", "an existing file stays");
    let (_, rules) = shard.json(Method::GET, "/rules.md", None).await;
    assert_eq!(rules["content"], "be kind");
    assert_eq!(rules["is_readonly"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_session_is_not_found() {
    let shard = shard().await;
    let other = WorkspaceId::from_uuid(SessionId::new().uuid());
    let request = Request::builder()
        .uri(format!("/v1/workspaces/{other}/fs"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = shard.send(request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn refusals_map_to_the_control_planes_statuses() {
    assert_eq!(status_of("Source not found: /a"), StatusCode::NOT_FOUND);
    assert_eq!(
        status_of("File already exists at path: /a"),
        StatusCode::CONFLICT
    );
    assert_eq!(status_of("file is read-only: /a"), StatusCode::FORBIDDEN);
    assert_eq!(
        status_of("the workspace is full"),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(status_of("not a directory: /a"), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn monitoring_previews_read_committed_bounded_windows_with_unicode() {
    let shard = shard().await;
    shard
        .gateway
        .admit(shard.session, "世🙂".repeat(10000), None)
        .await
        .unwrap();
    let previews = crate::event_previews::read(&shard.gateway, shard.session, 1, 32, 53)
        .await
        .unwrap();
    assert!(
        previews
            .iter()
            .any(|event| event.event_type == "input.message")
    );
    assert!(previews.iter().any(|event| event.truncated));
    assert!(
        previews
            .iter()
            .all(|event| event.text.len() <= 53 && event.sequence <= 32)
    );
    assert!(
        crate::event_previews::read(&shard.gateway, shard.session, 1, 65, 53)
            .await
            .is_err()
    );
    assert!(
        crate::event_previews::read(&shard.gateway, shard.session, 1, 32, 2049)
            .await
            .is_err()
    );
    let late = crate::event_previews::read(&shard.gateway, shard.session, 10000, 10031, 53)
        .await
        .unwrap();
    assert!(late.is_empty());
}
