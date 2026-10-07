//! The noun-verb command tree: resolving argv to one command, and the help a
//! caller reads when it does not resolve.
//!
//! Decision: this is the one place argv becomes a command. The terminal CLI,
//! the agent's shell builtin, the worker's shell and MCP `execute` each used
//! to walk the words themselves, and each walk drifted a little from the
//! others (which word is blamed for a typo, whether `--help` on an unknown
//! noun fails, whether a positional is real). Every host now hands its argv
//! here and gets back the same answer.
//!
//! The tree is schema-shaped: a leaf is a wire name and a [`clap::Command`],
//! nothing about what the command does. A host with a catalog builds leaves
//! from [`ContractCommand`](crate::ContractCommand)s (see
//! [`Mapper`](crate::mapper::Mapper)); a Framework application builds them
//! from its own clap derives.
//!
//! Help is bounded by the tree's shape rather than by a cap: the root lists
//! nouns, a node lists its children, a leaf renders its own flags. That is
//! what makes `--help` affordable where a flat namespace of hundreds of
//! commands has to forbid it.

use std::collections::BTreeMap;

/// Root token that introduces an invocation of the Everruns tree.
pub const ROOT: &str = "everruns";

/// One command in the tree: its spelling and the parser for its arguments.
#[derive(Debug, Clone)]
pub struct Leaf {
    /// Canonical identity, e.g. `list_agents`.
    pub wire_name: String,
    /// One-line summary, rendered in a node's help.
    pub description: String,
    /// Noun path from the root, e.g. `["agents", "triggers"]`.
    pub path: Vec<String>,
    /// Leaf verb, e.g. `list`.
    pub verb: String,
    /// The command's own grammar: flags, positionals, help, examples.
    pub parser: clap::Command,
}

impl Leaf {
    /// Space-joined spelling below the root, e.g. `agents triggers list`.
    pub fn spelling(&self) -> String {
        let mut parts = self.path.clone();
        parts.push(self.verb.clone());
        parts.join(" ")
    }
}

/// What argv resolved to.
#[derive(Debug)]
pub enum Parsed {
    /// A leaf whose arguments parsed.
    Run {
        wire_name: String,
        matches: clap::ArgMatches,
    },
    /// Text the caller asked for (help), with nothing run.
    Output(String),
    /// A rejection, with the guidance to fix it.
    Error(String),
}

/// The assembled tree. Nodes are keyed by their full path so lookup is a
/// single map hit; children are derived for help rendering.
#[derive(Debug, Clone)]
pub struct CommandTree {
    root: String,
    node_about: BTreeMap<String, String>,
    /// "agents list" -> leaf
    leaves: BTreeMap<String, Leaf>,
    /// Every ancestor path of a leaf, e.g. "agents", "agents triggers".
    nodes: BTreeMap<String, ()>,
}

impl Default for CommandTree {
    fn default() -> Self {
        Self::new(ROOT)
    }
}

impl CommandTree {
    /// An empty tree introduced by `root`.
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            node_about: BTreeMap::new(),
            leaves: BTreeMap::new(),
            nodes: BTreeMap::new(),
        }
    }

    /// One-line summaries for grouping nodes, keyed by full node path
    /// (`"agents"`, `"agents triggers"`).
    ///
    /// A node is a grouping, not a command, so it has no description to
    /// borrow. Without these the root renders "agents commands", which spends
    /// prompt budget and teaches nothing.
    pub fn with_node_about<K, V>(mut self, about: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.node_about
            .extend(about.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Add a leaf, registering every ancestor path as a node so `--help` on
    /// each of them resolves.
    pub fn insert(&mut self, leaf: Leaf) {
        for depth in 1..=leaf.path.len() {
            self.nodes.insert(leaf.path[..depth].join(" "), ());
        }
        self.leaves.insert(leaf.spelling(), leaf);
    }

    /// The token that introduces an invocation in this tree.
    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn leaf(&self, path: &str) -> Option<&Leaf> {
        self.leaves.get(path)
    }

    pub fn leaves(&self) -> impl Iterator<Item = &Leaf> {
        self.leaves.values()
    }

    pub fn is_node(&self, path: &str) -> bool {
        self.nodes.contains_key(path)
    }

    pub fn about(&self, path: &str) -> Option<&str> {
        self.node_about.get(path).map(String::as_str)
    }

    /// Direct children of a node path, as (last segment, description) pairs.
    /// The empty path returns the tree's top-level nouns.
    pub fn children(&self, path: &str) -> Vec<(String, String)> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path} ")
        };
        let mut seen: BTreeMap<String, String> = BTreeMap::new();

        for (spelling, leaf) in &self.leaves {
            let Some(rest) = spelling.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let mut parts = rest.split(' ');
            let Some(head) = parts.next().filter(|head| !head.is_empty()) else {
                continue;
            };
            if parts.next().is_none() {
                seen.insert(head.to_string(), leaf.description.clone());
            } else {
                let child_path = if path.is_empty() {
                    head.to_string()
                } else {
                    format!("{path} {head}")
                };
                seen.entry(head.to_string()).or_insert_with(|| {
                    self.about(&child_path)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("{head} commands"))
                });
            }
        }

        seen.into_iter().collect()
    }

    /// Resolve argv (the words after the root token) and parse the leaf's
    /// arguments.
    ///
    /// Help beats execution: `--help` anywhere renders and runs nothing. A
    /// path that does not resolve is an error naming the first word that did
    /// not, with the real neighbours listed, never a help page that reads like
    /// success.
    pub fn parse(&self, args: &[String]) -> Parsed {
        match self.plan(args) {
            Plan::Help { path, unknown } => match self.render_help(&path, unknown.as_deref()) {
                Ok(text) => Parsed::Output(text),
                Err(text) => Parsed::Error(text),
            },
            Plan::Run { spelling, rest } => {
                let Some(leaf) = self.leaf(&spelling) else {
                    return Parsed::Error(format!("unknown command `{spelling}`"));
                };
                let parser = leaf
                    .parser
                    .clone()
                    .name(format!("{} {spelling}", self.root));
                let argv = std::iter::once(parser.get_name().to_string())
                    .chain(args[rest..].iter().cloned());
                match parser.try_get_matches_from(argv) {
                    Ok(matches) => Parsed::Run {
                        wire_name: leaf.wire_name.clone(),
                        matches,
                    },
                    // `--help` is a clap response, not a failure: it printed
                    // what the caller asked for and ran nothing.
                    Err(error) if is_display(&error) => Parsed::Output(error.render().to_string()),
                    Err(error) => Parsed::Error(error.render().to_string()),
                }
            }
        }
    }

    /// Render help for a tree path, or the error for an unknown word under it.
    pub fn render_help(&self, path: &str, unknown: Option<&str>) -> Result<String, String> {
        let path = path.trim();
        let root = self.root();

        if let Some(word) = unknown.filter(|value| !value.is_empty()) {
            let scope = if path.is_empty() {
                root.to_string()
            } else {
                format!("{root} {path}")
            };
            let mut text = format!("unknown command `{word}` under `{scope}`\n\n");
            text.push_str(&self.render_children(path));
            return Err(text);
        }

        if let Some(leaf) = self.leaf(path) {
            return Ok(leaf
                .parser
                .clone()
                .name(format!("{root} {path}"))
                .render_long_help()
                .to_string());
        }

        if path.is_empty() || self.is_node(path) {
            return Ok(self.render_children(path));
        }

        Err(format!(
            "unknown command `{root} {path}`\n\n{}",
            self.render_children("")
        ))
    }

    fn plan(&self, args: &[String]) -> Plan {
        let wants_help = args.iter().any(|arg| arg == "--help" || arg == "-h");

        let mut path: Vec<&str> = Vec::new();
        let mut rest = args.len();
        for (index, arg) in args.iter().enumerate() {
            if arg.starts_with('-') {
                rest = index;
                break;
            }
            path.push(arg);
            rest = index + 1;
            if self.leaf(&path.join(" ")).is_some() {
                break;
            }
        }
        let spelling = path.join(" ");

        // A resolved leaf handles its own `--help`, so the flag rides along
        // with the rest of argv rather than being intercepted here.
        if self.leaf(&spelling).is_some() {
            return Plan::Run { spelling, rest };
        }
        if !wants_help && (spelling.is_empty() || self.is_node(&spelling)) {
            return Plan::Help {
                path: spelling,
                unknown: None,
            };
        }
        // No leaf: either help under a node, which must still fail when the
        // path is not real, or a word the tree does not know.
        let (path, unknown) = self.split_at_unknown(&spelling);
        Plan::Help { path, unknown }
    }

    /// Split a typed path into the deepest prefix the tree knows and the first
    /// segment that does not resolve under it.
    ///
    /// Walking forward matters. Collapsing to the nearest known ancestor and
    /// blaming whatever word was left over reports the *last* token, so
    /// `gadgets resize thing 4` becomes "unknown command `4`" when `gadgets` is
    /// the word that does not exist.
    fn split_at_unknown(&self, path: &str) -> (String, Option<String>) {
        let mut known: Vec<&str> = Vec::new();
        for segment in path.split(' ').filter(|segment| !segment.is_empty()) {
            let mut candidate = known.clone();
            candidate.push(segment);
            let joined = candidate.join(" ");
            if self.is_node(&joined) || self.leaf(&joined).is_some() {
                known = candidate;
            } else {
                return (known.join(" "), Some(segment.to_string()));
            }
        }
        (known.join(" "), None)
    }

    fn render_children(&self, path: &str) -> String {
        let children = self.children(path);
        let root = self.root();
        let scope = if path.is_empty() {
            root.to_string()
        } else {
            format!("{root} {path}")
        };

        if children.is_empty() {
            return format!("{scope}\n  no commands available\n");
        }

        let width = children
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0)
            .min(24);

        let mut text = match self.about(path) {
            Some(about) => {
                format!("{scope}\n  {about}\n\nUsage: {scope} <command> [--flags]\n\nCommands:\n")
            }
            None => format!("Usage: {scope} <command> [--flags]\n\nCommands:\n"),
        };
        for (name, description) in children {
            let summary = first_sentence(&description);
            text.push_str(&format!("  {name:<width$}  {summary}\n"));
        }
        text.push_str(&format!("\nRun `{scope} <command> --help` for flags.\n"));
        text
    }
}

enum Plan {
    Help {
        path: String,
        unknown: Option<String>,
    },
    Run {
        /// Tree spelling the caller typed, for usage and errors.
        spelling: String,
        /// Index in argv where the leaf's own arguments start.
        rest: usize,
    },
}

/// Whether a clap error is a rendered response rather than a rejection.
fn is_display(error: &clap::Error) -> bool {
    use clap::error::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

/// Help lines stay one line each: catalog descriptions are written for a JSON
/// catalog and often run several sentences.
fn first_sentence(description: &str) -> String {
    let trimmed = description.trim();
    let end = trimmed
        .find(". ")
        .map(|index| index + 1)
        .unwrap_or(trimmed.len());
    let mut line = trimmed[..end].trim().to_string();
    if line.len() > 96 {
        // Cut on a char boundary: a description may carry multi-byte text.
        let mut cut = 93;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push_str("...");
    }
    line
}
