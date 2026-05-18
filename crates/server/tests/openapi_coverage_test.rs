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
    for (_path, item) in doc.paths.paths.iter() {
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
