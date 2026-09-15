// Argument parsing for one leaf of the `everruns` tree.
//
// The grammar a caller types is a CLI, so it is parsed by the crate every
// other CLI in this workspace is parsed by. `crates/cli` cannot lend its
// definition: it is a different surface (clap derive over the SDK, with
// client-side work like reading an agent file or walking `initial_files`),
// and the commands spelled here are the control plane's own. What is shared
// is the parser and its conventions, which is what a caller actually feels:
// `--flag=value`, short flags, `--`, "unexpected argument", "did you mean".
//
// So a leaf's flags are not hand-parsed. Each command already publishes a
// JSON Schema for its parameters, and that schema is compiled into a
// [`clap::Command`]. Three things follow that no hand-rolled parser here had:
//
//   * An unknown flag is an error. The previous parsers kept `--limti 10` as
//     a string property and let it fall off a schema check much later, or
//     silently, which is the failure mode a typo should never have.
//   * A required field is enforced where the caller can still fix it, with
//     the usage block attached, rather than as a deserialization error from
//     the far side of a dispatch.
//   * A positional argument is declared, not faked. `get_agent agt_1` is an
//     ordinary clap positional, so the statement-boundary pre-rewrite that
//     inserts `--id` before a bare word (EVE-323) has nothing to do on this
//     path.
//
// `clap::Command` is a runtime builder over owned strings. That matters
// beyond tidiness: it is why a tree assembled from specs fetched at runtime
// is possible at all, where `CliRoute`, being `&'static`, cannot be.

use clap::builder::{BoolishValueParser, PossibleValuesParser};
use clap::{Arg, ArgAction, ArgMatches, ColorChoice, Command, value_parser};
use serde_json::{Map, Value};

/// How deep `$ref` and `allOf` chains are followed before a schema is treated
/// as opaque. Schemas are generated from Rust types, so real nesting is
/// shallow; the bound exists to stop a cyclic `$ref` from looping.
const MAX_SCHEMA_DEPTH: u8 = 8;

/// What one parameter accepts, reduced from its JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A switch. Accepts `--flag` and `--flag true`, because callers write
    /// both and one of them erroring is a round trip spent on syntax.
    Boolean,
    Integer,
    Number,
    String,
    /// Repeatable, and comma-splittable when the items are scalars.
    Array(Box<Kind>),
    /// An object, or anything else this reduction does not model: passed as
    /// one JSON document and parsed at conversion time.
    Json,
}

/// One parameter, resolved once so the clap argument and the JSON conversion
/// cannot disagree about what it is.
#[derive(Debug, Clone)]
struct Field {
    name: String,
    kind: Kind,
    required: bool,
    help: Option<String>,
    /// Declared `enum` values, surfaced to clap so help lists them and a
    /// wrong one is corrected rather than dispatched.
    choices: Vec<String>,
}

/// A leaf's parsed grammar: the clap command, and the fields to read a
/// successful match back out with.
pub struct LeafCommand {
    command: Command,
    fields: Vec<Field>,
    positional: Option<String>,
}

/// Suffix clap gives a positional's argument id so it can coexist with the
/// same field's long flag. Both spellings reach the same parameter, and
/// supplying both is a conflict clap reports itself.
const POSITIONAL_SUFFIX: &str = "\u{1}positional";

impl LeafCommand {
    /// Compile one leaf's schema into a clap command.
    ///
    /// `display_name` is the spelling the caller typed (`everruns agents
    /// list`), so usage and errors read back the grammar they used rather
    /// than the flat wire name they did not.
    pub fn new(
        display_name: &str,
        about: &str,
        schema: &Value,
        positional: Option<&str>,
        examples: &[String],
        wire_name: &str,
    ) -> Self {
        let fields = fields_of(schema);

        // clap panics on a duplicate argument id, and a parameter named
        // `help` would collide with the flag clap adds for itself. Commands
        // come from a catalog this code does not control, so the collision is
        // resolved deliberately: the declared parameter wins, and that leaf
        // simply has no `--help`. A panic inside the agent's shell is not an
        // acceptable answer to an unusual field name.
        let declares_help = fields.iter().any(|field| field.name == "help");

        let mut command = Command::new(display_name.to_string())
            .about(about.to_string())
            .no_binary_name(false)
            .disable_version_flag(true)
            .disable_help_flag(declares_help)
            // The workspace links clap with its default features because
            // `crates/cli` wants them, and cargo unifies that across every
            // crate in one build. Colour would then reach an agent's stdout as
            // escape bytes it has to read past. Say never, rather than
            // depending on a feature set chosen elsewhere.
            .color(ColorChoice::Never);

        let mut after_help = String::new();
        if !examples.is_empty() {
            after_help.push_str("Examples:\n");
            for example in examples {
                after_help.push_str(&format!("  {example}\n"));
            }
            after_help.push('\n');
        }
        after_help.push_str(&format!("Wire name: {wire_name}"));
        command = command.after_help(after_help);

        for field in &fields {
            command = command.arg(flag_arg(field, positional));
        }
        if let Some(name) = positional
            && let Some(field) = fields.iter().find(|field| field.name == name)
        {
            command = command.arg(positional_arg(field));
        }

        Self {
            command,
            fields,
            positional: positional.map(ToOwned::to_owned),
        }
    }

    /// Parse argv into the parameter object the source dispatches on.
    ///
    /// `--help` is not special-cased here: clap renders it, from the same
    /// schema the parse uses, so the help a caller reads and the arguments
    /// they are then held to cannot drift apart. `Ok(None)` means clap
    /// produced terminal output (help) and there is nothing to dispatch.
    pub fn parse(&self, args: &[String]) -> Result<Option<Value>, ClapFailure> {
        let argv = std::iter::once(self.command.get_name().to_string()).chain(args.iter().cloned());

        let matches = match self.command.clone().try_get_matches_from(argv) {
            Ok(matches) => matches,
            Err(error) => return Err(ClapFailure::from(error)),
        };

        Ok(Some(self.to_params(&matches)))
    }

    fn to_params(&self, matches: &ArgMatches) -> Value {
        let mut object = Map::new();

        for field in &self.fields {
            if let Some(value) = read_field(matches, &field.name, &field.kind) {
                object.insert(field.name.clone(), value);
            }
        }

        // A positional carries the same parameter under a distinct argument
        // id; fold it back under the field's real name.
        if let Some(name) = &self.positional
            && let Some(field) = self.fields.iter().find(|field| &field.name == name)
        {
            let id = format!("{name}{POSITIONAL_SUFFIX}");
            if let Some(value) = read_field(matches, &id, &field.kind) {
                object.insert(name.clone(), value);
            }
        }

        Value::Object(object)
    }
}

/// A clap parse that ended without parameters: either rendered help, or a
/// usage error. Both carry text the caller should see; only one is a failure.
#[derive(Debug)]
pub struct ClapFailure {
    pub message: String,
    /// True when clap printed help or a version-style response rather than
    /// rejecting the input, in which case the builtin exits zero.
    pub is_help: bool,
}

impl From<clap::Error> for ClapFailure {
    fn from(error: clap::Error) -> Self {
        use clap::error::ErrorKind;
        let is_help = matches!(
            error.kind(),
            ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        Self {
            message: error.render().to_string(),
            is_help,
        }
    }
}

/// The `--flag` spelling of one field.
fn flag_arg(field: &Field, positional: Option<&str>) -> Arg {
    let mut arg = Arg::new(field.name.clone()).long(field.name.clone());

    // Schemas name fields in snake_case because they are generated from Rust
    // structs; a CLI caller reasonably types kebab. Accept both, and keep
    // snake_case canonical so scripts already written against the flat
    // commands keep parsing.
    let kebab = field.name.replace('_', "-");
    if kebab != field.name {
        arg = arg.alias(kebab);
    }

    if let Some(help) = &field.help {
        arg = arg.help(first_line(help));
    }

    // A field that also has a positional spelling cannot be required as a
    // flag: the positional may be what supplies it.
    let required = field.required && positional != Some(field.name.as_str());
    arg = arg.required(required);

    apply_kind(arg, &field.kind, &field.choices)
}

/// The bare-word spelling of the field a command nominates as positional.
fn positional_arg(field: &Field) -> Arg {
    let arg = Arg::new(format!("{}{POSITIONAL_SUFFIX}", field.name))
        .index(1)
        .value_name(field.name.to_uppercase())
        .conflicts_with(field.name.clone())
        .required(false)
        .help(format!(
            "{} (may also be given as --{})",
            field
                .help
                .as_deref()
                .map(first_line)
                .unwrap_or_else(|| field.name.clone()),
            field.name
        ));
    apply_kind(arg, &field.kind, &field.choices)
}

fn apply_kind(arg: Arg, kind: &Kind, choices: &[String]) -> Arg {
    match kind {
        Kind::Boolean => arg
            .num_args(0..=1)
            .default_missing_value("true")
            .value_parser(BoolishValueParser::new()),
        Kind::Integer => arg.value_parser(value_parser!(i64)),
        Kind::Number => arg.value_parser(value_parser!(f64)),
        Kind::String if !choices.is_empty() => {
            arg.value_parser(PossibleValuesParser::new(choices.to_vec()))
        }
        Kind::String | Kind::Json => arg,
        Kind::Array(items) => {
            let arg = arg.action(ArgAction::Append);
            match **items {
                // Only scalars comma-split: a JSON document may contain a
                // comma of its own, and splitting it would corrupt it.
                Kind::Json | Kind::Array(_) => arg,
                _ => arg.value_delimiter(','),
            }
        }
    }
}

/// Read one parsed argument back out as JSON, in the shape its kind promised.
fn read_field(matches: &ArgMatches, id: &str, kind: &Kind) -> Option<Value> {
    // An argument clap defaulted rather than one the caller gave is left out:
    // the command's own schema owns its defaults, and sending clap's view of
    // them would overwrite a server-side default with a guess.
    if !matches!(
        matches.value_source(id),
        Some(clap::parser::ValueSource::CommandLine)
    ) {
        return None;
    }

    match kind {
        Kind::Boolean => matches.get_one::<bool>(id).copied().map(Value::Bool),
        Kind::Integer => matches
            .get_one::<i64>(id)
            .copied()
            .map(|value| Value::Number(value.into())),
        Kind::Number => matches
            .get_one::<f64>(id)
            .copied()
            .and_then(|value| serde_json::Number::from_f64(value).map(Value::Number)),
        Kind::String => matches.get_one::<String>(id).cloned().map(Value::String),
        Kind::Json => matches
            .get_one::<String>(id)
            .map(|text| json_or_string(text)),
        Kind::Array(items) => {
            let values = matches.get_many::<String>(id)?;
            let items: Vec<Value> = values
                .map(|text| match **items {
                    Kind::Integer | Kind::Number | Kind::Boolean | Kind::Json => {
                        json_or_string(text)
                    }
                    _ => Value::String(text.clone()),
                })
                .collect();
            Some(Value::Array(items))
        }
    }
}

/// Parse a value that should be JSON, keeping the raw text when it is not.
///
/// Handing the raw text on is deliberate: the command validates against its
/// own schema and will say what was wrong with it, which is a better error
/// than one invented here from no knowledge of the target type.
fn json_or_string(text: &str) -> Value {
    serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

// ============================================================================
// Schema reduction
// ============================================================================

/// Reduce a parameter schema to the fields a caller can pass.
fn fields_of(schema: &Value) -> Vec<Field> {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(Value::as_object);

    let mut properties: Map<String, Value> = Map::new();
    let mut required: Vec<String> = Vec::new();
    collect(schema, defs, &mut properties, &mut required, 0);

    properties
        .into_iter()
        .map(|(name, property)| {
            let resolved = resolve(&property, defs, 0);
            Field {
                kind: kind_of(&resolved, defs, 0),
                required: required.contains(&name),
                help: resolved
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                choices: choices_of(&resolved),
                name,
            }
        })
        .collect()
}

/// Gather properties and required names through `allOf` and `$ref` wrappers.
///
/// Generated schemas wrap flattened structs in `allOf`, so a command whose
/// parameters include `#[serde(flatten)] pagination` presents its fields one
/// level down. Missing them would make `--limit` an unknown flag.
fn collect(
    schema: &Value,
    defs: Option<&Map<String, Value>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    depth: u8,
) {
    if depth >= MAX_SCHEMA_DEPTH {
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
                // Only `allOf` contributes requirements: one branch of a
                // union requiring a field does not make it required overall.
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
    if depth >= MAX_SCHEMA_DEPTH {
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

fn kind_of(schema: &Value, defs: Option<&Map<String, Value>>, depth: u8) -> Kind {
    if depth >= MAX_SCHEMA_DEPTH {
        return Kind::Json;
    }

    // `Option<T>` renders as a union with null; the flag is simply optional,
    // and its type is whatever the non-null branch says.
    if let Some(branches) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let mut concrete = branches
            .iter()
            .map(|branch| resolve(branch, defs, depth))
            .filter(|branch| !is_null_type(branch));
        if let Some(first) = concrete.next()
            && concrete.next().is_none()
        {
            return kind_of(&first, defs, depth + 1);
        }
        return Kind::Json;
    }

    match type_name(schema) {
        Some("boolean") => Kind::Boolean,
        Some("integer") => Kind::Integer,
        Some("number") => Kind::Number,
        Some("string") => Kind::String,
        Some("array") => {
            let items = schema
                .get("items")
                .map(|items| resolve(items, defs, depth))
                .unwrap_or(Value::Null);
            Kind::Array(Box::new(kind_of(&items, defs, depth + 1)))
        }
        _ => Kind::Json,
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

fn is_null_type(schema: &Value) -> bool {
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
