use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(everruns_has_workspace_docs)");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let docs_dir = Path::new(&manifest_dir).join("../../docs");

    println!("cargo:rerun-if-changed={}", docs_dir.display());
    if docs_dir.is_dir() {
        println!("cargo:rustc-cfg=everruns_has_workspace_docs");
    }

    embed_platform_skills(Path::new(&manifest_dir));
}

/// Keep product skills in their authored tree, shared with the MCP catalog.
/// Like embedded docs, workspace skills are absent in standalone crate builds.
fn embed_platform_skills(manifest_dir: &Path) {
    println!("cargo:rustc-check-cfg=cfg(everruns_has_workspace_skills)");
    let root = manifest_dir.join("../../skills");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    if root.is_dir() {
        println!("cargo:rustc-cfg=everruns_has_workspace_skills");
        for name in ["everruns", "everruns-cli", "everruns-agent-authoring"] {
            let dir = root.join(name);
            assert!(
                dir.join("SKILL.md").is_file(),
                "missing platform skill: {name}"
            );
            collect_files(&dir, &mut files);
        }
    }
    files.sort();
    let mut source = String::from("&[\n");
    for path in files {
        let relative = path.strip_prefix(&root).unwrap().to_str().unwrap();
        let content = fs::read_to_string(&path).expect("platform skill files are UTF-8");
        source.push_str(&format!("({relative:?}, {content:?}),\n"));
    }
    source.push_str("]\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("platform_skills.rs");
    fs::write(out, source).expect("write embedded platform skills");
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).expect("read platform skill directory") {
        let entry = entry.expect("read platform skill entry");
        let path = entry.path();
        let kind = entry.file_type().expect("read platform skill file type");
        assert!(
            !kind.is_symlink(),
            "platform skill symlink: {}",
            path.display()
        );
        if kind.is_dir() {
            collect_files(&path, files);
        } else if kind.is_file() {
            files.push(path);
        }
    }
}
