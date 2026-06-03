// Entrypoint for the Everruns coding CLI example.
// Decision: support both interactive TUI and a `--print` one-shot mode so the
// example is testable in CI and easy to demo against a real codebase.

mod app;
mod approval;
mod capabilities;
mod diff;
mod mcp_config;
mod runtime;
mod session_log;
mod tools;

use anyhow::Result;
use app::{App, COMPOSER_VIEWPORT_HEIGHT};
use approval::ApprovalGate;
use clap::Parser;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use everruns_core::message::MessageRole;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use runtime::{BuiltRuntime, ProviderChoice};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

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

    /// OpenAI reasoning effort for model calls (default: medium)
    #[arg(long)]
    reasoning_effort: Option<String>,

    /// Run a single prompt non-interactively and print the result. Useful for CI smoke tests.
    #[arg(short = 'p', long)]
    print: Option<String>,

    /// Prompt for y/n before every destructive tool call (write/edit/delete/bash).
    /// Off by default — the agent acts autonomously. Ignored in `--print` mode
    /// (one-shot runs always auto-approve since there's no interactive terminal).
    #[arg(long)]
    ask: bool,

    /// Resume an existing session. Reads the JSONL log for this id and
    /// seeds the message history; the new run continues appending to the
    /// same file. If no log exists, a new session starts with this id.
    /// Without `--session`, a fresh id is generated each run.
    #[arg(long)]
    session: Option<String>,

    /// Directory where per-session folders are stored. Default: the
    /// platform-native user data directory (`$XDG_DATA_HOME/ercode/sessions/`
    /// on Linux, `~/Library/Application Support/ercode/sessions/` on macOS,
    /// `%APPDATA%\ercode\sessions\` on Windows).
    #[arg(long)]
    session_dir: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum ProviderArg {
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
    #[value(name = "llmsim", alias = "sim")]
    Sim,
}

fn pick_provider(cli: &Cli) -> ProviderChoice {
    let base = match cli.provider {
        Some(ProviderArg::Anthropic) => ProviderChoice::Anthropic {
            model: "claude-sonnet-4-5".into(),
        },
        Some(ProviderArg::Openai) => ProviderChoice::OpenAi {
            model: "gpt-5.5".into(),
            reasoning_effort: Some(
                cli.reasoning_effort
                    .clone()
                    .unwrap_or_else(|| "medium".to_string()),
            ),
        },
        Some(ProviderArg::Openrouter) => ProviderChoice::OpenRouter {
            model: std::env::var("EVERRUNS_CLI_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "openai/gpt-5.2".to_string()),
            base_url: std::env::var("OPENROUTER_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string()),
        },
        Some(ProviderArg::Ollama) => ProviderChoice::Ollama {
            model: std::env::var("EVERRUNS_CLI_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "llama3.2".to_string()),
            base_url: std::env::var("OLLAMA_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        },
        Some(ProviderArg::Sim) => ProviderChoice::Sim,
        None => ProviderChoice::from_env(),
    };
    let selected = match (base, cli.model.clone()) {
        (ProviderChoice::Anthropic { .. }, Some(m)) => ProviderChoice::Anthropic { model: m },
        (
            ProviderChoice::OpenAi {
                reasoning_effort, ..
            },
            Some(m),
        ) => ProviderChoice::OpenAi {
            model: m,
            reasoning_effort: cli.reasoning_effort.clone().or(reasoning_effort),
        },
        (ProviderChoice::OpenRouter { base_url, .. }, Some(m)) => {
            ProviderChoice::OpenRouter { model: m, base_url }
        }
        (ProviderChoice::Ollama { base_url, .. }, Some(m)) => {
            ProviderChoice::Ollama { model: m, base_url }
        }
        (other, _) => other,
    };
    match (selected, cli.reasoning_effort.clone()) {
        (
            ProviderChoice::OpenAi {
                model,
                reasoning_effort,
            },
            effort,
        ) => ProviderChoice::OpenAi {
            model,
            reasoning_effort: effort.or(reasoning_effort),
        },
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
    // Approval is opt-in: the agent runs autonomously by default. `--ask` in
    // TUI mode wires a channel that prompts before destructive ops.
    // Print/one-shot mode never prompts (no terminal to prompt at).
    // Either way we still hand the TUI an mpsc receiver — when the gate is
    // Auto nothing is ever sent on the channel, so the receiver sits idle.
    let (gate_tx, approval_rx) = tokio::sync::mpsc::unbounded_channel();
    let gate = if cli.ask && !is_print {
        ApprovalGate::channel(gate_tx)
    } else {
        ApprovalGate::auto()
    };
    let resume_session_id = match cli.session.as_deref() {
        Some(raw) => Some(
            raw.parse()
                .map_err(|e| anyhow::anyhow!("invalid --session id `{raw}`: {e}"))?,
        ),
        None => None,
    };
    let sessions_dir = match cli.session_dir.clone() {
        Some(p) => p,
        None => session_log::default_sessions_dir()?,
    };
    let mcp_servers = mcp_config::load_mcp_servers(&cwd)?;
    let runtime = runtime::build(
        cwd,
        provider,
        gate,
        resume_session_id,
        sessions_dir,
        mcp_servers,
    )
    .await?;

    if let Some(prompt) = cli.print {
        return run_print_mode(runtime, prompt).await;
    }
    run_tui(runtime, approval_rx).await
}

async fn run_tui(
    runtime: BuiltRuntime,
    approval_rx: tokio::sync::mpsc::UnboundedReceiver<(
        approval::ApprovalRequest,
        tokio::sync::oneshot::Sender<bool>,
    )>,
) -> Result<()> {
    let mut raw_mode = RawModeGuard::new()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(COMPOSER_VIEWPORT_HEIGHT),
        },
    )?;

    let mut app = App::new(runtime, approval_rx);
    let result = app.run(&mut terminal).await;
    let show_resume_hint = app.should_show_resume_hint();
    let session_id = app.session_id();

    let cleanup_result = terminal.clear().and_then(|_| terminal.show_cursor());
    drop(terminal);
    raw_mode.disable()?;
    cleanup_result?;

    if show_resume_hint {
        println!();
        print_resume_divider();
        println!("Resume with ercode --session {session_id}");
        println!();
        print_centered_ukraine_banner();
    }
    result
}

struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self { active: true })
    }

    fn disable(&mut self) -> Result<()> {
        if self.active {
            disable_raw_mode()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

fn print_resume_divider() {
    let width = crossterm::terminal::size()
        .map(|(width, _)| width as usize)
        .unwrap_or(80)
        .max(1);
    println!("\x1b[38;2;45;91;158m{}\x1b[0m", "─".repeat(width));
}

fn print_centered_ukraine_banner() {
    let text = ">> Зроблено в Україні <<";
    let width = crossterm::terminal::size()
        .map(|(width, _)| width as usize)
        .unwrap_or(0);
    let pad = width.saturating_sub(text.chars().count()) / 2;
    println!(
        "{}\x1b[38;2;45;91;158m>> Зроблено в \x1b[38;2;126;94;19mУкраїні <<\x1b[0m",
        " ".repeat(pad)
    );
}

async fn run_print_mode(runtime: BuiltRuntime, prompt: String) -> Result<()> {
    let BuiltRuntime {
        handles,
        startup,
        model,
    } = runtime;
    let color = io::stdout().is_terminal();
    println!("{}", paint(color, "90", &format!("› {prompt}")));
    println!();
    println!(
        "{} {}",
        paint(color, "90", "workspace"),
        startup.workspace_root.display()
    );
    println!(
        "{}  {}",
        paint(color, "90", "provider"),
        paint(color, "96", &model.provider_label())
    );
    println!(
        "{}     {}",
        paint(color, "90", "tools"),
        startup.tool_names.join(", ")
    );
    println!(
        "{}   {} (folder: {}; log: {}; {} prior event(s))",
        paint(color, "90", "session"),
        handles.session_id,
        startup.session_dir.display(),
        startup.session_log_path.display(),
        startup.replayed_events,
    );
    if !startup.capability_commands.is_empty() {
        let names: Vec<String> = startup
            .capability_commands
            .iter()
            .map(|c| format!("/{}", c.name))
            .collect();
        println!("{} {}", paint(color, "90", "commands"), names.join(", "));
    }
    println!();

    let before_events = handles.runtime.events().await.map(|e| e.len()).unwrap_or(0);
    let before_msgs = handles
        .runtime
        .messages(handles.session_id)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let input = model.input_message(prompt);
    let result = handles.runtime.run_turn(handles.session_id, input).await?;
    let events = handles.runtime.events().await.unwrap_or_default();
    let messages = handles
        .runtime
        .messages(handles.session_id)
        .await
        .unwrap_or_default();

    for event in events.iter().skip(before_events) {
        if let Some(status) = app::status_for_event(event) {
            print_status_line(&status.text, color);
        }
        for line in app::lines_for_event(event) {
            print_transcript_line(&line, color);
        }
    }
    for msg in messages.iter().skip(before_msgs) {
        if msg.role == MessageRole::Agent
            && !msg.has_tool_calls()
            && let Some(text) = msg.text()
        {
            let t = text.trim();
            if !t.is_empty() {
                println!();
                print_transcript_line(
                    &app::ChatLine {
                        author: app::Author::Assistant,
                        text: t.to_string(),
                    },
                    color,
                );
            }
        }
    }
    println!(
        "\n{} success={} iterations={} tool_calls={}",
        paint(color, if result.success { "92" } else { "91" }, "done"),
        result.success,
        result.iterations,
        result.tool_calls_count
    );
    if !result.success
        && let Some(err) = result.error
    {
        eprintln!("turn error: {err}");
        std::process::exit(1);
    }
    Ok(())
}

fn print_transcript_line(line: &app::ChatLine, color: bool) {
    match line.author {
        app::Author::Assistant => {
            println!("{} {}", paint(color, "90", "•"), line.text);
        }
        app::Author::Tool => {
            println!(
                "{} {} {}",
                paint(color, "92", "•"),
                paint(color, "93", line.author.label()),
                line.text
            );
        }
        app::Author::ToolDetail => {
            println!("           {}", line.text);
        }
        app::Author::Diff => {
            println!("  {}", paint(color, "95", &line.text));
        }
        app::Author::System | app::Author::User => {
            println!(
                "{} {} {}",
                paint(color, "90", "•"),
                paint(color, "90", line.author.label()),
                line.text
            );
        }
    }
}

fn print_status_line(text: &str, color: bool) {
    println!("{} {}", paint(color, "94", "•"), text);
}

fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}
