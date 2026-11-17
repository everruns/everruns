//! Embed the public `everruns` skill (`skills/everruns/` in the workspace) so
//! `everruns agent-skill` writes the copy that matches this CLI's version.
//!
//! Decision: `skills/` stays the single source of truth
//! (knowledge/project/public-skills.md); this is a build-time copy, like the
//! server's MCP catalog. A crates.io build has no workspace around it, so it
//! embeds nothing and the command points at the published copy instead.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a build script fails the build on a missing env var or unreadable skill file"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(everruns_has_workspace_skill)");
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("../../skills/everruns");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    if root.join("SKILL.md").is_file() {
        println!("cargo:rustc-cfg=everruns_has_workspace_skill");
        collect(&root, &mut files);
    }
    files.sort();
    let mut source = String::from("&[\n");
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        source.push_str(&format!("({relative:?}, include_str!({path:?})),\n"));
    }
    source.push_str("]\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("everruns_skill.rs");
    fs::write(out, source).expect("write embedded everruns skill");
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).expect("read skill directory") {
        let entry = entry.expect("read skill entry");
        let path = entry.path();
        let kind = entry.file_type().expect("read skill file type");
        assert!(!kind.is_symlink(), "skill symlink: {}", path.display());
        if kind.is_dir() {
            collect(&path, files);
        } else if kind.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            files.push(path);
        }
    }
}
