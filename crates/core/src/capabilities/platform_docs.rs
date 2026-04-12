// Platform Documentation Capability
//
// Mounts the project docs/ directory as a virtual readonly filesystem at /docs
// in the session. Content is embedded at compile time via include_dir! and
// served from memory — no DB writes per session.
//
// Intended consumer: platform-chat harness, so the platform chat agent can
// answer questions about Everruns using read_file, list_directory, grep,
// and virtual bash (cat, ls, grep).

use super::{Capability, CapabilityStatus, MountPoint};
use crate::capability_types::{MountAccess, MountSource, VirtualFileTree};
use include_dir::{Dir, include_dir};
use std::sync::Arc;

/// Docs directory embedded at compile time from the repo root `docs/`.
static DOCS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../docs");

/// Build a `VirtualFileTree` from an `include_dir::Dir`, mounting under `base`.
fn dir_to_tree(dir: &Dir, base: &str) -> VirtualFileTree {
    let mut tree = VirtualFileTree::new();
    // Register the root directory itself
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
        // Skip non-text files (images, JSON blobs, etc.)
        let ext = file
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "mdx") {
            continue;
        }
        let path = format!("{}/{}", prefix, name);
        let content = std::str::from_utf8(file.contents()).unwrap_or("");
        tree.insert_text(&path, content);
    }
    for subdir in dir.dirs() {
        let name = subdir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let path = format!("{}/{}", prefix, name);
        tree.insert_directory(&path);
        populate_tree(tree, subdir, &path);
    }
}

/// Lazily-initialized shared tree (built once on first access).
fn docs_tree() -> Arc<VirtualFileTree> {
    use std::sync::OnceLock;
    static TREE: OnceLock<Arc<VirtualFileTree>> = OnceLock::new();
    TREE.get_or_init(|| Arc::new(dir_to_tree(&DOCS_DIR, "/docs")))
        .clone()
}

pub struct PlatformDocsCapability;

impl Capability for PlatformDocsCapability {
    fn id(&self) -> &str {
        "platform_docs"
    }

    fn name(&self) -> &str {
        "Platform Documentation"
    }

    fn description(&self) -> &str {
        "Mounts Everruns platform documentation as a virtual read-only filesystem at /docs. Agents can browse, read, and search the docs using standard file tools and bash commands."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("book-open")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"<platform-docs>
Platform documentation is available at /workspace/docs in the session filesystem.
Use `read_file`, `list_directory`, or `grep` to browse and search it.
Virtual bash commands like `cat /workspace/docs/...`, `ls /workspace/docs/`, and
`grep -r "pattern" /workspace/docs/` also work.

Key sections:
- /workspace/docs/getting-started/ — Introduction, concepts, architecture, Docker setup
- /workspace/docs/features/ — SDK, CLI, UI, events, harnesses, capabilities, apps, skills
- /workspace/docs/capabilities/ — Per-capability reference (file-system, virtual-bash, web-fetch, etc.)
- /workspace/docs/integrations/ — External integrations (Slack, Daytona, Browserless, etc.)
- /workspace/docs/advanced/ — Budgets, compaction, embedding, network access, request signing
- /workspace/docs/sre/ — Environment variables, admin container, runbooks

When the user asks about Everruns features, configuration, or how things work,
consult these docs before answering.
</platform-docs>"#,
        )
    }

    fn mounts(&self) -> Vec<MountPoint> {
        vec![MountPoint::new(
            "/docs",
            MountAccess::ReadOnly,
            MountSource::Virtual { tree: docs_tree() },
            self.id(),
        )]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_has_files() {
        let tree = docs_tree();
        // Should have at least the getting-started/introduction.md
        let intro = tree.get("/docs/getting-started/introduction.md");
        assert!(
            intro.is_some(),
            "docs/getting-started/introduction.md should be embedded"
        );
        assert!(!intro.unwrap().content.is_empty());
    }

    #[test]
    fn tree_has_directories() {
        let tree = docs_tree();
        let dir = tree.get("/docs/capabilities");
        assert!(dir.is_some(), "/docs/capabilities directory should exist");
        assert!(dir.unwrap().is_directory);
    }

    #[test]
    fn mounts_returns_virtual() {
        let cap = PlatformDocsCapability;
        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/docs");
        assert!(mounts[0].is_readonly());
        assert!(matches!(&mounts[0].source, MountSource::Virtual { .. }));
    }

    #[test]
    fn system_prompt_mentions_docs() {
        let cap = PlatformDocsCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("/workspace/docs"));
        assert!(prompt.contains("getting-started"));
    }

    #[test]
    fn skips_non_markdown_files() {
        let tree = docs_tree();
        // images and JSON should not be in the tree
        let all_files: Vec<_> = tree.all_files().collect();
        for (path, _) in &all_files {
            assert!(
                path.ends_with(".md") || path.ends_with(".mdx"),
                "non-markdown file found in tree: {}",
                path
            );
        }
    }
}
