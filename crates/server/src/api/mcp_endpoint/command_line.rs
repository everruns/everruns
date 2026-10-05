// The `everruns` builtin inside the scripted toolset (MCP `execute` and
// `query`, and the Platform capability, which share it).
//
// Decision: a real raw-argv builtin over the shared mapper
// (`everruns_cli_contract::Mapper`), not a text rewrite into flat names. The
// rewrite had to guess at statement boundaries and quoting before the
// interpreter ran, and it parsed flags with bashkit's schema-driven parser
// rather than the contract's, so `everruns agents create --system-prompt ...`
// meant one thing here and another in the CLI. With argv in hand, the line is
// resolved by the same code every other surface uses; only what runs after
// resolution is this host's: the scripted pipeline (`catalog::run_for_shell`).
//
// Flat names (`list_agents --limit 5`) stay registered as the toolset's own
// builtins, so existing scripts keep working.

use async_trait::async_trait;
use bashkit::{BuiltinContext, ExecResult};
use everruns_cli_contract::{Mapper, Resolution};

use super::catalog::{self, CatalogContext, ToolsetMode};

pub(crate) struct EverrunsBuiltin {
    pub(crate) mapper: &'static Mapper,
    pub(crate) ctx: CatalogContext,
    pub(crate) mode: ToolsetMode,
}

impl EverrunsBuiltin {
    async fn run(&self, args: &[String]) -> Result<String, String> {
        let (wire_name, params) = match self.mapper.resolve(args) {
            Resolution::Output(text) => return Ok(text),
            Resolution::Error(text) => return Err(text),
            Resolution::Run { wire_name, params } => (wire_name, params),
        };
        // THREAT[TM-MCP-002]: the mapper knows the whole grammar, so whether a
        // command may run in this toolset is decided here, by the same rule
        // that decides which flat builtins `query` registers.
        let Some(desc) = catalog::scripted_descriptor(&wire_name, self.mode) else {
            let spelling = self
                .mapper
                .contract(&wire_name)
                .map(|contract| contract.spelling())
                .unwrap_or_else(|| wire_name.clone());
            return Err(match self.mode {
                ToolsetMode::ReadOnly => format!(
                    "`{} {spelling}` changes state, so it is not available in query; run it with execute",
                    self.mapper.tree().root()
                ),
                ToolsetMode::Full => format!("unknown command `{wire_name}`"),
            });
        };
        catalog::run_for_shell(desc, params, &self.ctx).await
    }
}

#[async_trait]
impl bashkit::Builtin for EverrunsBuiltin {
    async fn execute(&self, ctx: BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        Ok(match self.run(ctx.args).await {
            Ok(output) => ExecResult::ok(with_newline(output)),
            // Non-zero so help printed for an unknown verb never reads as
            // success to `set -e` or `&&`.
            Err(error) => ExecResult::err(with_newline(error), 1),
        })
    }
}

fn with_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
