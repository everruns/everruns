//! How a command declares its place in the command line.
//!
//! A command's parameters are already described by the schema it publishes.
//! What a schema cannot know is presentation: that `--harness` is worth a `-H`,
//! that `agents get` reads better with the id as a bare word, that a reader
//! needs two worked examples to choose this command over its neighbour. Those
//! are choices a person makes, so they are declared next to the command rather
//! than inferred from its fields.
//!
//! Everything here is `&'static` and const-constructible, so a command
//! declares its route as a constant and the compiler checks it.

/// Presentation for one of a command's parameters.
///
/// Only the parameters that need something beyond the default are declared.
/// The default is a kebab-cased long flag, which is what most parameters want.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliArg {
    /// Parameter name as the command deserializes it, e.g. `harness_name`.
    pub field: &'static str,
    /// Short option, when the parameter is common enough to earn one.
    ///
    /// Short options are declared, never derived: deriving from first letters
    /// collides, and worse, shifts as fields are added, so a script written
    /// today breaks when an unrelated parameter appears next to it.
    pub short: Option<char>,
    /// Position when the parameter is also spelled as a bare word, 1-based.
    pub position: Option<usize>,
    /// Long spelling, when it differs from the kebab-cased field name.
    pub long: Option<&'static str>,
}

impl CliArg {
    pub const fn new(field: &'static str) -> Self {
        Self {
            field,
            short: None,
            position: None,
            long: None,
        }
    }

    /// Give this parameter a short option, e.g. `-H` for `--harness`.
    pub const fn short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    /// Also accept this parameter as a bare word at `position` (1-based).
    pub const fn at(mut self, position: usize) -> Self {
        self.position = Some(position);
        self
    }

    /// Spell the long flag differently from the field name, e.g. the field
    /// `harness_name` spelled `--harness`.
    pub const fn long(mut self, long: &'static str) -> Self {
        self.long = Some(long);
        self
    }
}

/// One worked example, declared as an intent and the command line that serves
/// it. See [`crate::ContractExample`] for why both halves are required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliExample {
    pub intent: &'static str,
    pub command: &'static str,
}

impl CliExample {
    pub const fn new(intent: &'static str, command: &'static str) -> Self {
        Self { intent, command }
    }
}

/// Where a command sits in the command line, and how it presents itself.
///
/// Opt-in by construction: a command joins the command line only by declaring
/// one, so internal plumbing cannot reach an agent- or human-facing surface by
/// being written.
///
/// `path` is a slice rather than a single noun because flat command names hide
/// a hierarchy: `list_session_participants` is `sessions participants list`.
/// Deriving that by string surgery is wrong for exactly the irregular names
/// that matter, so the shape is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliRoute {
    /// Noun path from the root, e.g. `["agents"]` or `["agents", "versions"]`.
    pub path: &'static [&'static str],
    /// Leaf verb, e.g. `"list"`.
    pub verb: &'static str,
    /// Presentation for the parameters that need more than the default.
    pub args: &'static [CliArg],
    /// Worked examples, rendered under the command's help.
    pub examples: &'static [CliExample],
}

impl CliRoute {
    pub const fn new(path: &'static [&'static str], verb: &'static str) -> Self {
        Self {
            path,
            verb,
            args: &[],
            examples: &[],
        }
    }

    pub const fn with_args(mut self, args: &'static [CliArg]) -> Self {
        self.args = args;
        self
    }

    pub const fn with_examples(mut self, examples: &'static [CliExample]) -> Self {
        self.examples = examples;
        self
    }

    /// Space-joined spelling, e.g. `"agents versions list"`.
    pub fn spelling(&self) -> String {
        let mut parts = self.path.to_vec();
        parts.push(self.verb);
        parts.join(" ")
    }

    /// The declaration for one parameter, if it has one.
    pub fn arg(&self, field: &str) -> Option<&CliArg> {
        self.args.iter().find(|arg| arg.field == field)
    }
}
