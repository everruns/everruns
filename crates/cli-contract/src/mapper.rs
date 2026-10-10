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
        "agents channels",
        "Ways traffic reaches an agent: chat, API, webhooks, schedules.",
    ),
    (
        "agents channels keys",
        "Keys that let callers reach an agent's API channel.",
    ),
    (
        "agents check-rules",
        "Rules the agent analyzer applies, built-in and custom.",
    ),
    (
        "agents credentials",
        "Secrets an agent needs from whoever runs it.",
    ),
    (
        "agents exposures",
        "Pause or resume all of an agent's live channels at once.",
    ),
    (
        "agents health-checks",
        "Generated smoke tests run against an agent.",
    ),
    (
        "agents health-checks latest",
        "The most recent health check for an agent.",
    ),
    (
        "agents scripts",
        "Saved shell scripts an agent owns and can run as tools.",
    ),
    (
        "agents triggers",
        "Schedules and webhooks that start agent runs.",
    ),
    (
        "agents triggers deliveries",
        "Events a trigger received, and what happened to each.",
    ),
    (
        "agents triggers runs",
        "Outcomes of the runs a trigger started.",
    ),
    (
        "budgets",
        "Spending limits for sessions, agents, users and organizations.",
    ),
    (
        "budgets ledger",
        "Charges and top-ups recorded against a budget.",
    ),
    ("budgets top-up", "Add credits to a budget."),
    (
        "capabilities",
        "Capabilities agents can use, including declarative ones you define.",
    ),
    (
        "capabilities declarative",
        "Capabilities defined as configuration rather than code.",
    ),
    (
        "capabilities guardrails",
        "Input and output checks for agents.",
    ),
    (
        "capabilities guardrails dry-run",
        "Try a guardrails config on sample text.",
    ),
    (
        "capabilities guardrails examples",
        "Ready-made guardrail presets.",
    ),
    (
        "context",
        "Notes managers keep about an entity, read by agents that manage it.",
    ),
    ("durable", "Durable scheduled tasks and their executions."),
    ("durable executions", "Single runs of a durable schedule."),
    ("durable schedules", "Cron-driven durable tasks."),
    (
        "durable schedules executions",
        "Runs of one durable schedule.",
    ),
    (
        "durable schedules stats",
        "Counts and timings for durable schedules.",
    ),
    ("evals", "Evaluation cases, runs, results and scores."),
    (
        "evals atif-import",
        "Import agent trajectories (ATIF) as eval cases.",
    ),
    ("evals cases", "Inputs and expectations an eval run checks."),
    ("evals runs", "Executions of eval cases against an agent."),
    ("evals runs artifacts", "Raw outputs of an eval run."),
    (
        "evals runs dataset",
        "Reward-labeled trajectory datasets built from a run.",
    ),
    ("evals runs results", "Per-case results of an eval run."),
    ("evals runs results scores", "Scores on one eval result."),
    (
        "evals runs scores",
        "Scores across all results of an eval run.",
    ),
    ("evals runs share", "Read-only share links for an eval run."),
    (
        "harnesses",
        "Reusable base setups (prompt plus capabilities) agents build on.",
    ),
    (
        "harnesses check-name",
        "Check whether a harness name is free.",
    ),
    (
        "health-issues",
        "Operational problems detected in this installation.",
    ),
    (
        "history",
        "Recorded changes to entities: list, compare, restore.",
    ),
    ("images", "Uploaded images."),
    (
        "knowledge-bases",
        "Curated collections of knowledge entries.",
    ),
    (
        "knowledge-bases entries",
        "Individual entries in a knowledge base.",
    ),
    (
        "knowledge-bases okf-import",
        "Import an Open Knowledge Format bundle.",
    ),
    (
        "knowledge-indexes",
        "Searchable indexes synced from external sources.",
    ),
    (
        "knowledge-indexes documents",
        "Documents held in a knowledge index.",
    ),
    (
        "mcp-servers",
        "Registered MCP servers available to agents and sessions.",
    ),
    ("memories", "Workspace memories in the organization."),
    (
        "models",
        "LLM models available to agents, and organization defaults.",
    ),
    (
        "models decision-default",
        "The model that answers internal decision checks.",
    ),
    ("models default", "The organization's default model."),
    ("notifications", "Your in-app notifications."),
    ("notifications view", "Mark notifications as seen."),
    ("observers", "Online scoring of live traces."),
    ("observers scores", "Scores an observer produced."),
    ("orgs", "Organizations you belong to."),
    (
        "orgs audit-logs",
        "Security-relevant actions in your organization.",
    ),
    (
        "orgs egress-allowlist",
        "Extra hosts your organization's agents may reach.",
    ),
    (
        "orgs egress-allowlist grant",
        "Whether an organization may extend the allowlist.",
    ),
    (
        "payments",
        "Machine payments: wallets, spend policies and attempts.",
    ),
    ("payments accounts", "Wallet accounts agents pay from."),
    ("payments attempts", "Payments agents tried to make."),
    ("payments policies", "Rules limiting what agents may spend."),
    ("plugin-marketplaces", "Sources of installable plugins."),
    (
        "plugin-marketplaces plugins",
        "Plugins a marketplace offers.",
    ),
    ("plugins", "Plugins installed in this organization."),
    ("providers", "LLM providers and their credentials."),
    (
        "providers check-credentials",
        "Test a provider API key without saving it.",
    ),
    ("providers models", "Models offered by one provider."),
    ("providers sync-models", "Refresh a provider's model list."),
    (
        "reports",
        "Usage and cost reporting: queries, saved reports, catalog.",
    ),
    ("reports admin", "Maintenance for the reporting pipeline."),
    (
        "reports admin diagnostics",
        "Reporting pipeline lag and failures.",
    ),
    (
        "reports catalog",
        "Datasets, measures and filters reports can use.",
    ),
    ("reports projector", "Process pending reporting work."),
    ("reports query", "Ad-hoc reporting queries."),
    ("reports saved", "Saved report definitions."),
    (
        "sandbox-targets",
        "Sandbox providers this deployment offers.",
    ),
    (
        "sandboxes",
        "Sandboxes across providers: state, history, usage.",
    ),
    ("sandboxes stats", "Sandbox counts by state and provider."),
    (
        "sandboxes timeline",
        "When each sandbox was running, paused or lost.",
    ),
    (
        "sessions",
        "Running and archived sessions, their state and participants.",
    ),
    (
        "sessions budget-check",
        "Check every budget that applies to a session.",
    ),
    ("sessions budgets", "Budgets attached to a session."),
    (
        "sessions databases",
        "SQL databases created inside a session.",
    ),
    (
        "sessions databases schema",
        "Tables and columns of a session database.",
    ),
    ("sessions events", "The event log of a session."),
    (
        "sessions events summary",
        "A one-shot debugging summary of a session's events.",
    ),
    ("sessions fs", "Files in a session's filesystem."),
    (
        "sessions fs -",
        "Copy, move, search and inspect session files.",
    ),
    (
        "sessions mcp-servers",
        "MCP servers added to one session only.",
    ),
    (
        "sessions messages",
        "Messages in a session; sending one starts the next turn.",
    ),
    ("sessions participants", "Who is attached to a session."),
    (
        "sessions platform-chat",
        "Your permanent Platform Chat conversation.",
    ),
    ("sessions resources", "Resources registered in a session."),
    (
        "sessions sandbox",
        "A session's sandbox: inspect, pause, resume, delete.",
    ),
    ("sessions sse", "Live event stream of a session."),
    (
        "sessions storage",
        "Key-value data and secrets stored for a session.",
    ),
    (
        "sessions storage keys",
        "Key-value pairs stored for a session.",
    ),
    (
        "sessions storage secrets",
        "Encrypted secrets stored for a session.",
    ),
    ("sessions tasks", "Background tasks a session started."),
    (
        "sessions tasks messages",
        "Messages sent to a running task.",
    ),
    (
        "sessions tasks push-configs",
        "Where a task sends progress notifications.",
    ),
    (
        "sessions tool-results",
        "Results of tools that run on the client side.",
    ),
    ("skills", "Skill packages and their content."),
    ("system", "Server status."),
    (
        "tasks",
        "Background tasks across every session in the organization.",
    ),
    ("user", "Your own account settings."),
    (
        "user connections",
        "Your connected accounts on external services.",
    ),
    (
        "user connections providers",
        "Services you can connect an account for.",
    ),
    ("users", "People in the current organization."),
    (
        "virtual-users",
        "Runtime accounts for consumers or services.",
    ),
    (
        "workspaces",
        "Durable working areas holding the files agents work on.",
    ),
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

    /// Every group in the checked-in tree describes itself.
    ///
    /// The root help is the first thing a caller reads, and a group without a
    /// line falls back to "budgets commands", which says nothing about when to
    /// open it. A new noun needs its line in [`NODE_ABOUT`]; a stale line for
    /// a group that no longer exists is caught too.
    #[test]
    fn every_group_has_a_description() {
        let mut groups = std::collections::BTreeSet::new();
        for contract in commands() {
            for depth in 1..=contract.path.len() {
                groups.insert(contract.path[..depth].join(" "));
            }
        }
        let described: std::collections::BTreeSet<String> = NODE_ABOUT
            .iter()
            .map(|(path, _)| path.to_string())
            .collect();
        let missing: Vec<&String> = groups.difference(&described).collect();
        let stale: Vec<&String> = described.difference(&groups).collect();
        assert!(
            missing.is_empty(),
            "groups without a NODE_ABOUT line: {missing:?}"
        );
        assert!(
            stale.is_empty(),
            "NODE_ABOUT lines for groups that no longer exist: {stale:?}"
        );
    }

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
