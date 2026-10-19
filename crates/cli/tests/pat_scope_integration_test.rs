#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The built `everruns` binary with a read-only personal access token, against
//! a stub server that answers the way the real one does
//! (knowledge/security/pat-scopes.md, Tests item 7).
//!
//! The stub stands in for the server because the CLI end-to-end job runs it in
//! `AUTH_MODE=none`, where no personal access token exists. The refusal body
//! is the server's `insufficient_scope` problem document.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};

/// The server's 403 for a token whose scope does not cover the request.
const REFUSAL: &str = r#"{"type":"about:blank","title":"Forbidden","status":403,"detail":"This personal access token is read-only; this operation needs a full-access token.","code":"insufficient_scope"}"#;

/// Answer each request by its request line, for as long as the test runs.
fn serve(answer: fn(&str) -> (u16, String)) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 8192];
            // Read the head and as much body as content-length announces.
            loop {
                let read = stream.read(&mut chunk).unwrap_or(0);
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buffer).to_string();
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if buffer.len() >= end + 4 + length {
                    break;
                }
            }
            let request_line = String::from_utf8_lossy(&buffer)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let (status, body) = answer(&request_line);
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).ok();
        }
    });
    url
}

/// Run the CLI with `token` against `url`, isolated from the caller's own
/// credential file and organization.
fn everruns(url: &str, token: &str, args: &[&str]) -> Output {
    everruns_with_stdin(url, token, args, "")
}

fn everruns_with_stdin(url: &str, token: &str, args: &[&str], stdin: &str) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_everruns"))
        .args(args)
        .env("EVERRUNS_API_URL", url)
        .env("EVERRUNS_API_KEY", token)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env_remove("EVERRUNS_ORG_ID")
        .env_remove("EVERRUNS_IDEMPOTENCY_KEY")
        // An error's backtrace would follow its message on stderr.
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command that never reads stdin may exit first; that is not a failure.
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
    child.wait_with_output().unwrap()
}

fn me(scope: &str) -> String {
    format!(
        r#"{{"id":"user_1","email":"ada@example.com","name":"Ada","roles":[],"email_verified":true,"token_scope":{scope}}}"#
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn whoami_shows_a_read_tokens_access() {
    let url = serve(|line| match line {
        l if l.starts_with("GET /v1/auth/me ") => (200, me(r#""read""#)),
        _ => (404, "{}".into()),
    });
    let output = everruns(&url, "evr_pat_read", &["whoami"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "User:    Ada (ada@example.com)\nAccess:  read-only (personal access token)\n"
    );
}

#[test]
fn whoami_shows_a_full_tokens_access_and_keeps_the_field_in_json() {
    let url = serve(|line| match line {
        l if l.starts_with("GET /v1/auth/me ") => (200, me(r#""*""#)),
        _ => (404, "{}".into()),
    });
    let output = everruns(&url, "evr_pat_full", &["whoami"]);
    assert!(stdout(&output).contains("Access:  full (personal access token)"));

    let output = everruns(&url, "evr_pat_full", &["-o", "json", "whoami"]);
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["token_scope"], "*");
}

#[test]
fn whoami_prints_no_access_line_without_a_token_scope() {
    let url = serve(|line| match line {
        l if l.starts_with("GET /v1/auth/me ") => (200, me("null")),
        _ => (404, "{}".into()),
    });
    let output = everruns(&url, "a-session-token", &["whoami"]);
    assert_eq!(stdout(&output), "User:    Ada (ada@example.com)\n");
}

/// Tests item 7: a read token lists agents, and `agents create` fails with
/// the one-line scope message instead of the raw problem document.
#[test]
fn a_read_token_lists_agents_and_is_refused_agents_create() {
    let url = serve(|line| match line {
        l if l.starts_with("POST /v1/commands/list_agents ") => (
            200,
            r#"{"output":{"data":[{"id":"agent_1","name":"Helper"}],"total":1}}"#.into(),
        ),
        l if l.starts_with("POST /v1/commands/create_agent ") => (403, REFUSAL.into()),
        _ => (404, "{}".into()),
    });

    let listed = everruns(&url, "evr_pat_read", &["agents", "list"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    assert!(stdout(&listed).contains("agent_1"), "{}", stdout(&listed));

    let refused = everruns(
        &url,
        "evr_pat_read",
        &[
            "agents",
            "create",
            "--name",
            "x",
            "--instructions",
            "Be kind.",
        ],
    );
    assert!(!refused.status.success());
    assert_eq!(
        stderr(&refused).trim(),
        "Error: This personal access token is read-only; this command needs a full-access token."
    );
}

/// A refusal reached through a hand-written REST path, not the command
/// endpoint, is explained the same way.
#[test]
fn a_refusal_on_a_rest_route_gets_the_same_message() {
    let url = serve(|line| match line {
        l if l.starts_with("POST /v1/user/connections/") => (403, REFUSAL.into()),
        _ => (404, "{}".into()),
    });
    let output = everruns_with_stdin(
        &url,
        "evr_pat_read",
        &["connections", "set", "daytona", "--api-key-stdin"],
        "provider-key\n",
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("this command needs a full-access token"),
        "{}",
        stderr(&output)
    );
}

/// Any other 403 is reported as before, problem document and all.
#[test]
fn a_different_forbidden_is_not_rewritten() {
    let url = serve(|line| match line {
        l if l.starts_with("POST /v1/commands/create_agent ") => {
            (403, r#"{"status":403,"code":"forbidden"}"#.into())
        }
        _ => (404, "{}".into()),
    });
    let output = everruns(
        &url,
        "evr_pat_full",
        &[
            "agents",
            "create",
            "--name",
            "x",
            "--instructions",
            "Be kind.",
        ],
    );
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("403"), "{message}");
    assert!(!message.contains("full-access token"), "{message}");
}
