//! Isolated sandboxes on Kubernetes: the `kubernetes` managed Sandboxes
//! provider (Sandbox Template target "Isolated sandbox").
//!
//! One pod per sandbox in a namespace the deployment operator runs, with the
//! sandbox guest agent ([`crate::sandbox_guest`]) as its entrypoint for exec
//! and files. Isolation is a deployment setting: `runtimeClassName`
//! `isolated-sandbox` gives each pod its own VM under AWS Nitro Sandboxes on
//! EKS; `gvisor` or `kata` do the same job elsewhere. Without a runtime class
//! the pods are plain containers, which the UI flags as no security boundary
//! for untrusted code.
//!
//! Decisions:
//! - The target is on when the server can reach Kubernetes and
//!   `KUBERNETES_SANDBOX_NAMESPACE` is set ([`KubernetesSandboxSettings`]);
//!   no feature flag. Its credential source is `platform`: the deployment's
//!   own cluster access, never a user connection.
//! - Pause deletes the pod and keeps the workspace volume: compute stops,
//!   files stay, processes do not survive (Nitro Sandboxes cannot pause with
//!   memory). Resume starts a new pod on the same volume. Durability is
//!   `provider_snapshot`: the volume persists across pause, and losing it
//!   loses the workspace.
//! - Egress is enforced at the network, so the sandbox user (root in its VM)
//!   cannot bypass it: a per-sandbox NetworkPolicy allows only the tenant
//!   egress proxy (open), only IP ranges (IP allowlist), or nothing (deny).
//!   Domain allowlists need enforcement at the proxy and are refused until
//!   the proxy can do it ([`spec::pod_egress`]).
//! - A per-organization cap counts running pods only; paused sandboxes hold
//!   no compute.
//!
//! # Example
//!
//! ```
//! use everruns_integrations::kubernetes_sandbox::{SandboxSize, validate_options};
//!
//! assert_eq!(SandboxSize::Small.resources(), ("1", "2Gi"));
//! assert!(validate_options(&serde_json::json!({"size": "medium"})).is_ok());
//! assert!(validate_options(&serde_json::json!({"size": "huge"})).is_err());
//! ```

pub mod gate;
mod provider;
pub mod settings;
pub mod spec;
mod transport;

use serde_json::Value;

pub use provider::{KubernetesSandboxProvider, configured_settings};
pub use settings::{EgressRoute, GuestRoute, KubernetesSandboxSettings};

/// Provider id, and the leased-resource provider.
pub const KUBERNETES_PROVIDER: &str = "kubernetes";
/// Leased-resource type for sandboxes.
pub const KUBERNETES_RESOURCE_TYPE: &str = "sandbox";
/// Where the guest binary lives in every sandbox image (the Dev Container
/// Feature installs it there too).
pub const GUEST_BINARY: &str = "/usr/local/bin/everruns-sandbox-guest";
/// Default workspace inside the sandbox; the workspace volume mounts here.
pub const DEFAULT_WORKSPACE_PATH: &str = "/workspace";
/// The general-purpose sandbox image.
pub const DEFAULT_IMAGE_REPOSITORY: &str = "ghcr.io/everruns/sandbox-dev";
/// The sandbox image with a desktop.
pub const DESKTOP_IMAGE_REPOSITORY: &str = "ghcr.io/everruns/sandbox-desktop";
/// How long a sandbox stays owned after its last refresh.
pub const LEASE_DURATION_SECONDS: u32 = 30 * 60;

/// The default image, tagged like every Everruns release image (`v0.52.0`)
/// with the version the guest protocol came from.
pub fn default_image() -> String {
    format!("{DEFAULT_IMAGE_REPOSITORY}:v{}", env!("CARGO_PKG_VERSION"))
}

/// The desktop image for this version.
pub fn desktop_image() -> String {
    format!("{DESKTOP_IMAGE_REPOSITORY}:v{}", env!("CARGO_PKG_VERSION"))
}

/// Sandbox sizes. Requests equal limits, so a sandbox gets exactly its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxSize {
    Small,
    Medium,
    Large,
}

impl SandboxSize {
    pub const ALL: [SandboxSize; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            "large" => Some(Self::Large),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// `(cpu, memory)` as Kubernetes quantities.
    pub fn resources(self) -> (&'static str, &'static str) {
        match self {
            Self::Small => ("1", "2Gi"),
            Self::Medium => ("2", "4Gi"),
            Self::Large => ("4", "8Gi"),
        }
    }
}

/// Template options (`target.options`), parsed with the provider's rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub size: SandboxSize,
    /// `None` uses the deployment's image (or the desktop image).
    pub image: Option<String>,
    pub workspace_path: String,
}

fn optional_str<'a>(options: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match options.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= 256 => {
            Ok(Some(value.as_str()))
        }
        Some(_) => Err(format!(
            "option '{key}' must be a non-empty string no longer than 256 bytes"
        )),
    }
}

fn is_normalized_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.ends_with('/')
        && !path.contains("//")
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

/// An image reference: registry/path[:tag][@digest], no spaces or shell.
fn valid_image(image: &str) -> bool {
    !image.starts_with(['-', '/', ':', '@'])
        && image
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':' | '@'))
}

/// Parse `target.options`. The server calls this when a template is saved,
/// so a template never holds options creation would refuse. Deployment
/// limits (allowed sizes and images) are checked again at creation.
pub fn parse_options(options: &Value) -> Result<Options, String> {
    let size = match optional_str(options, "size")? {
        None => SandboxSize::Small,
        Some(size) => SandboxSize::parse(size)
            .ok_or_else(|| "option 'size' must be small, medium or large".to_string())?,
    };
    let image = optional_str(options, "image")?;
    if let Some(image) = image
        && !valid_image(image)
    {
        return Err(format!(
            "option 'image' is not an image reference: '{image}'"
        ));
    }
    let workspace_path = match optional_str(options, "workspace_path")? {
        None => DEFAULT_WORKSPACE_PATH.to_string(),
        Some(path) if is_normalized_absolute_path(path) => path.to_string(),
        Some(_) => {
            return Err(
                "option 'workspace_path' must be a normalized absolute non-root path".to_string(),
            );
        }
    };
    Ok(Options {
        size,
        image: image.map(str::to_string),
        workspace_path,
    })
}

/// Check Sandbox Template `target.options` with the provider's own rules.
pub fn validate_options(options: &Value) -> Result<(), String> {
    parse_options(options).map(|_| ())
}

inventory::submit! {
    everruns_contracts::session_sandbox::SessionSandboxProviderPlugin {
        factory: || Box::new(KubernetesSandboxProvider::from_environment()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn options_default_to_a_small_sandbox_in_workspace() {
        let options = parse_options(&json!({})).unwrap();
        assert_eq!(options.size, SandboxSize::Small);
        assert_eq!(options.image, None);
        assert_eq!(options.workspace_path, "/workspace");
    }

    #[test]
    fn options_refuse_what_creation_would() {
        for bad in [
            json!({"size": "xl"}),
            json!({"size": 2}),
            json!({"image": "ubuntu; rm -rf /"}),
            json!({"image": "-entrypoint"}),
            json!({"workspace_path": "relative"}),
            json!({"workspace_path": "/a/../b"}),
        ] {
            assert!(parse_options(&bad).is_err(), "{bad}");
        }
        assert!(
            parse_options(&json!({"image": "ghcr.io/acme/dev@sha256:abc", "size": "large"}))
                .is_ok()
        );
    }

    #[test]
    fn default_images_carry_this_version() {
        assert!(default_image().ends_with(concat!(":v", env!("CARGO_PKG_VERSION"))));
        assert!(desktop_image().starts_with(DESKTOP_IMAGE_REPOSITORY));
    }
}
