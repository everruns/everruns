//! The grammar itself: what a host offers, and the tree built from it.

use super::*;

/// One command offered to the tree by a [`CliCommandSource`].
#[derive(Debug, Clone)]
pub struct CliCommandSpec {
    /// Canonical identity passed back to [`CliCommandSource::dispatch`].
    pub wire_name: String,
    /// One-line summary, rendered in help.
    pub description: String,
    /// Noun path from the tree root, e.g. `["agents", "versions"]`.
    ///
    /// A slice of nouns rather than one, because flat command names hide a
    /// hierarchy: `list_session_participants` is `sessions participants list`.
    /// Deriving that by string surgery is wrong for exactly the irregular
    /// names that matter, so a source declares the shape.
    pub path: Vec<String>,
    /// Leaf verb, e.g. `"list"`.
    pub verb: String,
    /// The command's grammar: flags, positionals, help, examples.
    ///
    /// A `clap::Command` rather than a description of one. The tree resolves
    /// which command a caller meant and hands the rest of argv to this; what
    /// the arguments are, and how they are spelled, is the source's business
    /// and never this crate's. A host with its own operations passes a clap
    /// derive; a host with a catalog builds one from what the catalog
    /// publishes.
    pub command: clap::Command,
}

/// Where a host's commands come from, and how one runs.
///
/// The tree owns grammar, help, and dispatch *routing*; a source owns the
/// commands themselves and their authorization. Nothing here assumes a server:
/// a Framework application implements this over its own operations.
#[async_trait]
pub trait CliCommandSource: Send + Sync {
    /// Commands to expose in the tree.
    fn specs(&self) -> Vec<CliCommandSpec>;

    /// The word that introduces an invocation: `everruns agents list`.
    ///
    /// A host's own operations are its own, so the token that names them is
    /// the host's too. The hosted product keeps the default; a Framework
    /// application administering invoices has no reason to spell them under
    /// someone else's brand.
    fn root(&self) -> &str {
        ROOT
    }

    /// One-line summaries for grouping nodes, keyed by full node path
    /// (`"agents"`, `"agents versions"`).
    ///
    /// A node is a grouping, not a command, so it has no description to
    /// borrow. Without these the root renders "agents — agents commands",
    /// which spends prompt budget and teaches nothing.
    fn node_about(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Run a command, given the arguments clap parsed for it.
    ///
    /// `ArgMatches` rather than a JSON object, because reading a match back out
    /// needs the types the parser was built with, and those belong to whoever
    /// declared the command. A host using clap derive calls
    /// `FromArgMatches::from_arg_matches`; a host with a catalog reads the
    /// fields its contract declares. Either way this crate never has to guess
    /// what a value was meant to be.
    async fn dispatch(&self, wire_name: &str, matches: clap::ArgMatches) -> Result<String, String>;
}

/// One resolved leaf: the tree spelling and the command it runs.
#[derive(Debug, Clone)]
pub struct Leaf {
    pub command: String,
    pub description: String,
    pub path: Vec<String>,
    pub verb: String,
    /// The source's parser for this leaf.
    pub parser: clap::Command,
}

/// The assembled tree. Nodes are keyed by their full path so lookup is a
/// single map hit; children are derived for help rendering.
#[derive(Debug)]
pub struct CliTree {
    /// Token that introduces an invocation, from the source that built it.
    root: String,
    node_about: BTreeMap<String, String>,
    /// "agents list" -> leaf
    leaves: BTreeMap<String, Leaf>,
    /// "agents" -> ["agents versions", ...] non-leaf child paths
    nodes: BTreeMap<String, Vec<String>>,
}

impl Default for CliTree {
    fn default() -> Self {
        Self {
            root: ROOT.to_string(),
            node_about: BTreeMap::new(),
            leaves: BTreeMap::new(),
            nodes: BTreeMap::new(),
        }
    }
}

impl CliTree {
    /// The token that introduces an invocation in this tree.
    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn leaf(&self, path: &str) -> Option<&Leaf> {
        self.leaves.get(path)
    }

    pub fn is_node(&self, path: &str) -> bool {
        self.nodes.contains_key(path)
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
            if path.is_empty() && prefix.is_empty() && spelling.is_empty() {
                continue;
            }
            let mut parts = rest.split(' ');
            let Some(head) = parts.next() else { continue };
            if parts.next().is_none() {
                // Direct leaf child: its own description.
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

    /// Build a tree from a source's declared commands.
    pub fn from_source(source: &dyn CliCommandSource) -> Self {
        let mut tree = Self {
            root: source.root().to_string(),
            node_about: source.node_about().into_iter().collect(),
            ..Self::default()
        };
        for spec in source.specs() {
            tree.insert(Leaf {
                command: spec.wire_name,
                description: spec.description,
                path: spec.path,
                verb: spec.verb,
                parser: spec.command,
            });
        }
        tree
    }

    pub(super) fn about(&self, path: &str) -> Option<&str> {
        self.node_about.get(path).map(String::as_str)
    }

    fn insert(&mut self, leaf: Leaf) {
        let mut parts = leaf.path.clone();
        parts.push(leaf.verb.clone());
        let spelling = parts.join(" ");
        // Register every ancestor path as a node so `everruns agents --help`
        // and `everruns agents versions --help` both resolve.
        for depth in 1..=leaf.path.len() {
            let node = leaf.path[..depth].join(" ");
            self.nodes.entry(node).or_default();
        }
        self.leaves.insert(spelling, leaf);
    }
}
