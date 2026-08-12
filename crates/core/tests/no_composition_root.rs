//! EVE-887 composition-root guard.
//!
//! Selecting which capabilities, drivers and host services a deployment runs
//! with is composition, not kernel execution configuration. The bundle that
//! makes that selection is `everruns_host::HostComposition`; `everruns-core`
//! owns the registries and service contracts it carries, and nothing more.
//!
//! Core previously owned `PlatformDefinition`, and every layer above it —
//! host, local, the Framework facade, server and worker — reached into the
//! kernel for a product-shaped bundle. This test fails the core build if a
//! composition root reappears here, whatever it gets named.

use std::path::Path;

/// Type names that would signal a composition root growing back in the kernel.
const COMPOSITION_ROOT_TYPES: &[&str] = &[
    "PlatformDefinition",
    "PlatformDefinitionBuilder",
    "HostComposition",
    "HostCompositionBuilder",
];

#[test]
fn core_declares_no_composition_root() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();

    for (path, line_no, line) in source_lines(&src) {
        let Some(declared) = declared_type_name(&line) else {
            continue;
        };
        if COMPOSITION_ROOT_TYPES.contains(&declared.as_str()) {
            offenders.push(format!("{}:{line_no}: {}", path.display(), line.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "everruns-core must not declare a composition root — that bundle belongs to \
         the layer that executes a turn (everruns_host::HostComposition, EVE-887):\n{}",
        offenders.join("\n")
    );
}

/// The declared type name when `line` is a public struct/enum/trait item.
///
/// Comments and doc links are ignored: prose that names a moved type documents
/// the boundary rather than recreating it.
fn declared_type_name(line: &str) -> Option<String> {
    let code = line.split_once("//").map_or(line, |(code, _)| code);
    let rest = code
        .trim_start()
        .strip_prefix("pub ")?
        .trim_start()
        .strip_prefix("struct ")
        .or_else(|| {
            code.trim_start()
                .strip_prefix("pub ")?
                .strip_prefix("enum ")
        })
        .or_else(|| {
            code.trim_start()
                .strip_prefix("pub ")?
                .strip_prefix("trait ")
        })?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn source_lines(dir: &Path) -> Vec<(std::path::PathBuf, usize, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (i, line) in text.lines().enumerate() {
                    out.push((path.clone(), i + 1, line.to_string()));
                }
            }
        }
    }
    out
}

#[test]
fn guard_reads_declarations_not_prose() {
    assert_eq!(
        declared_type_name("pub struct HostComposition {").as_deref(),
        Some("HostComposition")
    );
    assert_eq!(
        declared_type_name("pub enum PlatformDefinition {").as_deref(),
        Some("PlatformDefinition")
    );
    assert_eq!(
        declared_type_name("// PlatformDefinition moved to everruns-host"),
        None
    );
    assert_eq!(
        declared_type_name("/// See [`HostComposition`] for the bundle."),
        None
    );
    assert_eq!(declared_type_name("    let composition = build();"), None);
}
