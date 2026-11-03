#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::process::Command;

#[test]
fn disabled_companion_requires_no_database_s3_directory_or_listener() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("must-not-exist");
    let mut command = Command::new(env!("CARGO_BIN_EXE_everruns-trace-store"));
    for (key, _) in
        std::env::vars().filter(|(key, _)| key.starts_with("TRACE_STORE_") || key == "DATABASE_URL")
    {
        command.env_remove(key);
    }
    let output = command
        .arg("--database-url")
        .arg("invalid://must-not-connect")
        .arg("--directory")
        .arg(&directory)
        .args(["run", "--metrics-addr", "192.0.2.1:1"])
        .env("TRACE_STORE_ORG_IDS", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"enabled":false})
    );
    assert!(!directory.exists());
}

#[test]
fn enabled_companion_requires_explicit_organizations_before_side_effects() {
    let output = Command::new(env!("CARGO_BIN_EXE_everruns-trace-store"))
        .env_remove("TRACE_STORE_ORG_IDS")
        .env_remove("TRACE_STORE_ALL_ORGS")
        .args(["run", "--enabled", "true"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("explicit org ids"));
}

#[test]
fn all_org_mode_rejects_ambiguous_scope_before_storage_setup() {
    let output = Command::new(env!("CARGO_BIN_EXE_everruns-trace-store"))
        .args([
            "run",
            "--enabled",
            "true",
            "--all-orgs",
            "true",
            "--org-ids",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exclusively"));
}

#[test]
fn readiness_probe_accepts_a_fragmented_http_status_line() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 256];
        let n = socket.read(&mut request).unwrap();
        assert!(n > 0);
        socket.write_all(b"HTTP/1.1 ").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        socket
            .write_all(b"200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready")
            .unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_everruns-trace-store"))
        .args(["probe", "--addr", &addr.to_string()])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
