// Session path normalization: the `/workspace` display alias ⇄ the canonical
// leading-slash session path (`/src/lib.rs`).
//
// These are host-agnostic *string* helpers shared by the VFS-backed stores, the
// `file_system` capability, and the `SessionFileSystem` display defaults. They
// carry no host-filesystem knowledge. Mapping the virtual namespace onto a real
// host directory (containment, symlink rejection, worktree-root switches) is a
// backend concern and lives with the host-backed store
// (`everruns_host::RealDiskFileStore`), not here — `/workspace` is just the
// model-facing view, resolved into mounts by `MountFs`.

/// The display alias shown to models and UIs for the workspace root.
///
/// `/workspace` is a common cloud-agent convention (DevContainers, Codespaces).
/// It is purely a *view*; addressing is done by [`crate::mount_fs::MountFs`].
pub const WORKSPACE_PREFIX: &str = "/workspace";

/// Compiled filter for [`crate::session_files::SessionFileSystem::grep_files`].
///
/// Patterns containing glob metacharacters use segment-aware glob semantics.
/// A basename-only glob (for example `*.rs`) matches at any depth. Patterns
/// without glob metacharacters retain the legacy substring behavior.
#[derive(Debug, Clone)]
pub struct GrepPathPattern {
    matcher: GrepPathMatcher,
}

#[derive(Debug, Clone)]
enum GrepPathMatcher {
    All,
    Substring(String),
    Glob(globset::GlobMatcher),
}

impl GrepPathPattern {
    pub fn new(pattern: &str) -> crate::error::Result<Self> {
        let normalized = to_session_path(pattern);
        let relative = normalized.trim_start_matches('/');
        if relative.is_empty() {
            return Ok(Self {
                matcher: GrepPathMatcher::All,
            });
        }
        if !relative
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | '{'))
        {
            return Ok(Self {
                matcher: GrepPathMatcher::Substring(relative.to_string()),
            });
        }

        let glob = if relative.contains('/') {
            relative.to_string()
        } else {
            format!("**/{relative}")
        };
        let matcher = globset::GlobBuilder::new(&glob)
            .literal_separator(true)
            .backslash_escape(false)
            .build()
            .map_err(|error| {
                crate::error::AgentLoopError::tool(format!(
                    "invalid grep path_pattern {pattern:?}: {error}"
                ))
            })?
            .compile_matcher();
        Ok(Self {
            matcher: GrepPathMatcher::Glob(matcher),
        })
    }

    pub fn is_match(&self, canonical_path: &str) -> bool {
        let relative = to_session_path(canonical_path);
        let relative = relative.trim_start_matches('/');
        match &self.matcher {
            GrepPathMatcher::All => true,
            GrepPathMatcher::Substring(needle) => relative.contains(needle),
            GrepPathMatcher::Glob(matcher) => matcher.is_match(relative),
        }
    }

    pub fn is_glob(&self) -> bool {
        matches!(&self.matcher, GrepPathMatcher::Glob(_))
    }
}

/// Canonical leading-slash session path from any accepted spelling: collapses
/// repeated slashes, strips the `/workspace` alias, ensures a single leading
/// slash, and trims a trailing slash.
///
/// This is the single normalizer for every session-path surface — the agent
/// (via `MountFs`/the VFS backends) and the control-plane HTTP FS API both route
/// through it, so a path resolves to the same key regardless of entry point.
///
/// Examples: `/workspace` → `/`, `/workspace/a.txt` → `/a.txt`,
/// `/workspacefoo` → `/workspacefoo`, `a.txt` → `/a.txt`, `/sub/dir/` → `/sub/dir`,
/// `/a//b/` → `/a/b`.
pub fn to_session_path(input: &str) -> String {
    let collapsed = collapse_slashes(input.trim());
    let stripped = strip_workspace_alias(&collapsed);
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

/// Collapse runs of `/` into a single `/` (`a//b` → `a/b`). Leaves the rest of
/// the string untouched.
fn collapse_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for ch in s.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push(ch);
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    out
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
    fn to_session_path_collapses_repeated_slashes() {
        assert_eq!(to_session_path("/a//b"), "/a/b");
        assert_eq!(to_session_path("//a///b//"), "/a/b");
        assert_eq!(to_session_path("//workspace//x"), "/x");
        assert_eq!(to_session_path("///"), "/");
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

    #[test]
    fn grep_path_patterns_support_globs_and_legacy_substrings() {
        let cases = [
            ("src/**/*.rs", "/src/lib.rs", true),
            ("src/**/*.rs", "/src/nested/mod.rs", true),
            ("src/**/*.rs", "/docs/lib.rs", false),
            ("**/*", "/notes.txt", true),
            ("**/*", "/src/lib.rs", true),
            ("docs/*", "/docs/readme.md", true),
            ("docs/*", "/docs/nested/guide.md", false),
            ("*.txt", "/notes.txt", true),
            ("*.txt", "/nested/notes.txt", true),
            ("/workspace/src/**/*.rs", "/src/lib.rs", true),
            ("docs", "/my-docs/readme.md", true),
            ("docs", "/src/lib.rs", false),
            ("", "/anything/file", true),
            ("/workspace", "/anything/file", true),
            ("file?.rs", "/src/file1.rs", true),
            ("file?.rs", "/src/file12.rs", false),
            ("[ab].rs", "/src/a.rs", true),
            ("[ab].rs", "/src/c.rs", false),
            ("*.{rs,ts}", "/src/main.ts", true),
            ("*.{rs,ts}", "/src/main.py", false),
        ];
        for (pattern, path, expected) in cases {
            let matcher = GrepPathPattern::new(pattern).unwrap();
            assert_eq!(
                matcher.is_match(path),
                expected,
                "pattern={pattern} path={path}"
            );
        }
        for (pattern, expected_glob) in [
            ("", false),
            ("docs", false),
            ("*.rs", true),
            ("file?.rs", true),
            ("[ab].rs", true),
            ("*.{rs,ts}", true),
        ] {
            assert_eq!(
                GrepPathPattern::new(pattern).unwrap().is_glob(),
                expected_glob,
                "{pattern}"
            );
        }
        assert!(
            GrepPathPattern::new("[unterminated")
                .unwrap_err()
                .to_string()
                .contains("invalid grep path_pattern")
        );
    }
}
