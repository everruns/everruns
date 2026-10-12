// Route coverage for personal access token scopes
// (knowledge/security/pat-scopes.md, Tests item 2).
//
// The universe is the OpenAPI document (`ApiDoc`, which already folds in the
// command-backed operations from `add_command_paths`); `openapi_coverage_test`
// keeps it equal to the annotated handlers. Every `(method, path)` in it must
// resolve to a class, from the command inventory or the hand-written table.
// Every table row must name a real route, so there are no stale rows, and
// every non-GET `Read` row must be in the reviewed list. A new route without a
// classification fails here; if it shipped anyway, a narrow token would be
// denied it, never let through.
//
// Run with: cargo test -p everruns-server --test domain pat_scope_coverage_test::

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use axum::http::Method;
use everruns_server::auth::pat_scope::{
    RouteAccess, classify, http_command_at, normalize_template, routes,
};
use everruns_server::openapi::ApiDoc;
use utoipa::OpenApi;

type Key = (String, String);

fn key(method: &str, path: &str) -> Key {
    (method.to_string(), normalize_template(path))
}

fn method(name: &str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("valid method")
}

/// `(method, path)` of every operation in the OpenAPI document.
fn universe() -> BTreeSet<(String, String)> {
    let doc = ApiDoc::openapi();
    let mut out = BTreeSet::new();
    for (path, item) in &doc.paths.paths {
        for (name, op) in [
            ("GET", &item.get),
            ("POST", &item.post),
            ("PUT", &item.put),
            ("PATCH", &item.patch),
            ("DELETE", &item.delete),
            ("HEAD", &item.head),
            ("OPTIONS", &item.options),
            ("TRACE", &item.trace),
        ] {
            if op.is_some() {
                out.insert((name.to_string(), path.clone()));
            }
        }
    }
    assert!(out.len() > 100, "OpenAPI universe is suspiciously small");
    out
}

fn universe_keys() -> BTreeSet<Key> {
    universe().iter().map(|(m, p)| key(m, p)).collect()
}

#[test]
fn every_documented_route_has_a_classification() {
    let missing: Vec<String> = universe()
        .into_iter()
        .filter(|(m, p)| classify(&method(m), p).is_none())
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    assert!(
        missing.is_empty(),
        "Routes without a personal access token scope class. Add a row to \
         crates/server/src/auth/pat_scope/routes.rs (ROUTES) deciding Read, \
         Write or Never; a narrow token is denied an unclassified route. \
         Missing: {missing:#?}"
    );
}

#[test]
fn table_rows_name_documented_routes_once() {
    let universe = universe_keys();
    let mut seen: HashMap<Key, RouteAccess> = HashMap::new();
    for (m, p, access) in routes::ROUTES {
        let k = key(m, p);
        assert!(
            universe.contains(&k),
            "stale ROUTES row {m} {p}: no such route in the OpenAPI document"
        );
        assert!(seen.insert(k, *access).is_none(), "duplicate row {m} {p}");
        if http_command_at(&method(m), p).is_some() {
            assert_eq!(
                *access,
                RouteAccess::Never,
                "{m} {p} is command-backed: it takes its class from the command; \
                 only a Never override may be listed"
            );
        }
    }
}

/// Undocumented rows are for routes outside the OpenAPI document. Each must
/// stay outside it (a documented route belongs in `ROUTES`) and must still be
/// registered: its path is checked syntactically against the router source.
#[test]
fn undocumented_rows_name_registered_routes_outside_the_document() {
    let universe = universe_keys();
    let documented: BTreeSet<Key> = routes::ROUTES.iter().map(|(m, p, _)| key(m, p)).collect();
    let source = server_source();
    for (m, p, _) in routes::UNDOCUMENTED_ROUTES {
        let k = key(m, p);
        assert!(
            !universe.contains(&k),
            "{m} {p} is in the OpenAPI document; move it to ROUTES"
        );
        assert!(!documented.contains(&k), "{m} {p} is listed twice");
        let literal = format!("\"{p}\"");
        assert!(
            source.contains(&literal),
            "stale UNDOCUMENTED_ROUTES row {m} {p}: no route literal {literal} \
             in crates/server/src"
        );
    }
}

#[test]
fn non_get_read_rows_are_reviewed() {
    let reviewed: BTreeSet<Key> = routes::REVIEWED_NON_GET_READ
        .iter()
        .map(|(m, p)| key(m, p))
        .collect();
    let mut read_non_get = BTreeSet::new();
    for (m, p, access) in routes::ROUTES.iter().chain(routes::UNDOCUMENTED_ROUTES) {
        if *access == RouteAccess::Read && *m != "GET" {
            let k = key(m, p);
            assert!(
                reviewed.contains(&k),
                "{m} {p} is marked Read but is not in REVIEWED_NON_GET_READ. A \
                 non-GET route is readable only after review that it changes nothing."
            );
            read_non_get.insert(k);
        }
    }
    let stale: Vec<_> = reviewed.difference(&read_non_get).collect();
    assert!(
        stale.is_empty(),
        "REVIEWED_NON_GET_READ rows with no matching Read row: {stale:?}"
    );
}

#[test]
fn per_command_class_is_only_the_command_endpoint() {
    for (m, p, access) in routes::ROUTES.iter().chain(routes::UNDOCUMENTED_ROUTES) {
        if *access == RouteAccess::PerCommand {
            assert_eq!(
                (*m, *p),
                ("POST", "/v1/commands/{name}"),
                "only the command endpoint defers to the command"
            );
        }
    }
}

fn server_source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = String::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && !path.ends_with("pat_scope/routes.rs")
            {
                out.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}
