#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The sandbox guest end to end: the real server on a local port, driven by
//! the real client over the direct transport.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::sandbox_guest::{
    GUEST_TOKEN_HEADER, GuestCapability, GuestExecRequest, GuestFileKind,
};
use everruns_integrations::sandbox_guest::client::{
    DirectTransport, GuestClient, GuestClientError, GuestMethod, GuestRequest, GuestTransport,
};
use everruns_integrations::sandbox_guest::server::{GuestConfig, router};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct Guest {
    url: String,
    workspace: tempfile::TempDir,
    client: GuestClient,
}

async fn start(desktop: bool, env_file: Option<&str>) -> Guest {
    let workspace = tempfile::tempdir().unwrap();
    let exec_env_file = env_file.map(|content| {
        let path = workspace.path().join(".guest-env");
        std::fs::write(&path, content).unwrap();
        path
    });
    let app = router(GuestConfig {
        token: TOKEN.to_string(),
        workspace: workspace.path().to_path_buf(),
        exec_env_file,
        desktop,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = GuestClient::new(Arc::new(DirectTransport::new(&url).unwrap()), TOKEN);
    Guest {
        url,
        workspace,
        client,
    }
}

fn run(command: &str) -> GuestExecRequest {
    GuestExecRequest {
        command: command.to_string(),
        ..Default::default()
    }
}

#[tokio::test]
async fn health_reports_capabilities_only_to_token_holders() {
    let guest = start(true, None).await;
    let health = guest
        .client
        .wait_ready(Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(health.protocol, 1);
    assert!(health.capabilities.contains(&GuestCapability::Exec));
    assert!(health.capabilities.contains(&GuestCapability::Desktop));
    assert_eq!(
        health.workspace.as_deref(),
        Some(guest.workspace.path().to_str().unwrap())
    );

    let anonymous = GuestClient::new(
        Arc::new(DirectTransport::new(&guest.url).unwrap()),
        "wrong-token-wrong-token-wrong-token",
    );
    let health = anonymous.health().await.unwrap();
    assert!(health.capabilities.is_empty());
    assert!(health.workspace.is_none());
}

#[tokio::test]
async fn every_other_path_refuses_a_wrong_or_missing_token() {
    let guest = start(false, None).await;
    let wrong = GuestClient::new(
        Arc::new(DirectTransport::new(&guest.url).unwrap()),
        "wrong-token-wrong-token-wrong-token",
    );
    match wrong.exec(&run("touch pwned")).await {
        Err(GuestClientError::Guest { status, code, .. }) => {
            assert_eq!(status, 401);
            assert_eq!(code, "unauthorized");
        }
        other => panic!("expected 401, got {other:?}"),
    }
    assert!(!guest.workspace.path().join("pwned").exists());

    let transport = DirectTransport::new(&guest.url).unwrap();
    for path in [
        "/v1/files?path=a",
        "/v1/files/list?path=.",
        "/v1/archive?path=.",
    ] {
        let response = transport
            .send(GuestRequest {
                method: GuestMethod::Get,
                path_and_query: path.to_string(),
                headers: Vec::new(),
                body: Vec::new(),
                timeout: Duration::from_secs(5),
            })
            .await
            .unwrap();
        assert_eq!(response.status, 401, "{path}");
    }
    let _ = GUEST_TOKEN_HEADER;
}

#[tokio::test]
async fn exec_runs_in_the_workspace_with_env_and_stdin() {
    let guest = start(false, Some("ALL_PROXY=socks5h://proxy:1080\n")).await;
    let mut request = run("pwd; echo \"$GREETING $ALL_PROXY\"; cat; echo oops >&2; exit 3");
    request.env.insert("GREETING".into(), "hello".into());
    request.stdin = Some("from stdin".into());
    let output = guest.client.exec(&request).await.unwrap();
    assert_eq!(output.exit_code, 3);
    let workspace = guest.workspace.path().canonicalize().unwrap();
    assert_eq!(
        output.stdout,
        format!(
            "{}\nhello socks5h://proxy:1080\nfrom stdin",
            workspace.display()
        )
    );
    assert_eq!(output.stderr, "oops\n");
    assert!(!output.timed_out && !output.truncated);
}

#[tokio::test]
async fn commands_never_see_the_guest_token() {
    let guest = start(false, None).await;
    let output = guest
        .client
        .exec(&run("env | grep -c EVERRUNS_GUEST_TOKEN || true"))
        .await
        .unwrap();
    assert_eq!(output.stdout.trim(), "0");
}

#[tokio::test]
async fn argv_reaches_the_program_without_a_shell() {
    let guest = start(false, None).await;
    let output = guest
        .client
        .exec(&GuestExecRequest {
            argv: vec!["printf".into(), "%s".into(), "$(id); `x` 'y'".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "$(id); `x` 'y'");
}

#[tokio::test]
async fn timeouts_kill_the_whole_process_group() {
    let guest = start(false, None).await;
    let mut request = run("sleep 30 & sleep 30; echo never");
    request.timeout_ms = Some(300);
    let started = std::time::Instant::now();
    let output = guest.client.exec(&request).await.unwrap();
    assert!(output.timed_out);
    assert_eq!(output.exit_code, 124);
    assert!(!output.stdout.contains("never"));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn huge_output_is_capped_not_buffered() {
    let guest = start(false, None).await;
    let output = guest
        .client
        .exec(&run("head -c 6000000 /dev/zero | tr '\\0' a"))
        .await
        .unwrap();
    assert!(output.truncated);
    assert_eq!(
        output.stdout.len(),
        everruns_contracts::sandbox_guest::MAX_EXEC_OUTPUT_BYTES
    );
}

#[tokio::test]
async fn files_round_trip_with_binary_content() {
    let guest = start(false, None).await;
    let bytes: Vec<u8> = (0..=255).collect();
    let written = guest
        .client
        .write_file("nested/dir/blob.bin", &bytes)
        .await
        .unwrap();
    assert_eq!(written.bytes_written, 256);
    assert!(written.path.ends_with("nested/dir/blob.bin"));
    assert_eq!(
        guest.client.read_file("nested/dir/blob.bin").await.unwrap(),
        bytes
    );

    let stat = guest.client.stat("nested/dir/blob.bin").await.unwrap();
    assert_eq!(stat.kind, GuestFileKind::File);
    assert_eq!(stat.size, 256);
    let listing = guest.client.list("nested/dir").await.unwrap();
    assert_eq!(listing.entries.len(), 1);

    guest.client.delete("nested").await.unwrap();
    let missing = guest
        .client
        .read_file("nested/dir/blob.bin")
        .await
        .unwrap_err();
    assert!(missing.is_not_found(), "{missing:?}");
    // Deleting what is gone succeeds.
    guest.client.delete("nested").await.unwrap();
}

#[tokio::test]
async fn archives_export_and_import_a_directory() {
    let guest = start(false, None).await;
    guest
        .client
        .write_file("src/a.txt", b"alpha")
        .await
        .unwrap();
    guest
        .client
        .write_file("src/sub/b.txt", b"beta")
        .await
        .unwrap();
    let archive = guest.client.export_archive("src").await.unwrap();
    let imported = guest
        .client
        .import_archive("restored", archive)
        .await
        .unwrap();
    assert!(imported.entries >= 2);
    assert_eq!(
        guest.client.read_file("restored/sub/b.txt").await.unwrap(),
        b"beta"
    );
}

#[tokio::test]
async fn archive_entries_cannot_escape_the_target() {
    let guest = start(false, None).await;
    // A hand-built tar whose entry climbs out of the target directory.
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    let name = b"../escaped.txt";
    header.as_old_mut().name[..name.len()].copy_from_slice(name);
    header.set_cksum();
    let mut raw = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw);
        builder.append(&header, &b"evil"[..]).unwrap();
        builder.finish().unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    let archive = gz.finish().unwrap();
    let _ = guest.client.import_archive("inside", archive).await;
    assert!(!guest.workspace.path().join("escaped.txt").exists());
}

#[tokio::test]
async fn an_unreachable_guest_is_reported_as_such() {
    let client = GuestClient::new(
        Arc::new(DirectTransport::new("http://127.0.0.1:9").unwrap()),
        TOKEN,
    );
    let err = client
        .wait_ready(Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(matches!(err, GuestClientError::Unreachable(_)), "{err:?}");
}
