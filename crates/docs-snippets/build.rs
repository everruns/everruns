//! Turn the Framework docs pages into doctest sources.
//!
//! Every `docs/framework/**/*.md` page becomes `OUT_DIR/pages/<page>.rs`: the
//! page, fences normalised for rustdoc, as `///` lines on a unit struct, one
//! line per Markdown line. `OUT_DIR/pages.rs` includes each one in a module
//! named after the page (see `src/lib.rs`), so a failing doctest is reported
//! as `pages/<page>.rs - pages::<page>::Page (line N)` where N is the line in
//! the Markdown page.
//!
//! Fence rules:
//! - A `rust` / `rs` block becomes `rust,no_run`: it must compile, but it is
//!   not executed, because most blocks call a real provider. A block that
//!   already says `ignore`, `no_run`, `compile_fail` or `should_panic` keeps it.
//!   `ignore` is the escape hatch for a block that is deliberately partial.
//! - A fence with no language would be treated as Rust by rustdoc; it becomes
//!   `text`. Other languages are left alone, and rustdoc skips them.
//! - Display-only metadata after the language (`title="main.rs"`) is dropped.

// Build script: a panic here is how a generation failure reaches the build.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Pages whose blocks are not compiled here, with the reason. Remove an entry
/// once the page's blocks compile.
const SKIPPED_PAGES: &[(&str, &str)] = &[
    (
        "a2a.md",
        "blocks need the A2A serve crates; not yet doctest-shaped",
    ),
    (
        "serve-agentcore.md",
        "blocks need everruns-serve-agentcore; not yet doctest-shaped",
    ),
    (
        "serve-celld.md",
        "blocks need everruns-serve-celld; not yet doctest-shaped",
    ),
];

const RUSTDOC_ATTRIBUTES: &[&str] = &["ignore", "no_run", "compile_fail", "should_panic"];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let docs = manifest_dir.join("../../docs/framework");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    println!("cargo:rerun-if-changed={}", docs.display());

    let mut pages = Vec::new();
    collect_pages(&docs, &mut pages);
    pages.sort();

    let pages_dir = out_dir.join("pages");
    std::fs::create_dir_all(&pages_dir).expect("create pages dir");
    let mut generated = String::new();
    for page in pages {
        let relative = page
            .strip_prefix(&docs)
            .expect("page under docs/framework")
            .to_string_lossy()
            .replace('\\', "/");
        println!("cargo:rerun-if-changed={}", page.display());
        if SKIPPED_PAGES
            .iter()
            .any(|(skipped, _)| *skipped == relative)
        {
            continue;
        }
        let source = std::fs::read_to_string(&page).expect("read docs page");
        let ident: String = relative
            .trim_end_matches(".md")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let mut doc = String::new();
        for line in normalise_fences(&source).lines() {
            if line.is_empty() {
                doc.push_str("///\n");
            } else {
                writeln!(doc, "/// {line}").expect("write to string");
            }
        }
        doc.push_str("pub struct Page;\n");
        std::fs::write(pages_dir.join(format!("{ident}.rs")), doc).expect("write page");
        writeln!(
            generated,
            "/// `docs/framework/{relative}`\n\
             pub mod {ident} {{\n    \
                 include!(concat!(env!(\"OUT_DIR\"), \"/pages/{ident}.rs\"));\n\
             }}\n"
        )
        .expect("write to string");
    }
    std::fs::write(out_dir.join("pages.rs"), generated).expect("write pages.rs");
}

fn collect_pages(dir: &Path, pages: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect_pages(&path, pages);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            pages.push(path);
        }
    }
}

fn normalise_fences(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    // The fence that opened the current block (indent + marker), if inside one.
    let mut open: Option<String> = None;
    for line in source.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim_start();
        let indent = &body[..body.len() - trimmed.len()];
        let marker_len = trimmed.chars().take_while(|c| *c == '`').count();
        if marker_len < 3 {
            out.push_str(line);
            continue;
        }
        let marker = &trimmed[..marker_len];
        let info = trimmed[marker_len..].trim();
        match &open {
            Some(opening) => {
                if info.is_empty() && marker.len() >= opening.len() {
                    open = None;
                }
                out.push_str(line);
            }
            None => {
                open = Some(marker.to_string());
                let rewritten = rewrite_info(info);
                writeln!(out, "{indent}{marker}{rewritten}").expect("write to string");
            }
        }
    }
    out
}

fn rewrite_info(info: &str) -> String {
    let mut tokens = info
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty());
    let Some(lang) = tokens.next() else {
        return "text".to_string();
    };
    if !matches!(lang, "rust" | "rs") {
        return info.to_string();
    }
    let attributes: Vec<&str> = tokens
        .filter(|token| RUSTDOC_ATTRIBUTES.contains(token))
        .collect();
    if attributes.is_empty() {
        "rust,no_run".to_string()
    } else {
        format!("rust,{}", attributes.join(","))
    }
}
