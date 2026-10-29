use super::super::record::{SandboxNetworkPolicy, SandboxTemplateSpec};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{
    SandboxTemplateBuildId, SandboxTemplateId, SandboxTemplateRevisionId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const MAX_SCRIPT_BYTES: usize = 32 * 1024;
pub const MAX_LOG_BYTES: usize = 32 * 1024;
pub const MAX_NOTES_BYTES: usize = 16 * 1024;

/// Inputs for a single isolated installation and reusable filesystem snapshot.
/// Build compute receives no workload credentials or private Session state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxBuildRequest {
    /// Source revision to guard against concurrent template edits.
    #[schema(value_type = String, example = "sbxtplrev_01933b5a000070008000000000000001")]
    pub source_revision_id: SandboxTemplateRevisionId,
    /// POSIX shell install script, 1–32768 UTF-8 bytes. Keep secrets out of it.
    #[schema(
        example = "python -m venv /home/daytona/.venvs/devtools\n/home/daytona/.venvs/devtools/bin/pip install httpie==3.2.4"
    )]
    pub install_script: String,
    /// Optional check run after installation and again in fresh snapshot compute.
    #[serde(default)]
    #[schema(example = "/home/daytona/.venvs/devtools/bin/http --version")]
    pub verification_command: Option<String>,
    /// Package sources for the build only. Defaults to the template policy.
    #[serde(default)]
    pub build_network: Option<SandboxNetworkPolicy>,
    /// Total build deadline, including installation and fresh verification, in seconds.
    #[serde(default = "default_timeout")]
    #[schema(minimum = 60, maximum = 1800, example = 600)]
    pub timeout_seconds: u64,
    /// Optional bounded, author-supplied Markdown captured from the builder.
    #[serde(default)]
    #[schema(example = "/home/daytona/.everruns/sandbox.md")]
    pub metadata_path: Option<String>,
}

fn default_timeout() -> u64 {
    600
}

impl SandboxBuildRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.install_script.trim().is_empty()
            || self.install_script.len() > MAX_SCRIPT_BYTES
            || self.install_script.contains('\0')
        {
            return Err(
                "install_script must contain 1 to 32768 bytes without NUL characters".into(),
            );
        }
        if self
            .verification_command
            .as_ref()
            .is_some_and(|s| s.len() > MAX_SCRIPT_BYTES || s.contains('\0'))
        {
            return Err(
                "verification_command must be at most 32768 bytes without NUL characters".into(),
            );
        }
        if !(60..=1800).contains(&self.timeout_seconds) {
            return Err("timeout_seconds must be between 60 and 1800".into());
        }
        if let Some(path) = self.metadata_path.as_ref().filter(|p| !p.is_empty())
            && (path.len() > 512
                || !path.starts_with('/')
                || path == "/"
                || path.contains('\0')
                || path
                    .split('/')
                    .skip(1)
                    .any(|p| p.is_empty() || p == "." || p == ".."))
        {
            return Err("metadata_path must be a normalized absolute file path".into());
        }
        Ok(())
    }
}

/// Durable build lifecycle. Ready artifacts can be applied only after cleanup completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBuildStatus {
    Queued,
    Running,
    Ready,
    Failed,
    Cancelled,
    Applied,
    Discarded,
}

impl SandboxBuildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Applied => "applied",
            Self::Discarded => "discarded",
        }
    }
}

/// Tenant-scoped build attempt, retained inputs, bounded output, and provider artifact.
/// Management credentials are stored separately and never appear in this record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxTemplateBuild {
    /// Unique build attempt identifier.
    #[schema(value_type = String, example = "sbxbuild_01933b5a000070008000000000000001")]
    pub id: SandboxTemplateBuildId,
    /// Parent template whose immutable source revision supplied the build configuration.
    #[schema(value_type = String, example = "sbxtpl_01933b5a000070008000000000000001")]
    pub sandbox_template_id: SandboxTemplateId,
    /// Original immutable installation and verification inputs.
    pub request: SandboxBuildRequest,
    /// Exact source spec; no runtime credentials or workspace are attached.
    pub source_spec: SandboxTemplateSpec,
    /// Persisted lifecycle state; inspect cleanup_pending before applying a Ready build.
    pub status: SandboxBuildStatus,
    /// Current execution or cleanup step.
    #[schema(example = "installing")]
    pub step: String,
    /// UTF-8 execution-output tail, bounded to 32 KiB.
    #[schema(example = "HTTPie 3.2.4\nVerification passed\n")]
    pub logs: String,
    /// Bounded failure explanation, absent when no failure has been recorded.
    #[schema(example = "Command failed with exit code 23")]
    pub error: Option<String>,
    /// Up to 16 KiB of author-supplied Markdown, presented as untrusted reference data.
    #[schema(example = "# Installed tools\n- HTTPie 3.2.4")]
    pub software_notes: Option<String>,
    /// Deterministic provider artifact name used to reconcile uncertain submissions.
    #[schema(example = "everruns-build-01933b5a000070008000000000000001")]
    pub snapshot_name: String,
    /// Provider snapshot identifier, present once the artifact has been resolved.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub snapshot_id: Option<String>,
    /// Submission time used when reconciling a snapshot response lost in transport.
    #[serde(default)]
    #[schema(example = "2026-10-29T00:00:00Z")]
    pub snapshot_requested_at: Option<DateTime<Utc>>,
    /// Disposable installation compute identifier or its persisted submission name.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440001")]
    pub builder_id: Option<String>,
    /// Disposable fresh-snapshot verification compute identifier or submission name.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440002")]
    pub verification_sandbox_id: Option<String>,
    /// Whether cancellation was accepted; stale worker saves cannot erase this request.
    #[schema(example = false)]
    pub cancel_requested: bool,
    /// Whether provider resources still require confirmed cleanup or reconciliation.
    #[schema(example = false)]
    pub cleanup_pending: bool,
    /// Immutable template revision created by applying this artifact, if applied.
    #[schema(value_type = Option<String>, example = "sbxtplrev_01933b5a000070008000000000000002")]
    pub applied_revision_id: Option<SandboxTemplateRevisionId>,
    /// Time this build attempt was accepted.
    #[schema(example = "2026-10-29T00:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Time of the last persisted build update.
    #[schema(example = "2026-10-29T00:01:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// Retain a bounded UTF-8 tail, including when provider output splits codepoints.
pub fn append_log(log: &mut String, text: &str) {
    log.push_str(text);
    if log.len() > MAX_LOG_BYTES {
        let mut cut = log.len() - MAX_LOG_BYTES;
        while !log.is_char_boundary(cut) {
            cut += 1;
        }
        log.drain(..cut);
    }
}

/// Safe user-facing conflicts, distinguished from internal persistence errors.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SandboxBuildConflict(pub &'static str);

/// Reference data resolved by the server from an applied artifact in this tenant.
/// Software notes remain untrusted author content, never runtime policy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxBuildProvenance {
    /// Applied build resolved from the exact tenant, provider account, and snapshot.
    #[schema(value_type = String, example = "sbxbuild_01933b5a000070008000000000000001")]
    pub build_id: SandboxTemplateBuildId,
    /// Exact immutable provider snapshot selected by the Session.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub snapshot_id: String,
    /// Time the originating build attempt was accepted.
    #[schema(example = "2026-10-29T00:01:00Z")]
    pub built_at: DateTime<Utc>,
    /// Author-supplied reference notes, never trusted instructions or policy.
    #[schema(example = "# Installed tools\n- HTTPie 3.2.4")]
    pub software_notes: Option<String>,
    /// Optional check that passed after installation and in fresh snapshot compute.
    #[schema(example = "/home/daytona/.venvs/devtools/bin/http --version")]
    pub verification_command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> SandboxBuildRequest {
        serde_json::from_value(serde_json::json!({"source_revision_id":SandboxTemplateRevisionId::new(),"install_script":"echo ready"})).unwrap()
    }
    #[test]
    fn rejects_unbounded_and_ambiguous_execution_inputs() {
        let mut req = request();
        assert!(req.validate().is_ok());
        assert_eq!(req.timeout_seconds, 600);
        for script in [
            "".into(),
            " \n".into(),
            "x".repeat(MAX_SCRIPT_BYTES + 1),
            "echo\0bad".into(),
        ] {
            req.install_script = script;
            assert!(req.validate().is_err());
        }
        req = request();
        for timeout in [0, 59, 1801, u64::MAX] {
            req.timeout_seconds = timeout;
            assert!(req.validate().is_err());
        }
        req = request();
        for path in [
            "relative.md",
            "/",
            "/tmp/../notes",
            "/tmp//notes",
            "/tmp/./notes",
            "/tmp/notes/",
        ] {
            req.metadata_path = Some(path.into());
            assert!(req.validate().is_err(), "{path}");
        }
        req.metadata_path = Some("/home/daytona/.everruns/sandbox.md".into());
        assert!(req.validate().is_ok());
        let mut forged = serde_json::to_value(request()).unwrap();
        forged["credentials"] = serde_json::json!({"TOKEN":"raw-secret"});
        assert!(serde_json::from_value::<SandboxBuildRequest>(forged).is_err());
    }
    #[test]
    fn logs_are_bounded_at_utf8_boundaries() {
        let mut log = String::new();
        append_log(&mut log, &"💻".repeat(MAX_LOG_BYTES));
        assert_eq!(log.len(), MAX_LOG_BYTES);
        append_log(&mut log, "done");
        assert_eq!(log.len(), MAX_LOG_BYTES);
        assert!(log.ends_with("done"));
    }
}
