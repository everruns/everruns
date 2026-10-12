// Ensure binaries using `sqlx::migrate!("./migrations")` rebuild when the
// migration set changes. Otherwise cached release artifacts can embed a stale
// migration list and fail startup after `sqlx migrate run` applies newer files.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    track_dir(Path::new("migrations"));
    embed_skills();
}

/// Embed the complete public skill trees. Runtime reads never touch the host
/// filesystem, and a newly added supporting file cannot be left off a manifest.
fn embed_skills() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../skills");
    let mut files = Vec::new();
    // Publish only MCP operating guidance. Framework and contributor skills
    // have different audiences; adding a repo skill must not publish it here.
    for name in ["everruns-mcp", "everruns-agent-authoring"] {
        let dir = root.join(name);
        assert!(
            dir.join("SKILL.md").is_file(),
            "missing public skill: {name}"
        );
        collect_skill_files(&dir, &mut files);
    }
    files.sort();
    let mut source = String::from("&[\n");
    for path in files {
        let relative = path.strip_prefix(&root).unwrap().to_str().unwrap();
        source.push_str(&format!(
            "({relative:?}, include_bytes!({path:?}) as &'static [u8]),\n"
        ));
    }
    source.push_str("]\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("public_skills.rs");
    fs::write(output, source).expect("write embedded public skills");
}

fn collect_skill_files(dir: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).expect("read public skill directory") {
        let entry = entry.expect("read public skill entry");
        let path = entry.path();
        let kind = entry.file_type().expect("read public skill file type");
        assert!(
            !kind.is_symlink(),
            "public skills must not contain symlinks: {}",
            path.display()
        );
        if kind.is_dir() {
            collect_skill_files(&path, files);
        } else if kind.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            files.push(path);
        }
    }
}

fn track_dir(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            track_dir(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
