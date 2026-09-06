//! Backend-independent policy for session workspace access.
//!
//! A policy speaks only in the model-facing `/workspace` namespace. Concrete
//! providers remain responsible for mapping that namespace to storage and, for
//! host filesystems, for preventing traversal and symlink escapes during I/O.

use std::fmt;

const SENSITIVE_COMPONENTS: &[&str] = &[
    ".aws",
    ".azure",
    ".docker",
    ".git",
    ".git-credentials",
    ".gnupg",
    ".kube",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".ssh",
    "credentials",
    "credentials.json",
    "gcloud",
];

const DEFAULT_WRITE_DENY_COMPONENTS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    ".tox",
    ".gradle",
];

/// A validated, composable workspace access policy.
///
/// [`Default`] is intentionally read-only: ordinary non-hidden files are
/// readable throughout `/workspace`, while writes, hidden paths (except the
/// framework-managed `.agents` tree), and common credential locations are
/// denied. Use [`WorkspacePolicy::builder`] for
/// narrower scopes or [`WorkspacePolicy::read_write`] for an explicit opt-in
/// to ordinary writes.
///
/// Denies always win over allows. Composed policies are restrictive: an
/// operation must be allowed by every layer, so adding a policy can never
/// broaden access.
///
/// Protected path names are defense in depth, not content-based secret
/// detection. Keep credentials outside the workspace and add application
/// denies for any project-specific secret locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePolicy {
    layers: Vec<PolicyLayer>,
}

/// Builder for a custom [`WorkspacePolicy`].
///
/// A custom builder starts with no readable or writable paths. Hidden and
/// sensitive paths remain protected unless explicitly opted in.
#[derive(Clone, Debug, Default)]
pub struct WorkspacePolicyBuilder {
    read: Vec<String>,
    write: Vec<String>,
    deny_read: Vec<String>,
    deny_write: Vec<String>,
    deny_write_components: Vec<String>,
    hidden: Vec<String>,
    sensitive: Vec<String>,
    recursive_delete: bool,
}

/// Invalid policy configuration or a denied workspace operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePolicyError {
    message: String,
}

impl WorkspacePolicyError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkspacePolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for WorkspacePolicyError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyLayer {
    read: Vec<PolicyPath>,
    write: Vec<PolicyPath>,
    deny_read: Vec<PolicyPath>,
    deny_write: Vec<PolicyPath>,
    deny_write_components: Vec<String>,
    hidden: Vec<PolicyPath>,
    sensitive: Vec<PolicyPath>,
    recursive_delete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyPath(Vec<String>);

impl WorkspacePolicy {
    /// Return the secure default policy.
    ///
    /// Ordinary files are readable, writes are denied, and hidden or common
    /// sensitive paths are inaccessible. The framework-managed `.agents` tree
    /// remains readable so configured skills and instructions continue to work.
    pub fn read_only() -> Self {
        Self {
            layers: vec![PolicyLayer {
                read: vec![PolicyPath::root()],
                write: Vec::new(),
                deny_read: Vec::new(),
                deny_write: Vec::new(),
                deny_write_components: Vec::new(),
                hidden: vec![PolicyPath(vec![".agents".to_string()])],
                sensitive: Vec::new(),
                recursive_delete: false,
            }],
        }
    }

    /// Allow reads and writes throughout the ordinary workspace.
    ///
    /// This is an explicit opt-in. Hidden and common sensitive paths remain
    /// denied, common dependency/build directory names remain non-writable at
    /// every depth, and recursive directory deletion remains disabled. Build a
    /// custom policy to choose different component restrictions.
    pub fn read_write() -> Self {
        Self {
            layers: vec![PolicyLayer {
                read: vec![PolicyPath::root()],
                write: vec![PolicyPath::root()],
                deny_read: Vec::new(),
                deny_write: vec![PolicyPath(vec![".agents".to_string()])],
                deny_write_components: DEFAULT_WRITE_DENY_COMPONENTS
                    .iter()
                    .map(|component| (*component).to_string())
                    .collect(),
                hidden: vec![PolicyPath(vec![".agents".to_string()])],
                sensitive: Vec::new(),
                recursive_delete: false,
            }],
        }
    }

    /// Start a custom, deny-by-default policy.
    pub fn builder() -> WorkspacePolicyBuilder {
        WorkspacePolicyBuilder::default()
    }

    /// Compose an additional restriction with this policy.
    ///
    /// Both policies must allow an operation. This makes composition safe for
    /// libraries that add constraints to an application-supplied policy.
    pub fn compose(mut self, additional: Self) -> Self {
        self.layers.extend(additional.layers);
        self
    }

    /// Whether `path` may be read.
    ///
    /// Invalid paths, including traversal attempts, are denied.
    pub fn permits_read(&self, path: &str) -> bool {
        PolicyPath::parse(path)
            .is_ok_and(|path| self.layers.iter().all(|layer| layer.permits_read(&path)))
    }

    /// Whether `path` may be written, created, or deleted.
    ///
    /// Invalid paths, including traversal attempts, are denied.
    pub fn permits_write(&self, path: &str) -> bool {
        PolicyPath::parse(path)
            .is_ok_and(|path| self.layers.iter().all(|layer| layer.permits_write(&path)))
    }

    /// Whether a directory may be traversed to reach a readable descendant.
    ///
    /// This is useful to filesystem providers implementing filtered directory
    /// listings for a narrowly scoped policy.
    pub fn permits_read_traversal(&self, path: &str) -> bool {
        PolicyPath::parse(path).is_ok_and(|path| {
            self.layers
                .iter()
                .all(|layer| layer.permits_read_traversal(&path))
        })
    }

    /// Validate workspace path syntax without making an access decision.
    ///
    /// Filesystem providers can use this before resolving provider-specific
    /// aliases, then authorize the resolved canonical path with
    /// [`check_read`](Self::check_read) or [`check_write`](Self::check_write).
    pub fn validate_path(path: &str) -> Result<(), WorkspacePolicyError> {
        PolicyPath::parse(path).map(|_| ())
    }

    /// Validate a read and return a diagnostic suitable for a tool error.
    pub fn check_read(&self, path: &str) -> Result<(), WorkspacePolicyError> {
        let normalized = PolicyPath::parse(path)?;
        if self
            .layers
            .iter()
            .all(|layer| layer.permits_read(&normalized))
        {
            Ok(())
        } else {
            Err(WorkspacePolicyError::new(format!(
                "workspace policy denied read of `{}`",
                normalized.display()
            )))
        }
    }

    /// Validate a write, create, or delete and return a tool-safe diagnostic.
    pub fn check_write(&self, path: &str) -> Result<(), WorkspacePolicyError> {
        let normalized = PolicyPath::parse(path)?;
        if self
            .layers
            .iter()
            .all(|layer| layer.permits_write(&normalized))
        {
            Ok(())
        } else {
            Err(WorkspacePolicyError::new(format!(
                "workspace policy denied write to `{}`",
                normalized.display()
            )))
        }
    }

    /// Whether recursive directory deletion is explicitly allowed by every
    /// composed policy layer.
    pub fn permits_recursive_delete(&self) -> bool {
        self.layers.iter().all(|layer| layer.recursive_delete)
    }
}

impl Default for WorkspacePolicy {
    fn default() -> Self {
        Self::read_only()
    }
}

impl WorkspacePolicyBuilder {
    /// Add a readable path scope.
    ///
    /// Paths may be relative to `/workspace`, workspace-absolute
    /// (`/workspace/src`), or session-absolute (`/src`).
    pub fn allow_read(mut self, path: impl Into<String>) -> Self {
        self.read.push(path.into());
        self
    }

    /// Add a writable path scope.
    pub fn allow_write(mut self, path: impl Into<String>) -> Self {
        self.write.push(path.into());
        self
    }

    /// Deny reads at and below a path, even when an allow scope matches.
    pub fn deny_read(mut self, path: impl Into<String>) -> Self {
        self.deny_read.push(path.into());
        self
    }

    /// Deny writes at and below a path, even when an allow scope matches.
    pub fn deny_write(mut self, path: impl Into<String>) -> Self {
        self.deny_write.push(path.into());
        self
    }

    /// Deny writes when any path segment has exactly this name.
    ///
    /// Unlike [`deny_write`](Self::deny_write), this applies at every depth.
    /// It is useful for dependency or build directories such as
    /// `node_modules` and `target`.
    pub fn deny_write_component(mut self, component: impl Into<String>) -> Self {
        self.deny_write_components.push(component.into());
        self
    }

    /// Permit hidden path components at and below a specific scope.
    ///
    /// This does not override the sensitive-path protection. For example,
    /// allowing `.github` is safe without also exposing `.git` or `.env`.
    pub fn allow_hidden(mut self, path: impl Into<String>) -> Self {
        self.hidden.push(path.into());
        self
    }

    /// Permit a common sensitive path at and below a specific scope.
    ///
    /// This is the strongest filesystem-policy opt-in and also permits hidden
    /// components within that scope. Prefer the narrowest possible path.
    pub fn allow_sensitive(mut self, path: impl Into<String>) -> Self {
        self.sensitive.push(path.into());
        self
    }

    /// Explicitly permit recursive directory deletion.
    ///
    /// The default is `false`, because deleting an allowed parent could also
    /// remove denied or sensitive descendants that a lexical path check cannot
    /// see.
    pub fn allow_recursive_delete(mut self, allow: bool) -> Self {
        self.recursive_delete = allow;
        self
    }

    /// Validate every configured scope and build the policy.
    pub fn build(self) -> Result<WorkspacePolicy, WorkspacePolicyError> {
        Ok(WorkspacePolicy {
            layers: vec![PolicyLayer {
                read: parse_paths("read allow", self.read)?,
                write: parse_paths("write allow", self.write)?,
                deny_read: parse_paths("read deny", self.deny_read)?,
                deny_write: parse_paths("write deny", self.deny_write)?,
                deny_write_components: parse_components(
                    "write deny component",
                    self.deny_write_components,
                )?,
                hidden: parse_paths("hidden allow", self.hidden)?,
                sensitive: parse_paths("sensitive allow", self.sensitive)?,
                recursive_delete: self.recursive_delete,
            }],
        })
    }
}

impl PolicyLayer {
    fn permits_read(&self, path: &PolicyPath) -> bool {
        self.read.iter().any(|scope| scope.contains(path))
            && !self
                .deny_read
                .iter()
                .any(|scope| scope.contains_ignoring_ascii_case(path))
            && self.permits_protected_components(path)
    }

    fn permits_read_traversal(&self, path: &PolicyPath) -> bool {
        (self
            .read
            .iter()
            .any(|scope| scope.contains(path) || path.contains(scope)))
            && !self
                .deny_read
                .iter()
                .any(|scope| scope.contains_ignoring_ascii_case(path))
            && self.permits_protected_traversal(path)
    }

    fn permits_write(&self, path: &PolicyPath) -> bool {
        self.write.iter().any(|scope| scope.contains(path))
            && !self
                .deny_write
                .iter()
                .any(|scope| scope.contains_ignoring_ascii_case(path))
            && !path.has_any_component(&self.deny_write_components)
            && self.permits_protected_components(path)
    }

    fn permits_protected_components(&self, path: &PolicyPath) -> bool {
        if self.sensitive.iter().any(|scope| scope.contains(path)) {
            return true;
        }
        if path.has_sensitive_component() {
            return false;
        }
        !path.has_hidden_component() || self.hidden.iter().any(|scope| scope.contains(path))
    }

    fn permits_protected_traversal(&self, path: &PolicyPath) -> bool {
        if path.has_sensitive_component() {
            return self
                .sensitive
                .iter()
                .any(|scope| scope.contains(path) || path.contains(scope));
        }
        !path.has_hidden_component()
            || self
                .hidden
                .iter()
                .any(|scope| scope.contains(path) || path.contains(scope))
    }
}

impl PolicyPath {
    fn root() -> Self {
        Self(Vec::new())
    }

    fn parse(input: &str) -> Result<Self, WorkspacePolicyError> {
        let trimmed = input.trim();
        if trimmed.contains('\0') {
            return Err(WorkspacePolicyError::new(format!(
                "workspace path contains a NUL byte: {input:?}"
            )));
        }
        if trimmed.contains('\\') {
            return Err(WorkspacePolicyError::new(format!(
                "workspace paths must use forward slashes: {input:?}"
            )));
        }

        let canonical = crate::session_path::to_session_path(trimmed);
        let without_alias = canonical.trim_start_matches('/');

        let mut components = Vec::new();
        for component in without_alias.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    return Err(WorkspacePolicyError::new(format!(
                        "workspace path traversal is not allowed: {input:?}"
                    )));
                }
                value => components.push(value.to_string()),
            }
        }
        Ok(Self(components))
    }

    fn contains(&self, candidate: &Self) -> bool {
        // Allows stay case-exact so a scope cannot cover a distinct Unix path.
        candidate.0.starts_with(&self.0)
    }

    fn contains_ignoring_ascii_case(&self, candidate: &Self) -> bool {
        // Denies fail closed for providers where case variants alias one object.
        self.0.len() <= candidate.0.len()
            && self
                .0
                .iter()
                .zip(&candidate.0)
                .all(|(scope, component)| scope.eq_ignore_ascii_case(component))
    }

    fn has_hidden_component(&self) -> bool {
        self.0.iter().any(|component| component.starts_with('.'))
    }

    fn has_any_component(&self, denied: &[String]) -> bool {
        self.0.iter().any(|component| {
            denied
                .iter()
                .any(|denied| denied.eq_ignore_ascii_case(component))
        })
    }

    fn has_sensitive_component(&self) -> bool {
        self.0.iter().any(|component| {
            let component = component.to_ascii_lowercase();
            component == ".env"
                || component.starts_with(".env.")
                || SENSITIVE_COMPONENTS.contains(&component.as_str())
        })
    }

    fn display(&self) -> String {
        if self.0.is_empty() {
            "/workspace".to_string()
        } else {
            format!("/workspace/{}", self.0.join("/"))
        }
    }
}

fn parse_paths(kind: &str, paths: Vec<String>) -> Result<Vec<PolicyPath>, WorkspacePolicyError> {
    paths
        .into_iter()
        .map(|path| {
            if path.trim().is_empty() {
                return Err(WorkspacePolicyError::new(format!(
                    "invalid {kind} scope {path:?}: path must not be empty"
                )));
            }
            PolicyPath::parse(&path).map_err(|error| {
                WorkspacePolicyError::new(format!("invalid {kind} scope {path:?}: {error}"))
            })
        })
        .collect()
}

fn parse_components(
    kind: &str,
    components: Vec<String>,
) -> Result<Vec<String>, WorkspacePolicyError> {
    components
        .into_iter()
        .map(|component| {
            let trimmed = component.trim();
            if trimmed.is_empty()
                || matches!(trimmed, "." | "..")
                || trimmed.contains(['/', '\\', '\0'])
            {
                return Err(WorkspacePolicyError::new(format!(
                    "invalid {kind} {component:?}: expected one path component"
                )));
            }
            Ok(trimmed.to_ascii_lowercase())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_read_only_and_hides_sensitive_paths() {
        let policy = WorkspacePolicy::default();
        assert!(policy.permits_read("/workspace/src/lib.rs"));
        assert!(!policy.permits_write("/workspace/src/lib.rs"));
        assert!(!policy.permits_read("/workspace/.env"));
        assert!(!policy.permits_read("/workspace/.git/config"));
        assert!(policy.permits_read("/workspace/.agents/skills/example/SKILL.md"));
        assert!(!policy.permits_recursive_delete());
    }

    #[test]
    fn read_write_keeps_framework_managed_content_read_only() {
        let policy = WorkspacePolicy::read_write();

        assert!(policy.permits_read("/.agents/skills/example/SKILL.md"));
        assert!(!policy.permits_write("/.agents/skills/example/SKILL.md"));
        assert!(policy.permits_write("/generated/report.md"));
        assert!(!policy.permits_write("/crates/example/target/output"));
        assert!(!policy.permits_write("/web/node_modules/package/index.js"));
    }

    #[test]
    fn custom_component_write_deny_applies_at_every_depth() {
        let policy = WorkspacePolicy::builder()
            .allow_write("/")
            .deny_write_component("vendor")
            .build()
            .unwrap();

        assert!(policy.permits_write("/src/generated.rs"));
        assert!(policy.permits_write("/src/vendorish/generated.rs"));
        assert!(!policy.permits_write("/src/vendor/generated.rs"));
        assert!(!policy.permits_write("/VENDOR/generated.rs"));
    }

    #[test]
    fn deny_wins_over_allow() {
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_write("/workspace/output")
            .deny_read("private")
            .deny_write("output/locked")
            .build()
            .unwrap();

        assert!(policy.permits_read("notes.txt"));
        assert!(!policy.permits_read("private/notes.txt"));
        assert!(policy.permits_write("output/result.txt"));
        assert!(!policy.permits_write("output/locked/result.txt"));
    }

    #[test]
    fn scoped_policy_allows_parent_traversal_but_not_parent_reads() {
        let policy = WorkspacePolicy::builder()
            .allow_read("src/generated")
            .build()
            .unwrap();

        assert!(policy.permits_read_traversal("/workspace"));
        assert!(policy.permits_read_traversal("/workspace/src"));
        assert!(!policy.permits_read("/workspace/src"));
        assert!(policy.permits_read("/workspace/src/generated/file.rs"));
        assert!(!policy.permits_read("/workspace/tests/test.rs"));
        assert!(!policy.permits_read_traversal("/workspace/tests"));
    }

    #[test]
    fn hidden_and_sensitive_paths_require_separate_explicit_opt_ins() {
        let hidden = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_hidden(".github")
            .allow_hidden(".git")
            .build()
            .unwrap();
        assert!(hidden.permits_read(".github/workflows/ci.yml"));
        assert!(!hidden.permits_read(".git/config"));

        let sensitive = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_sensitive(".env.example")
            .build()
            .unwrap();
        assert!(sensitive.permits_read(".env.example"));
        assert!(!sensitive.permits_read(".env"));
    }

    #[test]
    fn narrow_protected_scope_allows_only_ancestor_traversal() {
        let policy = WorkspacePolicy::builder()
            .allow_read(".ssh/id_ed25519")
            .allow_sensitive(".ssh/id_ed25519")
            .build()
            .unwrap();

        assert!(policy.permits_read_traversal("/.ssh"));
        assert!(!policy.permits_read("/.ssh"));
        assert!(policy.permits_read("/.ssh/id_ed25519"));
        assert!(!policy.permits_read("/.ssh/config"));
        assert!(!policy.permits_read_traversal("/.ssh/config"));
    }

    #[test]
    fn composition_can_only_restrict() {
        let application = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_write("/")
            .deny_read("src/private")
            .deny_write("src/generated/locked")
            .allow_recursive_delete(true)
            .build()
            .unwrap();
        let library = WorkspacePolicy::builder()
            .allow_read("src")
            .allow_write("src/generated")
            .build()
            .unwrap();
        for policy in [
            application.clone().compose(library.clone()),
            library.compose(application),
        ] {
            assert!(policy.permits_read("src/lib.rs"));
            assert!(!policy.permits_read("Cargo.toml"));
            assert!(!policy.permits_read("src/private/secret.rs"));
            assert!(policy.permits_write("src/generated/mod.rs"));
            assert!(!policy.permits_write("src/lib.rs"));
            assert!(!policy.permits_write("src/generated/locked/mod.rs"));
            assert!(!policy.permits_recursive_delete());
        }
        let recursive = WorkspacePolicy::builder()
            .allow_recursive_delete(true)
            .build()
            .unwrap();
        assert!(
            recursive
                .clone()
                .compose(recursive)
                .permits_recursive_delete()
        );
    }

    #[test]
    fn traversal_nul_and_platform_separators_fail_closed() {
        let policy = WorkspacePolicy::read_write();
        assert!(policy.permits_read("src/ordinary.txt"));
        assert!(policy.permits_write("src/ordinary.txt"));
        assert!(policy.check_read("src/ordinary.txt").is_ok());
        assert!(policy.check_write("src/ordinary.txt").is_ok());
        for path in [
            "src/../ordinary.txt",
            "../outside",
            "src\\..\\ordinary.txt",
            "bad\0path",
        ] {
            assert!(!policy.permits_read(path), "read: {path:?}");
            assert!(!policy.permits_write(path), "write: {path:?}");
            assert!(!policy.permits_read_traversal(path), "traverse: {path:?}");
            assert!(policy.check_read(path).is_err(), "check_read: {path:?}");
            assert!(policy.check_write(path).is_err(), "check_write: {path:?}");
        }
    }

    #[test]
    fn workspace_and_session_absolute_paths_share_one_namespace() {
        let policy = WorkspacePolicy::builder()
            .allow_read("/workspace/src")
            .allow_write("/output")
            .build()
            .unwrap();
        assert!(policy.permits_read("src/lib.rs"));
        assert!(policy.permits_read("/src/lib.rs"));
        assert!(policy.permits_read("/workspace/src/lib.rs"));
        assert!(policy.permits_write("/workspace/output/report.md"));
    }

    #[test]
    fn repeated_slashes_cannot_bypass_a_deny_scope() {
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .deny_read("private")
            .build()
            .unwrap();
        assert!(policy.permits_read("//workspace//public//readme.txt"));
        assert!(!policy.permits_read("//workspace//private//secret.txt"));
    }

    #[test]
    fn alternate_ascii_case_cannot_bypass_a_deny_or_sensitive_path() {
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_write("/")
            .allow_hidden("/")
            .deny_read("Private")
            .deny_write("Private")
            .build()
            .unwrap();

        assert!(policy.permits_read("/.ordinary"));
        assert!(policy.permits_write("/.ordinary"));
        for path in ["/private/secret.txt", "/.ENV", "/.Git/config"] {
            assert!(!policy.permits_read(path), "read: {path}");
            assert!(!policy.permits_write(path), "write: {path}");
        }
    }

    #[test]
    fn alternate_ascii_case_cannot_broaden_an_allow_scope() {
        let policy = WorkspacePolicy::builder()
            .allow_read("public")
            .allow_write("generated")
            .build()
            .unwrap();

        assert!(policy.permits_read("public/index.html"));
        assert!(!policy.permits_read("Public/secret.txt"));
        assert!(!policy.permits_read_traversal("Public"));
        assert!(policy.permits_write("generated/report.md"));
        assert!(!policy.permits_write("Generated/secret.txt"));
    }

    #[test]
    fn invalid_custom_scope_is_reported_at_build_time() {
        let error = WorkspacePolicy::builder()
            .allow_read("../outside")
            .build()
            .unwrap_err();
        assert!(error.to_string().contains("traversal"));

        let error = WorkspacePolicy::builder()
            .deny_write_component("nested/vendor")
            .build()
            .unwrap_err();
        assert!(error.to_string().contains("one path component"));

        let error = WorkspacePolicy::builder()
            .allow_write("  ")
            .build()
            .unwrap_err();
        assert!(error.to_string().contains("must not be empty"));
    }
}
