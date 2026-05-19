// Ratcheting coverage gates for OpenAPI prose — the spec is the agent tool
// catalog, and missing descriptions/examples are exactly what blocks "preflight
// thinking" by an LLM consumer. Each threshold below is a FLOOR: it MAY only
// move up when new annotations land. If you're tempted to lower a number,
// you're regressing the AI-friendly contract — add the description instead.
//
// Run-of-the-mill mechanical lift to raise these floors:
// 1. `./scripts/export-openapi.sh`
// 2. Inspect coverage with `tests/openapi_descriptions_test.rs` (see helpers).
// 3. Add `/// ...` doc comments to the offending types/fields, then
//    re-export and bump the floor here.

use everruns_server::openapi::ApiDoc;
use utoipa::OpenApi;

/// Lowest acceptable coverage. Bump up when you fix a batch.
const MIN_OP_DESC_PCT: u32 = 100;
const MIN_SCHEMA_DESC_PCT: u32 = 88;
const MIN_FIELD_DESC_PCT: u32 = 75;

fn spec_value() -> serde_json::Value {
    serde_json::from_str(&ApiDoc::openapi().to_pretty_json().unwrap())
        .expect("OpenAPI spec is valid JSON")
}

fn op_description_coverage(spec: &serde_json::Value) -> (u32, u32) {
    let mut total = 0u32;
    let mut with_desc = 0u32;
    let paths = spec.get("paths").and_then(|v| v.as_object());
    for item in paths.iter().flat_map(|p| p.values()) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        for (method, op) in obj {
            if !matches!(
                method.as_str(),
                "get" | "post" | "put" | "delete" | "patch" | "options" | "head" | "trace"
            ) {
                continue;
            }
            let Some(op) = op.as_object() else { continue };
            total += 1;
            if op.get("description").is_some() || op.get("summary").is_some() {
                with_desc += 1;
            }
        }
    }
    (with_desc, total)
}

fn schema_description_coverage(spec: &serde_json::Value) -> (u32, u32) {
    let mut total = 0u32;
    let mut with_desc = 0u32;
    let schemas = spec
        .pointer("/components/schemas")
        .and_then(|v| v.as_object());
    for schema in schemas.iter().flat_map(|s| s.values()) {
        total += 1;
        if schema.get("description").is_some() {
            with_desc += 1;
        }
    }
    (with_desc, total)
}

fn field_description_coverage(spec: &serde_json::Value) -> (u32, u32) {
    let mut total = 0u32;
    let mut with_desc = 0u32;
    let schemas = spec
        .pointer("/components/schemas")
        .and_then(|v| v.as_object());
    for schema in schemas.iter().flat_map(|s| s.values()) {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        for field in props.values() {
            total += 1;
            if field.get("description").is_some() {
                with_desc += 1;
            }
        }
    }
    (with_desc, total)
}

fn pct(num: u32, denom: u32) -> u32 {
    if denom == 0 {
        return 100;
    }
    (num * 100) / denom
}

#[test]
fn operation_description_coverage_does_not_regress() {
    let spec = spec_value();
    let (with_desc, total) = op_description_coverage(&spec);
    let coverage = pct(with_desc, total);
    assert!(
        coverage >= MIN_OP_DESC_PCT,
        "Operation description coverage regressed: {with_desc}/{total} = {coverage}%; \
         floor is {MIN_OP_DESC_PCT}%. Add `description = \"...\"` to every \
         `#[utoipa::path]` macro that lacks one, OR add a `///` doc comment \
         above the handler — utoipa picks either up automatically."
    );
}

#[test]
fn schema_description_coverage_does_not_regress() {
    let spec = spec_value();
    let (with_desc, total) = schema_description_coverage(&spec);
    let coverage = pct(with_desc, total);
    assert!(
        coverage >= MIN_SCHEMA_DESC_PCT,
        "Schema description coverage regressed: {with_desc}/{total} = {coverage}%; \
         floor is {MIN_SCHEMA_DESC_PCT}%. Add a `///` doc comment above every \
         `pub struct`/`pub enum` annotated with `#[derive(ToSchema)]`."
    );
}

#[test]
fn field_description_coverage_does_not_regress() {
    let spec = spec_value();
    let (with_desc, total) = field_description_coverage(&spec);
    let coverage = pct(with_desc, total);
    assert!(
        coverage >= MIN_FIELD_DESC_PCT,
        "Field description coverage regressed: {with_desc}/{total} = {coverage}%; \
         floor is {MIN_FIELD_DESC_PCT}%. Add `///` doc comments above struct \
         fields; utoipa surfaces them as schema field descriptions."
    );
}
