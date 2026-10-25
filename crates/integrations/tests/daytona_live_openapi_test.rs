#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live check that Daytona's OpenAPI specs still match the vendored copies
//! the Daytona client is generated from.
//!
//! Gated like `daytona_live_api_test`: feature `daytona-live-tests` and
//! `DAYTONA_API_KEY` (missing key panics, so CI cannot report false green).

#![cfg(feature = "daytona-live-tests")]

use everruns_integrations::daytona::client::DaytonaClient;

fn require_api_key() -> String {
    std::env::var("DAYTONA_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .expect(
            "DAYTONA_API_KEY not set — live tests require real credentials (fail-closed policy)",
        )
}

// ============================================================================
// OpenAPI drift
// ============================================================================

/// The client is generated from vendored copies of Daytona's specs
/// (`openapi/daytona/`). This checks that every operation Everruns uses
/// (`openapi/daytona/operations.json`) still has the same path, parameters,
/// request body and success response shape in the specs Daytona serves now.
/// On failure, refresh with `doppler run -- scripts/update-daytona-openapi.sh`
/// and review the diff.
///
/// Shapes compare property names, nesting and `required`, not descriptions,
/// examples or enum values: the generated client reads enums as strings, so a
/// new sandbox state is not drift. The toolbox spec is Swagger 2.0 when served
/// and OpenAPI 3.0 once vendored, so both forms are normalized.
#[tokio::test]
async fn test_live_openapi_matches_vendored() {
    let api_key = require_api_key();
    let operations: serde_json::Value =
        serde_json::from_str(include_str!("../openapi/daytona/operations.json")).unwrap();
    let ids = |api: &str| -> Vec<String> {
        let mut ids: Vec<String> = ["client", "raw", "agent"]
            .iter()
            .filter_map(|group| operations[group][api].as_array())
            .flatten()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };

    let http = reqwest::Client::new();
    let live_management: serde_json::Value = http
        .get("https://app.daytona.io/api-json")
        .send()
        .await
        .expect("fetch management spec")
        .json()
        .await
        .expect("management spec is JSON");

    // The toolbox spec is served by the daemon inside a sandbox.
    let client = DaytonaClient::new(api_key.clone());
    let sandbox = client
        .create_sandbox(
            serde_json::from_value(serde_json::json!({
                "snapshot": "daytona-small",
                "labels": {"everruns-test": "openapi-drift"},
                "autoDeleteInterval": 10
            }))
            .expect("valid create body"),
        )
        .await
        .expect("create sandbox");
    client
        .wait_for_ready(&sandbox.id)
        .await
        .expect("sandbox ready");
    let live_toolbox: serde_json::Value = http
        .get(format!(
            "https://proxy.app.daytona.io/toolbox/{}/swagger/doc.json",
            sandbox.id
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .expect("fetch toolbox spec")
        .json()
        .await
        .expect("toolbox spec is JSON");
    if let Err(error) = client.delete_sandbox(&sandbox.id).await {
        eprintln!("[cleanup] failed to delete sandbox {}: {error}", sandbox.id);
    }

    let vendored_management: serde_json::Value =
        serde_json::from_str(include_str!("../openapi/daytona/api.json")).unwrap();
    let vendored_toolbox: serde_json::Value =
        serde_json::from_str(include_str!("../openapi/daytona/toolbox.json")).unwrap();

    let mut drift = Vec::new();
    for (api, vendored, live) in [
        ("management", &vendored_management, &live_management),
        ("toolbox", &vendored_toolbox, &live_toolbox),
    ] {
        for id in ids(api) {
            let expected = spec_shape::operation(vendored, &id);
            let actual = spec_shape::operation(live, &id);
            if expected != actual {
                drift.push(format!(
                    "{api} {id}:\n  vendored: {expected}\n  live:     {actual}"
                ));
            }
        }
    }
    assert!(
        drift.is_empty(),
        "Daytona's specs changed for operations Everruns uses; run \
         scripts/update-daytona-openapi.sh and review:\n{}",
        drift.join("\n")
    );
}

/// Format-neutral view of one operation in an OpenAPI 3.0 or Swagger 2.0 spec.
mod spec_shape {
    use serde_json::{Value, json};

    const DEPTH: usize = 4;

    pub fn operation(spec: &Value, id: &str) -> Value {
        let Some((path, method, op)) = find(spec, id) else {
            return json!("missing");
        };
        let path = path.strip_prefix("/api").unwrap_or(&path).to_string();
        let mut params = Vec::new();
        let mut body = Value::Null;
        for param in op["parameters"].as_array().into_iter().flatten() {
            let param = resolve(spec, param);
            match param["in"].as_str() {
                // Swagger 2.0 body and form parameters become OpenAPI 3.0 request bodies.
                Some("body") => body = schema(spec, &param["schema"], DEPTH),
                Some("formData") => {
                    if body.is_null() {
                        body = json!({ "properties": {} });
                    }
                    body["properties"][param["name"].as_str().unwrap_or_default()] = json!("field");
                }
                Some(location) => params.push(format!(
                    "{location}:{}{}",
                    param["name"].as_str().unwrap_or_default(),
                    if param["required"].as_bool() == Some(true) {
                        "!"
                    } else {
                        ""
                    }
                )),
                None => {}
            }
        }
        if let Some(content) = op["requestBody"]["content"].as_object()
            && let Some((media, media_body)) = content.iter().next()
        {
            body = if media.starts_with("multipart/") {
                let fields: serde_json::Map<String, Value> = media_body["schema"]["properties"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(name, _)| (name.clone(), json!("field")))
                    .collect();
                json!({ "properties": fields })
            } else {
                schema(spec, &media_body["schema"], DEPTH)
            };
        }
        params.sort();
        let response = op["responses"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(status, _)| status.starts_with('2'))
            .map(|(_, response)| {
                let response = resolve(spec, response);
                let schema_ref = response["content"]
                    .as_object()
                    .and_then(|content| {
                        content
                            .get("application/json")
                            .or_else(|| content.values().next())
                    })
                    .map(|media| &media["schema"])
                    .unwrap_or(&response["schema"]);
                schema(spec, schema_ref, DEPTH)
            })
            .next()
            .unwrap_or(Value::Null);
        json!({
            "path": format!("{method} {path}"),
            "params": params,
            "body": body,
            "response": response,
        })
    }

    fn find(spec: &Value, id: &str) -> Option<(String, String, Value)> {
        spec["paths"].as_object()?.iter().find_map(|(path, item)| {
            item.as_object()?.iter().find_map(|(method, op)| {
                (op["operationId"].as_str() == Some(id))
                    .then(|| (path.clone(), method.clone(), op.clone()))
            })
        })
    }

    fn resolve<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
        let Some(reference) = value["$ref"].as_str() else {
            return value;
        };
        let pointer = reference.trim_start_matches('#');
        spec.pointer(pointer).unwrap_or(&Value::Null)
    }

    fn schema(spec: &Value, value: &Value, depth: usize) -> Value {
        let value = resolve(spec, value);
        if let Some(first) = value["allOf"].as_array().and_then(|parts| parts.first()) {
            return schema(spec, first, depth);
        }
        if depth == 0 {
            return json!("…");
        }
        if let Some(properties) = value["properties"].as_object() {
            let properties: serde_json::Map<String, Value> = properties
                .iter()
                .map(|(name, child)| (name.clone(), schema(spec, child, depth - 1)))
                .collect();
            let mut required: Vec<&str> = value["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            required.sort_unstable();
            return json!({ "properties": properties, "required": required });
        }
        if value["type"] == "array" {
            return json!({ "array": schema(spec, &value["items"], depth - 1) });
        }
        if let Some(additional) = value.get("additionalProperties")
            && additional.is_object()
        {
            return json!({ "map": schema(spec, additional, depth - 1) });
        }
        match value["type"].as_str() {
            // Swagger 2.0 `file` is OpenAPI 3.0 `string` + `format: binary`.
            Some("file") => json!("string"),
            Some(kind) => json!(kind),
            None => json!("any"),
        }
    }
}
