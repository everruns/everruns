//! Forwarding a tool's own CLI into the shell, for a host that cannot link the
//! commands in.

use super::builtin::{ensure_trailing_newline, interned_hint};
use super::*;

// ============================================================================
// Forwarding a tool's own CLI into the shell
// ============================================================================
//
// The tree above resolves locally: a host that links its commands in can parse,
// render help, and dispatch without leaving the process. A hosted worker cannot
// do that. `CliRoute` is `&'static`, so a tree cannot be rebuilt from data
// fetched at runtime, and the commands live behind the control plane anyway.
//
// So the worker forwards instead. A tool that already accepts a script (the
// `platform` capability's `execute`, whose whole job is running one against the
// command catalog) declares a [`CliSpelling`], and the shell installs a builtin
// that renders argv back into one command line and hands it to that tool.
// Grammar, help, authorization, and error shaping stay where they already are,
// on the other side of the tool call.
//
// Installing from the session's *tool registry* is what keeps this honest: the
// builtin exists only where the model could have called the tool directly, so
// the shell re-spells a surface rather than widening one. A harness that
// withholds the capability withholds the command.

use std::sync::atomic::{AtomicUsize, Ordering};

use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{CliSpelling, Tool, ToolExecutionResult};

/// How many forwarded invocations one shell execution may make.
///
/// Every invocation is a control-plane round trip, so a shell loop amplifies
/// one tool call into hundreds of them. The interpreter's own command and loop
/// limits do not see that cost. The cap is deliberately low, because looping in
/// the outer shell is the wrong shape anyway: the catalog's bash can do the
/// whole loop server-side in one call, and the error says so.
const MAX_FORWARDED_INVOCATIONS: usize = 50;

/// Builtin that renders argv into a command line and runs it through a tool.
pub struct ForwardingBuiltin {
    root: String,
    script_parameter: String,
    tool: Arc<dyn Tool>,
    context: ToolContext,
    invocations: AtomicUsize,
}

impl ForwardingBuiltin {
    pub fn new(spelling: CliSpelling, tool: Arc<dyn Tool>, context: ToolContext) -> Self {
        Self {
            root: spelling.root.to_string(),
            script_parameter: spelling.script_parameter.to_string(),
            tool,
            context,
            invocations: AtomicUsize::new(0),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }
}

/// Render one argument so the receiving parser sees exactly these bytes.
///
/// The outer shell has already done word splitting, expansion, and quote
/// removal, so an argument reaching the builtin is a literal. Re-quoting it
/// keeps it literal on the far side; passing it through raw would let a value
/// the model built from tool output (`--name "$title"`) re-parse there as
/// syntax.
fn shell_quote(argument: &str) -> String {
    if !argument.is_empty()
        && argument.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | ',' | '+')
        })
    {
        return argument.to_string();
    }
    format!("'{}'", argument.replace('\'', r"'\''"))
}

/// The command line a forwarded invocation sends on.
pub fn forwarded_command_line(root: &str, args: &[String]) -> String {
    let mut line = String::from(root);
    for argument in args {
        line.push(' ');
        line.push_str(&shell_quote(argument));
    }
    line
}

#[async_trait]
impl bashkit::Builtin for ForwardingBuiltin {
    async fn execute(&self, ctx: bashkit::BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        if self.invocations.fetch_add(1, Ordering::Relaxed) >= MAX_FORWARDED_INVOCATIONS {
            return Ok(ExecResult::err(
                ensure_trailing_newline(format!(
                    "{} ran more than {MAX_FORWARDED_INVOCATIONS} times in one shell call. \
                     Put the loop inside a single invocation instead: `{} <command>` accepts a \
                     whole script, so iterate there rather than here.",
                    self.root, self.root
                )),
                1,
            ));
        }
        let line = forwarded_command_line(&self.root, ctx.args);
        let arguments = serde_json::json!({ self.script_parameter.clone(): line });
        match self
            .tool
            .execute_with_context(arguments, &self.context)
            .await
        {
            ToolExecutionResult::Success(value)
            | ToolExecutionResult::SuccessWithImages { result: value, .. } => Ok(ExecResult::ok(
                ensure_trailing_newline(render_output(value)),
            )),
            // An unusable command must not read as success, and the message is
            // the tool's own: it already sanitized what may reach the model.
            ToolExecutionResult::ToolError(message) => {
                Ok(ExecResult::err(ensure_trailing_newline(message), 1))
            }
            other => Ok(ExecResult::err(
                ensure_trailing_newline(format!("{} failed: {other:?}", self.root)),
                1,
            )),
        }
    }

    fn llm_hint(&self) -> Option<&'static str> {
        Some(interned_hint(&self.root))
    }
}

/// Tool results are JSON; a string result is the command's own stdout and must
/// not reach the shell wrapped in quotes, or `jq` downstream sees a string.
fn render_output(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// The first registered tool that declares a CLI spelling, if any.
pub fn forwarding_builtin_for(context: &ToolContext) -> Option<ForwardingBuiltin> {
    let registry = context.tool_registry.as_ref()?;
    let mut named: Vec<&str> = registry.tool_names();
    // Deterministic: a registry is a hash map, and two tools declaring a
    // spelling must not install in arbitrary order.
    named.sort_unstable();
    for name in named {
        let tool = registry.get(name)?;
        if let Some(spelling) = tool.cli_spelling() {
            return Some(ForwardingBuiltin::new(
                spelling,
                tool.clone(),
                context.clone(),
            ));
        }
    }
    None
}

#[cfg(test)]
mod forwarding_tests {
    use super::*;

    #[test]
    fn plain_arguments_are_passed_through_unquoted() {
        assert_eq!(
            forwarded_command_line(
                "everruns",
                &[
                    "agents".into(),
                    "list".into(),
                    "--limit".into(),
                    "10".into()
                ]
            ),
            "everruns agents list --limit 10"
        );
    }

    #[test]
    fn arguments_with_spaces_stay_one_argument() {
        assert_eq!(
            forwarded_command_line(
                "everruns",
                &[
                    "agents".into(),
                    "create".into(),
                    "--name".into(),
                    "My Agent".into()
                ]
            ),
            "everruns agents create --name 'My Agent'"
        );
    }

    /// The far side parses this line, so a value carrying shell syntax must
    /// arrive as data. Model-built arguments routinely carry quotes and `$`.
    #[test]
    fn shell_syntax_in_a_value_is_neutralized() {
        let line = forwarded_command_line(
            "everruns",
            &[
                "agents".into(),
                "create".into(),
                "--name".into(),
                "it's; rm -rf /".into(),
            ],
        );
        assert_eq!(line, r#"everruns agents create --name 'it'\''s; rm -rf /'"#);
        let line = forwarded_command_line("everruns", &["--name".into(), "$(whoami)".into()]);
        assert_eq!(line, "everruns --name '$(whoami)'");
    }

    #[test]
    fn help_forwards_as_written() {
        assert_eq!(
            forwarded_command_line("everruns", &["agents".into(), "--help".into()]),
            "everruns agents --help"
        );
    }

    #[test]
    fn a_string_result_reaches_the_shell_as_stdout() {
        assert_eq!(
            render_output(Value::String("{\"id\":1}".into())),
            "{\"id\":1}"
        );
        assert_eq!(render_output(serde_json::json!({"id": 1})), "{\"id\":1}");
    }
}
