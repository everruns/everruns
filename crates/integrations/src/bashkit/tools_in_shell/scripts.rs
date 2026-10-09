//! `tools scripts`: run and save the agent's saved scripts (D8).
//!
//! Decisions:
//!
//! - **Same shell rules.** A saved script runs in its own shell over the same
//!   session files and working directory, with the same `tools` builtin: its
//!   calls go through the same approval gate, count against the same call cap,
//!   and a stop inside it stops the caller's script too. It runs with the
//!   caller's tools and identity, never its author's.
//! - **Plain shell.** The script's shell has the shell builtins and `tools`;
//!   `curl` and the `everruns` command tree are not installed there. Anything
//!   they reach is reachable through `tools`.
//! - **Input on stdin.** The input object, checked against the script's schema,
//!   arrives on stdin as JSON (`jq -r .repo`); stdout is the result.
//! - **Bounded nesting.** A script can call another, at most
//!   [`MAX_SCRIPT_DEPTH`] deep.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use bashkit::{Bash, ExecResult};
use everruns_contracts::runtime::saved_scripts::{SavedScript, is_valid_script_name};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Value, json};

use super::builtin::{SharedTools, ToolsBuiltin, code, error, stop_script, text};
use super::input::{self, Request};

/// How deep saved scripts may call each other.
pub(crate) const MAX_SCRIPT_DEPTH: usize = 4;

const USAGE: &str = "Usage:\n  tools scripts                       list saved scripts\n  \
tools scripts <name> '{...}'        run one; input arrives on stdin as JSON\n  \
tools scripts <name> --help         show its input and body\n  \
tools scripts save <name> --description '<one line>' [--input-schema '{...}'] < script.sh\n";

impl ToolsBuiltin {
    pub(super) async fn scripts(
        &self,
        args: &[String],
        ctx: &bashkit::BuiltinContext<'_>,
    ) -> ExecResult {
        let Some(first) = args.first() else {
            return self.list_scripts().await;
        };
        if first == "--help" || first == "-h" {
            return self.list_scripts().await;
        }
        if first == "save" {
            let stdin = ctx.stdin.and_then(|s| s.text().ok());
            return self.save_script(&args[1..], stdin.as_deref()).await;
        }
        let script = match self.find_script(first).await {
            Ok(Some(script)) => script,
            Ok(None) => {
                return error(
                    code::UNKNOWN_COMMAND,
                    format!("no saved script `{first}`. Run `tools scripts` to list them."),
                    false,
                );
            }
            Err(failed) => return failed,
        };
        let schema = script
            .input_schema
            .clone()
            .unwrap_or_else(|| json!({"type": "object"}));
        let stdin = ctx.stdin.and_then(|s| s.text().ok());
        let input = match input::parse(&args[1..], stdin.as_deref(), &schema) {
            Ok(Request::Help) => return text(script_help(&script)),
            Ok(Request::Call(input)) => input,
            Err(message) => {
                return error(
                    code::INVALID_INPUT,
                    format!("{message}. Run `tools scripts {first} --help` for its input."),
                    false,
                );
            }
        };
        if let Some(problem) = schema_problem(&script, &schema, &input) {
            return error(
                code::INVALID_INPUT,
                format!("{problem}. Run `tools scripts {first} --help` for its input."),
                false,
            );
        }
        if self.run.is_stopped() {
            return stop_script(error(
                code::STOPPED,
                "the script was stopped at an earlier tools call; nothing after it runs",
                false,
            ));
        }
        self.run_script(&script, input, ctx).await
    }

    async fn saved_scripts(&self) -> Result<Vec<SavedScript>, ExecResult> {
        if let Some(list) = self
            .saved_list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(list);
        }
        let Some(saved) = self.saved.as_ref() else {
            return Err(error(code::UNAVAILABLE, "no saved scripts here", false));
        };
        let list = saved.store.list().await.map_err(|message| {
            error(
                code::TOOL_ERROR,
                format!("could not list saved scripts: {message}"),
                true,
            )
        })?;
        *self.saved_list.lock().unwrap_or_else(|e| e.into_inner()) = Some(list.clone());
        Ok(list)
    }

    async fn find_script(&self, name: &str) -> Result<Option<SavedScript>, ExecResult> {
        let name = name.to_ascii_lowercase();
        Ok(self
            .saved_scripts()
            .await?
            .into_iter()
            .find(|s| s.name == name))
    }

    async fn list_scripts(&self) -> ExecResult {
        let list = match self.saved_scripts().await {
            Ok(list) => list,
            Err(failed) => return failed,
        };
        let mut out = String::new();
        if list.is_empty() {
            out.push_str("No saved scripts yet.\n\n");
        } else {
            out.push_str("Saved scripts:\n");
            for script in &list {
                out.push_str(&format!("  {}  {}\n", script.name, script.description));
            }
            out.push('\n');
        }
        out.push_str(USAGE);
        text(out)
    }

    async fn save_script(&self, args: &[String], body: Option<&str>) -> ExecResult {
        let Some(saved) = self.saved.as_ref() else {
            return error(code::UNAVAILABLE, "no saved scripts here", false);
        };
        if !saved.can_save {
            return error(
                code::DENIED,
                "this agent may run saved scripts but not save them (the capability's \
                 `manage_scripts` setting is off)",
                false,
            );
        }
        let schema = json!({"type": "object", "properties": {
            "description": {"type": "string"}, "input_schema": {"type": "object"}}});
        let (name, rest) = match args.split_first() {
            Some((name, rest)) if !name.starts_with('-') => (name.as_str(), rest),
            _ => {
                return error(
                    code::INVALID_INPUT,
                    format!("a name is required.\n{USAGE}"),
                    false,
                );
            }
        };
        if !is_valid_script_name(name) {
            return error(
                code::INVALID_INPUT,
                "a script name is lowercase letters, digits, `-` and `_`, starting with a \
                 letter, at most 64 characters",
                false,
            );
        }
        let fields = match input::parse(rest, None, &schema) {
            Ok(Request::Call(fields)) => fields,
            Ok(Request::Help) => return text(USAGE.to_string()),
            Err(message) => return error(code::INVALID_INPUT, message, false),
        };
        let Some(description) = fields.get("description").and_then(Value::as_str) else {
            return error(code::INVALID_INPUT, "--description is required", false);
        };
        let Some(body) = body.filter(|b| !b.trim().is_empty()) else {
            return error(
                code::INVALID_INPUT,
                "the script body is read from stdin, which is empty",
                false,
            );
        };
        let script = SavedScript {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: fields.get("input_schema").cloned(),
            body: body.to_string(),
        };
        match saved.store.save(script).await {
            Ok(script) => {
                *self.saved_list.lock().unwrap_or_else(|e| e.into_inner()) = None;
                text(json!({"saved": script.name}).to_string())
            }
            Err(message) => error(code::INVALID_INPUT, message, false),
        }
    }

    async fn run_script(
        &self,
        script: &SavedScript,
        input: Value,
        ctx: &bashkit::BuiltinContext<'_>,
    ) -> ExecResult {
        let Some(me) = self.me.upgrade() else {
            return error(code::UNAVAILABLE, "saved scripts cannot run here", false);
        };
        if self.depth.fetch_add(1, Ordering::Relaxed) >= MAX_SCRIPT_DEPTH {
            self.depth.fetch_sub(1, Ordering::Relaxed);
            return error(
                code::CALL_LIMIT,
                format!("saved scripts nest more than {MAX_SCRIPT_DEPTH} deep"),
                false,
            );
        }
        let mut shell = Bash::builder()
            .fs(Arc::clone(&ctx.fs))
            .cwd(ctx.cwd.clone())
            .username("everruns")
            .hostname("everruns")
            .limits(crate::bashkit::execution_limits())
            .max_memory(10 * 1024 * 1024);
        for (key, value) in ctx.env {
            shell = shell.env(key, value);
        }
        let mut shell = shell
            .builtin(super::TOOLS_COMMAND.to_string(), Box::new(SharedTools(me)))
            .build();
        // The input reaches the script on stdin, as `jq` expects it.
        let program = format!(
            "{} <<'__EVERRUNS_SCRIPT_INPUT__'\n{input}\n__EVERRUNS_SCRIPT_INPUT__\n",
            wrap(&script.body)
        );
        let outcome = shell.exec(&program).await;
        self.depth.fetch_sub(1, Ordering::Relaxed);
        tracing::info!(
            target: "bashkit.tools",
            session_id = %self.context.session_id,
            script = %script.name,
            "saved script run from shell"
        );
        let result = match outcome {
            Ok(output) => {
                let mut result = ExecResult::ok(output.stdout);
                result.stderr = output.stderr;
                result.exit_code = output.exit_code;
                result
            }
            Err(failure) => error(
                code::TOOL_ERROR,
                format!("saved script {} failed: {failure}", script.name),
                false,
            ),
        };
        // A stop inside the saved script stops its caller too.
        if self.run.is_stopped() {
            return stop_script(result);
        }
        result
    }
}

/// The body as a group whose stdin is the input heredoc.
fn wrap(body: &str) -> String {
    format!("{{\n{body}\n}}")
}

fn script_help(script: &SavedScript) -> String {
    let schema = script
        .input_schema
        .as_ref()
        .map(|s| serde_json::to_string_pretty(s).unwrap_or_default())
        .unwrap_or_else(|| "any JSON object".to_string());
    format!(
        "tools scripts {}\n\n{}\n\nInput (on stdin as JSON):\n{schema}\n\nBody:\n{}\n",
        script.name, script.description, script.body
    )
}

/// The schema check a tool call gets, for the script's input.
fn schema_problem(script: &SavedScript, schema: &Value, input: &Value) -> Option<String> {
    let tool = SchemaOnly {
        name: script.name.clone(),
        schema: schema.clone(),
    };
    let call = ToolCall {
        id: String::new(),
        name: script.name.clone(),
        arguments: input.clone(),
    };
    match everruns_contracts::runtime::tools::validate_tool_arguments(&tool, &call) {
        Ok(None) => None,
        Ok(Some(problem)) => Some(problem),
        Err(_) => Some("the script's input schema is not valid JSON Schema".to_string()),
    }
}

/// Lends a script's schema to the tool argument check.
struct SchemaOnly {
    name: String,
    schema: Value,
}

#[async_trait]
impl Tool for SchemaOnly {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        ""
    }
    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("not callable")
    }
}
