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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxBuildRequest {
    /// Source revision to guard against concurrent template edits.
    #[schema(value_type = String)]
    pub source_revision_id: SandboxTemplateRevisionId,
    pub install_script: String,
    #[serde(default)]
    pub verification_command: Option<String>,
    /// Package sources for the build only. Defaults to the template policy.
    #[serde(default)]
    pub build_network: Option<SandboxNetworkPolicy>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Optional bounded, author-supplied Markdown captured from the builder.
    #[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxTemplateBuild {
    #[schema(value_type = String)]
    pub id: SandboxTemplateBuildId,
    #[schema(value_type = String)]
    pub sandbox_template_id: SandboxTemplateId,
    pub request: SandboxBuildRequest,
    /// Exact source spec; no runtime credentials or workspace are attached.
    pub source_spec: SandboxTemplateSpec,
    pub status: SandboxBuildStatus,
    pub step: String,
    pub logs: String,
    pub error: Option<String>,
    pub software_notes: Option<String>,
    pub snapshot_name: String,
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub snapshot_requested_at: Option<DateTime<Utc>>,
    pub builder_id: Option<String>,
    pub verification_sandbox_id: Option<String>,
    pub cancel_requested: bool,
    pub cleanup_pending: bool,
    #[schema(value_type = Option<String>)]
    pub applied_revision_id: Option<SandboxTemplateRevisionId>,
    pub created_at: DateTime<Utc>,
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
    #[schema(value_type = String)]
    pub build_id: SandboxTemplateBuildId,
    pub snapshot_id: String,
    pub built_at: DateTime<Utc>,
    pub software_notes: Option<String>,
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
