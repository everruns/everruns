//! Tool parameters mirrored into `Mcp-Param-{Name}` request headers.
//!
//! Spec: MCP 2026-07-28 Streamable HTTP, "Custom Headers from Tool Parameters".
//! A server may mark a property of a tool's `inputSchema` with `x-mcp-header`;
//! clients MUST then send that argument's value as `Mcp-Param-{Name}` on every
//! `tools/call`, and servers reject a call whose header is missing or differs
//! from the body with `400` / `-32020 HeaderMismatch`. GitHub's MCP server marks
//! `owner` and `repo` this way, so without this every repository tool failed.
//!
//! Decisions:
//! - Annotations are validated when tools are listed. A tool whose annotation
//!   breaks the spec's rules is dropped from the list with a warning (the spec
//!   requires exclusion), so one bad tool never hides the server's others.
//! - Values are encoded here (plain when header-safe, else the
//!   `=?base64?...?=` sentinel), never by the caller, so a model-chosen
//!   argument cannot inject a CR/LF or a second header.
//!
//! THREAT[TM-TOOL-068]: header injection through tool arguments. Every mirrored
//! value goes through [`encode_value`], which emits only visible ASCII, space
//! or tab, and Base64-wraps anything else.

use base64::Engine as _;
use serde_json::Value;

/// Header name prefix for mirrored tool parameters.
pub const PARAM_HEADER_PREFIX: &str = "Mcp-Param-";

/// JSON-RPC error code a server returns when headers and body disagree.
pub const HEADER_MISMATCH_CODE: i64 = -32020;

const ANNOTATION: &str = "x-mcp-header";
const SENTINEL_PREFIX: &str = "=?base64?";
const SENTINEL_SUFFIX: &str = "?=";
/// Largest integer JavaScript represents exactly (2^53 - 1).
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// One annotated parameter: the header name portion and where the value sits
/// in the call arguments (a chain of `properties` keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderParam {
    pub name: String,
    pub path: Vec<String>,
}

impl HeaderParam {
    pub fn header_name(&self) -> String {
        format!("{PARAM_HEADER_PREFIX}{}", self.name)
    }
}

/// Collect a tool's `x-mcp-header` annotations, or explain why the tool
/// definition is invalid under the spec's rules.
pub fn header_params(input_schema: &Value) -> Result<Vec<HeaderParam>, String> {
    let mut params = Vec::new();
    collect_reachable(input_schema, &mut Vec::new(), &mut params)?;
    let reachable = params.len();
    let total = count_annotations(input_schema);
    if total != reachable {
        return Err(format!(
            "{ANNOTATION} may only mark properties reachable through `properties` keys \
             ({} annotation(s) sit under arrays, composition, conditionals or $ref)",
            total - reachable
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for param in &params {
        if !seen.insert(param.name.to_ascii_lowercase()) {
            return Err(format!(
                "{ANNOTATION} value '{}' is not unique (case-insensitive)",
                param.name
            ));
        }
    }
    Ok(params)
}

/// Walk `properties` chains only, recording annotated properties.
fn collect_reachable(
    schema: &Value,
    path: &mut Vec<String>,
    out: &mut Vec<HeaderParam>,
) -> Result<(), String> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (key, property) in properties {
        path.push(key.clone());
        if let Some(annotation) = property.get(ANNOTATION) {
            let name = annotation
                .as_str()
                .ok_or_else(|| format!("{ANNOTATION} on '{}' must be a string", path.join(".")))?;
            if name.is_empty() || !name.bytes().all(is_tchar) {
                return Err(format!(
                    "{ANNOTATION} value '{name}' on '{}' is not a valid header token",
                    path.join(".")
                ));
            }
            if !has_primitive_type(property) {
                return Err(format!(
                    "{ANNOTATION} on '{}' must mark a string, integer or boolean property",
                    path.join(".")
                ));
            }
            out.push(HeaderParam {
                name: name.to_string(),
                path: path.clone(),
            });
        }
        collect_reachable(property, path, out)?;
        path.pop();
    }
    Ok(())
}

/// Every `x-mcp-header` key anywhere in the schema, reachable or not.
fn count_annotations(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key(ANNOTATION))
                + map.values().map(count_annotations).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_annotations).sum(),
        _ => 0,
    }
}

/// `type` must name string, integer or boolean (optionally alongside `null`).
/// `number` is not allowed by the spec.
fn has_primitive_type(property: &Value) -> bool {
    let allowed = |t: &str| matches!(t, "string" | "integer" | "boolean");
    match property.get("type") {
        Some(Value::String(t)) => allowed(t),
        Some(Value::Array(types)) => {
            let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            names.len() == types.len()
                && names.iter().any(|t| allowed(t))
                && names.iter().all(|t| allowed(t) || *t == "null")
        }
        _ => false,
    }
}

/// RFC 9110 `tchar`.
fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Header pairs for one call. A parameter whose value is absent or `null` is
/// omitted, as the spec requires. A value of the wrong type is an error: the
/// server would reject the call anyway, and this message says which argument.
pub fn headers_for(
    params: &[HeaderParam],
    arguments: &Value,
) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::with_capacity(params.len());
    for param in params {
        let mut value = arguments;
        let mut present = true;
        for key in &param.path {
            match value.get(key) {
                Some(next) => value = next,
                None => {
                    present = false;
                    break;
                }
            }
        }
        if !present || value.is_null() {
            continue;
        }
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => match n.as_i64() {
                Some(i) if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&i) => i.to_string(),
                _ => {
                    return Err(format!(
                        "argument '{}' must be an integer within ±(2^53-1) to be sent as {}",
                        param.path.join("."),
                        param.header_name()
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "argument '{}' must be a string, integer or boolean to be sent as {}",
                    param.path.join("."),
                    param.header_name()
                ));
            }
        };
        headers.push((param.header_name(), encode_value(&text)));
    }
    Ok(headers)
}

/// Encode a header value: as-is when it is header-safe ASCII, otherwise (or
/// when it would be mistaken for an encoded value) as `=?base64?...?=`.
pub fn encode_value(value: &str) -> String {
    let safe_chars = value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b));
    let padded = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    let looks_encoded = value.starts_with(SENTINEL_PREFIX) && value.ends_with(SENTINEL_SUFFIX);
    if safe_chars && !padded && !looks_encoded {
        value.to_string()
    } else {
        format!(
            "{SENTINEL_PREFIX}{}{SENTINEL_SUFFIX}",
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        )
    }
}

/// Decode a `=?base64?...?=` header value. `None` when the value is plain (not
/// in the sentinel form) or the payload is not valid Base64 UTF-8.
pub fn decode_value(value: &str) -> Option<String> {
    let inner = value
        .strip_prefix(SENTINEL_PREFIX)?
        .strip_suffix(SENTINEL_SUFFIX)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(inner)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Whether an error from a call is the server's `HeaderMismatch` rejection.
pub fn is_header_mismatch(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains(&format!("\"code\":{HEADER_MISMATCH_CODE}"))
        || text.contains(&format!("\"code\": {HEADER_MISMATCH_CODE}"))
        || text.contains(&format!("code: {HEADER_MISMATCH_CODE}"))
        || text.contains(&format!("({HEADER_MISMATCH_CODE})"))
}

/// How long a tool's annotations are remembered after the last `tools/list`.
/// A stale entry costs at most one rejected call: the server's
/// `HeaderMismatch` triggers a fresh `tools/list` and one retry.
const SPEC_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Per-connection memory of which tools mirror which parameters, filled from
/// `tools/list` so `tools/call` (which only has the tool name) can add headers.
pub struct ParamHeaderCache<K> {
    entries: std::sync::Mutex<std::collections::HashMap<(K, String), CachedSpec>>,
}

/// A tool's annotations and when `tools/list` last reported them.
type CachedSpec = (Vec<HeaderParam>, std::time::Instant);

impl<K> Default for ParamHeaderCache<K> {
    fn default() -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl<K: Clone + Eq + std::hash::Hash> ParamHeaderCache<K> {
    /// Remember each tool's annotations and drop tools whose annotations break
    /// the spec, logging why (the spec requires excluding them).
    pub fn screen(
        &self,
        key: &K,
        server: &str,
        tools: Vec<crate::McpToolDefinition>,
    ) -> Vec<crate::McpToolDefinition> {
        let mut kept = Vec::with_capacity(tools.len());
        let mut specs = Vec::new();
        for tool in tools {
            match header_params(&tool.input_schema) {
                Ok(params) => {
                    specs.push((tool.name.clone(), params));
                    kept.push(tool);
                }
                Err(reason) => tracing::warn!(
                    server,
                    tool = %tool.name,
                    %reason,
                    "Excluding MCP tool with an invalid x-mcp-header annotation"
                ),
            }
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|_, (_, at)| at.elapsed() < SPEC_TTL);
            let now = std::time::Instant::now();
            for (name, params) in specs {
                entries.insert((key.clone(), name), (params, now));
            }
        }
        kept
    }

    /// Annotations last seen for a tool; `None` when the tool is unknown here.
    pub fn get(&self, key: &K, tool: &str) -> Option<Vec<HeaderParam>> {
        let entries = self.entries.lock().ok()?;
        let (params, at) = entries.get(&(key.clone(), tool.to_string()))?;
        (at.elapsed() < SPEC_TTL).then(|| params.clone())
    }
}

#[cfg(test)]
#[path = "param_headers_tests.rs"]
mod tests;
