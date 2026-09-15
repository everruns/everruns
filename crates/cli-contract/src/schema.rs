//! Turning a command's parameter schema into its command line.
//!
//! A command already publishes a JSON Schema for its parameters, generated
//! from the Rust type it deserializes. That schema knows the names, the types,
//! which fields are required and what each one is for: everything a parser
//! needs except presentation. Combined with the command's [`CliRoute`], it
//! produces the [`ContractCommand`] both surfaces build their parser from.
//!
//! This runs where the commands are declared, not in the parser. The contract
//! it produces is the artifact that travels.

use serde_json::{Map, Value};

use crate::declare::CliRoute;
use crate::{ArgKind, ContractArg, ContractCommand, ContractExample};

/// How deep `$ref` and `allOf` chains are followed before a schema is treated
/// as opaque. Schemas are generated from Rust types, so real nesting is
/// shallow; the bound exists so a cyclic `$ref` cannot loop.
const MAX_DEPTH: u8 = 8;

/// Build one command's contract from what it publishes plus what it declares.
pub fn contract_for(
    wire_name: &str,
    description: &str,
    method: &str,
    http_path: &str,
    route: &CliRoute,
    schema: &Value,
) -> ContractCommand {
    ContractCommand {
        wire_name: wire_name.to_string(),
        path: route.path.iter().map(|part| (*part).to_string()).collect(),
        verb: route.verb.to_string(),
        description: description.to_string(),
        method: method.to_string(),
        http_path: http_path.to_string(),
        args: args_for(route, schema),
        examples: route
            .examples
            .iter()
            .map(|example| ContractExample {
                intent: example.intent.to_string(),
                command: example.command.to_string(),
            })
            .collect(),
    }
}

fn args_for(route: &CliRoute, schema: &Value) -> Vec<ContractArg> {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(Value::as_object);

    let mut properties: Map<String, Value> = Map::new();
    let mut required: Vec<String> = Vec::new();
    collect(schema, defs, &mut properties, &mut required, 0);

    let mut args: Vec<ContractArg> = properties
        .into_iter()
        .map(|(field, property)| {
            let resolved = resolve(&property, defs, 0);
            let declared = route.arg(&field);
            ContractArg {
                long: declared
                    .and_then(|arg| arg.long)
                    .map(ToOwned::to_owned)
                    // Kebab by default, because that is what a command line
                    // looks like; the parameter's own snake_case name stays an
                    // alias, so a script written against either keeps working.
                    .unwrap_or_else(|| field.replace('_', "-")),
                short: declared.and_then(|arg| arg.short),
                position: declared.and_then(|arg| arg.position),
                kind: kind_of(&resolved, defs, 0),
                required: required.contains(&field),
                help: resolved
                    .get("description")
                    .and_then(Value::as_str)
                    .map(first_line),
                choices: choices_of(&resolved),
                field,
            }
        })
        .collect();

    // Positionals first and in their declared order, so a rendered usage line
    // reads the way it is typed.
    args.sort_by(|left, right| match (left.position, right.position) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.long.cmp(&right.long),
    });
    args
}

/// Gather properties and required names through `allOf` and `$ref` wrappers.
///
/// Generated schemas wrap flattened structs in `allOf`, so a command whose
/// parameters include `#[serde(flatten)] pagination` presents those fields one
/// level down. Missing them would make `--limit` an unknown flag.
fn collect(
    schema: &Value,
    defs: Option<&Map<String, Value>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    depth: u8,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let schema = resolve(schema, defs, depth);

    if let Some(own) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in own {
            properties.entry(name.clone()).or_insert(property.clone());
        }
    }
    if let Some(names) = schema.get("required").and_then(Value::as_array) {
        for name in names.iter().filter_map(Value::as_str) {
            if !required.iter().any(|existing| existing == name) {
                required.push(name.to_string());
            }
        }
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = schema.get(key).and_then(Value::as_array) {
            for branch in branches {
                // Only `allOf` contributes requirements: one branch of a union
                // requiring a field does not make it required overall.
                let mut branch_required = Vec::new();
                collect(branch, defs, properties, &mut branch_required, depth + 1);
                if key == "allOf" {
                    for name in branch_required {
                        if !required.iter().any(|existing| existing == &name) {
                            required.push(name);
                        }
                    }
                }
            }
        }
    }
}

/// Follow a `$ref` into `$defs`, once per level.
fn resolve(schema: &Value, defs: Option<&Map<String, Value>>, depth: u8) -> Value {
    if depth >= MAX_DEPTH {
        return schema.clone();
    }
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return schema.clone();
    };
    let name = reference
        .rsplit('/')
        .next()
        .filter(|_| reference.starts_with("#/"));
    match name.and_then(|name| defs?.get(name)) {
        Some(target) => resolve(target, defs, depth + 1),
        None => schema.clone(),
    }
}

fn kind_of(schema: &Value, defs: Option<&Map<String, Value>>, depth: u8) -> ArgKind {
    if depth >= MAX_DEPTH {
        return ArgKind::Json;
    }

    // `Option<T>` renders as a union with null: the flag is simply optional,
    // and its type is whatever the non-null branch says.
    if let Some(branches) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let mut concrete = branches
            .iter()
            .map(|branch| resolve(branch, defs, depth))
            .filter(|branch| !is_null(branch));
        if let Some(first) = concrete.next()
            && concrete.next().is_none()
        {
            return kind_of(&first, defs, depth + 1);
        }
        return ArgKind::Json;
    }

    match type_name(schema) {
        Some("boolean") => ArgKind::Boolean,
        Some("integer") => ArgKind::Integer,
        Some("number") => ArgKind::Number,
        Some("string") => ArgKind::String,
        Some("array") => {
            let items = schema
                .get("items")
                .map(|items| resolve(items, defs, depth))
                .unwrap_or(Value::Null);
            match kind_of(&items, defs, depth + 1) {
                ArgKind::Integer | ArgKind::Number => ArgKind::IntegerList,
                ArgKind::String | ArgKind::Boolean => ArgKind::StringList,
                // A list of documents is one document: comma-splitting it
                // would cut through a comma of its own.
                _ => ArgKind::Json,
            }
        }
        _ => ArgKind::Json,
    }
}

/// The schema's type, ignoring a `null` alternative in a `["T","null"]` union.
fn type_name(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(name) => Some(name.as_str()),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null"),
        _ => None,
    }
}

fn is_null(schema: &Value) -> bool {
    matches!(schema.get("type"), Some(Value::String(name)) if name == "null")
}

fn choices_of(schema: &Value) -> Vec<String> {
    schema
        .get("enum")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Help stays one line per flag: catalog descriptions are written for a JSON
/// catalog and often run several sentences.
fn first_line(description: &str) -> String {
    let trimmed = description.trim();
    let end = trimmed
        .find(". ")
        .map(|index| index + 1)
        .unwrap_or(trimmed.len());
    let mut line = trimmed[..end].trim().replace('\n', " ");
    if line.chars().count() > 96 {
        line = line.chars().take(93).collect::<String>();
        line.push_str("...");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declare::{CliArg, CliExample};
    use serde_json::json;

    const ROUTE: CliRoute = CliRoute::new(&["agents"], "update")
        .with_args(&[
            CliArg::new("id").at(1),
            CliArg::new("harness_name").short('H').long("harness"),
            CliArg::new("tag").short('t'),
        ])
        .with_examples(&[CliExample::new(
            "Rename an agent",
            "everruns agents update agt_01h9 --name triage-v2",
        )]);

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Agent id." },
                "harness_name": { "type": "string" },
                "tag": { "type": "array", "items": { "type": "string" } },
                "name": { "type": "string" },
                "max_iterations": { "type": ["integer", "null"] },
                "metadata": { "type": "object" },
                "status": { "type": "string", "enum": ["active", "archived"] }
            },
            "required": ["id"]
        })
    }

    fn contract() -> ContractCommand {
        contract_for(
            "update_agent",
            "Update an agent.",
            "PATCH",
            "/v1/agents/{id}",
            &ROUTE,
            &schema(),
        )
    }

    #[test]
    fn a_long_flag_is_kebab_by_default() {
        let contract = contract();
        let arg = contract
            .args
            .iter()
            .find(|arg| arg.field == "max_iterations")
            .expect("field is present");
        assert_eq!(arg.long, "max-iterations");
        // `Option<i64>` is a union with null; the flag is optional and typed.
        assert_eq!(arg.kind, ArgKind::Integer);
        assert!(!arg.required);
    }

    /// Presentation is a choice, so it comes from the declaration and can
    /// override the field name entirely.
    #[test]
    fn declared_presentation_wins_over_the_field_name() {
        let contract = contract();
        let harness = contract
            .args
            .iter()
            .find(|arg| arg.field == "harness_name")
            .expect("field is present");
        assert_eq!(harness.long, "harness");
        assert_eq!(harness.short, Some('H'));
    }

    #[test]
    fn a_positional_sorts_first_and_keeps_its_place() {
        let contract = contract();
        assert_eq!(contract.args[0].field, "id");
        assert_eq!(contract.args[0].position, Some(1));
        assert!(contract.args[0].required);
    }

    #[test]
    fn types_reduce_to_what_a_command_line_can_carry() {
        let contract = contract();
        let kind = |field: &str| {
            contract
                .args
                .iter()
                .find(|arg| arg.field == field)
                .map(|arg| arg.kind)
        };
        assert_eq!(kind("tag"), Some(ArgKind::StringList));
        assert_eq!(kind("metadata"), Some(ArgKind::Json));
        assert_eq!(kind("name"), Some(ArgKind::String));
    }

    #[test]
    fn a_declared_enum_becomes_the_flags_choices() {
        let contract = contract();
        let status = contract
            .args
            .iter()
            .find(|arg| arg.field == "status")
            .expect("field is present");
        assert_eq!(status.choices, vec!["active", "archived"]);
    }

    /// Pagination reaches commands through `#[serde(flatten)]`, which renders
    /// as an `allOf` branch. A reduction that only read top-level properties
    /// would make `--limit` an unknown flag.
    #[test]
    fn a_flattened_branch_contributes_its_fields() {
        let schema = json!({
            "type": "object",
            "properties": { "search": { "type": "string" } },
            "allOf": [{ "$ref": "#/$defs/Pagination" }],
            "$defs": {
                "Pagination": {
                    "type": "object",
                    "properties": { "limit": { "type": "integer" } },
                    "required": ["limit"]
                }
            }
        });
        let route = CliRoute::new(&["agents"], "list");
        let contract = contract_for("list_agents", "List.", "GET", "/v1/agents", &route, &schema);

        let limit = contract
            .args
            .iter()
            .find(|arg| arg.field == "limit")
            .expect("flattened field is a flag");
        assert_eq!(limit.kind, ArgKind::Integer);
        assert!(limit.required, "the branch's requirement carries through");
    }

    #[test]
    fn help_is_one_line_per_flag() {
        let schema = json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The agent id. Long tail of prose that a JSON catalog wants and a flag list does not."
                }
            }
        });
        let route = CliRoute::new(&["agents"], "get");
        let contract = contract_for(
            "get_agent",
            "Get.",
            "GET",
            "/v1/agents/{id}",
            &route,
            &schema,
        );
        assert_eq!(contract.args[0].help.as_deref(), Some("The agent id."));
    }
}
