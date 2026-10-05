//! What a host offers the tree, and the tree built from it.

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

    /// A tree built once and shared, for a source whose commands never change.
    ///
    /// Building a tree compiles a parser per command, and the shell installs
    /// the builtin per execution. A source over a fixed catalog returns its
    /// prebuilt tree here so that cost is paid once per process; the default
    /// builds from [`specs`](Self::specs) each time.
    fn shared_tree(&self) -> Option<Arc<CliTree>> {
        None
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

/// Build the tree for a source's declared commands.
pub fn tree_from_source(source: &dyn CliCommandSource) -> CliTree {
    let mut tree = CliTree::new(source.root()).with_node_about(source.node_about());
    for spec in source.specs() {
        tree.insert(Leaf {
            wire_name: spec.wire_name,
            description: spec.description,
            path: spec.path,
            verb: spec.verb,
            parser: spec.command,
        });
    }
    tree
}
