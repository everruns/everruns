// Session path normalization: the `/workspace` display alias ⇄ the canonical
// leading-slash session path (`/src/lib.rs`).
//
// These are host-agnostic *string* helpers shared by the VFS-backed stores, the
// `file_system` capability, and the `SessionFileSystem` display defaults. They
// carry no host-filesystem knowledge. Mapping the virtual namespace onto a real
// host directory (containment, symlink rejection, worktree-root switches) is a
// backend concern and lives with the host-backed store
// (`everruns_runtime::RealDiskFileStore`), not here — `/workspace` is just the
// model-facing view, resolved into mounts by `MountFs`.

/// The display alias shown to models and UIs for the workspace root.
///
/// `/workspace` is a common cloud-agent convention (DevContainers, Codespaces).
/// It is purely a *view*; addressing is done by [`crate::mount_fs::MountFs`].
pub const WORKSPACE_PREFIX: &str = "/workspace";

/// Canonical leading-slash session path from any accepted spelling: strips the
/// `/workspace` alias, ensures a single leading slash, trims a trailing slash.
///
/// Examples: `/workspace` → `/`, `/workspace/a.txt` → `/a.txt`,
/// `/workspacefoo` → `/workspacefoo`, `a.txt` → `/a.txt`, `/sub/dir/` → `/sub/dir`.
pub fn to_session_path(input: &str) -> String {
    let stripped = strip_workspace_alias(input.trim());
    if stripped.is_empty() || stripped == "/" {
        return "/".to_string();
    }
    let mut normalized = if stripped.starts_with('/') {
        stripped.to_string()
    } else {
        format!("/{stripped}")
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

/// Model-facing display for a canonical session path: the `/workspace` alias.
///
/// `/` → `/workspace`, `/a.txt` → `/workspace/a.txt`. This is the default
/// rendering for VFS stores; backends with a different notion of "where files
/// live" (e.g. a host directory) override `display_path`/`display_root`.
pub fn to_display_path(session_path: &str) -> String {
    let canonical = to_session_path(session_path);
    if canonical == "/" {
        WORKSPACE_PREFIX.to_string()
    } else {
        format!("{WORKSPACE_PREFIX}{canonical}")
    }
}

/// Strip the canonical `/workspace` alias only (exact match or `/workspace/`
/// prefix). Returns the remainder, which may or may not have a leading slash.
fn strip_workspace_alias(s: &str) -> &str {
    if s == WORKSPACE_PREFIX {
        return "";
    }
    if let Some(rest) = s.strip_prefix(&format!("{WORKSPACE_PREFIX}/")) {
        return rest;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_session_path_normalizes_alias_and_slashes() {
        assert_eq!(to_session_path("/workspace"), "/");
        assert_eq!(to_session_path("/workspace/test.txt"), "/test.txt");
        assert_eq!(
            to_session_path("/workspace/foo/bar/test.txt"),
            "/foo/bar/test.txt"
        );
        assert_eq!(to_session_path("/test.txt"), "/test.txt");
        // `/workspacefoo` is not the `/workspace` segment.
        assert_eq!(to_session_path("/workspacefoo"), "/workspacefoo");
        assert_eq!(to_session_path("foo.txt"), "/foo.txt");
        assert_eq!(to_session_path("/sub/dir/"), "/sub/dir");
        assert_eq!(to_session_path("  /workspace/x  "), "/x");
    }

    #[test]
    fn to_display_path_adds_alias() {
        assert_eq!(to_display_path("/"), "/workspace");
        assert_eq!(to_display_path("/src/lib.rs"), "/workspace/src/lib.rs");
        // Idempotent over an already-aliased path.
        assert_eq!(
            to_display_path("/workspace/src/lib.rs"),
            "/workspace/src/lib.rs"
        );
    }
}
