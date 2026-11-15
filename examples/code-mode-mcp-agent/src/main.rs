#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Code mode over a huge MCP server. Run from the repository checkout; see
//! README.md for credentials.
//!
//! The binary plays both parts: with `--mcp-server` it is the Acme MCP server
//! (1,000 tools over stdio); without it, it is the host that attaches that
//! server to an agent and asks a question.
use std::collections::BTreeMap;
use std::io::{self, Write};

use everruns::Engine;
use everruns_example_demo::shell as demo;
use serde_json::Value;

mod acme;
mod agent;

const DEFAULT_QUESTION: &str = "Which EU customers have overdue invoices? \
     Give the three with the largest total overdue amount, in dollars.";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--mcp-server") => Ok(acme::serve()?),
        Some("--interactive") => run(true),
        None => run(false),
        Some(other) => Err(format!("unknown argument {other:?}; usage: [--interactive]").into()),
    }
}

#[tokio::main]
async fn run(interactive: bool) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = acme::catalog();
    let native_bytes = serde_json::to_vec(&catalog)?.len();
    let (provider, model) = agent::provider()?;
    let server_command = std::env::current_exe()?;
    let agent = agent::build(provider, model, &server_command.to_string_lossy())?;
    let session = Engine::new().create(agent);

    demo::banner("everruns · code mode over a huge MCP server");
    demo::field("model", model);
    demo::field(
        "mcp server",
        &format!(
            "{} — {} tools, {} KiB of tool schemas",
            acme::SERVER,
            catalog.len(),
            native_bytes / 1024
        ),
    );
    demo::field("capabilities", "bashkit_shell + tools_in_shell (mode: cli)");

    let question = if interactive {
        read_question()?
    } else {
        DEFAULT_QUESTION.to_owned()
    };
    let mut events = session.events();
    let turn = demo::run_showing(&session, &question, 40).await?;
    let stats = Stats::collect(&mut events)?;

    demo::section("WHAT THE MODEL CARRIED");
    demo::field("tools", &stats.model_tools.join(", "));
    demo::field(
        "schemas",
        &format!(
            "{} KiB per request (all {} MCP tools natively: {} KiB)",
            stats.tool_bytes.div_ceil(1024),
            catalog.len(),
            native_bytes / 1024
        ),
    );
    demo::field(
        "prompt",
        &format!("{} input tokens at most", stats.peak_input_tokens),
    );
    demo::section("MCP TOOLS CALLED FROM SCRIPTS");
    for (tool, count) in &stats.nested_calls {
        demo::body(&format!("{count:>3} × {tool}"), demo::PLAIN);
    }

    demo::section("CHECKS");
    let mcp_visible = stats
        .model_tools
        .iter()
        .any(|name| name.starts_with("mcp_"));
    let mcp_direct = stats
        .direct_calls
        .keys()
        .any(|name| name.starts_with("mcp_"));
    let nested: usize = stats.nested_calls.values().sum();
    let mut checks = vec![
        (
            !mcp_visible,
            "no MCP tool in the model's tool list".to_owned(),
        ),
        (
            !mcp_direct,
            "the model called no MCP tool directly".to_owned(),
        ),
        (
            nested > 0,
            format!("{nested} MCP calls ran inside Bashkit scripts"),
        ),
    ];
    if question == DEFAULT_QUESTION {
        let expected = acme::top_overdue_eu_customers(3);
        let answer = turn.response.replace(',', "");
        let found = expected.iter().all(|(name, total)| {
            answer.contains(name.as_str()) && answer.contains(&total.to_string())
        });
        checks.push((
            found,
            "answer matches totals computed from the server's data".to_owned(),
        ));
    }
    for (passed, label) in &checks {
        demo::check(*passed, label);
    }
    let failures = checks.iter().filter(|(passed, _)| !passed).count();
    if failures > 0 {
        return Err(format!("{failures} check(s) failed").into());
    }
    println!("\nCompleted: answer verified against the MCP server");
    Ok(())
}

fn read_question() -> io::Result<String> {
    print!("Question: ");
    io::stdout().flush()?;
    let mut question = String::new();
    io::stdin().read_line(&mut question)?;
    let question = question.trim();
    if question.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "question cannot be empty",
        ));
    }
    Ok(question.to_owned())
}

/// What the model saw and did, read back from the session's canonical events.
#[derive(Default)]
struct Stats {
    model_tools: Vec<String>,
    tool_bytes: usize,
    peak_input_tokens: u64,
    direct_calls: BTreeMap<String, usize>,
    nested_calls: BTreeMap<String, usize>,
}

impl Stats {
    fn collect(events: &mut everruns::EventStream) -> Result<Self, Box<dyn std::error::Error>> {
        let mut stats = Self::default();
        while let Some(event) = events.try_recv()? {
            stats.record(event.event_type(), &event.canonical_json()["data"]);
        }
        Ok(stats)
    }

    fn record(&mut self, event_type: &str, data: &Value) {
        let name = || data["tool_name"].as_str().unwrap_or_default().to_owned();
        match event_type {
            "tool.completed" => *self.direct_calls.entry(name()).or_default() += 1,
            "tool.nested_call" if data["status"] == "completed" => {
                *self.nested_calls.entry(name()).or_default() += 1;
            }
            "llm.generation" => {
                if let Some(tools) = data.pointer("/request_body/tools") {
                    self.tool_bytes = self.tool_bytes.max(tools.to_string().len());
                    self.model_tools = tools
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|tool| {
                            tool["name"]
                                .as_str()
                                .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
                                .map(str::to_owned)
                        })
                        .collect();
                }
                if let Some(usage) = data.pointer("/metadata/usage") {
                    let input = ["input_tokens", "cache_read_tokens", "cache_creation_tokens"]
                        .iter()
                        .filter_map(|key| usage[*key].as_u64())
                        .sum();
                    self.peak_input_tokens = self.peak_input_tokens.max(input);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stats_separate_model_calls_from_script_calls() {
        let mut stats = Stats::default();
        stats.record("tool.completed", &json!({"tool_name": "bash"}));
        stats.record(
            "tool.nested_call",
            &json!({"tool_name": "mcp_acme__billing_list_invoices", "status": "completed"}),
        );
        stats.record(
            "tool.nested_call",
            &json!({"tool_name": "mcp_acme__crm_delete_customer", "status": "refused"}),
        );
        stats.record(
            "llm.generation",
            &json!({
                "request_body": {"tools": [{"name": "bash"}, {"function": {"name": "read_file"}}]},
                "metadata": {"usage": {"input_tokens": 100, "cache_read_tokens": 900}},
            }),
        );
        assert_eq!(stats.direct_calls.get("bash"), Some(&1));
        assert_eq!(stats.nested_calls.len(), 1);
        assert_eq!(stats.model_tools, ["bash", "read_file"]);
        assert_eq!(stats.peak_input_tokens, 1_000);
    }
}
