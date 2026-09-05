// Model-visible path identity conformance (EVE-750).
//
// Shared helpers for asserting that serialized values exposed to the model use
// the active `SessionFileSystem` display identity. The contract is documented
// in `knowledge/runtime-resources/file-store.md` under "Model-visible path identity".

use crate::session_files::SessionFileSystem;
use crate::session_path::WORKSPACE_PREFIX;
use serde_json::Value;

/// Expected path identity for a filesystem backend.
#[derive(Debug, Clone)]
pub struct PathIdentityExpectations {
    /// Canonical visible root (`/workspace` for mounted agent contexts, host path
    /// for direct real-disk integrations).
    pub expected_root: String,
    /// Prefixes that must not appear in model-visible absolute paths.
    pub forbidden_prefixes: Vec<String>,
    /// Additional mount namespaces allowed even when they share a substring with
    /// a forbidden primary-root alias (e.g. `/workspace/roots/backend/`).
    pub allowed_mount_prefixes: Vec<String>,
}

impl PathIdentityExpectations {
    /// Derive expectations from a live store.
    pub fn for_store(store: &dyn SessionFileSystem) -> Self {
        let root = store.display_root();
        if root == WORKSPACE_PREFIX {
            Self::vfs()
        } else {
            Self::host_backed(&root)
        }
    }

    /// In-memory/VFS sessions expose `/workspace`.
    pub fn vfs() -> Self {
        Self {
            expected_root: WORKSPACE_PREFIX.to_string(),
            forbidden_prefixes: vec![],
            allowed_mount_prefixes: vec![],
        }
    }

    /// Direct host-backed integrations expose the real root and must not emit
    /// `/workspace` except under explicit secondary-mount namespaces.
    pub fn host_backed(root: &str) -> Self {
        Self {
            expected_root: root.to_string(),
            forbidden_prefixes: vec![WORKSPACE_PREFIX.to_string()],
            allowed_mount_prefixes: vec![format!("{WORKSPACE_PREFIX}/roots/")],
        }
    }

    fn is_allowed_path(&self, path: &str) -> bool {
        self.allowed_mount_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
    }

    fn violates_forbidden_prefix(&self, path: &str) -> Option<String> {
        if self.is_allowed_path(path) {
            return None;
        }
        self.forbidden_prefixes
            .iter()
            .find(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
            .cloned()
    }
}

/// Whether a string looks like an absolute filesystem path rather than prose,
/// markup, or instructional text.
pub fn looks_like_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.contains(' ')
        && value.len() > 1
        && !value.contains('<')
        && !value.contains('>')
        && !value.contains('`')
}

/// Recursively collect absolute path-like strings from a serialized value.
pub fn collect_absolute_paths(value: &Value, json_path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) if looks_like_absolute_path(text) => {
            out.push((json_path.to_string(), text.clone()));
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_absolute_paths(item, &format!("{json_path}[{index}]"), out);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let child_path = if json_path.is_empty() {
                    key.clone()
                } else {
                    format!("{json_path}.{key}")
                };
                collect_absolute_paths(item, &child_path, out);
            }
        }
        _ => {}
    }
}

/// Assert no collected absolute path violates the forbidden-prefix contract.
///
/// Panics with a descriptive message on violation.
pub fn assert_no_forbidden_prefixes(
    value: &Value,
    expectations: &PathIdentityExpectations,
    context: &str,
) {
    let mut paths = Vec::new();
    collect_absolute_paths(value, "", &mut paths);
    for (json_path, path) in paths {
        if let Some(prefix) = expectations.violates_forbidden_prefix(&path) {
            panic!("{context}: forbidden path prefix `{prefix}` in `{path}` at `{json_path}`");
        }
    }
}

/// Assert every absolute path in `value` is under the expected root or an
/// allowed mount namespace.
pub fn assert_paths_under_expected_root(
    value: &Value,
    expectations: &PathIdentityExpectations,
    context: &str,
) {
    let mut paths = Vec::new();
    collect_absolute_paths(value, "", &mut paths);
    let root = &expectations.expected_root;
    for (json_path, path) in paths {
        if expectations.is_allowed_path(&path) {
            continue;
        }
        let ok = path == root.as_str() || path.starts_with(&format!("{root}/"));
        assert!(
            ok,
            "{context}: path `{path}` at `{json_path}` is not under expected root `{root}`"
        );
    }
}

/// Assert a serialized model-visible value conforms to the store's path identity.
pub fn assert_model_visible_value(value: &Value, store: &dyn SessionFileSystem, context: &str) {
    let expectations = PathIdentityExpectations::for_store(store);
    assert_no_forbidden_prefixes(value, &expectations, context);
    assert_paths_under_expected_root(value, &expectations, context);
}

/// Assert assembled system prompt text uses the expected root and does not
/// advertise forbidden aliases in host-backed mode.
pub fn assert_system_prompt(prompt: &str, expectations: &PathIdentityExpectations) {
    assert!(
        prompt.contains(&format!("Workspace root: `{}`", expectations.expected_root))
            || prompt.contains(&format!(
                "Workspace root: `{}/`",
                expectations.expected_root
            )),
        "system prompt must identify expected root `{}`; got:\n{prompt}",
        expectations.expected_root
    );

    if expectations.expected_root != WORKSPACE_PREFIX {
        for prefix in &expectations.forbidden_prefixes {
            assert!(
                !prompt.contains(&format!("`{prefix}` is also accepted")),
                "host-backed system prompt must not advertise `{prefix}` alias; got:\n{prompt}"
            );
        }
    }
}

/// Assert a tool JSON result conforms to the active store identity.
pub fn assert_tool_result_paths_conform(
    store: &dyn SessionFileSystem,
    tool_name: &str,
    response: &Value,
) {
    assert_model_visible_value(response, store, tool_name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn looks_like_absolute_path_rejects_prose() {
        for text in [
            "A leading '/' or '/workspace/' prefix is also accepted.",
            "/workspace is the root",
            "/",
            "relative/path",
            "",
            "/<root>/file",
            "/root>/file",
            "/`root`/file",
        ] {
            assert!(!looks_like_absolute_path(text), "{text:?}");
        }
        for path in ["/workspace/crates/server", "/tmp/repo/src/lib.rs"] {
            assert!(looks_like_absolute_path(path), "{path}");
        }
    }

    #[test]
    fn collect_absolute_paths_walks_nested_values() {
        let value = json!({
            "path": "/repo/crates/server",
            "entries": [{"path": "/repo/crates/server/main.rs"}, "/repo/README.md"],
            "note": "not a path",
            "ignored": [null, false, 42]
        });
        let mut paths = Vec::new();
        collect_absolute_paths(&value, "", &mut paths);
        paths.sort();
        assert_eq!(
            paths,
            vec![
                (
                    "entries[0].path".to_string(),
                    "/repo/crates/server/main.rs".to_string()
                ),
                ("entries[1]".to_string(), "/repo/README.md".to_string()),
                ("path".to_string(), "/repo/crates/server".to_string()),
            ]
        );
        let mut root_path = Vec::new();
        collect_absolute_paths(&json!("/repo"), "root", &mut root_path);
        assert_eq!(root_path, vec![("root".to_string(), "/repo".to_string())]);
    }

    #[test]
    fn assert_no_forbidden_prefixes_allows_secondary_mounts() {
        let expectations = PathIdentityExpectations::host_backed("/repo");
        let value = json!({ "path": "/workspace/roots/backend/Cargo.toml" });
        assert_no_forbidden_prefixes(&value, &expectations, "secondary mount");
        assert_paths_under_expected_root(&value, &expectations, "secondary mount");
        // Prefix boundaries must not classify a distinct directory as an alias.
        assert_no_forbidden_prefixes(
            &json!("/workspace-other/file"),
            &expectations,
            "distinct prefix",
        );
    }

    #[test]
    #[should_panic(expected = "forbidden path prefix `/workspace`")]
    fn assert_no_forbidden_prefixes_rejects_primary_alias() {
        let expectations = PathIdentityExpectations::host_backed("/repo");
        let value = json!({ "path": "/workspace/crates/server" });
        assert_no_forbidden_prefixes(&value, &expectations, "read_file");
    }

    #[test]
    fn expected_root_assertion_checks_segment_boundaries() {
        let expectations = PathIdentityExpectations::host_backed("/repo");
        for path in [
            "/repo",
            "/repo/src/lib.rs",
            "/workspace/roots/backend/Cargo.toml",
        ] {
            assert_paths_under_expected_root(&json!(path), &expectations, "valid path");
        }
        for path in [
            "/repository/lib.rs",
            "/other/lib.rs",
            "/workspace",
            "/workspace/roots-other/file",
        ] {
            assert!(
                std::panic::catch_unwind(|| {
                    assert_paths_under_expected_root(&json!(path), &expectations, "invalid path");
                })
                .is_err(),
                "accepted {path}"
            );
        }
    }

    #[test]
    fn system_prompt_assertion_requires_root_without_host_alias() {
        let expectations = PathIdentityExpectations::host_backed("/repo");
        for prompt in ["Workspace root: `/repo`", "Workspace root: `/repo/`"] {
            assert_system_prompt(prompt, &expectations);
        }
        assert_system_prompt(
            "Workspace root: `/workspace`",
            &PathIdentityExpectations::vfs(),
        );
        for prompt in [
            "",
            "Workspace root: `/repository`",
            "Workspace root: `/workspace`",
            "Workspace root: `/repo`. `/workspace` is also accepted",
        ] {
            assert!(
                std::panic::catch_unwind(|| assert_system_prompt(prompt, &expectations)).is_err(),
                "accepted {prompt:?}"
            );
        }
    }
}
