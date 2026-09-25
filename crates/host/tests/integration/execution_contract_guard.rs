// EVE-872: dependency/source guards for the host execution contract.
//
// `crates/host/src/host.rs` owns the public host execution contract
// (`RuntimeHostAdapter`, `ResolvedTurnInputs`, `execute_*_activity`). Host
// turn execution must consume the resolved execution snapshot, never stored
// platform record aggregates, so this guard fails when the contract module
// names the `Agent`, `Harness`, or `Session` record types again.
//
// The seeding surfaces (`InProcessRuntimeBuilder::harness/agent/session`,
// `builders.rs`) intentionally keep accepting records: they are host
// configuration compatibility APIs, not the execution contract, and are
// removed separately.

use std::fs;
use std::path::Path;

/// Strip `//` line comments (including doc comments) and string literals so
/// the identifier scan only sees code. A simple state machine is enough for
/// this guard; it does not need to handle every Rust edge case, only avoid
/// false positives from prose.
fn strip_comments_and_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut in_line_comment = false;
    while let Some(c) = chars.next() {
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                out.push('\n');
            }
            continue;
        }
        if in_string {
            if c == '\\' {
                chars.next();
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'/') => in_line_comment = true,
            '"' => in_string = true,
            _ => out.push(c),
        }
    }
    out
}

#[test]
fn execution_contract_does_not_name_platform_record_types() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let contract =
        fs::read_to_string(crate_root.join("src/host.rs")).expect("host execution contract source");
    let code = strip_comments_and_strings(&contract);

    let forbidden = ["Agent", "Harness", "Session"];
    let mut identifier = String::new();
    let flush = |identifier: &mut String| {
        if forbidden.contains(&identifier.as_str()) {
            panic!(
                "crates/host/src/host.rs names platform record type `{identifier}`; \
                 the host execution contract must consume ResolvedExecutionSnapshot \
                 instead of stored record aggregates (EVE-872)"
            );
        }
        identifier.clear();
    };
    for c in code.chars() {
        if c.is_alphanumeric() || c == '_' {
            identifier.push(c);
        } else {
            flush(&mut identifier);
        }
    }
    flush(&mut identifier);
}

#[test]
fn host_does_not_depend_on_control_plane_or_database_crates() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(crate_root.join("Cargo.toml")).expect("host manifest");
    for forbidden in ["everruns-server", "everruns-worker", "sqlx"] {
        assert!(
            !manifest.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with(&format!("{forbidden} "))
                    || line.starts_with(&format!("{forbidden}="))
                    || line.starts_with(&format!("{forbidden}."))
            }),
            "everruns-host must not depend on {forbidden}"
        );
    }
}
