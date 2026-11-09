//! A second process driving a running session through `everruns::control`
//! (EVE-1340): list, attach, send, cancel, and the token and permission
//! rules that decide who may. Imports only the public `everruns` API.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use everruns::control::{
    ControlClient, ControlClientError, ControlEventKind, ControlRegistry, ControlServer,
};
use everruns::{Agent, Engine, LlmSimConfig, Model, Session};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const DEADLINE: Duration = Duration::from_secs(20);

fn session(model: Model) -> Session {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(model)
        .build()
        .unwrap();
    Engine::new().create(agent)
}

/// A host with one session registered as `main`, served from a fresh dir.
fn host(model: Model) -> (tempfile::TempDir, PathBuf, ControlRegistry, ControlServer) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    let registry = ControlRegistry::new();
    registry.register("main", &session(model));
    let server = ControlServer::start(&dir, registry.clone()).unwrap();
    (root, dir, registry, server)
}

fn slow() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::fixed("too late").with_response_delay(Duration::from_secs(120)),
    )
}

async fn raw(socket: &Path, line: &str) -> String {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream.write_all(line.as_bytes()).await.unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).await.unwrap();
    reply
}

/// What `list` and `cancel` report for a tracked turn: fixed at send time,
/// the turn id when `send` returned one and otherwise the accepted message's
/// id. It stays that way after the turn is assigned.
fn identity(sent: &everruns::SentMessage) -> Vec<String> {
    vec![sent.turn().identity()]
}

async fn wait_until_running(client: &ControlClient) -> Vec<String> {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let turns = client.list().await.unwrap()[0].active_turns.clone();
            if !turns.is_empty() {
                return turns;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a turn becomes active")
}

async fn wait_until_idle(registry: &ControlRegistry) {
    tokio::time::timeout(DEADLINE, async {
        while !registry.list()[0].active_turns.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked turns leave the registry");
}

fn rejected(result: Result<impl std::fmt::Debug, ControlClientError>) -> String {
    match result {
        Err(ControlClientError::Rejected { code, .. }) => code,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_client_lists_sends_and_watches_output() {
    let (_root, dir, _registry, _server) = host(Model::simulated("Hello from the host."));
    let client = ControlClient::new(&dir);

    let sessions = client.list().await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].label, "main");
    assert!(sessions[0].active_turns.is_empty());

    // A watcher attaches before another process sends.
    let mut watcher = client.attach("main", None).await.unwrap();
    assert_eq!(watcher.session(), sessions[0].id);
    let (sent, turn) = client.send_and_wait("main", "hi there").await.unwrap();
    // The default Engine admits durably first, so the turn may not be
    // assigned yet when the receipt returns.
    assert!(matches!(sent.disposition.as_str(), "started" | "pending"));
    assert!(turn.success);
    assert_eq!(turn.stop_reason, "end_turn");
    assert_eq!(turn.response, "Hello from the host.");

    let mut rendered = String::new();
    let mut kinds = Vec::new();
    tokio::time::timeout(DEADLINE, async {
        while let Some(event) = watcher.next().await.unwrap() {
            rendered.push_str(&event.render().unwrap_or_default());
            let done = event.kind == ControlEventKind::TurnCompleted;
            kinds.push(event.kind);
            if done {
                break;
            }
        }
    })
    .await
    .expect("turn completes on the attached stream");
    assert!(kinds.contains(&ControlEventKind::TurnStarted));
    assert!(rendered.contains("> hi there"), "{rendered}");
    assert!(rendered.contains("Hello from the host."), "{rendered}");

    // Re-attaching from sequence 0 replays the durable conversation.
    let mut replay = client.attach(&sessions[0].id, Some(0)).await.unwrap();
    let mut replayed = String::new();
    tokio::time::timeout(DEADLINE, async {
        while let Some(event) = replay.next().await.unwrap() {
            replayed.push_str(&event.render().unwrap_or_default());
            if event.kind == ControlEventKind::TurnCompleted {
                break;
            }
        }
    })
    .await
    .expect("replay reaches the completed turn");
    assert!(replayed.contains("> hi there"), "{replayed}");
    assert!(replayed.contains("Hello from the host."), "{replayed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_cancel_stops_the_running_turn_and_keeps_the_session() {
    let (_root, dir, registry, _server) = host(slow());
    let client = ControlClient::new(&dir);

    // Nothing is running yet.
    assert_eq!(rejected(client.cancel("main").await), "no_active_turn");

    let waiting = {
        let client = client.clone();
        tokio::spawn(async move { client.send_and_wait("main", "take your time").await })
    };
    let active = wait_until_running(&client).await;

    let cancelled = client.cancel("main").await.unwrap();
    assert_eq!(cancelled, active);
    let (_, turn) = tokio::time::timeout(DEADLINE, waiting)
        .await
        .expect("the waiting send returns")
        .unwrap()
        .unwrap();
    assert!(!turn.success);
    assert_eq!(turn.stop_reason, "cancelled");

    // The slot clears and the session still accepts work.
    wait_until_idle(&registry).await;
    assert_eq!(rejected(client.cancel("main").await), "no_active_turn");
    let again = registry.send("main", "still there?").await.unwrap();
    assert_eq!(client.cancel("main").await.unwrap(), identity(&again));
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_the_host_tracks_are_cancellable_from_the_attached_side() {
    let (_root, dir, registry, _server) = host(slow());
    let session = registry.session("main").unwrap();
    // The host's own REPL sends directly and reports the turn.
    let sent = session.send("host input").await.unwrap();
    registry.track(&session.id(), &sent);

    let client = ControlClient::new(&dir);
    assert_eq!(client.cancel("main").await.unwrap(), identity(&sent));
    let turn = tokio::time::timeout(DEADLINE, sent.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!turn.success);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_are_private_and_a_loose_directory_is_refused() {
    let (root, dir, _registry, server) = host(Model::simulated("ok"));
    let info = server.attach_info().clone();
    assert_eq!(info.dir, dir);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().mode() & 0o777;
    assert_eq!(mode(&info.dir), 0o700);
    assert_eq!(mode(&info.socket), 0o600);
    assert_eq!(mode(&info.token_file), 0o600);
    let token = std::fs::read_to_string(&info.token_file).unwrap();
    assert_eq!(token.len(), 64);

    // A second server on the same directory is refused while the first runs.
    let busy = ControlServer::start(&dir, ControlRegistry::new())
        .err()
        .unwrap();
    assert_eq!(busy.kind(), std::io::ErrorKind::AddrInUse);

    // Dropping the server removes the socket and token.
    drop(server);
    assert!(!info.socket.exists());
    assert!(!info.token_file.exists());

    // A directory others can read is refused by both ends.
    let loose = root.path().join("loose");
    std::fs::create_dir(&loose).unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
    let refused = ControlServer::start(&loose, ControlRegistry::new())
        .err()
        .unwrap();
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    match ControlClient::new(&loose).list().await {
        Err(ControlClientError::Io(error)) => {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied)
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_gate_requests_rotate_and_revoke() {
    let (_root, dir, _registry, server) = host(Model::simulated("ok"));
    let info = server.attach_info().clone();
    let client = ControlClient::new(&dir);

    let wrong = raw(
        &info.socket,
        "{\"token\":\"nope\",\"command\":{\"op\":\"list\"}}\n",
    )
    .await;
    assert!(wrong.contains("\"unauthorized\""), "{wrong}");
    let garbage = raw(&info.socket, "not json\n").await;
    assert!(garbage.contains("\"bad_request\""), "{garbage}");
    let unknown = rejected(client.send("nobody", "hi").await);
    assert_eq!(unknown, "not_found");

    // Rotation invalidates the old token; the client re-reads the file.
    let old = std::fs::read_to_string(&info.token_file).unwrap();
    server.rotate_token().unwrap();
    let new = std::fs::read_to_string(&info.token_file).unwrap();
    assert_ne!(old, new);
    let stale = raw(
        &info.socket,
        &format!("{{\"token\":\"{old}\",\"command\":{{\"op\":\"list\"}}}}\n"),
    )
    .await;
    assert!(stale.contains("\"unauthorized\""), "{stale}");
    assert_eq!(client.list().await.unwrap().len(), 1);

    // Revocation ends open attach streams and refuses new requests.
    let mut watcher = client.attach("main", None).await.unwrap();
    server.revoke();
    let ended = tokio::time::timeout(DEADLINE, watcher.next())
        .await
        .unwrap();
    assert!(matches!(ended, Err(ControlClientError::Closed(ref reason)) if reason == "revoked"));
    assert!(!info.token_file.exists());
    let refused = raw(
        &info.socket,
        &format!("{{\"token\":\"{new}\",\"command\":{{\"op\":\"list\"}}}}\n"),
    )
    .await;
    assert!(refused.contains("\"unauthorized\""), "{refused}");

    // A fresh token re-opens access.
    server.rotate_token().unwrap();
    assert_eq!(client.list().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_rotation_keeps_clients_working() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    let registry = ControlRegistry::new();
    registry.register("main", &session(Model::simulated("ok")));
    let server = ControlServer::builder(&dir)
        .token_ttl(Some(Duration::from_millis(50)))
        .start(registry)
        .unwrap();
    let first = std::fs::read_to_string(&server.attach_info().token_file).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let later = std::fs::read_to_string(&server.attach_info().token_file).unwrap();
    assert_ne!(first, later);
    assert_eq!(ControlClient::new(&dir).list().await.unwrap().len(), 1);

    // A scheduled rotation never re-opens a revoked server.
    server.revoke();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!server.attach_info().token_file.exists());
    assert!(ControlClient::new(&dir).list().await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_requests_and_excess_connections_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    let registry = ControlRegistry::new();
    registry.register("main", &session(Model::simulated("ok")));
    let server = ControlServer::builder(&dir)
        .max_connections(1)
        .start(registry)
        .unwrap();
    let socket = server.attach_info().socket.clone();

    let huge = "x".repeat(everruns::control::MAX_REQUEST_BYTES as usize + 16);
    let reply = raw(&socket, &huge).await;
    assert!(reply.contains("\"bad_request\""), "{reply}");

    // One open attach stream holds the only slot.
    let client = ControlClient::new(&dir);
    let _watcher = client.attach("main", None).await.unwrap();
    assert_eq!(rejected(client.list().await), "busy");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abandoned_waiting_send_frees_its_connection_slot() {
    // Regression: a client that dropped `send_and_wait` kept its slot until
    // the turn ended, so with the slots used up `cancel` answered `busy`.
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    let registry = ControlRegistry::new();
    registry.register("main", &session(slow()));
    let _server = ControlServer::builder(&dir)
        .max_connections(1)
        .start(registry.clone())
        .unwrap();
    let client = ControlClient::new(&dir);

    let waiting = {
        let client = client.clone();
        tokio::spawn(async move { client.send_and_wait("main", "take your time").await })
    };
    tokio::time::timeout(DEADLINE, async {
        while registry.list()[0].active_turns.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the turn becomes active");
    waiting.abort();
    let _ = waiting.await;

    // The slot comes back promptly and the turn is still running.
    let listed = tokio::time::timeout(DEADLINE, async {
        loop {
            match client.list().await {
                Ok(sessions) => return sessions,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("the slot is released");
    assert_eq!(listed[0].active_turns.len(), 1);
    assert_eq!(client.cancel("main").await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_reaches_every_unfinished_turn_of_concurrent_sends() {
    // Regression: one tracked slot per session lost the running turn when
    // concurrent sends tracked out of order.
    let (_root, dir, registry, _server) = host(slow());
    let (first, second) = tokio::join!(
        registry.send("main", "first"),
        registry.send("main", "second")
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    let client = ControlClient::new(&dir);
    let mut expected = identity(&first);
    for turn in identity(&second) {
        if !expected.contains(&turn) {
            expected.push(turn);
        }
    }
    let mut listed = client.list().await.unwrap()[0].active_turns.clone();
    listed.sort();
    let mut sorted = expected.clone();
    sorted.sort();
    assert_eq!(listed, sorted);

    let mut cancelled = client.cancel("main").await.unwrap();
    cancelled.sort();
    assert_eq!(cancelled, sorted);
    // The running turn ends cancelled; the queued message is dropped with
    // it, and nothing stays tracked.
    let turn = tokio::time::timeout(DEADLINE, first.wait())
        .await
        .expect("the running turn ends")
        .unwrap();
    assert!(!turn.success);
    assert!(registry.list()[0].active_turns.is_empty());
    drop(second);
    wait_until_idle(&registry).await;
    assert_eq!(rejected(client.cancel("main").await), "no_active_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_host_on_the_same_directory_leaves_the_first_intact() {
    // Regression: a second host's stale-socket cleanup could delete the
    // first host's socket and token.
    let (_root, dir, _registry, server) = host(Model::simulated("ok"));
    let info = server.attach_info().clone();
    let token = std::fs::read_to_string(&info.token_file).unwrap();
    // Even with the socket gone (as if stale), the directory lock refuses.
    std::fs::remove_file(&info.socket).unwrap();
    let second = ControlServer::start(&dir, ControlRegistry::new())
        .err()
        .unwrap();
    assert_eq!(second.kind(), std::io::ErrorKind::AddrInUse);
    assert_eq!(std::fs::read_to_string(&info.token_file).unwrap(), token);

    // Once the first host stops, the directory is free again.
    drop(server);
    let third = ControlServer::start(&dir, ControlRegistry::new()).unwrap();
    assert!(third.attach_info().socket.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_ends_when_the_host_drops_the_session() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    let registry = ControlRegistry::new();
    let id = registry.register("main", &session(Model::simulated("ok")));
    let _server = ControlServer::start(&dir, registry.clone()).unwrap();
    let mut watcher = ControlClient::new(&dir).attach("main", None).await.unwrap();

    drop(registry.unregister(&id));
    let ended = tokio::time::timeout(DEADLINE, watcher.next())
        .await
        .expect("the stream ends");
    assert!(
        matches!(ended, Err(ControlClientError::Closed(ref reason)) if reason == "session_closed"),
        "{ended:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_response_line_is_refused_by_the_client() {
    use std::io::Write as _;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("ctl");
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("token"))
        .unwrap()
        .write_all(b"t")
        .unwrap();
    let listener = tokio::net::UnixListener::bind(dir.join("control.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let huge = vec![b'x'; everruns::control::MAX_REQUEST_BYTES as usize + 1024];
        let _ = stream.write_all(&huge).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    match tokio::time::timeout(DEADLINE, ControlClient::new(&dir).list()).await {
        Ok(Err(ControlClientError::Protocol(detail))) => {
            assert!(detail.contains("size limit"), "{detail}")
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}
