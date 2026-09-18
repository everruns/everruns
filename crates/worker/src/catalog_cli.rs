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
//! wire.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_integrations_bashkit::cli::{
    CliCommandSource, CliCommandSourceHandle, CliCommandSpec,
};
use everruns_platform::PlatformStore;

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

    async fn dispatch(&self, wire_name: &str, matches: clap::ArgMatches) -> Result<String, String> {
        let Some(contract) = everruns_cli_contract::commands()
            .iter()
            .find(|contract| contract.wire_name == wire_name)
        else {
            return Err(format!("unknown command `{wire_name}`"));
        };
        let params = everruns_cli_contract::params_from(contract, &matches);

        // The transport takes a script, so the parsed arguments are rendered
        // back into one canonical line. That is not a round trip saved, but it
        // is a round trip the caller no longer spends on help or on a flag that
        // was never going to parse. A direct (name, params) command transport
        // would remove the re-render; it does not exist yet.
        let line = render_line(wire_name, &params);
        self.store
            .platform_execute(serde_json::json!({ "commands": line }))
            .await
            .map_err(|error| error.to_string())
    }
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

/// Render parsed arguments as the flat invocation the catalog's bash accepts.
fn render_line(wire_name: &str, params: &serde_json::Value) -> String {
    let mut line = String::from(wire_name);
    let Some(object) = params.as_object() else {
        return line;
    };
    for (field, value) in object {
        line.push_str(&format!(" --{field}"));
        match value {
            // A boolean flag is a switch on the far side; a value would be
            // parsed as the next token.
            serde_json::Value::Bool(true) => {}
            serde_json::Value::Bool(false) => line.push_str(" false"),
            serde_json::Value::String(text) => {
                line.push(' ');
                line.push_str(&quote(text));
            }
            other => {
                line.push(' ');
                line.push_str(&quote(&other.to_string()));
            }
        }
    }
    line
}

/// Quote a value so the receiving parser sees exactly these bytes. The caller's
/// argv was already split and unquoted by this shell; without re-quoting, a
/// value built from tool output would re-parse as syntax on the far side.
fn quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | ',' | '+')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_switch_renders_without_a_value() {
        let line = render_line("list_agents", &json!({ "include_archived": true }));
        assert_eq!(line, "list_agents --include_archived");
    }

    #[test]
    fn a_number_and_a_document_survive_the_render() {
        let line = render_line("list_agents", &json!({ "limit": 10, "filter": {"a": 1} }));
        assert!(line.contains("--limit 10"), "{line}");
        assert!(line.contains(r#"--filter '{"a":1}'"#), "{line}");
    }

    /// The property the forwarding builtin had to earn and this must not lose:
    /// a value carrying shell syntax arrives as data.
    #[test]
    fn shell_syntax_in_a_value_is_neutralized() {
        let line = render_line("create_agent", &json!({ "name": "a; rm -rf /" }));
        assert_eq!(line, "create_agent --name 'a; rm -rf /'");
    }

    /// The tree is the contract's, so every routed command is spelled here and
    /// the worker needs no round trip to know the grammar.
    #[test]
    fn the_source_serves_every_routed_command() {
        let specs = contract_specs();
        assert_eq!(specs.len(), everruns_cli_contract::commands().len());
        assert!(specs.iter().any(|spec| spec.wire_name == "list_agents"));
        assert!(
            specs
                .iter()
                .any(|spec| spec.path == ["agents", "versions"] && spec.verb == "rollback")
        );
    }
}
