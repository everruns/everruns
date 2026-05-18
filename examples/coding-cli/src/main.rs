// Entrypoint for the Everruns coding CLI example.
// Decision: support both interactive TUI and a `--print` one-shot mode so the
// example is testable in CI and easy to demo against a real codebase.

mod app;
mod approval;
mod instructions;
mod runtime;
mod tools;

use anyhow::Result;
use app::App;
use approval::ApprovalGate;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use everruns_core::events::EventData;
use everruns_core::message::MessageRole;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use runtime::{ProviderChoice, RuntimeBundle};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "ercode",
    version,
    about = "Everruns coding CLI — embedded terminal agent built on everruns-runtime"
)]
struct Cli {
    /// Workspace root the agent operates inside (default: current dir)
    #[arg(short = 'C', long = "cwd")]
    cwd: Option<PathBuf>,

    /// Force a provider (auto-detected from env vars otherwise)
    #[arg(long, value_enum)]
    provider: Option<ProviderArg>,

    /// Override the model id
    #[arg(short, long)]
    model: Option<String>,

    /// Run a single prompt non-interactively and print the result. Useful for CI smoke tests.
    #[arg(short = 'p', long)]
    print: Option<String>,

    /// Auto-approve every destructive tool call (write/edit/bash). Print mode implies this.
    #[arg(long)]
    yes: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum ProviderArg {
    Anthropic,
    Openai,
    Sim,
}

fn pick_provider(cli: &Cli) -> ProviderChoice {
    let base = match cli.provider {
        Some(ProviderArg::Anthropic) => ProviderChoice::Anthropic {
            model: "claude-sonnet-4-5".into(),
        },
        Some(ProviderArg::Openai) => ProviderChoice::OpenAi {
            model: "gpt-5.5".into(),
        },
        Some(ProviderArg::Sim) => ProviderChoice::Sim,
        None => ProviderChoice::from_env(),
    };
    match (base, cli.model.clone()) {
        (ProviderChoice::Anthropic { .. }, Some(m)) => ProviderChoice::Anthropic { model: m },
        (ProviderChoice::OpenAi { .. }, Some(m)) => ProviderChoice::OpenAi { model: m },
        (other, _) => other,
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error")),
        )
        .with_writer(io::stderr)
        .init();

    let cli = Cli::parse();
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let provider = pick_provider(&cli);

    let is_print = cli.print.is_some();
    let (gate, approval_rx) = if is_print || cli.yes {
        (ApprovalGate::auto(), None)
    } else {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (ApprovalGate::channel(tx), Some(rx))
    };
    let bundle = runtime::build(cwd, provider, gate).await?;
    let bundle = Arc::new(bundle);

    if let Some(prompt) = cli.print {
        return run_print_mode(bundle, prompt).await;
    }
    run_tui(bundle, approval_rx.expect("approval channel for TUI")).await
}

async fn run_tui(
    bundle: Arc<RuntimeBundle>,
    approval_rx: tokio::sync::mpsc::UnboundedReceiver<(
        approval::ApprovalRequest,
        tokio::sync::oneshot::Sender<bool>,
    )>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(bundle, approval_rx);
    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

async fn run_print_mode(bundle: Arc<RuntimeBundle>, prompt: String) -> Result<()> {
    println!("[workspace] {}", bundle.workspace_root.display());
    println!("[provider]  {}", bundle.provider_label);
    println!("[tools]     {}", bundle.tool_names.join(", "));
    if !bundle.instruction_summary.is_empty() {
        println!("[instructions] {}", bundle.instruction_summary.join(", "));
    }
    println!("[prompt] {prompt}\n");

    let before_events = bundle.runtime.events().await.map(|e| e.len()).unwrap_or(0);
    let before_msgs = bundle
        .runtime
        .messages(bundle.session_id)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let result = bundle
        .runtime
        .run_text_turn(bundle.session_id, prompt)
        .await?;
    let events = bundle.runtime.events().await.unwrap_or_default();
    let messages = bundle
        .runtime
        .messages(bundle.session_id)
        .await
        .unwrap_or_default();

    for event in events.iter().skip(before_events) {
        if let EventData::ToolCompleted(data) = &event.data {
            let status = if data.success { "✓" } else { "✗" };
            let summary = app::summarize_tool_result(data);
            if summary.is_empty() {
                println!("[tool {status}] {}", data.tool_name);
            } else {
                println!("[tool {status}] {} — {summary}", data.tool_name);
            }
        }
    }
    for msg in messages.iter().skip(before_msgs) {
        if msg.role == MessageRole::Agent
            && let Some(text) = msg.text()
        {
            let t = text.trim();
            if !t.is_empty() {
                println!("\n[agent]\n{t}");
            }
        }
    }
    println!(
        "\n[done] success={} iterations={} tool_calls={}",
        result.success, result.iterations, result.tool_calls_count
    );
    if !result.success
        && let Some(err) = result.error
    {
        eprintln!("turn error: {err}");
        std::process::exit(1);
    }
    Ok(())
}
