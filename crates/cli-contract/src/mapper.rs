//! The single argv-to-command mapper for the Everruns contract.
//!
//! Every surface that accepts `everruns <noun> <verb> --flags` (the terminal
//! CLI, MCP `execute` and `query`, the Platform capability, the worker's
//! shell) resolves the words here and gets the same answer: a wire name and a
//! JSON params object, help text, or an error with guidance. What happens
//! next differs only in transport: an in-process dispatch, a gRPC call, or
//! `POST /v1/commands/{name}`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::tree::{CommandTree, Leaf, Parsed, ROOT};
use crate::{ContractCommand, commands, params_from};

/// One-line summaries for the grouping nodes of the Everruns tree.
///
/// A node has no command to borrow a description from, and the fallback
/// ("agents commands") spends prompt budget teaching nothing. Kept beside the
/// contract so every host's root help reads the same.
pub const NODE_ABOUT: &[(&str, &str)] = &[
    ("agents", "Agent definitions and their configuration."),
    (
        "mcp-servers",
        "Registered MCP servers available to agents and sessions.",
    ),
    (
        "sessions",
        "Running and archived sessions, their state and participants.",
    ),
    ("sessions participants", "Who is attached to a session."),
    ("skills", "Skill packages and their content."),
];

/// What one invocation resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Run this command with these params. Only arguments the caller gave
    /// are present; defaults belong to the command.
    Run { wire_name: String, params: Value },
    /// Text the caller asked for (help); nothing runs.
    Output(String),
    /// A rejection the caller can act on: unknown word, bad flag, missing
    /// value. Carries the usage that fixes it.
    Error(String),
}

/// Contract commands arranged as a tree, with argv resolution.
#[derive(Debug, Clone)]
pub struct Mapper {
    tree: CommandTree,
    contracts: BTreeMap<String, ContractCommand>,
}

impl Mapper {
    /// Build a mapper over `commands`, introduced by `root`.
    pub fn new<'a>(
        root: &str,
        node_about: &[(&str, &str)],
        commands: impl IntoIterator<Item = &'a ContractCommand>,
    ) -> Self {
        let mut tree = CommandTree::new(root).with_node_about(node_about.iter().copied());
        let mut contracts = BTreeMap::new();
        for contract in commands {
            tree.insert(Leaf {
                wire_name: contract.wire_name.clone(),
                description: contract.description.clone(),
                path: contract.path.clone(),
                verb: contract.verb.clone(),
                parser: contract.clap_command(&format!("{root} {}", contract.spelling())),
            });
            contracts.insert(contract.wire_name.clone(), contract.clone());
        }
        Self { tree, contracts }
    }

    /// The checked-in Everruns contract, built once per process.
    pub fn everruns() -> &'static Mapper {
        static MAPPER: OnceLock<Mapper> = OnceLock::new();
        MAPPER.get_or_init(|| Mapper::new(ROOT, NODE_ABOUT, commands()))
    }

    pub fn tree(&self) -> &CommandTree {
        &self.tree
    }

    pub fn contract(&self, wire_name: &str) -> Option<&ContractCommand> {
        self.contracts.get(wire_name)
    }

    /// Every command this mapper resolves to, by wire name.
    pub fn contracts(&self) -> impl Iterator<Item = &ContractCommand> {
        self.contracts.values()
    }

    /// Resolve argv, the words after the root token.
    pub fn resolve<S: AsRef<str>>(&self, argv: &[S]) -> Resolution {
        let args: Vec<String> = argv.iter().map(|arg| arg.as_ref().to_string()).collect();
        match self.tree.parse(&args) {
            Parsed::Output(text) => Resolution::Output(text),
            Parsed::Error(text) => Resolution::Error(text),
            Parsed::Run { wire_name, matches } => match self.contracts.get(&wire_name) {
                Some(contract) => Resolution::Run {
                    params: params_from(contract, &matches),
                    wire_name,
                },
                None => Resolution::Error(format!("unknown command `{wire_name}`")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolve(line: &str) -> Resolution {
        let argv: Vec<&str> = line.split_whitespace().collect();
        Mapper::everruns().resolve(&argv)
    }

    fn run(line: &str) -> (String, Value) {
        match resolve(line) {
            Resolution::Run { wire_name, params } => (wire_name, params),
            other => panic!("{line}: expected a run, got {other:?}"),
        }
    }

    fn error(line: &str) -> String {
        match resolve(line) {
            Resolution::Error(text) => text,
            other => panic!("{line}: expected an error, got {other:?}"),
        }
    }

    fn output(line: &str) -> String {
        match resolve(line) {
            Resolution::Output(text) => text,
            other => panic!("{line}: expected output, got {other:?}"),
        }
    }

    #[test]
    fn a_leaf_resolves_to_its_wire_name_and_typed_params() {
        let (name, params) = run("agents list --limit 5");
        assert_eq!(name, "list_agents");
        assert_eq!(params, json!({ "limit": 5 }));
    }

    #[test]
    fn a_nested_leaf_and_a_positional_resolve() {
        assert_eq!(
            run("agents triggers list --agent-id a_1").0,
            "list_agent_triggers"
        );
        let (name, params) = run("agents get agent_1");
        assert_eq!(name, "get_agent");
        assert_eq!(params, json!({ "id": "agent_1" }));
    }

    /// The four commands whose derived spelling collided with a group are
    /// reachable through the mapper too.
    #[test]
    fn hard_delete_is_its_own_verb() {
        assert_eq!(run("harnesses destroy h_1").0, "destroy_harness");
        assert_eq!(run("harnesses delete --id h_1").0, "delete_harness");
    }

    #[test]
    fn help_runs_nothing() {
        let root = output("--help");
        assert!(root.contains("agents"), "{root}");
        assert!(root.contains(NODE_ABOUT[0].1), "{root}");
        let leaf = output("agents create --help");
        assert!(leaf.contains("Usage: everruns agents create"), "{leaf}");
        assert!(leaf.contains("Wire name: create_agent"), "{leaf}");
    }

    #[test]
    fn a_bare_node_lists_its_children() {
        let node = output("agents");
        assert!(node.contains("triggers"), "{node}");
        assert!(node.contains("list"), "{node}");
    }

    #[test]
    fn an_unknown_noun_names_the_first_wrong_word() {
        let text = error("gadgets resize thing 4");
        assert!(text.contains("unknown command `gadgets`"), "{text}");
        // Help on it fails too, rather than reading as a working page.
        assert!(error("gadgets --help").contains("unknown command `gadgets`"));
    }

    #[test]
    fn a_misspelled_flag_is_corrected_before_dispatch() {
        let text = error("agents list --limti 10");
        assert!(text.contains("--limit"), "{text}");
        assert!(!text.contains('\u{1b}'), "escape bytes: {text:?}");
    }

    #[test]
    fn a_required_flag_is_enforced() {
        assert!(error("agents create").contains("--name"));
    }

    #[test]
    fn a_mapper_over_other_commands_wears_its_own_root() {
        let only = Mapper::everruns()
            .contract("list_agents")
            .expect("list_agents");
        let mapper = Mapper::new("acme", &[], [only]);
        let help = match mapper.resolve::<&str>(&[]) {
            Resolution::Output(text) => text,
            other => panic!("{other:?}"),
        };
        assert!(help.contains("Usage: acme <command>"), "{help}");
        assert!(!help.contains("everruns"), "{help}");
        assert!(matches!(
            mapper.resolve(&["agents", "create"]),
            Resolution::Error(_)
        ));
    }
}
