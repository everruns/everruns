//! Per-agent configuration for the host shell.
//!
//! Everything here is chosen by whoever configures the agent. None of it is
//! reachable from model input, which is the invariant that makes a contained
//! shell worth having: a tool call may pass a script and ask for an escalation,
//! never a containment mode, a writable root, or a timeout.

use std::path::PathBuf;

use crate::containment::{ContainmentMode, SandboxLauncher, SandboxOptions};
use serde_json::Value;

/// When a human is asked before a command runs, or before it runs uncontained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Never ask. Commands run under the configured containment, and a request
    /// to escalate past it is refused. The default: an unattended agent has
    /// nobody to ask, and a policy that silently blocks is worse than one that
    /// says no.
    #[default]
    Never,
    /// Ask only when a command fails in a way that looks like the containment
    /// blocked it, then offer to retry uncontained.
    OnFailure,
    /// Ask only when the model explicitly requests an escalation and gives a
    /// justification.
    OnRequest,
    /// Ask for anything outside a tiny read-only set of commands.
    Untrusted,
}

impl ApprovalPolicy {
    /// The wire name, shared with Yolop's `approval_policy` config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Untrusted => "untrusted",
        }
    }

    /// Parse a wire name. Unknown values are rejected rather than defaulted, so
    /// a typo cannot quietly turn a prompt into silence.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "never" => Some(Self::Never),
            "on-failure" => Some(Self::OnFailure),
            "on-request" => Some(Self::OnRequest),
            "untrusted" => Some(Self::Untrusted),
            _ => None,
        }
    }
}

/// How the host shell runs commands for one agent.
#[derive(Clone, Debug)]
pub struct HostShellConfig {
    /// What a command may touch.
    pub containment: ContainmentMode,
    /// Who is asked, and when.
    pub approval: ApprovalPolicy,
    /// Directories writable beyond the workspace, `/tmp`, and private temp.
    pub writable_roots: Vec<PathBuf>,
    /// How the Linux helper process is launched.
    pub launcher: SandboxLauncher,
    /// Wall clock for a foreground call.
    pub foreground_timeout_secs: u64,
    /// Wall clock for a detached call.
    pub background_timeout_secs: u64,
    /// Bytes captured per stream before the command is killed.
    pub max_output_bytes: usize,
}

impl Default for HostShellConfig {
    fn default() -> Self {
        Self {
            containment: ContainmentMode::WorkspaceWrite,
            approval: ApprovalPolicy::Never,
            writable_roots: Vec::new(),
            launcher: SandboxLauncher::Discover,
            // Two minutes is long enough for a build step and short enough that
            // a hung command does not hold a turn open; anything waiting on an
            // external event belongs in a detached call instead.
            foreground_timeout_secs: 120,
            background_timeout_secs: 24 * 60 * 60,
            max_output_bytes: 1024 * 1024,
        }
    }
}

impl HostShellConfig {
    /// Read the agent-facing JSON config.
    ///
    /// Unknown containment or approval names are errors: a misspelled boundary
    /// must not resolve to a default that is wider than what was written.
    pub fn from_json(config: &Value) -> Result<Self, String> {
        let mut parsed = Self::default();
        if config.is_null() {
            return Ok(parsed);
        }
        let Some(object) = config.as_object() else {
            return Err("host_shell config must be an object".to_string());
        };

        for (key, value) in object {
            match key.as_str() {
                "containment" => {
                    let name = value
                        .as_str()
                        .ok_or_else(|| "containment must be a string".to_string())?;
                    parsed.containment = ContainmentMode::parse(name).ok_or_else(|| {
                        format!(
                            "unknown containment `{name}`; expected read-only, workspace-write, \
                             or danger-full-access"
                        )
                    })?;
                }
                "approval" => {
                    let name = value
                        .as_str()
                        .ok_or_else(|| "approval must be a string".to_string())?;
                    parsed.approval = ApprovalPolicy::parse(name).ok_or_else(|| {
                        format!(
                            "unknown approval `{name}`; expected never, on-failure, on-request, \
                             or untrusted"
                        )
                    })?;
                }
                "writable_roots" => {
                    let roots = value
                        .as_array()
                        .ok_or_else(|| "writable_roots must be an array".to_string())?;
                    parsed.writable_roots = roots
                        .iter()
                        .map(|root| {
                            root.as_str()
                                .map(PathBuf::from)
                                .ok_or_else(|| "writable_roots entries must be strings".to_string())
                        })
                        .collect::<Result<_, _>>()?;
                }
                "foreground_timeout_secs" => {
                    parsed.foreground_timeout_secs = positive_u64(key, value)?;
                }
                "background_timeout_secs" => {
                    parsed.background_timeout_secs = positive_u64(key, value)?;
                }
                "max_output_bytes" => {
                    parsed.max_output_bytes = positive_u64(key, value)? as usize;
                }
                "launcher" => parsed.launcher = parse_launcher(value)?,
                other => return Err(format!("unknown host_shell config key: {other}")),
            }
        }
        Ok(parsed)
    }

    /// The containment options this config asks for.
    pub fn sandbox_options(&self) -> SandboxOptions {
        let mut options = SandboxOptions::new(self.containment).launcher(self.launcher.clone());
        for root in &self.writable_roots {
            options = options.writable_root(root);
        }
        options
    }

    /// The timeout for a call, detached or not.
    pub fn timeout_secs(&self, background: bool) -> u64 {
        if background {
            self.background_timeout_secs
        } else {
            self.foreground_timeout_secs
        }
    }
}

/// Read a [`SandboxLauncher`] from config.
///
/// A process-level fact rather than an agent-level one, but the capability
/// config is the channel it has, and every agent in one process gives the same
/// answer. It is here because cargo does not build a dependency's binaries: a
/// single-binary embedder has no `everruns-sandbox-exec` beside it, and
/// `{"reexec_self": [...]}` is how it says "route those arguments back into
/// `containment::worker::run_from_args` in my own `main`".
fn parse_launcher(value: &Value) -> Result<SandboxLauncher, String> {
    if let Some(name) = value.as_str() {
        return match name {
            "discover" => Ok(SandboxLauncher::Discover),
            other => Err(format!(
                "unknown launcher `{other}`; expected discover, or an object with helper or \
                 reexec_self"
            )),
        };
    }
    let Some(object) = value.as_object() else {
        return Err("launcher must be a string or an object".to_string());
    };
    match object
        .iter()
        .next()
        .map(|(key, value)| (key.as_str(), value))
    {
        Some(("helper", path)) if object.len() == 1 => path
            .as_str()
            .map(|path| SandboxLauncher::Helper(PathBuf::from(path)))
            .ok_or_else(|| "launcher.helper must be a string".to_string()),
        Some(("reexec_self", arguments)) if object.len() == 1 => arguments
            .as_array()
            .ok_or_else(|| "launcher.reexec_self must be an array".to_string())?
            .iter()
            .map(|argument| {
                argument
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "launcher.reexec_self entries must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(SandboxLauncher::ReexecSelf),
        _ => Err("launcher object must have exactly one of helper or reexec_self".to_string()),
    }
}

fn positive_u64(key: &str, value: &Value) -> Result<u64, String> {
    match value.as_u64() {
        Some(0) | None => Err(format!("{key} must be a positive integer")),
        Some(number) => Ok(number),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_absent_config_is_the_contained_default() {
        let parsed = HostShellConfig::from_json(&Value::Null).expect("defaults");
        assert_eq!(parsed.containment, ContainmentMode::WorkspaceWrite);
        assert_eq!(parsed.approval, ApprovalPolicy::Never);
        assert!(parsed.writable_roots.is_empty());
    }

    #[test]
    fn every_field_round_trips() {
        let parsed = HostShellConfig::from_json(&json!({
            "containment": "read-only",
            "approval": "on-request",
            "writable_roots": ["/cache"],
            "foreground_timeout_secs": 30,
            "background_timeout_secs": 600,
            "max_output_bytes": 4096,
        }))
        .expect("parses");

        assert_eq!(parsed.containment, ContainmentMode::ReadOnly);
        assert_eq!(parsed.approval, ApprovalPolicy::OnRequest);
        assert_eq!(parsed.writable_roots, vec![PathBuf::from("/cache")]);
        assert_eq!(parsed.timeout_secs(false), 30);
        assert_eq!(parsed.timeout_secs(true), 600);
        assert_eq!(parsed.max_output_bytes, 4096);
    }

    #[test]
    fn a_misspelled_boundary_is_an_error_rather_than_a_default() {
        for config in [
            json!({"containment": "workspace_write"}),
            json!({"containment": "none"}),
            json!({"approval": "always"}),
            json!({"sandbox": "read-only"}),
            json!({"foreground_timeout_secs": 0}),
        ] {
            let error = HostShellConfig::from_json(&config).expect_err("rejected");
            assert!(!error.is_empty(), "the error must say what was wrong");
        }
    }

    #[test]
    fn every_launcher_shape_round_trips() {
        assert_eq!(
            HostShellConfig::from_json(&json!({"launcher": "discover"}))
                .expect("parses")
                .launcher,
            SandboxLauncher::Discover
        );
        assert_eq!(
            HostShellConfig::from_json(&json!({"launcher": {"helper": "/usr/bin/sbx"}}))
                .expect("parses")
                .launcher,
            SandboxLauncher::Helper(PathBuf::from("/usr/bin/sbx"))
        );
        assert_eq!(
            HostShellConfig::from_json(&json!({"launcher": {"reexec_self": ["__sandbox-exec"]}}))
                .expect("parses")
                .launcher,
            SandboxLauncher::ReexecSelf(vec!["__sandbox-exec".to_string()])
        );
    }

    #[test]
    fn an_ambiguous_or_unknown_launcher_is_refused() {
        for config in [
            json!({"launcher": "reexec"}),
            json!({"launcher": {"helper": "/a", "reexec_self": ["b"]}}),
            json!({"launcher": {}}),
            json!({"launcher": 7}),
        ] {
            assert!(HostShellConfig::from_json(&config).is_err(), "{config}");
        }
    }

    #[test]
    fn approval_policies_round_trip_through_their_wire_names() {
        for policy in [
            ApprovalPolicy::Never,
            ApprovalPolicy::OnFailure,
            ApprovalPolicy::OnRequest,
            ApprovalPolicy::Untrusted,
        ] {
            assert_eq!(ApprovalPolicy::parse(policy.as_str()), Some(policy));
        }
    }

    #[test]
    fn configured_roots_reach_the_sandbox_options() {
        let parsed = HostShellConfig::from_json(&json!({"writable_roots": ["/cache", "/state"]}))
            .expect("parses");
        let options = parsed.sandbox_options();
        assert_eq!(options.mode(), ContainmentMode::WorkspaceWrite);
        assert_eq!(options.writable_roots().len(), 2);
    }
}
