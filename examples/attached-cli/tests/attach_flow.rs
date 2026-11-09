//! The example end to end: a host serving a session, and the attached CLI
//! listing, sending, watching and cancelling from outside it. A simulated model
//! stands in for the provider; the binary always runs a real one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use everruns::{Agent, Engine, LlmSimConfig, Model, Session};
use everruns_attached_cli::{AttachedCommand, run_attached, serve};
use tokio::io::{AsyncWriteExt, BufReader, DuplexStream};
use tokio::task::JoinHandle;

const DEADLINE: Duration = Duration::from_secs(20);

fn session(model: Model) -> Session {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(model)
        .build()
        .unwrap();
    Engine::new().create(agent)
}

/// Start the host; returns its prompt input and the task yielding its output.
async fn start_host(dir: &Path, model: Model) -> (DuplexStream, JoinHandle<String>) {
    let (input, host_side) = tokio::io::duplex(1024);
    let dir_owned = dir.to_path_buf();
    let session = session(model);
    let host = tokio::spawn(async move {
        let mut out = Vec::new();
        serve(&dir_owned, session, BufReader::new(host_side), &mut out)
            .await
            .unwrap();
        String::from_utf8(out).unwrap()
    });
    tokio::time::timeout(DEADLINE, async {
        while !dir.join("control.sock").exists() || !dir.join("token").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("host starts serving");
    (input, host)
}

async fn attached(dir: &Path, command: AttachedCommand) -> String {
    let mut out = Vec::new();
    run_attached(dir, command, &mut out).await.unwrap();
    String::from_utf8(out).unwrap()
}

fn control_dir(root: &tempfile::TempDir) -> PathBuf {
    root.path().join("ctl")
}

#[tokio::test(flavor = "multi_thread")]
async fn list_send_and_attach_from_a_second_client() {
    let root = tempfile::tempdir().unwrap();
    let dir = control_dir(&root);
    let (input, host) = start_host(&dir, Model::simulated("Four.")).await;

    let listed = attached(&dir, AttachedCommand::List).await;
    assert!(listed.starts_with("main\t"), "{listed}");
    assert!(listed.trim_end().ends_with("idle"), "{listed}");

    let reply = attached(
        &dir,
        AttachedCommand::Send {
            session: "main".into(),
            text: vec!["what is".into(), "2 + 2?".into()],
            no_wait: false,
        },
    )
    .await;
    assert_eq!(reply, "Four.\n");

    // Replaying from the start shows the conversation the attached side drove.
    let watch_dir = dir.clone();
    let watcher = tokio::spawn(async move {
        attached(
            &watch_dir,
            AttachedCommand::Attach {
                session: "main".into(),
                after: Some(0),
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Closing the host's input stops it; the attach stream ends with it.
    drop(input);
    let host_out = tokio::time::timeout(DEADLINE, host).await.unwrap().unwrap();
    assert!(host_out.contains("session main"), "{host_out}");
    let watched = tokio::time::timeout(DEADLINE, watcher)
        .await
        .unwrap()
        .unwrap();
    assert!(watched.contains("> what is 2 + 2?"), "{watched}");
    assert!(watched.contains("Four."), "{watched}");
    assert!(watched.contains("[detached: shutdown]"), "{watched}");
    assert!(!dir.join("control.sock").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_client_cancels_the_hosts_own_turn() {
    let root = tempfile::tempdir().unwrap();
    let dir = control_dir(&root);
    let slow = Model::simulated_with_config(
        LlmSimConfig::fixed("too late").with_response_delay(Duration::from_secs(120)),
    );
    let (mut input, host) = start_host(&dir, slow).await;

    // The host's own prompt starts a long turn.
    input.write_all(b"take your time\n").await.unwrap();
    tokio::time::timeout(DEADLINE, async {
        while !attached(&dir, AttachedCommand::List)
            .await
            .contains("running")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the host's turn shows as running");

    let cancelled = attached(
        &dir,
        AttachedCommand::Cancel {
            session: "main".into(),
        },
    )
    .await;
    assert!(cancelled.starts_with("cancelled "), "{cancelled}");

    drop(input);
    let host_out = tokio::time::timeout(DEADLINE, host).await.unwrap().unwrap();
    assert!(host_out.contains("[Cancelled"), "{host_out}");
    assert!(!host_out.contains("too late"), "{host_out}");
}
