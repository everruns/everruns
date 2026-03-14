// Ensure binaries using `sqlx::migrate!("./migrations")` rebuild when the
// migration set changes. Otherwise cached release artifacts can embed a stale
// migration list and fail startup after `sqlx migrate run` applies newer files.

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    track_dir(Path::new("migrations"));
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
