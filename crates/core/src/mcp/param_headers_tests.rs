use super::*;
use serde_json::json;

fn github_like_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "owner": { "type": "string", "x-mcp-header": "owner" },
            "repo": { "type": "string", "x-mcp-header": "repo" },
            "pullNumber": { "type": "number" }
        },
        "required": ["owner", "repo", "pullNumber"]
    })
}

#[test]
fn collects_top_level_annotations() {
    let params = header_params(&github_like_schema()).unwrap();
    let names: Vec<_> = params.iter().map(HeaderParam::header_name).collect();
    assert_eq!(names, ["Mcp-Param-owner", "Mcp-Param-repo"]);
}

#[test]
fn schema_without_annotations_has_no_params() {
    let schema = json!({"type": "object", "properties": {"q": {"type": "string"}}});
    assert!(header_params(&schema).unwrap().is_empty());
    assert!(header_params(&json!({})).unwrap().is_empty());
}

#[test]
fn mirrors_values_and_omits_absent_or_null() {
    let params = header_params(&github_like_schema()).unwrap();
    let headers = headers_for(
        &params,
        &json!({"owner": "everruns", "repo": null, "pullNumber": 150}),
    )
    .unwrap();
    assert_eq!(
        headers,
        [("Mcp-Param-owner".to_string(), "everruns".to_string())]
    );
}

#[test]
fn nested_properties_follow_their_path() {
    let schema = json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "object",
                "properties": {
                    "region": {"type": "string", "x-mcp-header": "Region"},
                    "shard": {"type": "integer", "x-mcp-header": "Shard"},
                    "dry": {"type": ["boolean", "null"], "x-mcp-header": "Dry"}
                }
            }
        }
    });
    let params = header_params(&schema).unwrap();
    let headers = headers_for(
        &params,
        &json!({"target": {"region": "us-west1", "shard": -7, "dry": true}}),
    )
    .unwrap();
    let mut headers = headers;
    headers.sort();
    assert_eq!(
        headers,
        [
            ("Mcp-Param-Dry".to_string(), "true".to_string()),
            ("Mcp-Param-Region".to_string(), "us-west1".to_string()),
            ("Mcp-Param-Shard".to_string(), "-7".to_string()),
        ]
    );
}

#[test]
fn rejects_annotations_the_spec_forbids() {
    let cases = [
        // under an array
        json!({"properties": {"list": {"type": "array", "items": {"type": "string", "x-mcp-header": "A"}}}}),
        // under composition
        json!({"properties": {"x": {"oneOf": [{"type": "string", "x-mcp-header": "A"}]}}}),
        // number type
        json!({"properties": {"n": {"type": "number", "x-mcp-header": "N"}}}),
        // object type
        json!({"properties": {"o": {"type": "object", "x-mcp-header": "O"}}}),
        // empty name
        json!({"properties": {"s": {"type": "string", "x-mcp-header": ""}}}),
        // not a token
        json!({"properties": {"s": {"type": "string", "x-mcp-header": "a b"}}}),
        json!({"properties": {"s": {"type": "string", "x-mcp-header": "a\r\nX-Evil: 1"}}}),
        // duplicate, case-insensitive
        json!({"properties": {
            "a": {"type": "string", "x-mcp-header": "Repo"},
            "b": {"type": "string", "x-mcp-header": "repo"}
        }}),
        // not a string
        json!({"properties": {"s": {"type": "string", "x-mcp-header": 1}}}),
    ];
    for schema in cases {
        assert!(header_params(&schema).is_err(), "accepted {schema}");
    }
}

#[test]
fn wrong_argument_types_are_reported() {
    let schema = json!({"properties": {"n": {"type": "integer", "x-mcp-header": "N"}}});
    let params = header_params(&schema).unwrap();
    assert!(headers_for(&params, &json!({"n": 1.5})).is_err());
    assert!(headers_for(&params, &json!({"n": 9_007_199_254_740_992_u64})).is_err());
    assert!(headers_for(&params, &json!({"n": {"x": 1}})).is_err());
    assert_eq!(
        headers_for(&params, &json!({"n": 9_007_199_254_740_991_i64})).unwrap()[0].1,
        "9007199254740991"
    );
}

#[test]
fn encodes_values_per_spec_examples() {
    assert_eq!(encode_value("us-west1"), "us-west1");
    assert_eq!(
        encode_value("Hello, 世界"),
        "=?base64?SGVsbG8sIOS4lueVjA==?="
    );
    assert_eq!(encode_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
    assert_eq!(encode_value("line1\nline2"), "=?base64?bGluZTEKbGluZTI=?=");
    assert_eq!(
        encode_value("=?base64?literal?="),
        "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?="
    );
}

#[test]
fn injected_header_text_is_wrapped() {
    let params =
        header_params(&json!({"properties": {"repo": {"type": "string", "x-mcp-header": "repo"}}}))
            .unwrap();
    let headers = headers_for(&params, &json!({"repo": "x\r\nAuthorization: evil"})).unwrap();
    assert!(headers[0].1.starts_with("=?base64?"));
    assert!(!headers[0].1.contains('\n'));
}

#[test]
fn recognizes_header_mismatch_errors() {
    let github = anyhow::anyhow!(
        "MCP server returned error: 400 - {{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32020,\"message\":\"header mismatch: missing Mcp-Param-owner header\"}}}}"
    );
    assert!(is_header_mismatch(&github));
    assert!(!is_header_mismatch(&anyhow::anyhow!(
        "MCP server returned error: 400 - {{\"error\":{{\"code\":-32602}}}}"
    )));
}

#[test]
fn decode_round_trips_encoded_values() {
    for value in ["Hello, 世界", " padded ", "=?base64?literal?="] {
        assert_eq!(decode_value(&encode_value(value)).as_deref(), Some(value));
    }
    assert_eq!(decode_value("plain"), None);
    assert_eq!(decode_value("=?base64?not base64!?="), None);
}
