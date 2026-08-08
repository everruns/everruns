//! EVE-839 guard: `everruns-core` must not name the hosted `PlatformStore`.
//!
//! The hosted store seam moved to `everruns-platform`; core depends only on the
//! narrow `SubagentSessionDelegate`. This test scans core's own source (skipping
//! comment lines, which may reference the platform type in prose/doc links) and
//! fails if the `PlatformStore` symbol reappears in code — the regression this
//! refactor exists to prevent.

use std::fs;
use std::path::Path;

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, hits);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let text = fs::read_to_string(&path).unwrap();
            for (i, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                // Skip line comments and doc comments — prose may reference the
                // platform type by name (e.g. doc links to everruns-platform).
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("PlatformStore") {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn core_source_has_no_platform_store_symbol() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    scan(&src, &mut hits);
    assert!(
        hits.is_empty(),
        "everruns-core must not use the `PlatformStore` symbol (it lives in \
         everruns-platform; core uses the narrow `SubagentSessionDelegate`). \
         Offending lines:\n{}",
        hits.join("\n")
    );
}
