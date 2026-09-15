//! The `everruns` command-line contract: one grammar, shared by the CLI and
//! the agent-facing command tree.
//!
//! A caller learns one CLI. What a person types in a terminal and what an
//! agent types in its shell are the same words, with the same flags, the same
//! short options, the same positionals and the same help. That only stays true
//! if there is one definition, so this crate is it: the grammar as data, and
//! the single function that turns it into a [`clap::Command`].
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is consumed
//! by `everruns-cli` and by the server's agent-facing command tree.
//!
//! # Example
//!
//! ```
//! use everruns_cli_contract::{ArgKind, ContractArg, ContractCommand, ContractExample};
//!
//! let command = ContractCommand {
//!     wire_name: "list_agents".into(),
//!     path: vec!["agents".into()],
//!     verb: "list".into(),
//!     description: "List agents in the organization.".into(),
//!     method: "GET".into(),
//!     http_path: "/v1/agents".into(),
//!     args: vec![ContractArg {
//!         field: "limit".into(),
//!         long: "limit".into(),
//!         short: None,
//!         position: None,
//!         kind: ArgKind::Integer,
//!         required: false,
//!         help: Some("Maximum rows to return.".into()),
//!         choices: vec![],
//!     }],
//!     examples: vec![ContractExample {
//!         intent: "List the ten most recent agents".into(),
//!         command: "everruns agents list --limit 10".into(),
//!     }],
//! };
//!
//! assert_eq!(command.spelling(), "agents list");
//! assert!(command.after_help().contains("List the ten most recent agents:"));
//! ```

use clap::builder::{BoolishValueParser, PossibleValuesParser};
use clap::{Arg, ArgAction, ColorChoice, Command};
use serde::{Deserialize, Serialize};

/// What one argument accepts.
///
/// A reduction of JSON Schema to the shapes a command line has: everything
/// else is passed as one JSON document, because a shell argument is text and
/// pretending otherwise only moves the parse somewhere less helpful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgKind {
    /// A switch. Accepts `--flag` and `--flag true`, because callers write
    /// both and one of them erroring is a round trip spent on syntax.
    Boolean,
    Integer,
    Number,
    String,
    /// Repeatable, and comma-splittable when the items are scalars.
    StringList,
    IntegerList,
    /// An object or array of objects: one JSON document.
    Json,
}

/// One argument of one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractArg {
    /// Parameter name as the command deserializes it, e.g. `system_prompt`.
    /// This is what dispatch sends, whatever the caller typed.
    pub field: String,
    /// Long spelling, without dashes, e.g. `system-prompt`.
    pub long: String,
    /// Short spelling, when the command declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    /// Position when this argument is also spelled as a bare word, 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub kind: ArgKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Declared values, surfaced so help lists them and a wrong one is
    /// corrected rather than dispatched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

/// One worked example, in the shape yolop's commands use: a line saying what
/// the caller is trying to do, then the command that does it.
///
/// Both halves matter, and a bare command line is the half that gets written
/// when the type does not ask for the other. `yolop sessions search --query X`
/// tells a reader the syntax they could have guessed; "Search prior sessions
/// for an exact marker" tells them when to reach for it. An agent reading
/// `--help` is choosing between commands, not recalling one it already knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractExample {
    /// What this invocation accomplishes, as a phrase. No trailing period:
    /// the renderer adds the colon.
    pub intent: String,
    /// The complete, runnable command line.
    pub command: String,
}

/// One command: where it sits, what it is called on the wire, how it is
/// reached over HTTP, and what it accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractCommand {
    /// Canonical identity, e.g. `list_agents`.
    pub wire_name: String,
    /// Noun path from the root, e.g. `["agents", "versions"]`.
    pub path: Vec<String>,
    /// Leaf verb, e.g. `list`.
    pub verb: String,
    pub description: String,
    /// HTTP method and path template, so a consumer holding only an API client
    /// can run the command without a hand-written implementation for it.
    pub method: String,
    pub http_path: String,
    #[serde(default)]
    pub args: Vec<ContractArg>,
    /// Worked examples, rendered under the command's help.
    ///
    /// Not optional in practice: a guard asserts every routed command carries
    /// at least one, because a command surface an agent cannot learn from
    /// `--help` is one it will guess at instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ContractExample>,
}

impl ContractCommand {
    /// The block rendered under a command's help: worked examples, then the
    /// flat name the same command answers to.
    pub fn after_help(&self) -> String {
        let mut text = String::new();
        if !self.examples.is_empty() {
            text.push_str("Examples:\n");
            for (index, example) in self.examples.iter().enumerate() {
                if index > 0 {
                    text.push('\n');
                }
                text.push_str(&format!("  {}:\n    {}\n", example.intent, example.command));
            }
            text.push('\n');
        }
        text.push_str(&format!("Wire name: {}", self.wire_name));
        text
    }

    /// Space-joined spelling, e.g. `agents versions list`.
    pub fn spelling(&self) -> String {
        let mut parts = self.path.clone();
        parts.push(self.verb.clone());
        parts.join(" ")
    }

    /// The parser and the help for this command, as both consumers see it.
    ///
    /// `display_name` is the spelling the caller typed, so usage and errors
    /// read back the grammar they used rather than a wire name they did not.
    pub fn clap_command(&self, display_name: &str) -> Command {
        // clap panics on a duplicate argument id, and an argument named `help`
        // would collide with the flag clap adds for itself. Commands come from
        // a catalog this code does not control, so the collision is resolved
        // deliberately: the declared argument wins and that command has no
        // `--help`. A panic inside an agent's shell is not an acceptable
        // answer to an unusual field name.
        let declares_help = self.args.iter().any(|arg| arg.long == "help");

        let mut command = Command::new(display_name.to_string())
            .about(self.description.clone())
            .disable_version_flag(true)
            .disable_help_flag(declares_help)
            // The workspace links clap with its default features for the CLI,
            // and cargo unifies that across the build, so colour is on unless
            // a command says otherwise. Escape bytes are noise in a terminal
            // and cost tokens in a tool result.
            .color(ColorChoice::Never);

        command = command.after_help(self.after_help());

        for arg in &self.args {
            command = command.arg(flag(arg));
            if arg.position.is_some() {
                command = command.arg(positional(arg));
            }
        }
        command
    }
}

/// Suffix that lets a bare-word spelling coexist with the same argument's
/// long flag. Both reach one parameter, and giving both is a conflict clap
/// reports itself rather than a silent winner.
pub const POSITIONAL_SUFFIX: &str = "\u{1}positional";

fn flag(arg: &ContractArg) -> Arg {
    let mut built = Arg::new(arg.field.clone()).long(arg.long.clone());

    // Parameters are named in snake_case because they are generated from Rust
    // types, while the CLI spells them kebab. Both reach the same parameter,
    // so a script written against either keeps working.
    if arg.long != arg.field {
        built = built.alias(arg.field.clone());
    }
    if let Some(short) = arg.short {
        built = built.short(short);
    }
    if let Some(help) = &arg.help {
        built = built.help(help.clone());
    }
    // An argument that also has a bare-word spelling cannot be required as a
    // flag: the positional may be what supplies it.
    built = built.required(arg.required && arg.position.is_none());
    apply_kind(built, arg)
}

fn positional(arg: &ContractArg) -> Arg {
    let built = Arg::new(format!("{}{POSITIONAL_SUFFIX}", arg.field))
        .index(arg.position.unwrap_or(1))
        .value_name(arg.long.to_uppercase().replace('-', "_"))
        .conflicts_with(arg.field.clone())
        .required(false)
        .help(match &arg.help {
            Some(help) => format!("{help} (may also be given as --{})", arg.long),
            None => format!("{} (may also be given as --{})", arg.field, arg.long),
        });
    apply_kind(built, arg)
}

fn apply_kind(built: Arg, arg: &ContractArg) -> Arg {
    match arg.kind {
        ArgKind::Boolean => built
            .num_args(0..=1)
            .default_missing_value("true")
            .value_parser(BoolishValueParser::new()),
        ArgKind::Integer => built.value_parser(clap::value_parser!(i64)),
        ArgKind::Number => built.value_parser(clap::value_parser!(f64)),
        ArgKind::String if !arg.choices.is_empty() => {
            built.value_parser(PossibleValuesParser::new(arg.choices.clone()))
        }
        ArgKind::String | ArgKind::Json => built,
        // Comma-splitting is safe for scalars and wrong for documents, which
        // may carry a comma of their own.
        ArgKind::StringList | ArgKind::IntegerList => {
            built.action(ArgAction::Append).value_delimiter(',')
        }
    }
}

pub mod render;
