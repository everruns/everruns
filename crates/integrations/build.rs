// Generated protocol clients stay behind the owning vendor feature.
#![allow(clippy::unwrap_used, clippy::expect_used)]

fn main() {
    #[cfg(feature = "modal")]
    modal();
    #[cfg(feature = "e2b")]
    e2b();
    #[cfg(feature = "daytona")]
    daytona::generate();
}

#[cfg(feature = "modal")]
fn modal() {
    println!("cargo:rerun-if-changed=proto/modal/api.proto");
    println!("cargo:rerun-if-changed=proto/modal/task_command_router.proto");
    let fds = protox::compile(
        ["modal/api.proto", "modal/task_command_router.proto"],
        ["proto"],
    )
    .expect("failed to compile Modal protos");
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(fds)
        .expect("failed to generate Modal gRPC code");
}

#[cfg(feature = "e2b")]
fn e2b() {
    use prost::Message;
    println!("cargo:rerun-if-changed=proto/e2b/process.proto");
    let fds = protox::compile(["e2b/process.proto"], ["proto"])
        .expect("failed to compile E2B process proto");
    let fds_path =
        std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("e2b.fds.bin");
    std::fs::write(&fds_path, fds.encode_to_vec()).expect("failed to write E2B descriptor set");
    // The descriptor is generated above, not an input. Watching it invalidates
    // every warm build; track the source proto and decoder setting instead.
    println!("cargo:rerun-if-env-changed=BUFFA_ELEMENT_MEMORY_LIMIT");
    connectrpc_build::Config::new()
        .emit_rerun_directives(false)
        .descriptor_set(&fds_path)
        .files(&["e2b/process.proto"])
        .include_file("_e2b.rs")
        .compile()
        .expect("failed to generate E2B ConnectRPC code");
}

/// Daytona clients generated from the vendored OpenAPI specs
/// (`openapi/daytona/`, refreshed by `scripts/update-daytona-openapi.sh`).
///
/// Decisions:
/// - Only the operations listed here are generated. The full specs carry
///   endpoints we do not call (and some the generator cannot express, such
///   as multipart bodies), and a smaller client compiles faster.
/// - Strict in what we send, lenient in what we read. Request bodies keep the
///   spec's required fields and enums, so a renamed or removed field breaks
///   the build. Schemas reachable from a response drop `required` and turn
///   string enums into plain strings: a new sandbox state or an omitted field
///   must not fail a live call that only reads one property.
/// - The management spec's `/api` prefix is stripped so the configured base
///   URL keeps ending in `/api`, as it does for every other caller.
#[cfg(feature = "daytona")]
mod daytona {
    use serde_json::{Map, Value};
    use std::collections::{BTreeSet, VecDeque};

    /// `openapi/daytona/operations.json`: which operations the client calls
    /// (`client`) and which the spec mounted for `daytona_api_call` at
    /// `/daytona/openapi.yaml` describes (`agent`).
    fn operations(group: &str, api: &str) -> Vec<String> {
        let list: Value = serde_json::from_str(
            &std::fs::read_to_string("openapi/daytona/operations.json")
                .expect("read operations.json"),
        )
        .expect("operations.json is JSON");
        list[group][api]
            .as_array()
            .unwrap_or_else(|| panic!("operations.json has no {group}.{api}"))
            .iter()
            .map(|op| op.as_str().expect("operation id").to_string())
            .collect()
    }

    pub fn generate() {
        println!("cargo:rerun-if-changed=openapi/daytona/operations.json");
        agent_spec();
        generate_one(
            "openapi/daytona/api.json",
            &operations("client", "management"),
            Some("/api"),
            "daytona_management.rs",
        );
        generate_one(
            "openapi/daytona/toolbox.json",
            &operations("client", "toolbox"),
            None,
            "daytona_toolbox.rs",
        );
    }

    fn generate_one(src: &str, operations: &[String], strip_prefix: Option<&str>, out: &str) {
        println!("cargo:rerun-if-changed={src}");
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(src).expect("read spec"))
            .expect("spec is JSON");
        let pruned = prune(raw, operations, strip_prefix, true);
        let spec: openapiv3::OpenAPI =
            serde_json::from_value(pruned).expect("pruned spec is OpenAPI 3.0");
        let settings = {
            let mut settings = progenitor::GenerationSettings::default();
            settings.with_interface(progenitor::InterfaceStyle::Builder);
            settings
        };
        let tokens = progenitor::Generator::new(&settings)
            .generate_tokens(&spec)
            .unwrap_or_else(|e| panic!("failed to generate the client from {src}: {e}"));
        let path = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join(out);
        std::fs::write(path, tokens.to_string()).expect("write generated client");
    }

    /// One spec covering both APIs the way `daytona_api_call` routes them:
    /// management paths as Daytona serves them under the API base, toolbox
    /// paths under `/toolbox/{sandboxId}`. JSON is valid YAML, so it keeps the
    /// `openapi.yaml` name agents already know.
    fn agent_spec() {
        let read = |src: &str| -> Value {
            serde_json::from_str(&std::fs::read_to_string(src).expect("read spec"))
                .expect("spec is JSON")
        };
        let management = prune(
            read("openapi/daytona/api.json"),
            &operations("agent", "management"),
            Some("/api"),
            false,
        );
        let toolbox = prune(
            read("openapi/daytona/toolbox.json"),
            &operations("agent", "toolbox"),
            None,
            false,
        );
        // Both specs define same-named schemas with different shapes, so every
        // toolbox schema moves under a `Toolbox` prefix.
        let toolbox: Value = serde_json::from_str(
            &serde_json::to_string(&toolbox)
                .expect("serialize")
                .replace("#/components/schemas/", "#/components/schemas/Toolbox"),
        )
        .expect("reparse");

        let sandbox_param = serde_json::json!({
            "name": "sandboxId",
            "in": "path",
            "required": true,
            "description": "ID of the sandbox",
            "schema": { "type": "string" }
        });
        let mut paths = management["paths"].as_object().cloned().unwrap_or_default();
        for (path, item) in toolbox["paths"].as_object().cloned().unwrap_or_default() {
            let mut item = item;
            for op in item
                .as_object_mut()
                .into_iter()
                .flat_map(|o| o.values_mut())
            {
                if let Some(params) = op["parameters"].as_array_mut() {
                    params.insert(0, sandbox_param.clone());
                } else {
                    op["parameters"] = Value::Array(vec![sandbox_param.clone()]);
                }
                if let Some(id) = op["operationId"].as_str() {
                    op["operationId"] = Value::String(format!("toolbox{id}"));
                }
            }
            paths.insert(format!("/toolbox/{{sandboxId}}{path}"), item);
        }
        let mut schemas = management["components"]["schemas"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for (name, schema) in toolbox["components"]["schemas"]
            .as_object()
            .cloned()
            .unwrap_or_default()
        {
            schemas.insert(format!("Toolbox{name}"), schema);
        }
        let spec = serde_json::json!({
            "openapi": "3.0.3",
            "info": {
                "title": "Daytona API (Management and Toolbox)",
                "version": management["info"]["version"],
                "description": "Generated from Daytona's published specs. Paths under /toolbox/{sandboxId} go to the in-sandbox Toolbox API; every other path goes to the Management API. Authentication is added by the tool: do not pass headers."
            },
            "paths": paths,
            "components": { "schemas": schemas }
        });
        let path = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"))
            .join("daytona_agent_openapi.json");
        std::fs::write(
            path,
            serde_json::to_string_pretty(&spec).expect("serialize agent spec"),
        )
        .expect("write agent spec");
    }

    fn prune(
        mut spec: Value,
        operations: &[String],
        strip_prefix: Option<&str>,
        lenient_responses: bool,
    ) -> Value {
        let wanted: BTreeSet<&str> = operations.iter().map(String::as_str).collect();
        let mut found = BTreeSet::new();
        let mut paths = Map::new();
        let mut request_roots = Vec::new();
        let mut response_roots = Vec::new();
        let source_paths = spec["paths"].as_object().cloned().unwrap_or_default();
        for (path, item) in source_paths {
            let mut kept = Map::new();
            for (method, op) in item.as_object().cloned().unwrap_or_default() {
                let id = op["operationId"].as_str().unwrap_or_default();
                if !wanted.contains(id) {
                    continue;
                }
                found.insert(id.to_string());
                collect_refs(&op["parameters"], &mut request_roots);
                collect_refs(&op["requestBody"], &mut request_roots);
                // Only success responses: errors then reach the caller as raw
                // responses, keeping the status and body text the tools show.
                let mut op = op;
                if let Some(responses) = op["responses"].as_object_mut() {
                    responses.retain(|status, _| status.starts_with('2'));
                }
                collect_refs(&op["responses"], &mut response_roots);
                kept.insert(method, op);
            }
            if !kept.is_empty() {
                let path = match strip_prefix {
                    Some(prefix) => path.strip_prefix(prefix).unwrap_or(&path).to_string(),
                    None => path,
                };
                paths.insert(path, Value::Object(kept));
            }
        }
        let missing: Vec<_> = wanted.iter().filter(|id| !found.contains(**id)).collect();
        assert!(
            missing.is_empty(),
            "operations missing from the vendored Daytona spec: {missing:?}"
        );

        let schemas = spec["components"]["schemas"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let lenient = reachable(&schemas, response_roots.clone());
        let mut all_roots = request_roots;
        all_roots.extend(response_roots);
        let keep = reachable(&schemas, all_roots);
        let mut kept_schemas = Map::new();
        for name in keep {
            let mut schema = schemas[&name].clone();
            if lenient_responses && lenient.contains(&name) {
                relax(&mut schema);
            }
            kept_schemas.insert(name, schema);
        }

        spec["paths"] = Value::Object(paths);
        spec["components"] = serde_json::json!({ "schemas": kept_schemas });
        spec.as_object_mut().map(|o| o.remove("security"));
        spec
    }

    fn collect_refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(r)) = map.get("$ref")
                    && let Some(name) = r.strip_prefix("#/components/schemas/")
                {
                    out.push(name.to_string());
                }
                map.values().for_each(|v| collect_refs(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
            _ => {}
        }
    }

    fn reachable(schemas: &Map<String, Value>, roots: Vec<String>) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<String> = roots.into();
        while let Some(name) = queue.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let mut refs = Vec::new();
            collect_refs(
                schemas
                    .get(&name)
                    .unwrap_or_else(|| panic!("Daytona spec references missing schema {name}")),
                &mut refs,
            );
            queue.extend(refs);
        }
        seen
    }

    fn relax(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("required");
                if map.get("type").and_then(Value::as_str) == Some("string") {
                    map.remove("enum");
                }
                if map.contains_key("enum") && !map.contains_key("type") {
                    map.remove("enum");
                    map.insert("type".into(), Value::String("string".into()));
                }
                // Daytona sends `null` for properties the spec types as
                // arrays or objects, so every property may be null.
                if let Some(Value::Object(properties)) = map.get_mut("properties") {
                    for property in properties.values_mut() {
                        nullable(property);
                    }
                }
                map.values_mut().for_each(relax);
            }
            Value::Array(items) => items.iter_mut().for_each(relax),
            _ => {}
        }
    }

    fn nullable(schema: &mut Value) {
        let Value::Object(map) = schema else { return };
        if map.contains_key("$ref") {
            // `nullable` beside a `$ref` is ignored, so wrap the reference.
            let reference = std::mem::take(map);
            map.insert("allOf".into(), Value::Array(vec![Value::Object(reference)]));
        }
        map.insert("nullable".into(), Value::Bool(true));
    }
}
