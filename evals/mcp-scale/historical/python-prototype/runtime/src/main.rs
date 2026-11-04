use std::{env, fs, path::PathBuf, time::Instant};

use everruns::{Agent, CapabilityRef, Engine, McpServer, Model};
use serde::Serialize;
use serde_json::{Value, json};

const DEFAULT_MODEL: &str = "gpt-6-luna";
const MCP_SERVER_ID: &str = "ops";
const INSTRUCTIONS_NATIVE: &str = "You are an operations assistant. Use the connected MCP tools to carry out the request. Act without asking for confirmation and reply with one concise sentence.";
const INSTRUCTIONS_CLI: &str = "You are an operations assistant. Use the Bashkit shell and its `tools` command to discover and call the connected MCP tools. Do not answer from memory. Act without asking for confirmation and reply with one concise sentence.";
const INSTRUCTIONS_HYBRID: &str = "You are an operations assistant. The connected MCP gateway functions are callable directly and through Bashkit's `tools` command. When asked to use both surfaces, use a direct gateway function and then use `bash` with `tools` for another read through the same gateway. Do not answer from memory. Act without asking for confirmation and reply with one concise sentence.";
const SURFACE_SCOPE: &str = "Validates native and Bashkit/Tools in Shell invocation adapters around the synthetic two-function gateway. It does not validate native individual-tool discovery, a new first-class catalog contract, or production MCP pagination.";

#[derive(Debug)]
struct Args {
    url: String,
    prompt: Option<String>,
    model: String,
    provider: String,
    api_base_url: Option<String>,
    surface: Surface,
    search: SearchMode,
    stdio_command: Option<String>,
    stdio_args: Vec<String>,
    output: Option<String>,
    trace_output: Option<String>,
    list: bool,
}

#[derive(Serialize)]
struct RunReport {
    url: String,
    model: String,
    provider: String,
    surface: &'static str,
    surface_scope: &'static str,
    prompt: Option<String>,
    mode: &'static str,
    mcp_transport: &'static str,
    search: &'static str,
    success: Option<bool>,
    response: Option<String>,
    error: Option<String>,
    iterations: Option<usize>,
    duration_ms: Option<u128>,
    usage: Value,
    discovered_tool_count: usize,
    discovered_tools: Vec<String>,
    tool_calls: Vec<Value>,
    event_errors: Vec<Value>,
}

#[derive(Clone, Copy, Debug)]
enum SearchMode {
    Generic,
    Auto,
    None,
}

#[derive(Clone, Copy, Debug)]
enum Surface {
    Native,
    Cli,
    Hybrid,
}

impl Surface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Cli => "cli",
            Self::Hybrid => "hybrid",
        }
    }
}

impl Args {
    fn search_label(&self) -> &'static str {
        match self.surface {
            Surface::Native => self.search.as_str(),
            Surface::Cli | Surface::Hybrid => "tools_in_shell",
        }
    }
}

impl SearchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Auto => "auto",
            Self::None => "none",
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut url = None;
    let mut prompt = None;
    let mut model = DEFAULT_MODEL.to_string();
    let mut provider = "openai".to_string();
    let mut api_base_url = None;
    let mut surface = Surface::Native;
    let mut search = SearchMode::Generic;
    let mut stdio_command = None;
    let mut stdio_args = Vec::new();
    let mut output = None;
    let mut trace_output = None;
    let mut list = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = Some(next_value(&mut args, "--url")?),
            "--prompt" => prompt = Some(next_value(&mut args, "--prompt")?),
            "--model" => model = next_value(&mut args, "--model")?,
            "--provider" => {
                provider = next_value(&mut args, "--provider")?;
                if !["openai", "openrouter"].contains(&provider.as_str()) {
                    return Err(format!("unsupported --provider: {provider}"));
                }
            }
            "--api-base-url" => api_base_url = Some(next_value(&mut args, "--api-base-url")?),
            "--surface" => {
                surface = match next_value(&mut args, "--surface")?.as_str() {
                    "native" => Surface::Native,
                    "cli" => Surface::Cli,
                    "hybrid" => Surface::Hybrid,
                    other => return Err(format!("invalid --surface: {other}")),
                }
            }
            "--search" => {
                search = match next_value(&mut args, "--search")?.as_str() {
                    "generic" => SearchMode::Generic,
                    "auto" => SearchMode::Auto,
                    "none" => SearchMode::None,
                    other => return Err(format!("invalid --search mode: {other}")),
                }
            }
            "--stdio-command" => stdio_command = Some(next_value(&mut args, "--stdio-command")?),
            "--stdio-arg" => stdio_args.push(next_value(&mut args, "--stdio-arg")?),
            "--output" => output = Some(next_value(&mut args, "--output")?),
            "--trace-output" => trace_output = Some(next_value(&mut args, "--trace-output")?),
            "--list" | "--probe" => list = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }
    let url = match (url, stdio_command.is_some()) {
        (Some(url), _) => url,
        (None, true) => "stdio://ops".to_string(),
        (None, false) => return Err("--url is required unless --stdio-command is set".into()),
    };
    if !list && prompt.is_none() {
        return Err("--prompt is required unless --list is set".into());
    }
    Ok(Args {
        url,
        prompt,
        model,
        provider,
        api_base_url,
        surface,
        search,
        stdio_command,
        stdio_args,
        output,
        trace_output,
        list,
    })
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "Usage: mcp-scale-runtime (--url URL | --stdio-command COMMAND [--stdio-arg ARG ...]) [--prompt TEXT] [--model MODEL] [--provider openai|openrouter] [--api-base-url URL] [--surface native|cli|hybrid] [--search generic|auto|none] [--output json|PATH] [--trace-output PATH] [--list]\n\
         --list / --probe inspects tools discovered by the Framework without an LLM call.\n\
         --trace-output writes the full canonical event stream separately from the compact report.\n\
         native is the default; cli enables Bashkit and Tools in Shell; hybrid keeps the two MCP gateway functions directly visible and callable from the shell."
    );
}

fn make_agent(args: &Args, probe: bool) -> Result<Agent, Box<dyn std::error::Error>> {
    let model = if probe {
        Model::simulated("probe")
    } else {
        let mut provider = match args.provider.as_str() {
            "openai" => everruns_drivers::openai::provider("openai", env::var("OPENAI_API_KEY")?),
            "openrouter" => everruns_drivers::openrouter::provider(
                "openrouter",
                env::var("OPENROUTER_API_KEY")?,
            ),
            _ => unreachable!("provider validated in argument parser"),
        };
        if let Some(base_url) = &args.api_base_url {
            provider = provider.base_url(base_url.clone());
        }
        Model::new(args.model.clone(), provider)
    };
    let server = if let Some(command) = &args.stdio_command {
        McpServer::stdio("ops", command).args(args.stdio_args.iter().cloned())
    } else {
        McpServer::http("ops", args.url.clone())
    };
    let instructions = match args.surface {
        Surface::Native => INSTRUCTIONS_NATIVE,
        Surface::Cli => INSTRUCTIONS_CLI,
        Surface::Hybrid => INSTRUCTIONS_HYBRID,
    };
    let mut builder = Agent::builder()
        .name("mcp-scale-runtime-example")
        .instructions(instructions)
        .model(model)
        .mcp_server(server)
        .max_iterations(10);

    builder = match args.surface {
        Surface::Native if probe => builder,
        Surface::Native => match args.search {
            SearchMode::Generic => builder
                .capability(CapabilityRef::new("tool_search").config(json!({ "threshold": 1 }))),
            SearchMode::Auto => builder.capability(
                CapabilityRef::new("auto_tool_search").config(json!({ "threshold": 1 })),
            ),
            SearchMode::None => builder,
        },
        Surface::Cli => builder.capability("bashkit_shell").capability(
            CapabilityRef::new("tools_in_shell").config(json!({
                "keep_visible": [],
                "rate_unlabeled_tools": false
            })),
        ),
        Surface::Hybrid => builder.capability("bashkit_shell").capability(
            CapabilityRef::new("tools_in_shell").config(json!({
                "keep_visible": [
                    format!("mcp_{MCP_SERVER_ID}__catalog_search"),
                    format!("mcp_{MCP_SERVER_ID}__catalog_call")
                ],
                "rate_unlabeled_tools": false
            })),
        ),
    };
    Ok(builder.build()?)
}

fn tool_reports(tools: &[everruns::ToolInfo]) -> Vec<String> {
    tools.iter().map(|tool| tool.name.clone()).collect()
}

fn event_values(events: &mut everruns::EventStream) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        match events.try_recv() {
            Ok(Some(event)) => out.push(event.canonical_json().clone()),
            Ok(None) | Err(_) => break,
        }
    }
    out
}

fn extract_calls(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| match event.get("type").and_then(Value::as_str) {
            Some("tool.completed") => {
                let mut data = event.pointer("/data")?.clone();
                let tool_name = data
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                data["invocation_surface"] =
                    json!(if tool_name == "bash" { "cli" } else { "native" });
                Some(data)
            }
            Some("tool.nested_call") => {
                let mut data = event.pointer("/data")?.clone();
                data["invocation_surface"] = json!("cli");
                Some(data)
            }
            _ => None,
        })
        .collect()
}

fn write_report(
    report: &RunReport,
    output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let serialized = serde_json::to_string_pretty(report)?;
    match output {
        None | Some("json") | Some("-") => println!("{serialized}"),
        Some(path) => fs::write(PathBuf::from(path), serialized)?,
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().map_err(|error| format!("{error}\n\nUse --help for usage."))?;
    let agent = make_agent(&args, args.list)?;
    let session = Engine::new().create(agent);
    let context = session.inspect().await?;
    let discovered_tools = tool_reports(&context.tools);

    if args.list {
        let search_label = args.search_label();
        let report = RunReport {
            url: args.url,
            model: args.model,
            provider: args.provider,
            surface: args.surface.as_str(),
            surface_scope: SURFACE_SCOPE,
            prompt: None,
            mode: "list",
            mcp_transport: if args.stdio_command.is_some() {
                "stdio"
            } else {
                "http"
            },
            search: search_label,
            success: None,
            response: None,
            error: None,
            iterations: None,
            duration_ms: None,
            usage: Value::Null,
            discovered_tool_count: discovered_tools.len(),
            discovered_tools,
            tool_calls: Vec::new(),
            event_errors: Vec::new(),
        };
        return write_report(&report, args.output.as_deref());
    }

    let mut events = session.events();
    let started = Instant::now();
    let result = session.run(args.prompt.clone().unwrap_or_default()).await;
    let duration_ms = started.elapsed().as_millis();
    let recorded_events = event_values(&mut events);
    if let Some(path) = &args.trace_output {
        fs::write(path, serde_json::to_vec_pretty(&recorded_events)?)?;
    }
    let (success, response, error, iterations) = match result {
        Ok(result) => (
            Some(result.success),
            Some(result.response),
            result.error,
            Some(result.iterations),
        ),
        Err(error) => (Some(false), None, Some(error.to_string()), None),
    };
    let search_label = args.search_label();
    let report = RunReport {
        url: args.url,
        model: args.model,
        provider: args.provider,
        surface: args.surface.as_str(),
        surface_scope: SURFACE_SCOPE,
        prompt: args.prompt,
        mode: "run",
        mcp_transport: if args.stdio_command.is_some() {
            "stdio"
        } else {
            "http"
        },
        search: search_label,
        success,
        response,
        error,
        iterations,
        duration_ms: Some(duration_ms),
        usage: aggregate_usage(&recorded_events),
        discovered_tool_count: discovered_tools.len(),
        discovered_tools,
        tool_calls: extract_calls(&recorded_events),
        event_errors: extract_errors(&recorded_events),
    };
    write_report(&report, args.output.as_deref())
}

fn aggregate_usage(events: &[Value]) -> Value {
    if let Some(usage) = events
        .iter()
        .rev()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("turn.completed"))
        .and_then(|event| event.pointer("/data/usage"))
    {
        return usage.clone();
    }

    let mut totals = json!({
        "input_tokens": 0u64,
        "output_tokens": 0u64,
        "cache_read_tokens": 0u64,
        "cache_creation_tokens": 0u64,
        "actual_cost_usd": 0.0,
        "effective_cost_usd": 0.0,
    });
    let mut found = false;
    for event in events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("llm.generation"))
    {
        let Some(usage) = event.pointer("/data/metadata/usage") else {
            continue;
        };
        found = true;
        for key in [
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
        ] {
            let value = usage.get(key).and_then(Value::as_u64).unwrap_or_default();
            let accumulated = totals[key].as_u64().unwrap_or_default();
            totals[key] = json!(accumulated + value);
        }
        for key in ["actual_cost_usd", "effective_cost_usd"] {
            let value = usage.get(key).and_then(Value::as_f64).unwrap_or_default();
            let accumulated = totals[key].as_f64().unwrap_or_default();
            totals[key] = json!(accumulated + value);
        }
    }
    if found { totals } else { Value::Null }
}

fn extract_errors(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("llm.generation"))
        .filter_map(|event| {
            let error = event.pointer("/data/metadata/error")?;
            (!error.is_null()).then(|| {
                json!({
                    "type": event.get("type").cloned().unwrap_or(Value::Null),
                    "error": error,
                })
            })
        })
        .collect()
}
