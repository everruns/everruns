// Asserts that every `#[utoipa::path]`-decorated handler under
// `crates/server/src/api/` is registered in `openapi::ApiDoc`. This is the
// invariant that keeps the public OpenAPI spec — which agents consume as a
// tool catalog — in sync with the actual route set. Without this check,
// handlers get annotated but invisibly fall out of the spec (see PR #1834
// follow-up for the 43-handler gap this test now prevents).
//
// The pairing is by handler fn name. utoipa derives `operationId` from the
// function name by default; the same convention is used by domain commands
// (e.g. `create_agent`) so the OpenAPI operationId == MCP `execute` builtin
// name for built-in commands. Locking the contract via explicit
// `operation_id = "..."` annotations is a separate, additive change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use everruns_server::openapi::ApiDoc;
use utoipa::OpenApi;

/// Collect every handler fn name decorated with `#[utoipa::path...]` under
/// `crates/server/src/api/`. The lookup is intentionally syntactic so we
/// don't need to evaluate cfg-gates.
fn declared_handlers() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk(
        &workspace_root().join("crates/server/src/api"),
        &mut |path| {
            let Ok(text) = std::fs::read_to_string(path) else {
                return;
            };
            let mut idx = 0;
            while let Some(found) = text[idx..].find("#[utoipa::path") {
                let after = idx + found + "#[utoipa::path".len();
                // Find the next `fn IDENT` token after the macro invocation.
                if let Some(fn_at) = find_fn_after(&text, after) {
                    out.insert(fn_at);
                }
                idx = after;
            }
        },
    );
    out
}

fn find_fn_after(text: &str, start: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = start;
    while i + 3 < bytes.len() {
        // Look for `fn ` preceded by whitespace and not inside an attribute.
        if &bytes[i..i + 3] == b"fn " && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let mut j = i + 3;
            let name_start = j;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > name_start {
                return Some(text[name_start..j].to_string());
            }
        }
        i += 1;
    }
    None
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `crates/server`; walk two levels up.
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set by cargo test");
    PathBuf::from(manifest)
        .parent()
        .and_then(|p| p.parent())
        .expect("CARGO_MANIFEST_DIR is rooted in <workspace>/crates/server")
        .to_path_buf()
}

fn spec_operation_ids() -> BTreeSet<String> {
    let doc = ApiDoc::openapi();
    let mut out = BTreeSet::new();
    for item in doc.paths.paths.values() {
        for op in [
            item.get.as_ref(),
            item.put.as_ref(),
            item.post.as_ref(),
            item.delete.as_ref(),
            item.options.as_ref(),
            item.head.as_ref(),
            item.patch.as_ref(),
            item.trace.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(id) = &op.operation_id {
                out.insert(id.clone());
            }
        }
    }
    out
}

#[test]
fn every_utoipa_handler_is_registered_in_apidoc() {
    let declared = declared_handlers();
    let registered = spec_operation_ids();
    let missing: Vec<_> = declared.difference(&registered).cloned().collect();
    assert!(
        missing.is_empty(),
        "Handlers decorated with #[utoipa::path] but not registered in \
         openapi::ApiDoc — agents reading the OpenAPI spec cannot see them. \
         Either add them to the `paths(...)` block in \
         crates/server/src/openapi.rs, or remove the unused annotation. \
         Missing: {missing:?}"
    );
}

#[test]
fn endpoint_scoped_ingress_paths_are_documented_with_channel_parameters() {
    let doc = ApiDoc::openapi();
    for path in [
        "/v1/channels/{channel_id}/webhook",
        "/v1/channels/{channel_id}/a2a",
        "/v1/channels/{channel_id}/a2a/.well-known/agent-card.json",
        "/v1/channels/{channel_id}/fcp",
        "/v1/channels/{channel_id}/sessions",
        "/v1/channels/{channel_id}/sessions/{session_id}",
        "/v1/channels/{channel_id}/sessions/{session_id}/messages",
        "/v1/channels/{channel_id}/sessions/{session_id}/cancel",
    ] {
        let item = doc
            .paths
            .paths
            .get(path)
            .unwrap_or_else(|| panic!("missing endpoint-scoped OpenAPI path {path}"));
        let operation = item
            .get
            .as_ref()
            .or(item.post.as_ref())
            .expect("endpoint path has an operation");
        let parameter_names: BTreeSet<_> = operation
            .parameters
            .as_ref()
            .into_iter()
            .flatten()
            // utoipa 6 types operation parameters as `RefOr<Parameter>`. A
            // `$ref` would carry no name here, and silently skipping it would
            // let the `app_id` assertion below pass for the wrong reason.
            .map(|parameter| match parameter {
                utoipa::openapi::RefOr::T(parameter) => parameter.name.as_str(),
                utoipa::openapi::RefOr::Ref(reference) => {
                    panic!(
                        "{path}: unexpected $ref parameter {}",
                        reference.ref_location
                    )
                }
            })
            .collect();
        assert!(parameter_names.contains("channel_id"), "{path}");
        assert!(!parameter_names.contains("app_id"), "{path}");
    }
}

#[test]
fn every_apidoc_operation_id_is_snake_case() {
    // operationId is the agent tool name. snake_case keeps it stable across
    // OpenAPI generators and matches the MCP `execute` builtin convention.
    let ids = spec_operation_ids();
    let bad: Vec<_> = ids
        .iter()
        .filter(|id| {
            !id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                || id.starts_with('_')
                || id.ends_with('_')
                || id.contains("__")
        })
        .cloned()
        .collect();
    assert!(
        bad.is_empty(),
        "operationId values must be lower_snake_case: {bad:?}"
    );
}

/// Commands that declare a REST path the OpenAPI document does not describe.
///
/// Ratchet: this list may only shrink. Migrating a command to
/// `#[command(http = ..)]` documents it; remove its entry then. A command
/// added here is a route agents cannot discover.
const UNDOCUMENTED_COMMAND_ROUTES: &[&str] = &[
    "bulk_update_eval_run_scores",
    "cancel_eval_run",
    "copy_workspace_file",
    "create_eval",
    "create_eval_case",
    "create_eval_run",
    "create_eval_run_share",
    "create_workspace_file",
    "delete_agent_check_rule",
    "delete_eval",
    "delete_eval_case",
    "delete_workspace_file",
    "destroy_agent",
    "destroy_harness",
    "destroy_mcp_server",
    "eval_import_preflight",
    "export_eval_run_artifacts",
    "export_eval_run_dataset",
    "get_eval",
    "get_eval_case",
    "get_eval_run",
    "get_eval_run_dataset",
    "get_eval_run_share",
    "get_workspace_file",
    "grep_workspace_files",
    "import_atif_trajectories",
    "import_eval_run",
    "list_agent_check_rules",
    "list_connection_providers",
    "list_eval_cases",
    "list_eval_runs",
    "list_evals",
    "list_notifications",
    "list_user_connections",
    "list_workspace_files",
    "mark_notification_viewed",
    "move_workspace_file",
    "revoke_eval_run_share",
    "search_workspace_files",
    "stat_workspace_file",
    "update_eval",
    "update_eval_case",
    "update_eval_result_scores",
    "update_workspace_file",
    "upsert_agent_check_rule",
];

/// `METHOD /path` with placeholder names erased, so `{id}` and `{skill_id}`
/// compare equal.
fn route_key(method: &str, path: &str) -> String {
    let mut normalized = String::new();
    let mut in_param = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_param = true;
                normalized.push_str("{}");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => normalized.push(c),
        }
    }
    format!("{} {normalized}", method.to_ascii_uppercase())
}

fn spec_routes() -> BTreeSet<String> {
    let doc = ApiDoc::openapi();
    let mut out = BTreeSet::new();
    for (path, item) in &doc.paths.paths {
        for (method, op) in [
            ("GET", item.get.as_ref()),
            ("PUT", item.put.as_ref()),
            ("POST", item.post.as_ref()),
            ("PATCH", item.patch.as_ref()),
            ("DELETE", item.delete.as_ref()),
        ] {
            if op.is_some() {
                out.insert(route_key(method, path));
            }
        }
    }
    out
}

#[test]
fn every_command_route_is_documented_in_openapi() {
    use everruns_server::domains::common::CommandDescriptor;

    let spec = spec_routes();
    let allowed: BTreeSet<&str> = UNDOCUMENTED_COMMAND_ROUTES.iter().copied().collect();
    let mut missing = Vec::new();
    let mut fixed = Vec::new();
    for desc in inventory::iter::<CommandDescriptor> {
        let meta = (desc.meta)();
        if !meta.path.starts_with("/v1/") {
            continue;
        }
        let documented = spec.contains(&route_key(meta.method, meta.path));
        let listed = allowed.contains(meta.name);
        if !documented && !listed {
            missing.push(meta.name);
        }
        if documented && listed {
            fixed.push(meta.name);
        }
    }
    missing.sort_unstable();
    fixed.sort_unstable();
    assert!(
        missing.is_empty(),
        "Commands with a REST path missing from the OpenAPI document. Declare \
         them with `#[command(http = ..)]` (or annotate the hand-written \
         handler with #[utoipa::path] and register it): {missing:?}"
    );
    assert!(
        fixed.is_empty(),
        "These commands are documented now; remove them from \
         UNDOCUMENTED_COMMAND_ROUTES: {fixed:?}"
    );
}
