// MCP scripting catalog — inventory is the only source of truth.

use crate::domains::common::CommandError;
use bashkit::{ScriptedTool, ToolArgs, ToolDef};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;

/// Shared context for inventory-registered domain commands.
#[derive(Clone)]
pub struct CatalogContext {
    pub domain_ctx: crate::domains::common::Ctx,
    pub link_builder: crate::api::common::UrlBuilder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsetMode {
    Full,
    ReadOnly,
}

fn excluded_from_read_only_query(name: &str) -> bool {
    // THREAT[TM-MCP-002]: read-only MCP `query` must not allow commands that
    // can perform open-world side effects (for example, scoped MCP tool
    // discovery performs outbound HTTP requests).
    matches!(name, "preview_agent" | "preview_harness")
}

fn exposed_to_scripting(meta: &crate::domains::common::CommandMeta) -> bool {
    // Durable control-plane APIs are infrastructure internals. App schedule
    // channels expose their own user-facing commands instead.
    !meta.path.starts_with("/v1/durable/")
}

static INVENTORY_TOOL_DEFS: LazyLock<HashMap<&'static str, ToolDef>> = LazyLock::new(|| {
    inventory::iter::<crate::domains::common::CommandDescriptor>
        .into_iter()
        .filter_map(|desc| {
            let meta = (desc.meta)();
            exposed_to_scripting(&meta).then_some((desc, meta))
        })
        .map(|desc| {
            let (desc, meta) = desc;
            let schema = bashkit_inventory_schema((desc.param_schema)());
            let def = ToolDef::new(meta.name, meta.description)
                .with_schema(schema)
                .with_category(meta.category);
            (meta.name, def)
        })
        .collect()
});

impl CatalogContext {
    /// Convert to a domain Ctx for inventory-registered command dispatch.
    pub fn to_domain_ctx(&self) -> crate::domains::common::Ctx {
        self.domain_ctx.clone()
    }
}

/// Build one scripted tool from inventory-registered commands only.
pub fn build_toolset(ctx: CatalogContext, mode: ToolsetMode) -> ScriptedTool {
    let mut builder = ScriptedTool::builder("everruns")
        .short_description(match mode {
            ToolsetMode::Full => "Everruns API operations as bash builtins",
            ToolsetMode::ReadOnly => "Read-only Everruns API operations as bash builtins",
        })
        .limits(
            bashkit::ExecutionLimits::new()
                .max_commands(500)
                .max_loop_iterations(5000)
                .max_function_depth(50)
                .max_input_bytes(500_000)
                .max_ast_depth(50)
                .parser_timeout(std::time::Duration::from_secs(3)),
        )
        .sanitize_errors(false);

    for desc in inventory::iter::<crate::domains::common::CommandDescriptor> {
        // THREAT[TM-MCP-002]: MCP `query` must not expose mutating server
        // tools. Read-only classification is backed by inventory tests.
        let meta = (desc.meta)();
        if !exposed_to_scripting(&meta) {
            continue;
        }
        if mode == ToolsetMode::ReadOnly
            && (!(desc.read_only)() || excluded_from_read_only_query(meta.name))
        {
            continue;
        }
        let def = command_descriptor_to_def(desc);
        let callback = make_inventory_callback(desc, ctx.clone());
        // THREAT[TM-MCP-002]: Domain errors are sanitized by
        // `format_dispatch_error`; disabling Bashkit's blanket replacement
        // preserves actionable kind tokens without exposing internal details.
        builder = builder.async_tool_fn(def, callback);
    }

    builder.build()
}

fn command_descriptor_to_def(desc: &crate::domains::common::CommandDescriptor) -> ToolDef {
    let meta = (desc.meta)();
    INVENTORY_TOOL_DEFS
        .get(meta.name)
        .cloned()
        .unwrap_or_else(|| {
            ToolDef::new(meta.name, meta.description)
                .with_schema(bashkit_inventory_schema((desc.param_schema)()))
                .with_category(meta.category)
        })
}

// EVE-409: bashkit's flag parser only coerces scalar types (integer, number,
// boolean) and treats everything else as raw `string`. So array/object fields
// like `--capabilities '[...]'` or `--tags '[...]'` arrive at dispatch as JSON
// strings. We bridge this in two complementary passes:
//
// 1. `bashkit_inventory_schema` rewrites every aggregate property to
//    `type: "string"` so bashkit's `tools/list` advertises a flat string flag
//    and `parse_flags` accepts the raw JSON text without typed coercion
//    attempts (which still don't reach into `allOf`/`$ref` composition;
//    bashkit#1516/#1527 only coerces top-level array/object schemas).
// 2. `coerce_json_text_params` parses those JSON strings back into typed
//    values before the domain dispatcher sees them. It also repairs scalar
//    values in composed schemas because bashkit does not coerce flags reached
//    through `allOf`/`$ref` (for example `--enabled true` on Agent Triggers).
//
// Both passes walk `$ref` into `$defs`/`definitions`, traverse `allOf`,
// `oneOf`, `anyOf` branches, and recurse into nested `$defs` entries — this
// is what makes the `#[serde(flatten)]` shape used by `UpdateAgentCmd`
// (`allOf: [{$ref: ".../UpdateAgentRequest"}, {properties: {id}}]`) work the
// same as the simpler `CreateAgentCmd` shape with direct top-level
// properties.
const SCHEMA_WALK_MAX_DEPTH: u8 = 8;

fn bashkit_inventory_schema(mut schema: serde_json::Value) -> serde_json::Value {
    let defs_snapshot = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|value| value.as_object())
        .cloned();
    retype_aggregate_properties_in_place(&mut schema, defs_snapshot.as_ref(), 0);
    schema
}

fn retype_aggregate_properties_in_place(
    schema: &mut serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    depth: u8,
) {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return;
    }
    if let Some(properties) = schema
        .get_mut("properties")
        .and_then(|value| value.as_object_mut())
    {
        for (_name, property) in properties.iter_mut() {
            if !property_is_aggregate(property, defs, 0) {
                continue;
            }
            let Some(object) = property.as_object_mut() else {
                continue;
            };
            object.insert(
                "type".to_string(),
                serde_json::Value::String("string".to_string()),
            );
            // Drop type-discriminating constructs that no longer apply after
            // retyping; this keeps `tools/list` consumers from generating
            // bogus item validators or following stale composition branches.
            object.remove("items");
            object.remove("properties");
            object.remove("oneOf");
            object.remove("anyOf");
            object.remove("allOf");
            object.remove("$ref");
            let description = object
                .get("description")
                .and_then(|value| value.as_str())
                .map(|value| format!("{value} Pass JSON text in bash mode."))
                .unwrap_or_else(|| "Pass JSON text in bash mode.".to_string());
            object.insert(
                "description".to_string(),
                serde_json::Value::String(description),
            );
        }
    }
    let defs_key = if schema.get("$defs").is_some() {
        Some("$defs")
    } else if schema.get("definitions").is_some() {
        Some("definitions")
    } else {
        None
    };
    if let Some(key) = defs_key
        && let Some(defs_obj) = schema.get_mut(key).and_then(|value| value.as_object_mut())
    {
        for (_, def) in defs_obj.iter_mut() {
            retype_aggregate_properties_in_place(def, defs, depth + 1);
        }
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema.get_mut(key).and_then(|value| value.as_array_mut()) {
            for branch in branches.iter_mut() {
                retype_aggregate_properties_in_place(branch, defs, depth + 1);
            }
        }
    }
}

/// Walk the original param schema and parse JSON-text values for properties
/// whose declared type is array/object, or a scalar type that bashkit left as
/// text because it lives under composition. The domain dispatcher expects the
/// typed shape.
///
/// Empty / whitespace-only strings are left untouched so optional fields that
/// rely on `deserialize_opt_json_value_lenient` (e.g. `channel_config`) keep
/// their `--flag ''` → `None` behavior. The dispatcher applies the same trim
/// rule for non-empty strings, so we only intervene when there is JSON to
/// parse on the bash side.
fn coerce_json_text_params(
    schema: &serde_json::Value,
    params: &mut serde_json::Value,
) -> Result<(), String> {
    let Some(params_obj) = params.as_object_mut() else {
        return Ok(());
    };
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|value| value.as_object());
    let mut all_properties = serde_json::Map::new();
    collect_all_properties(schema, defs, &mut all_properties, 0);
    for (name, value) in params_obj.iter_mut() {
        let Some(property) = all_properties.get(name) else {
            continue;
        };
        let should_parse = property_is_aggregate(property, defs, 0)
            || property_has_json_scalar_type(property, defs, 0);
        if !should_parse {
            continue;
        }
        let serde_json::Value::String(text) = value else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<serde_json::Value>(text.trim())
            .map_err(|err| format!("--{name}: invalid JSON ({err})"))?;
        *value = parsed;
    }
    Ok(())
}

fn normalize_and_validate_params(
    schema: &serde_json::Value,
    params: &mut serde_json::Value,
) -> Result<(), String> {
    let Some(params) = params.as_object_mut() else {
        return Ok(());
    };
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|value| value.as_object());
    let mut properties = serde_json::Map::new();
    collect_all_properties(schema, defs, &mut properties, 0);
    for name in params.keys().cloned().collect::<Vec<_>>() {
        if !name.contains('-') {
            continue;
        }
        let canonical = name.replace('-', "_");
        if properties.contains_key(&canonical)
            && !params.contains_key(&canonical)
            && let Some(value) = params.remove(&name)
        {
            params.insert(canonical, value);
        }
    }
    let mut unknown = params
        .keys()
        .filter(|name| !properties.contains_key(*name))
        .map(|name| format!("--{name}"))
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    let mut supported = properties
        .keys()
        .map(|name| format!("--{name}"))
        .collect::<Vec<_>>();
    supported.sort();
    Err(format!(
        "unknown flag(s): {}. Use exactly the discovered flags: {}",
        unknown.join(", "),
        supported.join(", ")
    ))
}

fn property_has_json_scalar_type(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    depth: u8,
) -> bool {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return false;
    }
    if let Some(value) = schema.get("type") {
        if let Some(name) = value.as_str()
            && matches!(name, "boolean" | "integer" | "number")
        {
            return true;
        }
        if let Some(types) = value.as_array()
            && types.iter().any(|entry| {
                entry
                    .as_str()
                    .is_some_and(|name| matches!(name, "boolean" | "integer" | "number"))
            })
        {
            return true;
        }
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
    {
        return property_has_json_scalar_type(resolved, Some(defs), depth + 1);
    }
    ["allOf", "oneOf", "anyOf"].iter().any(|key| {
        schema
            .get(key)
            .and_then(|value| value.as_array())
            .is_some_and(|branches| {
                branches
                    .iter()
                    .any(|branch| property_has_json_scalar_type(branch, defs, depth + 1))
            })
    })
}

/// Merge property maps across `$ref`, `allOf`, `oneOf`, and `anyOf` branches
/// so composed command schemas surface every declared field. The outermost
/// schema's own `properties` are visited first, then `$ref` resolution and
/// composition branches; combined with `or_insert_with` this gives outer
/// declarations precedence over nested copies on key conflicts.
fn collect_all_properties(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    out: &mut serde_json::Map<String, serde_json::Value>,
    depth: u8,
) {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return;
    }
    if let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) {
        for (name, property) in properties {
            out.entry(name.clone()).or_insert_with(|| property.clone());
        }
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
    {
        collect_all_properties(resolved, Some(defs), out, depth + 1);
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(|value| value.as_array()) {
            for branch in branches {
                collect_all_properties(branch, defs, out, depth + 1);
            }
        }
    }
}

fn collect_required_properties(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    out: &mut BTreeSet<String>,
    depth: u8,
) {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return;
    }
    if let Some(required) = schema.get("required").and_then(|value| value.as_array()) {
        out.extend(
            required
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned)),
        );
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
    {
        collect_required_properties(resolved, Some(defs), out, depth + 1);
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(|value| value.as_array()) {
            for branch in branches {
                collect_required_properties(branch, defs, out, depth + 1);
            }
        }
    }
}

fn property_has_type(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    expected: &str,
    depth: u8,
) -> bool {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return false;
    }
    if schema.get("type").is_some_and(|value| {
        value.as_str() == Some(expected)
            || value
                .as_array()
                .is_some_and(|types| types.iter().any(|value| value.as_str() == Some(expected)))
    }) {
        return true;
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
        && property_has_type(resolved, Some(defs), expected, depth + 1)
    {
        return true;
    }
    ["allOf", "oneOf", "anyOf"].iter().any(|key| {
        schema
            .get(key)
            .and_then(|value| value.as_array())
            .is_some_and(|branches| {
                branches
                    .iter()
                    .any(|branch| property_has_type(branch, defs, expected, depth + 1))
            })
    })
}

/// Render copyable Bashkit syntax from the authoritative command schema.
/// Aggregate flags stay JSON text because that is the scripting wire format.
pub(crate) fn bash_usage(command_name: &str, schema: &serde_json::Value) -> String {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|value| value.as_object());
    let mut properties = serde_json::Map::new();
    collect_all_properties(schema, defs, &mut properties, 0);
    let mut required = BTreeSet::new();
    collect_required_properties(schema, defs, &mut required, 0);

    let mut flags = properties
        .iter()
        .map(|(name, property)| {
            let placeholder = if property_is_aggregate(property, defs, 0) {
                property
                    .get("example")
                    .and_then(|example| serde_json::to_string(example).ok())
                    .filter(|example| example.len() <= 160)
                    .map(|example| format!("'{}'", example.replace('\'', "'\"'\"'")))
                    .unwrap_or_else(|| "'<json>'".to_string())
            } else if property_has_type(property, defs, "boolean", 0) {
                "<true|false>".to_string()
            } else if property_has_type(property, defs, "integer", 0)
                || property_has_type(property, defs, "number", 0)
            {
                "<number>".to_string()
            } else {
                "'<string>'".to_string()
            };
            let flag = format!("--{name} {placeholder}");
            (required.contains(name), name, flag)
        })
        .collect::<Vec<_>>();
    flags.sort_by_key(|(is_required, name, _)| (!*is_required, *name));

    std::iter::once(command_name.to_string())
        .chain(flags.into_iter().map(|(is_required, _, flag)| {
            if is_required {
                flag
            } else {
                format!("[{flag}]")
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_schema_field_paths(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    prefix: &str,
    out: &mut BTreeSet<String>,
    depth: u8,
) {
    if depth >= SCHEMA_WALK_MAX_DEPTH || out.len() >= 100 {
        return;
    }
    if let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) {
        for (name, property) in properties {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            out.insert(path.clone());
            collect_schema_field_paths(property, defs, &path, out, depth + 1);
        }
    }
    if let Some(items) = schema.get("items") {
        collect_schema_field_paths(items, defs, prefix, out, depth + 1);
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
    {
        collect_schema_field_paths(resolved, Some(defs), prefix, out, depth + 1);
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(|value| value.as_array()) {
            for branch in branches {
                collect_schema_field_paths(branch, defs, prefix, out, depth + 1);
            }
        }
    }
}

/// Return bounded dot paths that describe fields available to jq without
/// forcing a model to retain a large expanded output schema.
pub(crate) fn schema_field_paths(schema: &serde_json::Value) -> Vec<String> {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|value| value.as_object());
    let mut fields = BTreeSet::new();
    collect_schema_field_paths(schema, defs, "", &mut fields, 0);
    fields.into_iter().collect()
}

/// Decide whether a property's declared schema represents an array or object
/// aggregate, including the utoipa-style nullable shapes (`type: ["array",
/// "null"]`, `oneOf` with a `null` branch + array branch) and `$ref` chains
/// into `$defs`.
fn property_is_aggregate(
    schema: &serde_json::Value,
    defs: Option<&serde_json::Map<String, serde_json::Value>>,
    depth: u8,
) -> bool {
    if depth >= SCHEMA_WALK_MAX_DEPTH {
        return false;
    }
    if let Some(value) = schema.get("type") {
        if let Some(name) = value.as_str()
            && matches!(name, "array" | "object")
        {
            return true;
        }
        if let Some(types) = value.as_array()
            && types.iter().any(|entry| {
                entry
                    .as_str()
                    .is_some_and(|name| matches!(name, "array" | "object"))
            })
        {
            return true;
        }
    }
    if schema.get("items").is_some() || schema.get("properties").is_some() {
        return true;
    }
    if let Some(reference) = schema.get("$ref").and_then(|value| value.as_str())
        && let Some(name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
        && let Some(defs) = defs
        && let Some(resolved) = defs.get(name)
        && property_is_aggregate(resolved, Some(defs), depth + 1)
    {
        return true;
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(|value| value.as_array())
            && branches
                .iter()
                .any(|branch| property_is_aggregate(branch, defs, depth + 1))
        {
            return true;
        }
    }
    false
}

/// Stable, lower_snake_case label for a `CommandError` kind. Surfaced as
/// the leading token of the dispatch error string so MCP/bashkit consumers
/// can tell whether a failure is a bad request, a forbidden call, a missing
/// resource, or an internal exception without parsing free-form text.
fn command_error_kind(err: &CommandError) -> &'static str {
    use crate::domains::common::CommandErrorKind;
    match &err.kind {
        CommandErrorKind::BadRequest(_) => "bad_request",
        CommandErrorKind::Unprocessable(_) => "unprocessable",
        CommandErrorKind::Forbidden(_) => "forbidden",
        CommandErrorKind::NotFound(_) => "not_found",
        CommandErrorKind::Conflict(_) => "conflict",
        CommandErrorKind::RateLimited(_) => "rate_limited",
        CommandErrorKind::Internal(_) => "internal",
    }
}

/// Format a dispatch failure as `<kind>: <message>`. Bashkit prefixes the
/// builtin name on its own (e.g. `update_model: …`), so we deliberately do
/// not duplicate it here. The result is the structured error contract for
/// MCP-facing builtins; see the "Structured dispatch errors" section in
/// `knowledge/foundations/domains.md` for the canonical contract and the kind token table.
///
/// Agent-actionable extensions (`code`, `allowed_actions`, `retry_after_seconds`)
/// flow through `CommandError` but are intentionally not appended here — the
/// `<kind>: <message>` wire is a documented contract. Surfacing extensions
/// over MCP is tracked as an additive contract change.
fn format_dispatch_error(err: &CommandError) -> String {
    if let crate::domains::common::CommandErrorKind::Internal(inner) = &err.kind {
        tracing::error!(error = %inner, "Platform command failed");
        return "internal: Internal server error".to_string();
    }
    format!("{}: {err}", command_error_kind(err))
}

fn make_inventory_callback(
    desc: &'static crate::domains::common::CommandDescriptor,
    ctx: CatalogContext,
) -> impl Fn(ToolArgs) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
+ Send
+ Sync
+ 'static {
    move |args: ToolArgs| {
        let mut params = args.params;
        let domain_ctx = ctx.to_domain_ctx();
        let link_builder = ctx.link_builder.clone();
        Box::pin(async move {
            // Schema rewriting in `bashkit_inventory_schema` advertises array
            // and object fields as `string` so bashkit's parser accepts JSON
            // text on the command line. Translate them back here so the
            // domain dispatcher sees the structured value it expects.
            let original_schema = (desc.param_schema)();
            normalize_and_validate_params(&original_schema, &mut params)?;
            coerce_json_text_params(&original_schema, &mut params)?;
            let result = (desc.dispatch)(params, &domain_ctx)
                .await
                .map_err(|error| format_dispatch_error(&error))?;
            decorate_command_output(&result, &link_builder)
        })
    }
}

fn decorate_command_output(
    result: &str,
    link_builder: &crate::api::common::UrlBuilder,
) -> Result<String, String> {
    let mut value = match serde_json::from_str::<serde_json::Value>(result) {
        Ok(value) => value,
        Err(_) => return Ok(result.to_string()),
    };
    link_builder.decorate_value_links(&mut value);
    decorate_mcp_capability_refs(&mut value);
    serde_json::to_string(&value).map_err(|error| format!("Internal error: {error}"))
}

fn decorate_mcp_capability_refs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            let capability_ref = object
                .get("id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| {
                    value
                        .parse::<everruns_provider::typed_id::McpServerId>()
                        .ok()
                })
                .map(|id| format!("mcp:{}", id.uuid()));
            if let Some(capability_ref) = capability_ref {
                object
                    .entry("capability_ref".to_string())
                    .or_insert_with(|| serde_json::Value::String(capability_ref));
            }
            for child in object.values_mut() {
                decorate_mcp_capability_refs(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                decorate_mcp_capability_refs(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_command_defs_expose_structured_param_schemas() {
        let desc = inventory::iter::<crate::domains::common::CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == "create_agent")
            .expect("create_agent command descriptor");

        let def = command_descriptor_to_def(desc);
        let properties = def
            .input_schema
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("create_agent schema properties");

        assert!(properties.contains_key("name"));
        assert!(properties.contains_key("system_prompt"));
        assert_ne!(
            def.input_schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn command_output_decoration_adds_ui_link_for_mcp_execute_results() {
        let builder =
            crate::api::common::UrlBuilder::new("https://api.example/api", "https://app.example");
        let result = decorate_command_output(
            r#"{"id":"agent_00000000000000000000000000000001","name":"demo"}"#,
            &builder,
        )
        .expect("decorated output");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(
            value["self_url"],
            "https://api.example/api/v1/agents/agent_00000000000000000000000000000001"
        );
        assert_eq!(
            value["ui_link"],
            "https://app.example/agents/agent_00000000000000000000000000000001"
        );
    }

    #[test]
    fn command_output_decoration_adds_mcp_capability_reference() {
        let builder =
            crate::api::common::UrlBuilder::new("https://api.example/api", "https://app.example");
        let result = decorate_command_output(
            r#"{"id":"mcp_00000000000000000000000000000001","name":"visti"}"#,
            &builder,
        )
        .expect("decorated output");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(
            value["capability_ref"],
            "mcp:00000000-0000-0000-0000-000000000001"
        );

        let nested = decorate_command_output(
            r#"{"data":[{"id":"mcp_00000000000000000000000000000002"}]}"#,
            &builder,
        )
        .expect("nested decorated output");
        let nested_value: serde_json::Value = serde_json::from_str(&nested).unwrap();
        assert_eq!(
            nested_value["data"][0]["capability_ref"],
            "mcp:00000000-0000-0000-0000-000000000002"
        );
    }

    #[test]
    fn bashkit_schema_retypes_array_and_object_fields_to_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "capabilities": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "List of capabilities"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "channel_config": {
                    "type": "object",
                    "description": "Channel config"
                }
            }
        });
        let rewritten = bashkit_inventory_schema(schema);
        let props = rewritten["properties"].as_object().unwrap();
        // Scalars unchanged
        assert_eq!(props["id"]["type"], "string");
        // Arrays and objects retyped to string with JSON-text hint
        for key in ["capabilities", "tags", "channel_config"] {
            assert_eq!(
                props[key]["type"], "string",
                "{key} should be retyped to string",
            );
            assert!(
                props[key]["description"]
                    .as_str()
                    .unwrap()
                    .contains("JSON text in bash mode"),
            );
            assert!(
                props[key].get("items").is_none(),
                "items metadata should be dropped for {key}",
            );
        }
    }

    #[test]
    fn coerce_parses_json_text_for_array_and_object_fields() {
        let schema = serde_json::json!({
            "properties": {
                "id": { "type": "string" },
                "capabilities": { "type": "array", "items": { "type": "object" } },
                "tags": { "type": "array", "items": { "type": "string" } }
            }
        });
        let mut params = serde_json::json!({
            "id": "agent_1",
            "capabilities": "[{\"ref\":\"current_time\"}]",
            "tags": "[\"a\",\"b\"]"
        });
        coerce_json_text_params(&schema, &mut params).expect("coerce");
        assert_eq!(params["id"], "agent_1");
        assert_eq!(
            params["capabilities"],
            serde_json::json!([{"ref": "current_time"}]),
        );
        assert_eq!(params["tags"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn unknown_flags_are_rejected_before_mutation_dispatch() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "api_key": { "type": "string" },
                "auth_mode": { "type": "string" }
            }
        });
        let mut params = serde_json::json!({
            "api-key": "secret",
            "auth-type": "api_key"
        });

        let error = normalize_and_validate_params(&schema, &mut params).expect_err("unknown flags");
        assert!(error.contains("--auth-type"));
        assert!(error.contains("--auth_mode"));
        assert_eq!(params["api_key"], "secret");
        assert!(params.get("api-key").is_none());
    }

    #[test]
    fn coerce_parses_scalar_flags_from_composed_schema() {
        let schema = serde_json::json!({
            "$defs": {
                "Trigger": {
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "limit": { "type": "integer" },
                        "ratio": { "type": "number" },
                        "session_mode": { "type": "string" }
                    }
                }
            },
            "allOf": [
                { "$ref": "#/$defs/Trigger" },
                { "properties": { "agent_id": { "type": "string" } } }
            ]
        });
        let mut params = serde_json::json!({
            "enabled": "true",
            "limit": "5",
            "ratio": "1.5",
            "session_mode": "session_per_invocation"
        });

        coerce_json_text_params(&schema, &mut params).expect("coerce");

        assert_eq!(params["enabled"], true);
        assert_eq!(params["limit"], 5);
        assert_eq!(params["ratio"], 1.5);
        assert_eq!(params["session_mode"], "session_per_invocation");
    }

    #[test]
    fn coerce_surfaces_json_parse_errors_with_field_name() {
        let schema = serde_json::json!({
            "properties": {
                "capabilities": { "type": "array", "items": {} }
            }
        });
        let mut params = serde_json::json!({ "capabilities": "[not-json" });
        let err = coerce_json_text_params(&schema, &mut params).unwrap_err();
        assert!(err.starts_with("--capabilities:"), "got: {err}");
        assert!(err.contains("invalid JSON"));
    }

    #[test]
    fn coerce_leaves_empty_and_whitespace_strings_untouched() {
        // Optional non-scalar fields like `channel_config` rely on
        // `deserialize_opt_json_value_lenient`, which maps empty/whitespace
        // strings to `None`. Eagerly parsing those would regress the bash
        // ergonomics (`--channel_config ''` should still mean "unset").
        let schema = serde_json::json!({
            "properties": {
                "channel_config": { "type": "object", "properties": {} }
            }
        });
        for blank in ["", "   ", "\t\n"] {
            let mut params = serde_json::json!({ "channel_config": blank });
            coerce_json_text_params(&schema, &mut params).expect("coerce");
            assert_eq!(
                params["channel_config"],
                serde_json::Value::String(blank.to_string()),
                "blank input {blank:?} should pass through untouched",
            );
        }
    }

    #[test]
    fn bashkit_schema_retypes_aggregate_fields_buried_under_allof_and_ref() {
        // `UpdateAgentCmd` shape: `#[serde(flatten)]` projects the wrapped
        // request's properties into an `allOf: [{$ref}, {properties: {id}}]`
        // composition. Top-level `properties` is empty; aggregate flags
        // (`capabilities`, `tags`, `mcpServers`) live under
        // `$defs.UpdateAgentRequest.properties`. The retyper must reach them
        // so bashkit advertises plain `string` flags instead of typed
        // arrays/objects (which bashkit#1516/#1527 still does not coerce
        // through `allOf`/`$ref`).
        let schema = serde_json::json!({
            "type": "object",
            "$defs": {
                "UpdateAgentRequest": {
                    "type": "object",
                    "properties": {
                        "tags": {
                            "type": ["array", "null"],
                            "items": { "type": "string" }
                        },
                        "capabilities": {
                            "type": ["array", "null"],
                            "items": { "$ref": "#/$defs/AgentCapabilityConfig" }
                        },
                        "mcpServers": {
                            "oneOf": [
                                { "type": "null" },
                                { "$ref": "#/$defs/ScopedMcpServers" }
                            ]
                        }
                    }
                },
                "AgentCapabilityConfig": { "type": "object", "properties": {} },
                "ScopedMcpServers": { "type": "object", "properties": {} }
            },
            "allOf": [
                { "$ref": "#/$defs/UpdateAgentRequest" },
                {
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                }
            ]
        });
        let rewritten = bashkit_inventory_schema(schema);
        let req_props = rewritten["$defs"]["UpdateAgentRequest"]["properties"]
            .as_object()
            .expect("UpdateAgentRequest properties");
        for key in ["tags", "capabilities", "mcpServers"] {
            assert_eq!(
                req_props[key]["type"], "string",
                "{key} should be retyped to string under $defs"
            );
            assert!(
                req_props[key].get("items").is_none(),
                "{key} items metadata should be dropped"
            );
            assert!(
                req_props[key].get("oneOf").is_none(),
                "{key} oneOf branches should be dropped"
            );
        }
        // Top-level allOf id flag remains a plain string.
        let id_props = rewritten["allOf"][1]["properties"].as_object().unwrap();
        assert_eq!(id_props["id"]["type"], "string");
    }

    #[test]
    fn coerce_walks_allof_ref_to_parse_flatten_aggregate_fields() {
        // Same `UpdateAgentCmd` shape as above. Bashkit hands us the
        // string-typed value because we advertised the field as `string`;
        // the coercer must walk into `$defs` via the `allOf` `$ref` to
        // discover that `capabilities` was originally an array and parse it.
        let schema = serde_json::json!({
            "$defs": {
                "UpdateAgentRequest": {
                    "properties": {
                        "tags": { "type": ["array", "null"], "items": {} },
                        "capabilities": {
                            "type": ["array", "null"],
                            "items": { "type": "object" }
                        }
                    }
                }
            },
            "allOf": [
                { "$ref": "#/$defs/UpdateAgentRequest" },
                { "properties": { "id": { "type": "string" } } }
            ]
        });
        let mut params = serde_json::json!({
            "id": "agent_x",
            "capabilities": "[{\"ref\":\"current_time\"}]",
            "tags": "[\"a\",\"b\"]"
        });
        coerce_json_text_params(&schema, &mut params).expect("coerce");
        assert_eq!(params["id"], "agent_x");
        assert_eq!(
            params["capabilities"],
            serde_json::json!([{ "ref": "current_time" }])
        );
        assert_eq!(params["tags"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn coerce_passes_through_already_parsed_values() {
        let schema = serde_json::json!({
            "properties": {
                "capabilities": { "type": "array", "items": {} }
            }
        });
        // Non-bash MCP clients send the structured value directly; the coercer
        // must leave it alone.
        let original = serde_json::json!([{ "ref": "current_time" }]);
        let mut params = serde_json::json!({ "capabilities": original.clone() });
        coerce_json_text_params(&schema, &mut params).expect("coerce");
        assert_eq!(params["capabilities"], original);
    }

    #[test]
    fn command_output_decoration_preserves_non_json_results() {
        let builder =
            crate::api::common::UrlBuilder::new("https://api.example/api", "https://app.example");

        assert_eq!(
            decorate_command_output("plain result", &builder).expect("decorated output"),
            "plain result"
        );
    }

    // ========================================================================
    // EVE-419: structured dispatch error contract
    //
    // Bashkit prefixes the builtin name on its own ("update_model: ..."), so
    // the dispatch layer adds the `<kind>: <message>` body. Together they
    // produce errors like `update_model: bad_request: invalid type: string
    // "true", expected bool` instead of the historical opaque
    // `update_model: callback failed`.
    // ========================================================================

    #[test]
    fn command_error_kind_covers_every_variant() {
        use anyhow::anyhow;
        // If a new variant is added to `CommandError`, this match must be
        // updated — exhaustively covering each one here keeps the contract
        // honest. The literal strings are part of the public MCP error
        // contract and should not change without a corresponding spec update.
        assert_eq!(
            command_error_kind(&CommandError::bad_request("x")),
            "bad_request"
        );
        assert_eq!(
            command_error_kind(&CommandError::unprocessable("x")),
            "unprocessable"
        );
        assert_eq!(
            command_error_kind(&CommandError::forbidden("x")),
            "forbidden"
        );
        assert_eq!(
            command_error_kind(&CommandError::not_found_msg("x")),
            "not_found"
        );
        assert_eq!(command_error_kind(&CommandError::conflict("x")), "conflict");
        assert_eq!(
            command_error_kind(&CommandError::internal(anyhow!("boom"))),
            "internal"
        );
    }

    #[test]
    fn format_dispatch_error_prefixes_kind_before_message() {
        let err = CommandError::bad_request("invalid type: string \"true\", expected bool");
        assert_eq!(
            format_dispatch_error(&err),
            "bad_request: invalid type: string \"true\", expected bool"
        );
    }

    #[test]
    fn format_dispatch_error_does_not_duplicate_operation_name() {
        // Bashkit is responsible for the `<op>:` prefix. The dispatch layer
        // must not re-emit it, otherwise consumers see `update_model:
        // update_model: ...`.
        let err = CommandError::not_found_msg("Model not found");
        let formatted = format_dispatch_error(&err);
        assert!(formatted.starts_with("not_found:"));
        assert!(
            !formatted.contains("update_model"),
            "operation name must not appear twice: {formatted}"
        );
    }

    #[test]
    fn format_dispatch_error_distinguishes_internal_from_authz() {
        use anyhow::anyhow;
        let internal = format_dispatch_error(&CommandError::internal(anyhow!(
            "database connection refused"
        )));
        let forbidden = format_dispatch_error(&CommandError::forbidden("permission denied"));
        assert_eq!(internal, "internal: Internal server error");
        assert!(!internal.contains("database connection refused"));
        assert!(forbidden.starts_with("forbidden:"));
        assert_ne!(internal, forbidden);
    }

    #[test]
    fn read_only_query_excludes_preview_commands_with_network_side_effects() {
        assert!(excluded_from_read_only_query("preview_agent"));
        assert!(excluded_from_read_only_query("preview_harness"));
        assert!(!excluded_from_read_only_query("list_agents"));
    }
}
