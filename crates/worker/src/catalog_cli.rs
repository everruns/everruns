//! The `everruns` command line, served from inside the worker's shell.
//!
//! A shell-surface harness has no `execute` tool, so the shell has to reach the
//! catalog itself. It does that here: the grammar is local, and only dispatch
//! crosses to the control plane.
//!
//! That the grammar can be local at all is recent. The first version of this
//! forwarded raw argv to the `execute` tool, because the tree was built from
//! `&'static CliRoute` values that a worker cannot synthesize from fetched
//! data. The shared contract replaced those with owned `clap::Command`s
//! shipped in `everruns-cli-contract`, so the worker now holds the whole
//! grammar as data and forwarding buys nothing.
//!
//! What that changes for a caller: `everruns agents list --help` costs no round
//! trip, and `--limit` on a command without one is rejected here rather than
//! after a control-plane call. Only a command that actually runs goes over the
//! wire, and it goes as what the grammar produced, a wire name and params
//! (`PlatformStore::platform_run`), not a command line rendered back for the
//! server to parse again. Values never pass through a second shell, so there
//! is no quoting to get wrong.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use everruns_capabilities::PlatformStore;
use everruns_cli_contract::Mapper;
use everruns_cli_contract::mapper::NODE_ABOUT;
use everruns_integrations::bashkit::cli::{
    CliCommandSource, CliCommandSourceHandle, CliCommandSpec, CliTree,
};

/// Serves the `everruns` tree from the checked-in contract, dispatching over
/// the platform store.
pub struct CatalogCommandSource {
    store: Arc<dyn PlatformStore>,
}

impl CatalogCommandSource {
    pub fn handle(store: Arc<dyn PlatformStore>) -> CliCommandSourceHandle {
        CliCommandSourceHandle(Arc::new(Self { store }))
    }
}

#[async_trait]
impl CliCommandSource for CatalogCommandSource {
    fn specs(&self) -> Vec<CliCommandSpec> {
        contract_specs()
    }

    fn node_about(&self) -> Vec<(String, String)> {
        NODE_ABOUT
            .iter()
            .map(|(path, about)| ((*path).to_string(), (*about).to_string()))
            .collect()
    }

    /// The shared mapper's tree, so the worker resolves a line exactly as the
    /// server and the CLI do, and builds it once per process.
    fn shared_tree(&self) -> Option<Arc<CliTree>> {
        Some(shared_tree())
    }

    async fn dispatch(&self, wire_name: &str, matches: clap::ArgMatches) -> Result<String, String> {
        let Some(contract) = Mapper::everruns().contract(wire_name) else {
            return Err(format!("unknown command `{wire_name}`"));
        };
        let params = everruns_cli_contract::params_from(contract, &matches);
        self.store
            .platform_run(wire_name, params)
            .await
            .map_err(|error| error.to_string())
    }
}

fn shared_tree() -> Arc<CliTree> {
    static TREE: OnceLock<Arc<CliTree>> = OnceLock::new();
    TREE.get_or_init(|| Arc::new(Mapper::everruns().tree().clone()))
        .clone()
}

/// Every routed command, as the tree's specs. Free of the store so the grammar
/// can be checked without one.
fn contract_specs() -> Vec<CliCommandSpec> {
    everruns_cli_contract::commands()
        .iter()
        .map(|contract| CliCommandSpec {
            wire_name: contract.wire_name.clone(),
            description: contract.description.clone(),
            path: contract.path.clone(),
            verb: contract.verb.clone(),
            command: contract.clap_command(&format!("everruns {}", contract.spelling())),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree is the contract's, so every routed command is spelled here and
    /// the worker needs no round trip to know the grammar.
    /// The worker serves the shared mapper's tree, node descriptions included,
    /// so its root help reads as the server's does.
    #[test]
    fn the_shell_uses_the_shared_tree() {
        let tree = shared_tree();
        assert_eq!(tree.about("agents"), Some(NODE_ABOUT[0].1));
        assert!(tree.leaf("harnesses destroy").is_some());
    }

    #[test]
    fn the_source_serves_every_routed_command() {
        let specs = contract_specs();
        assert_eq!(specs.len(), everruns_cli_contract::commands().len());
        assert!(specs.iter().any(|spec| spec.wire_name == "list_agents"));
        assert!(
            specs
                .iter()
                .any(|spec| spec.path == ["agents", "channels"] && spec.verb == "publish")
        );
    }
}

#[cfg(test)]
mod dispatch_tests {
    use std::sync::Mutex;

    use super::*;
    use crate::core::{AgentDefinition, ExecutionSession, HarnessDefinition};
    use everruns_capabilities::{PlatformCreateSessionRequest, PlatformMessage};
    use everruns_contracts::error::Result;
    use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId, SessionParticipantId};

    /// Records what the shell sends; nothing else is reachable from it.
    #[derive(Default)]
    struct RecordingStore {
        runs: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait]
    impl PlatformStore for RecordingStore {
        async fn platform_execute(&self, _arguments: serde_json::Value) -> Result<String> {
            panic!("the worker shell must not send a rendered command line");
        }
        async fn platform_run(&self, command: &str, params: serde_json::Value) -> Result<String> {
            self.runs
                .lock()
                .unwrap()
                .push((command.to_string(), params));
            Ok("{}".into())
        }
        async fn get_harness(&self, _: HarnessId) -> Result<Option<HarnessDefinition>> {
            unimplemented!()
        }
        async fn get_agent_by_id(&self, _: AgentId) -> Result<Option<AgentDefinition>> {
            unimplemented!()
        }
        async fn create_session_with_options(
            &self,
            _: PlatformCreateSessionRequest,
        ) -> Result<ExecutionSession> {
            unimplemented!()
        }
        async fn get_session_by_id(&self, _: SessionId) -> Result<Option<ExecutionSession>> {
            unimplemented!()
        }
        async fn add_agent_session_participant(
            &self,
            _: SessionId,
            _: AgentId,
        ) -> Result<SessionParticipantId> {
            unimplemented!()
        }
        async fn send_message(&self, _: SessionId, _: &str) -> Result<()> {
            unimplemented!()
        }
        async fn get_messages(
            &self,
            _: SessionId,
            _: Option<usize>,
        ) -> Result<Vec<PlatformMessage>> {
            unimplemented!()
        }
        async fn wait_for_idle(&self, _: SessionId, _: Option<u64>) -> Result<String> {
            unimplemented!()
        }
    }

    /// A parsed line crosses as the wire name and the params the grammar
    /// produced, so a value full of shell syntax arrives byte for byte.
    #[tokio::test]
    async fn a_parsed_line_is_sent_as_name_and_params() {
        let store = Arc::new(RecordingStore::default());
        let source = CatalogCommandSource {
            store: store.clone(),
        };
        let spec = contract_specs()
            .into_iter()
            .find(|spec| spec.wire_name == "list_agents")
            .expect("list_agents is routed");
        let hostile = "a'; rm -rf / $(id) `x`";
        let matches = spec
            .command
            .try_get_matches_from(["list", "--search", hostile, "--limit", "3"])
            .expect("the line parses");

        source.dispatch("list_agents", matches).await.unwrap();

        let runs = store.runs.lock().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "list_agents");
        assert_eq!(runs[0].1["search"], hostile);
        assert_eq!(runs[0].1["limit"], 3);
    }
}
