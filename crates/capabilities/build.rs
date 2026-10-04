use std::path::Path;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(everruns_has_workspace_docs)");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let docs_dir = Path::new(&manifest_dir).join("../../docs");

    println!("cargo:rerun-if-changed={}", docs_dir.display());
    if docs_dir.is_dir() {
        println!("cargo:rustc-cfg=everruns_has_workspace_docs");
    }
}
