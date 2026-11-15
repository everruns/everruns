//! Sandbox guest protocol: how Everruns talks to the small agent that runs
//! inside a sandbox whose provider has no exec or file API of its own.
//!
//! The guest (`everruns-sandbox-guest`, a binary in `everruns-integrations`)
//! serves this protocol over HTTP on [`GUEST_PORT`]. A provider only decides
//! how a request reaches the guest (its `GuestTransport`); the requests and
//! answers are the same everywhere, so a new provider without an exec API
//! needs no new protocol.
//!
//! Decisions:
//! - Plain HTTP + JSON with raw bodies for file bytes and archives. Every
//!   transport Everruns needs (pod IP, the Kubernetes API server's pod proxy,
//!   a provider's own HTTP endpoint) can carry it, and `curl` can debug it.
//! - Every request carries [`GUEST_TOKEN_HEADER`], a secret minted per
//!   sandbox. The guest refuses everything else except `GET /v1/health`'s
//!   liveness answer, which reveals nothing but the version. Not
//!   `Authorization`: the Kubernetes API server's pod proxy needs that header
//!   for its own authentication.
//! - Versioned paths (`/v1/...`) plus [`GUEST_PROTOCOL_VERSION`] in the health
//!   answer. A breaking change adds `/v2`; the client checks the version on
//!   connect and says which side to upgrade.
//! - Exec is buffered with caps ([`MAX_EXEC_OUTPUT_BYTES`]), not streamed:
//!   the sandbox provider contract returns whole outputs, and a cap bounds the
//!   memory either side spends on a noisy command.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Protocol version the guest speaks; bumped on any breaking change.
pub const GUEST_PROTOCOL_VERSION: u32 = 1;
/// TCP port the guest listens on inside the sandbox.
pub const GUEST_PORT: u16 = 8080;
/// Request header carrying the per-sandbox token.
pub const GUEST_TOKEN_HEADER: &str = "x-everruns-guest-token";
/// Environment variable holding the token.
pub const GUEST_TOKEN_ENV: &str = "EVERRUNS_GUEST_TOKEN";
/// Environment variable naming a file that holds the token. Preferred
/// over [`GUEST_TOKEN_ENV`]: a mounted secret file is not visible in the
/// process environment of every command the guest runs.
pub const GUEST_TOKEN_FILE_ENV: &str = "EVERRUNS_GUEST_TOKEN_FILE";
/// Environment variable naming an optional `KEY=VALUE` file whose entries are
/// added to every command's environment (egress proxy settings). Read on each
/// exec, so a provider can rotate it without restarting the guest.
pub const GUEST_EXEC_ENV_FILE_ENV: &str = "EVERRUNS_GUEST_EXEC_ENV_FILE";
/// Largest stdout or stderr the guest returns for one command, each.
pub const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
/// Largest file body the guest accepts or returns in one request.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Longest a single command may run.
pub const MAX_EXEC_TIMEOUT_MS: u64 = 60 * 60 * 1000;
/// Default command timeout when the request names none.
pub const DEFAULT_EXEC_TIMEOUT_MS: u64 = 120 * 1000;

/// Paths of the protocol, relative to the guest's base URL.
pub mod paths {
    pub const HEALTH: &str = "/v1/health";
    pub const EXEC: &str = "/v1/exec";
    /// `GET` reads, `PUT` writes, `DELETE` removes; `?path=` names the file.
    pub const FILES: &str = "/v1/files";
    /// `GET` with `?path=`: metadata of one path.
    pub const STAT: &str = "/v1/files/stat";
    /// `GET` with `?path=`: entries of one directory.
    pub const LIST: &str = "/v1/files/list";
    /// `GET` with `?path=`: a gzipped tar of a directory. `PUT` with
    /// `?path=`: unpack one into a directory.
    pub const ARCHIVE: &str = "/v1/archive";
}

/// Optional features a guest build may offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestCapability {
    Exec,
    Files,
    Archive,
    /// An X display (`DISPLAY=:0`) with `xdotool` and screenshot tools; the
    /// image provides it, the guest only reports it.
    Desktop,
}

/// `GET /v1/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestHealth {
    /// Guest build version (the Everruns version it was built from).
    pub version: String,
    /// [`GUEST_PROTOCOL_VERSION`] of the guest.
    pub protocol: u32,
    /// Filled only for authenticated callers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<GuestCapability>,
    /// Default working directory for relative paths and commands, filled only
    /// for authenticated callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// `POST /v1/exec`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestExecRequest {
    /// Run with `sh -c`. Ignored when [`Self::argv`] is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Run this program with these arguments directly, no shell in between,
    /// so model-typed text (desktop input) reaches it byte for byte.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    /// Working directory; the guest's workspace when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Extra environment entries, layered over the guest's own.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Kill the command after this long; [`DEFAULT_EXEC_TIMEOUT_MS`] when
    /// absent, capped at [`MAX_EXEC_TIMEOUT_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Bytes for the command's stdin, base64-free UTF-8 text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

/// Answer to [`GuestExecRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestExecResponse {
    /// Process exit code; 124 after a timeout, `128 + signal` when killed.
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Output was cut at [`MAX_EXEC_OUTPUT_BYTES`].
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub duration_ms: u64,
}

/// Kind of a filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// `GET /v1/files/stat`, and one entry of `GET /v1/files/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestFileStat {
    /// Absolute path.
    pub path: String,
    pub kind: GuestFileKind,
    pub size: u64,
    /// Unix permission bits.
    pub mode: u32,
    /// Seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
}

/// `GET /v1/files/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestFileList {
    pub path: String,
    pub entries: Vec<GuestFileStat>,
}

/// `PUT /v1/files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestWriteResponse {
    pub path: String,
    pub bytes_written: u64,
}

/// `PUT /v1/archive`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestArchiveImport {
    pub path: String,
    pub entries: u64,
}

/// Error body for every non-2xx answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestError {
    /// Stable machine code: `unauthorized`, `not_found`, `bad_request`,
    /// `too_large`, `internal`.
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The guest and the server ship separately: an old guest must keep
    // parsing what a new server sends, so optional fields stay optional.
    #[test]
    fn exec_requests_accept_only_a_command() {
        let request: GuestExecRequest = serde_json::from_str(r#"{"command":"ls"}"#).unwrap();
        assert_eq!(request.command, "ls");
        assert!(request.cwd.is_none() && request.env.is_empty());
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"command": "ls"})
        );
    }

    #[test]
    fn unauthenticated_health_reveals_only_the_version() {
        let health = GuestHealth {
            version: "1.0.0".into(),
            protocol: GUEST_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            workspace: None,
        };
        assert_eq!(
            serde_json::to_value(&health).unwrap(),
            serde_json::json!({"version": "1.0.0", "protocol": 1})
        );
    }

    #[test]
    fn capabilities_and_kinds_are_snake_case() {
        assert_eq!(
            serde_json::to_value([GuestCapability::Desktop, GuestCapability::Archive]).unwrap(),
            serde_json::json!(["desktop", "archive"])
        );
        assert_eq!(
            serde_json::to_value(GuestFileKind::Directory).unwrap(),
            serde_json::json!("directory")
        );
    }
}
