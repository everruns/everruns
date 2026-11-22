//! A session's SQL databases: the index's rules, then the tools' store and
//! the Databases API over a one-node shard served on a real socket.

use super::*;

use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, header};
use tower::ServiceExt as _;

use crate::cluster::{Topology, TopologySource};
use crate::node::NodeConfig;
use crate::state::Command;
use crate::store::Store;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn put(name: &str, sha256: &str, size: u64, expected: Option<&str>) -> SqlChange {
    SqlChange::Put {
        name: name.into(),
        sha256: sha256.into(),
        size,
        expected: expected.map(str::to_owned),
    }
}

#[test]
fn a_write_commits_only_over_the_file_it_started_from() {
    let mut dbs = Databases::default();
    assert_eq!(
        dbs.commit("1".into(), put("notes", SHA_A, 10, None), 1)["changed"],
        true
    );
    // A second writer that also started from "no database" loses.
    let lost = dbs.commit("2".into(), put("notes", SHA_B, 10, None), 2);
    assert_eq!(lost["refused"], CONFLICT, "{lost}");
    // So does one that started from a file the index no longer names.
    let stale = dbs.commit("3".into(), put("notes", SHA_B, 10, Some(SHA_B)), 3);
    assert_eq!(stale["refused"], CONFLICT, "{stale}");
    let next = dbs.commit("4".into(), put("notes", SHA_B, 20, Some(SHA_A)), 4);
    assert_eq!(next["changed"], true, "{next}");
    let notes = &dbs.entries["notes"];
    assert_eq!((notes.sha256.as_str(), notes.size), (SHA_B, 20));
    assert_eq!((notes.created_ms, notes.updated_ms), (1, 4));
    // A retry of a commit answers what the first one did, changing nothing.
    assert_eq!(
        dbs.commit("2".into(), put("notes", SHA_A, 1, Some(SHA_B)), 5),
        lost
    );
    assert_eq!(dbs.entries["notes"].sha256, SHA_B);
}

#[test]
fn the_index_holds_the_session_to_its_limits() {
    let mut dbs = Databases::default();
    for i in 0..MAX_DATABASES_PER_SESSION {
        let reply = dbs.commit(format!("{i}"), put(&format!("db_{i}"), SHA_A, 0, None), 1);
        assert_eq!(reply["changed"], true, "{reply}");
    }
    let one_more = dbs.commit("x".into(), put("db_x", SHA_A, 0, None), 1);
    assert_eq!(one_more["refused"], LIMIT, "{one_more}");

    let mut dbs = Databases::default();
    let half = MAX_SESSION_TOTAL_SIZE_BYTES as u64 / 2;
    dbs.commit("a".into(), put("a", SHA_A, half, None), 1);
    dbs.commit("b".into(), put("b", SHA_A, half, None), 1);
    // Growing one past what the other leaves is refused; staying is fine.
    let grown = dbs.commit("c".into(), put("a", SHA_B, half + 1, Some(SHA_A)), 2);
    assert_eq!(grown["refused"], LIMIT, "{grown}");
    let same = dbs.commit("d".into(), put("a", SHA_B, half, Some(SHA_A)), 2);
    assert_eq!(same["changed"], true, "{same}");

    for (name, sha) in [("../escape", SHA_A), ("ok", "not-a-hash")] {
        let reply = dbs.commit(name.into(), put(name, sha, 0, None), 3);
        assert_eq!(reply["refused"], INVALID, "{reply}");
    }
    assert_eq!(
        dbs.commit("e".into(), SqlChange::Delete { name: "a".into() }, 4)["changed"],
        true
    );
    assert_eq!(
        dbs.commit("f".into(), SqlChange::Delete { name: "a".into() }, 4)["changed"],
        false
    );
}

struct Shard {
    app: Router,
    node: Arc<Node>,
    session: SessionId,
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
                "name": "databases",
                "instructions": "Keep notes in SQL.",
                "model": { "provider": "llmsim", "model": "databases" },
            }),
            Some(0),
            Some(session),
            None,
        )
        .await
        .unwrap();
    Shard {
        app: crate::gateway::router(gateway),
        node,
        session,
        _dirs: (data, topology),
    }
}

impl Shard {
    async fn api(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/v1/sessions/{}/databases{path}", self.session));
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn index(&self) -> BTreeMap<String, Database> {
        self.node.shared.state.read().sessions[&self.session]
            .sqldb
            .entries
            .clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_databases_live_in_its_shard_and_the_api_reads_them_there() {
    let shard = shard().await;
    let session = shard.session;
    let tools = shard.node.hosting.databases();

    tools
        .sql_execute(session, "notes", "CREATE TABLE t (n INTEGER, body TEXT)")
        .await
        .unwrap();
    let written = tools
        .sql_execute(session, "notes", "INSERT INTO t VALUES (1, 'a'), (2, 'b')")
        .await
        .unwrap();
    assert_eq!(written.rows_affected, 2);
    let rows = tools
        .sql_query(session, "notes", "SELECT body FROM t ORDER BY n")
        .await
        .unwrap();
    assert_eq!(rows.rows, vec![vec![json!("a")], vec![json!("b")]]);

    // The index names the file, and the object store holds it.
    let notes = shard.index()["notes"].clone();
    let bytes = shard
        .node
        .shared
        .store
        .get_blob(&notes.sha256)
        .await
        .unwrap();
    assert_eq!(bytes.len() as u64, notes.size);
    assert!(bytes.starts_with(b"SQLite format 3\0"));

    // The Databases API answers from the shard, in the control plane's shapes.
    let (status, listed) = shard.api(Method::GET, "", None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"][0]["name"], "notes");
    assert_eq!(listed["data"][0]["size_bytes"], notes.size);
    let (status, schema) = shard.api(Method::GET, "/notes/schema", None).await;
    assert_eq!(status, StatusCode::OK, "{schema}");
    assert_eq!(schema["database"], "notes");
    assert_eq!(schema["tables"][0]["name"], "t");
    assert_eq!(schema["tables"][0]["row_count"], 2);

    let (status, created) = shard
        .api(Method::POST, "", Some(json!({ "name": "scratch" })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "scratch");
    let (status, _) = shard
        .api(Method::POST, "", Some(json!({ "name": "scratch" })))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = shard
        .api(Method::POST, "", Some(json!({ "name": "../escape" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, got) = shard.api(Method::GET, "/scratch", None).await;
    assert_eq!(
        (status, got["size_bytes"].clone()),
        (StatusCode::OK, json!(0))
    );
    // The agent writes into the database a person made.
    tools
        .sql_execute(session, "scratch", "CREATE TABLE s (v)")
        .await
        .unwrap();
    let (status, _) = shard.api(Method::DELETE, "/scratch", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = shard.api(Method::GET, "/scratch", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = shard.api(Method::DELETE, "/scratch", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = shard.api(Method::GET, "/scratch/schema", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A deleted database is gone for the tools too, kind kept.
    let error = tools
        .sql_query(session, "scratch", "SELECT 1")
        .await
        .unwrap_err();
    assert!(
        matches!(error, SessionSqlDbError::DatabaseNotFound(_)),
        "{error:?}"
    );
    assert_eq!(shard.index().len(), 1);

    // Another session's databases are its own.
    let (status, other) = shard.api(Method::GET, "", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other["data"].as_array().unwrap().len(), 1);
    assert!(
        tools.list_databases(SessionId::new()).await.is_err(),
        "an unknown session has no databases to list"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_working_copy_follows_the_index_not_its_own_file() {
    let shard = shard().await;
    let session = shard.session;
    let tools = shard.node.hosting.databases();
    tools
        .sql_execute(session, "notes", "CREATE TABLE t (n INTEGER)")
        .await
        .unwrap();
    let empty_db = shard.index()["notes"].clone();
    let empty = empty_db.sha256.clone();
    tools
        .sql_execute(session, "notes", "INSERT INTO t VALUES (1)")
        .await
        .unwrap();
    let one_row = shard.index()["notes"].sha256.clone();
    assert_ne!(empty, one_row, "a write is a new file");

    // A read changes nothing.
    tools
        .sql_query(session, "notes", "SELECT count(*) FROM t")
        .await
        .unwrap();
    assert_eq!(shard.index()["notes"].sha256, one_row);

    // Another writer (a restore, another executor) points the session at
    // the earlier file: this copy reads that, not what it last wrote.
    let (reply, _) = shard
        .node
        .propose(Command::SqlDb {
            session,
            fence: None,
            op_id: "revert".into(),
            change: put("notes", &empty, empty_db.size, Some(&one_row)),
            now_ms: crate::node::now_ms(),
        })
        .await
        .unwrap();
    assert!(matches!(reply, crate::state::Response::Value { .. }));
    let count = tools
        .sql_query(session, "notes", "SELECT count(*) AS c FROM t")
        .await
        .unwrap();
    assert_eq!(count.rows, vec![vec![json!(0)]]);

    // A failed statement leaves the database as it was.
    let error = tools
        .sql_execute(session, "notes", "INSERT INTO missing VALUES (1)")
        .await
        .unwrap_err();
    assert!(
        matches!(error, SessionSqlDbError::QueryError(_)),
        "{error:?}"
    );
    assert_eq!(shard.index()["notes"].sha256, empty);
    // The authorizer still guards what a statement may do.
    let error = tools
        .sql_execute(session, "notes", "ATTACH DATABASE ':memory:' AS escape")
        .await
        .unwrap_err();
    assert!(error.is_tool_error(), "{error:?}");
}
