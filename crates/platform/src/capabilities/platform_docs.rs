// Embedded Everruns documentation, mounted into the session filesystem by the
// `platform` capability.
//
// Decision: the repo-root `docs/` tree is optionally embedded at compile time
// and served from memory as a virtual readonly mount at /docs. This gives the
// platform chat agent the product documentation without DB writes per session,
// while letting the crate build outside the workspace (where `docs/` is absent)
// with the mount simply missing.
//
// Decision: the mount rode the legacy `platform_management` capability until it
// was retired; it now belongs to `platform`, which is the capability Platform
// Chat actually carries.

#![cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]

use everruns_core::capability_types::VirtualFileTree;
use include_dir::{Dir, include_dir};
use std::sync::Arc;

/// Docs directory embedded at compile time from the repo root `docs/`.
static DOCS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../docs");

fn dir_to_tree(dir: &Dir, base: &str) -> VirtualFileTree {
    let mut tree = VirtualFileTree::new();
    tree.insert_directory(base);
    populate_tree(&mut tree, dir, base);
    tree
}

fn populate_tree(tree: &mut VirtualFileTree, dir: &Dir, prefix: &str) {
    for file in dir.files() {
        let name = file
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // Only include markdown content (skip images, JSON, etc.)
        let ext = file
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "mdx") {
            continue;
        }
        let path = format!("{prefix}/{name}");
        let content = std::str::from_utf8(file.contents()).unwrap_or("");
        tree.insert_text(&path, content);
    }
    for subdir in dir.dirs() {
        let name = subdir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let path = format!("{prefix}/{name}");
        tree.insert_directory(&path);
        populate_tree(tree, subdir, &path);
    }
}

/// Lazily-initialized shared tree (built once on first access).
pub(super) fn docs_tree() -> Arc<VirtualFileTree> {
    use std::sync::OnceLock;
    static TREE: OnceLock<Arc<VirtualFileTree>> = OnceLock::new();
    TREE.get_or_init(|| Arc::new(dir_to_tree(&DOCS_DIR, "/docs")))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_tree_contains_markdown_only() {
        let tree = docs_tree();
        assert!(tree.get("/docs/capabilities/platform.md").is_some());
        assert!(tree.get("/docs/capabilities/citations-ui.png").is_none());
    }
}
