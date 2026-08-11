// Tool-call repair capability (EVE-600)
//
// Detects and repairs malformed tool-call arguments emitted by a model and
// recovers the turn instead of surfacing a raw parse error (or silently
// collapsing the call to `{}` the way some drivers do on a parse failure).
//
// # Chosen seam
//
// The provider-agnostic interception point is the `reason` atom, right after
// the assistant completion has been finalized into `Vec<ToolCall>` (see
// `crates/core/src/atoms/reason.rs`, just before the assistant message is built
// and `output.message.completed` is emitted). At that point each `ToolCall`
// already carries its `arguments` as a parsed `serde_json::Value`, but for a
// malformed call that value is either:
//   * a raw JSON *string* the driver could not parse into an object, or
//   * an object that does not satisfy the target tool's JSON schema.
// Individual drivers (`crates/anthropic`, `crates/openai`, ...) are not touched;
// they keep parsing as before. The capability runs only when explicitly enabled,
// so the default path is byte-for-byte unchanged.
//
// # Two-stage repair
//
// 1. **Deterministic local salvage** ([`salvage_tool_arguments`]) — a pure,
//    table-testable function that extracts the JSON object from fenced blocks,
//    surrounding prose, trailing commas, and single-quoted keys/strings, then
//    coerces known argument keys against the tool's JSON schema. The
//    already-valid case is a no-op.
// 2. **Bounded corrective re-prompt** — when local salvage cannot recover the
//    call, the capability allows up to `max_reprompts` (default 1) attempts per
//    call before falling through to today's exact error behavior. The re-prompt
//    itself is realized by the outer agent loop: the unrepaired call proceeds to
//    the act phase, produces today's tool error, and the model retries on the
//    next iteration. The capability only *bounds* and *labels* this, so there is
//    never an infinite repair loop.
//
// # Safety
//
// Salvage parses untrusted model output, so it is bounded: it refuses inputs
// over `MAX_SALVAGE_INPUT_BYTES`, scans without recursion, and never allocates
// proportionally more than the input. See the security notes on each helper.

use std::sync::Arc;

use crate::capabilities::{Capability, CapabilityLocalization};
use serde_json::Value;

pub const TOOL_CALL_REPAIR_CAPABILITY_ID: &str = "tool_call_repair";

/// Default number of corrective re-prompt attempts allowed per tool call before
/// falling through to the existing error path. Kept small on purpose — repair is
/// a recovery aid, not a retry loop.
pub const DEFAULT_MAX_REPROMPTS: u32 = 1;

/// Hard cap on the size of a raw argument blob the salvage routine will inspect.
/// Larger inputs are rejected outright (treated as un-salvageable) so a hostile
/// or runaway model cannot drive unbounded CPU/allocation in the parser.
pub const MAX_SALVAGE_INPUT_BYTES: usize = 256 * 1024;

/// Observable outcome of a repair attempt, used as the event/metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairOutcome {
    /// Local deterministic salvage recovered a valid JSON object.
    LocalSalvage,
    /// Local salvage failed but a bounded corrective re-prompt is still allowed.
    Reprompt,
    /// All bounded attempts are exhausted; fall through to the error path.
    GaveUp,
}

impl RepairOutcome {
    /// Stable machine-readable label for events/metrics.
    pub fn label(self) -> &'static str {
        match self {
            RepairOutcome::LocalSalvage => "local-salvage",
            RepairOutcome::Reprompt => "re-prompt",
            RepairOutcome::GaveUp => "gave-up",
        }
    }
}

/// Result of running [`salvage_tool_arguments`] on one tool call's raw arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum SalvageResult {
    /// The arguments were already a valid JSON object satisfying the schema (or
    /// no schema was available to check against). No change is needed.
    AlreadyValid,
    /// Local salvage produced a repaired JSON object. The wrapped value replaces
    /// the call's arguments.
    Repaired(Value),
    /// Local salvage could not recover a usable object.
    Unsalvageable,
}

/// Deterministically salvage a single tool call's raw/garbled `arguments`.
///
/// Pure and side-effect free, so it is exhaustively table-tested. Handles, in
/// order: already-valid objects; raw strings wrapping a JSON object (possibly
/// inside ```json fences, surrounded by prose); trailing commas; and
/// single-quoted keys/strings. When a `schema` is supplied, recovered string
/// values for known keys are coerced to the declared primitive type where it is
/// unambiguous (e.g. `"42"` -> `42` for an `integer` property).
///
/// Returns [`SalvageResult::AlreadyValid`] when nothing needs changing,
/// [`SalvageResult::Repaired`] with the recovered object, or
/// [`SalvageResult::Unsalvageable`] when no JSON object can be extracted.
///
/// Security: input larger than [`MAX_SALVAGE_INPUT_BYTES`] is treated as
/// unsalvageable without parsing. All scanning is linear and non-recursive.
pub fn salvage_tool_arguments(raw: &Value, schema: Option<&Value>) -> SalvageResult {
    // Case 1: already a JSON object. Try to coerce string-typed known keys to
    // their declared primitive type (e.g. `"7"` -> `7` for an integer property);
    // a string where the schema wants a number is malformed even when all
    // `required` keys are present. If coercion changes nothing, the object is
    // valid as-is and the call is a no-op.
    if let Value::Object(_) = raw {
        let coerced = coerce_known_keys(raw.clone(), schema);
        // A call that still violates the schema after coercion (a missing
        // `required` key, or a value whose type cannot be coerced to the declared
        // primitive) is malformed-vs-schema even though it is syntactically valid
        // JSON. Treat it as unsalvageable so it flows to the bounded re-prompt /
        // error path and is observable, rather than being silently AlreadyValid.
        if let Value::Object(ref m) = coerced
            && violates_schema(m, schema)
        {
            return SalvageResult::Unsalvageable;
        }
        if coerced == *raw {
            return SalvageResult::AlreadyValid;
        }
        return SalvageResult::Repaired(coerced);
    }

    // Case 2: a raw string. Drivers that fail to parse the model's argument
    // string sometimes pass it through verbatim as a JSON string. Extract the
    // embedded JSON object from it.
    let Some(raw_str) = raw.as_str() else {
        // Non-object, non-string (number/bool/null/array): not a tool-argument
        // shape we can repair.
        return SalvageResult::Unsalvageable;
    };

    if raw_str.len() > MAX_SALVAGE_INPUT_BYTES {
        return SalvageResult::Unsalvageable;
    }

    let trimmed = raw_str.trim();
    if trimmed.is_empty() {
        // An empty argument string is conventionally an empty object — unless the
        // schema requires keys we cannot supply, in which case `{}` would just
        // fail downstream, so route it to the re-prompt / error path instead.
        let empty = serde_json::Map::new();
        if violates_schema(&empty, schema) {
            return SalvageResult::Unsalvageable;
        }
        return SalvageResult::Repaired(Value::Object(empty));
    }

    match extract_json_object(trimmed) {
        Some(obj) => {
            let coerced = coerce_known_keys(obj, schema);
            // A recovered object that still violates the schema does not actually
            // recover the turn (the tool would reject it downstream), so surface
            // it as unsalvageable for the bounded re-prompt path.
            if let Value::Object(ref m) = coerced
                && violates_schema(m, schema)
            {
                return SalvageResult::Unsalvageable;
            }
            SalvageResult::Repaired(coerced)
        }
        None => SalvageResult::Unsalvageable,
    }
}

/// Conservative schema check used to decide whether a syntactically-valid object
/// is still malformed *relative to the tool's schema*. Returns true only on
/// unambiguous violations: a declared `required` key is absent, or a present
/// (non-null) value's JSON type does not match a declared primitive `type`.
///
/// Intentionally shallow and non-recursive: it does not validate nested objects,
/// enums, formats, or unknown keys, so it never flags a call the downstream tool
/// would accept. With no schema (or no `type`/`required` info) nothing is flagged.
fn violates_schema(obj: &serde_json::Map<String, Value>, schema: Option<&Value>) -> bool {
    let Some(schema) = schema else {
        return false;
    };

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                return true;
            }
        }
    }

    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (key, prop_schema) in props {
            let Some(declared) = prop_schema.get("type").and_then(Value::as_str) else {
                continue;
            };
            let Some(val) = obj.get(key) else {
                continue;
            };
            // Null is left to the downstream tool (fields are often nullable).
            if val.is_null() {
                continue;
            }
            let matches = match declared {
                "integer" => val.is_i64() || val.is_u64(),
                "number" => val.is_number(),
                "boolean" => val.is_boolean(),
                "string" => val.is_string(),
                "array" => val.is_array(),
                "object" => val.is_object(),
                // Unknown/compound declared type: do not flag.
                _ => true,
            };
            if !matches {
                return true;
            }
        }
    }

    false
}

/// Extract the first balanced JSON object from a blob that may be wrapped in
/// ```json fences and/or surrounded by prose, tolerating trailing commas and
/// single-quoted strings/keys. Returns the parsed object, or `None`.
///
/// Non-recursive: brace matching is a single linear scan with a depth counter.
fn extract_json_object(input: &str) -> Option<Value> {
    let candidate = strip_code_fences(input);

    // Try a direct parse of the (fence-stripped) candidate first.
    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(candidate.trim()) {
        return Some(value);
    }

    // Locate the first balanced `{ ... }` span, honoring string literals so a
    // brace inside a string does not throw off the depth counter.
    let span = first_balanced_object_span(candidate)?;
    let slice = &candidate[span];

    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(slice) {
        return Some(value);
    }

    // Apply lenient fix-ups (single quotes -> double, drop trailing commas) and
    // retry once. Bounded: a single rewrite pass over the already-bounded slice.
    let relaxed = relax_json(slice);
    match serde_json::from_str::<Value>(&relaxed) {
        Ok(value @ Value::Object(_)) => Some(value),
        _ => None,
    }
}

/// Remove a leading/trailing Markdown code fence (```json ... ``` or ``` ... ```).
/// When no fence is present the input is returned unchanged.
fn strip_code_fences(input: &str) -> &str {
    let trimmed = input.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Drop an optional language tag on the opening fence line.
    let after_lang = match after_open.find('\n') {
        Some(nl) => &after_open[nl + 1..],
        None => after_open,
    };
    after_lang.strip_suffix("```").unwrap_or(after_lang).trim()
}

/// Byte range of the first balanced top-level `{...}` object in `input`,
/// respecting double-quoted string literals and escapes. `None` if unbalanced.
fn first_balanced_object_span(input: &str) -> Option<std::ops::Range<usize>> {
    let bytes = input.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    // Track the active quote char so single- and double-quoted strings both mask
    // braces during the scan (the relax pass later normalizes quotes).
    let mut quote: u8 = 0;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == quote {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' | b'\'' => {
                in_string = true;
                quote = b;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start..i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Best-effort lenient JSON normalization for a single object slice:
/// converts single-quoted strings/keys to double-quoted and removes trailing
/// commas before `}`/`]`. Single linear pass, no recursion.
fn relax_json(slice: &str) -> String {
    // Phase 1: quote normalization. Walk char-by-char; outside a double-quoted
    // string, replace `'` with `"`. Inside a double-quoted string, leave content
    // untouched (so apostrophes in values survive).
    let mut out = String::with_capacity(slice.len());
    let mut in_double = false;
    let mut escaped = false;
    for ch in slice.chars() {
        if in_double {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_double = true;
                out.push(ch);
            }
            '\'' => out.push('"'),
            _ => out.push(ch),
        }
    }

    // Phase 2: strip trailing commas (`,` followed by optional whitespace then
    // `}` or `]`). Operate on the quote-normalized string, skipping commas that
    // live inside double-quoted strings.
    strip_trailing_commas(&out)
}

/// Remove commas that immediately precede a closing `}`/`]` (ignoring
/// whitespace), skipping any comma inside a double-quoted string.
fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for i in 0..chars.len() {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == ',' {
            // Look ahead past whitespace for a closing bracket.
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                // Skip this trailing comma.
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Coerce string-typed values for known schema keys to the declared primitive
/// type when unambiguous (`integer`, `number`, `boolean`). Unknown keys and
/// values that do not cleanly convert are left untouched.
fn coerce_known_keys(value: Value, schema: Option<&Value>) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    let Some(props) = schema
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
    else {
        return Value::Object(obj);
    };

    for (key, prop_schema) in props {
        let Some(declared) = prop_schema.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(current) = obj.get(key) else {
            continue;
        };
        let Some(text) = current.as_str() else {
            continue;
        };
        let coerced = match declared {
            "integer" => text.trim().parse::<i64>().ok().map(Value::from),
            "number" => text.trim().parse::<f64>().ok().map(Value::from),
            "boolean" => match text.trim() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            _ => None,
        };
        if let Some(coerced) = coerced {
            obj.insert(key.clone(), coerced);
        }
    }
    Value::Object(obj)
}

/// Per-agent config for the tool-call-repair capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCallRepairConfig {
    /// Maximum corrective re-prompt attempts per tool call before falling
    /// through to the existing error path.
    pub max_reprompts: u32,
}

impl Default for ToolCallRepairConfig {
    fn default() -> Self {
        Self {
            max_reprompts: DEFAULT_MAX_REPROMPTS,
        }
    }
}

impl ToolCallRepairConfig {
    /// Parse config from the per-agent JSON blob, falling back to defaults for
    /// missing/unknown fields.
    pub fn from_json(config: &Value) -> Self {
        let max_reprompts = config
            .get("max_reprompts")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(DEFAULT_MAX_REPROMPTS);
        Self { max_reprompts }
    }

    /// Decide the bounded outcome when local salvage has failed, given how many
    /// re-prompt attempts have already happened for this call.
    ///
    /// Returns [`RepairOutcome::Reprompt`] while attempts remain, otherwise
    /// [`RepairOutcome::GaveUp`]. This is the only place the attempt cap is
    /// enforced, keeping the loop bounded.
    pub fn outcome_after_failed_salvage(&self, prior_attempts: u32) -> RepairOutcome {
        if prior_attempts < self.max_reprompts {
            RepairOutcome::Reprompt
        } else {
            RepairOutcome::GaveUp
        }
    }
}

/// Opt-in capability that repairs malformed tool calls. Disabled by default:
/// it is registered in the global registry but contributes nothing unless an
/// agent explicitly enables it, and the `reason` atom only runs repair when the
/// capability is present in the resolved capability set.
pub struct ToolCallRepairCapability;

impl Capability for ToolCallRepairCapability {
    fn id(&self) -> &str {
        TOOL_CALL_REPAIR_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tool Call Repair"
    }

    fn description(&self) -> &str {
        "Detects and repairs malformed tool-call arguments from the model, \
         recovering the turn instead of surfacing a raw parse error."
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    fn config_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "max_reprompts": {
                    "type": "integer",
                    "title": "Max corrective re-prompts",
                    "description": "How many corrective re-prompt attempts are allowed per malformed tool call before falling through to the normal error path.",
                    "minimum": 0,
                    "maximum": 5,
                    "default": DEFAULT_MAX_REPROMPTS
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("tool_call_repair config must be an object".to_string());
        }
        match config.get("max_reprompts") {
            None => Ok(()),
            Some(value) => match value.as_u64() {
                Some(n) if n <= 5 => Ok(()),
                _ => Err(format!(
                    "max_reprompts must be an integer between 0 and 5, got {value}"
                )),
            },
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization {
            locale: "en",
            name: None,
            description: None,
            config_description: Some(
                "Controls how many corrective re-prompts are attempted before a malformed tool call falls through to the normal error path.",
            ),
            config_overlay: None,
        }]
    }
}

/// Convenience: the registered capability as an `Arc` (mirrors how other
/// capabilities are surfaced where an `Arc<dyn Capability>` is needed).
pub fn tool_call_repair_capability() -> Arc<dyn Capability> {
    Arc::new(ToolCallRepairCapability)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "limit": { "type": "integer" },
                "ratio": { "type": "number" },
                "recursive": { "type": "boolean" }
            },
            "required": ["path"]
        })
    }

    // ---- Salvage: table-driven across each malformation class ----

    #[test]
    fn already_valid_object_is_noop() {
        let raw = json!({ "path": "/foo", "limit": 10 });
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::AlreadyValid
        );
    }

    #[test]
    fn already_valid_object_without_schema_is_noop() {
        let raw = json!({ "anything": 1 });
        assert_eq!(
            salvage_tool_arguments(&raw, None),
            SalvageResult::AlreadyValid
        );
    }

    #[test]
    fn empty_string_becomes_empty_object() {
        let raw = json!("   ");
        assert_eq!(
            salvage_tool_arguments(&raw, None),
            SalvageResult::Repaired(json!({}))
        );
    }

    #[test]
    fn empty_string_with_required_schema_is_unsalvageable() {
        // `{}` would just fail downstream when the schema requires `path`, so
        // route it to the bounded re-prompt / error path instead.
        let raw = json!("");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn object_missing_required_key_is_unsalvageable() {
        // Syntactically valid JSON, but missing the required `path` key — this is
        // malformed vs the schema and cannot be locally repaired.
        let raw = json!({ "limit": 3 });
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn object_with_uncoercible_type_is_unsalvageable() {
        // `limit` is an integer property; "abc" cannot be coerced, so after the
        // coercion pass the type still mismatches.
        let raw = json!({ "path": "/foo", "limit": "abc" });
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn extracted_object_missing_required_is_unsalvageable() {
        // Recovered from prose but still missing the required key: not a usable
        // repair, so surface it for the re-prompt path.
        let raw = json!("here you go: {\"limit\": 3}");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn object_missing_required_key_without_schema_is_noop() {
        // With no schema we cannot know a key is required, so do not flag it.
        let raw = json!({ "limit": 3 });
        assert_eq!(
            salvage_tool_arguments(&raw, None),
            SalvageResult::AlreadyValid
        );
    }

    #[test]
    fn raw_string_object_is_parsed() {
        let raw = json!("{\"path\": \"/foo\"}");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo" }))
        );
    }

    #[test]
    fn fenced_json_block_is_unwrapped() {
        let raw = json!("```json\n{\"path\": \"/foo\"}\n```");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo" }))
        );
    }

    #[test]
    fn bare_fenced_block_is_unwrapped() {
        let raw = json!("```\n{\"path\": \"/bar\"}\n```");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/bar" }))
        );
    }

    #[test]
    fn leading_and_trailing_prose_is_stripped() {
        let raw = json!("Sure! Here are the args: {\"path\": \"/foo\"} hope that helps");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo" }))
        );
    }

    #[test]
    fn trailing_commas_are_removed() {
        let raw = json!("{\"path\": \"/foo\", \"limit\": 3,}");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo", "limit": 3 }))
        );
    }

    #[test]
    fn single_quotes_are_normalized() {
        let raw = json!("{'path': '/foo', 'limit': 5}");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo", "limit": 5 }))
        );
    }

    #[test]
    fn apostrophe_inside_double_quoted_value_survives() {
        let raw = json!("{\"path\": \"it's here\"}");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "it's here" }))
        );
    }

    #[test]
    fn brace_inside_string_does_not_break_span() {
        let raw = json!("prose {\"path\": \"a}b\"} more");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "a}b" }))
        );
    }

    #[test]
    fn known_keys_are_coerced_against_schema() {
        // Model emitted everything as strings; coerce against the schema types.
        let raw = json!(
            "{\"path\": \"/foo\", \"limit\": \"42\", \"ratio\": \"1.5\", \"recursive\": \"true\"}"
        );
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(
                json!({ "path": "/foo", "limit": 42, "ratio": 1.5, "recursive": true })
            )
        );
    }

    #[test]
    fn object_with_string_typed_known_key_is_coerced_in_place() {
        // Already an object, but `limit` is a string and required `path` present.
        let raw = json!({ "path": "/foo", "limit": "7" });
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Repaired(json!({ "path": "/foo", "limit": 7 }))
        );
    }

    #[test]
    fn unparseable_garbage_is_unsalvageable() {
        let raw = json!("path equals slash foo, no json here at all");
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn oversized_input_is_rejected_without_parsing() {
        let big = format!("{{\"path\": \"{}\"}}", "a".repeat(MAX_SALVAGE_INPUT_BYTES));
        let raw = json!(big);
        assert_eq!(
            salvage_tool_arguments(&raw, Some(&schema())),
            SalvageResult::Unsalvageable
        );
    }

    #[test]
    fn non_object_non_string_is_unsalvageable() {
        assert_eq!(
            salvage_tool_arguments(&json!(42), None),
            SalvageResult::Unsalvageable
        );
        assert_eq!(
            salvage_tool_arguments(&json!([1, 2]), None),
            SalvageResult::Unsalvageable
        );
    }

    // ---- Bounded re-prompt / give-up ----

    #[test]
    fn outcome_reprompts_until_cap_then_gives_up() {
        let cfg = ToolCallRepairConfig { max_reprompts: 2 };
        assert_eq!(cfg.outcome_after_failed_salvage(0), RepairOutcome::Reprompt);
        assert_eq!(cfg.outcome_after_failed_salvage(1), RepairOutcome::Reprompt);
        assert_eq!(cfg.outcome_after_failed_salvage(2), RepairOutcome::GaveUp);
        assert_eq!(cfg.outcome_after_failed_salvage(3), RepairOutcome::GaveUp);
    }

    #[test]
    fn zero_reprompts_gives_up_immediately() {
        let cfg = ToolCallRepairConfig { max_reprompts: 0 };
        assert_eq!(cfg.outcome_after_failed_salvage(0), RepairOutcome::GaveUp);
    }

    #[test]
    fn config_parses_from_json_with_defaults() {
        assert_eq!(
            ToolCallRepairConfig::from_json(&json!({})),
            ToolCallRepairConfig::default()
        );
        assert_eq!(
            ToolCallRepairConfig::from_json(&json!({ "max_reprompts": 3 })).max_reprompts,
            3
        );
    }

    #[test]
    fn outcome_labels_are_stable() {
        assert_eq!(RepairOutcome::LocalSalvage.label(), "local-salvage");
        assert_eq!(RepairOutcome::Reprompt.label(), "re-prompt");
        assert_eq!(RepairOutcome::GaveUp.label(), "gave-up");
    }

    // ---- Capability wiring ----

    #[test]
    fn capability_id_and_validation() {
        let cap = ToolCallRepairCapability;
        assert_eq!(cap.id(), TOOL_CALL_REPAIR_CAPABILITY_ID);
        assert!(cap.is_guardrail());
        assert!(cap.config_schema().is_some());

        assert!(cap.validate_config(&Value::Null).is_ok());
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&json!({ "max_reprompts": 2 })).is_ok());
        assert!(cap.validate_config(&json!({ "max_reprompts": 9 })).is_err());
        assert!(
            cap.validate_config(&json!({ "max_reprompts": "x" }))
                .is_err()
        );
    }
}
