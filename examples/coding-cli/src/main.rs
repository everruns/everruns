//! `ercode` — a terminal coding agent using public Framework APIs.

mod mcp_config;

use std::io::{IsTerminal, Write};

use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use crossterm::event::{self, Event as TerminalEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use everruns::{
    Agent, AgentBuilder, BearerAuth, Controls, InMemoryEngine, InputMessage, Model, Provider,
    ReasoningConfig, Session, SessionEventKind, StaticHeaderAuth, ToolStartContext,
    WorkspaceHeadAccess,
};
use everruns_anthropic::AnthropicChatDriver;
use everruns_coding_cli::{Cli, CodingWorkspace, ProviderChoice, SessionMode, agent_builder};
use everruns_openai::OpenAIChatDriver;
use everruns_openrouter::OpenRouterChatDriver;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Terminal, TerminalOptions, Viewport};
use ratatui_textarea::TextArea;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

    let cli = Cli::parse();
    let state_dir = cli.state_dir()?;
    let session_mode = cli.session_mode();
    let workspace = match &session_mode {
        SessionMode::NewHead { .. } => CodingWorkspace::open(cli.repository()?, &state_dir).await?,
        SessionMode::SharedHead { .. } | SessionMode::Resume { .. } => {
            CodingWorkspace::from_state(&state_dir)?
        }
    };

    let provider = cli.provider();
    let model = cli.model();
    let agent = build_agent(&workspace, &cli, provider, &model)?;
    let engine = InMemoryEngine::new();

    let session = match session_mode {
        SessionMode::NewHead { name, base, shared } => {
            let head = workspace.create_head(name, base.as_deref(), shared).await?;
            workspace.start_session(&engine, &agent, head).await?
        }
        SessionMode::SharedHead { recorded_session } => {
            workspace
                .start_shared_session(&engine, &agent, recorded_session)
                .await?
        }
        SessionMode::Resume { session_id } => {
            workspace
                .resume_session(&engine, &agent, session_id)
                .await?
        }
    };

    let head = session
        .workspace_head()
        .context("session did not bind a workspace head")?;
    let access = match head.access() {
        WorkspaceHeadAccess::Isolated => "isolated",
        WorkspaceHeadAccess::Shared => "shared",
    };
    eprintln!(
        "session={} workspace={} head={} name={:?} access={} base={} provider={:?} model={}",
        session.session_id(),
        head.workspace_id(),
        head.id(),
        head.name(),
        access,
        head.base().unwrap_or("HEAD"),
        cli.provider(),
        cli.model(),
    );
    eprintln!("state={}", state_dir.display());

    if let Some(prompt) = cli.prompt_text() {
        run_prompt(&session, &prompt, false, cli.reasoning_effort()).await
    } else if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        tui(&workspace, &cli, engine, agent, session, provider, model).await
    } else {
        Err(anyhow!(
            "interactive mode requires a terminal; use --print <PROMPT>"
        ))
    }
}

fn build_agent(
    workspace: &CodingWorkspace,
    cli: &Cli,
    provider: ProviderChoice,
    model: &str,
) -> Result<Agent> {
    let mut builder = workspace.configure_agent(agent_builder(), cli.workspace_policy());
    if let Some(path) = cli.mcp_config() {
        for server in mcp_config::load(&path)? {
            builder = builder.mcp_server(server);
        }
    }
    if cli.ask() {
        builder = with_approval(builder);
    }
    configure_model(builder, provider, model)?
        .build()
        .context("build the coding agent")
}

fn configure_model(
    builder: AgentBuilder,
    provider: ProviderChoice,
    model: &str,
) -> Result<AgentBuilder> {
    let builder = match provider {
        ProviderChoice::Llmsim => builder.model(Model::simulated(
            "Offline simulator: configure a real model to execute coding tool calls.",
        )),
        ProviderChoice::Openai => builder
            .provider(everruns_openai::provider(
                "openai",
                required_env("OPENAI_API_KEY")?,
            ))
            .model(model),
        ProviderChoice::Anthropic => builder
            .provider(
                Provider::new("anthropic", AnthropicChatDriver::new())
                    .base_url("https://api.anthropic.com/v1")
                    .auth(StaticHeaderAuth::new(
                        "x-api-key",
                        required_env("ANTHROPIC_API_KEY")?,
                    )),
            )
            .model(model),
        ProviderChoice::Openrouter => builder
            .provider(
                Provider::new("openrouter", OpenRouterChatDriver::new())
                    .base_url(
                        std::env::var("OPENROUTER_BASE_URL")
                            .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string()),
                    )
                    .auth(BearerAuth::new(required_env("OPENROUTER_API_KEY")?)),
            )
            .model(model),
        ProviderChoice::Ollama => builder
            .provider(
                Provider::new("ollama", OpenAIChatDriver::new())
                    .base_url(
                        std::env::var("OLLAMA_BASE_URL")
                            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string()),
                    )
                    .auth(BearerAuth::new(
                        std::env::var("OLLAMA_API_KEY").unwrap_or_else(|_| "ollama".to_string()),
                    )),
            )
            .model(model),
    };
    Ok(builder)
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is required for the selected provider"))
}

fn with_approval(builder: AgentBuilder) -> AgentBuilder {
    builder.on_tool_start(|context: ToolStartContext| async move {
        if !requires_approval(&context.tool_name) {
            return Ok::<(), String>(());
        }
        tokio::task::spawn_blocking(move || prompt_for_approval(&context))
            .await
            .map_err(|error| format!("approval prompt failed: {error}"))?
    })
}

fn requires_approval(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_file" | "edit_file" | "delete_file" | "bash" | "bashkit" | "bashkit_shell"
    )
}

fn prompt_for_approval(context: &ToolStartContext) -> std::result::Result<(), String> {
    eprintln!("\n[approval] {} {}", context.tool_name, context.arguments);
    eprint!("Allow? [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("tool call denied by user".to_string())
    }
}

fn prompt_input(prompt: &str, reasoning_effort: Option<&str>) -> InputMessage {
    let mut input = InputMessage::user(prompt);
    if let Some(effort) = reasoning_effort {
        input.controls = Some(Controls {
            reasoning: Some(ReasoningConfig {
                effort: Some(effort.to_string()),
            }),
            ..Controls::default()
        });
    }
    input
}

async fn run_prompt(
    session: &Session,
    prompt: &str,
    verbose: bool,
    reasoning_effort: Option<&str>,
) -> Result<()> {
    let mut events = session.events();
    let turn = session
        .run(prompt_input(prompt, reasoning_effort))
        .await
        .context("run turn")?;

    while let Some(event) = events.try_recv().context("read turn events")? {
        match event.kind {
            SessionEventKind::ToolStarted { tool_name, .. } => eprintln!("  → {tool_name}"),
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => eprintln!("  {} {tool_name}", if success { "✓" } else { "✗" }),
            SessionEventKind::ToolOutputDelta { stream, delta, .. } if verbose => {
                eprint!("[{stream}] {delta}")
            }
            SessionEventKind::TextDelta { delta } if verbose => eprint!("{delta}"),
            SessionEventKind::ReasonStarted if verbose => eprintln!("  reasoning…"),
            _ => {}
        }
    }

    if !turn.success
        && let Some(err) = &turn.error
    {
        eprintln!("turn did not complete: {err}");
    }
    println!("{}", turn.response);
    Ok(())
}

struct ActiveSession {
    engine: InMemoryEngine,
    agent: Agent,
    session: Session,
}

async fn handle_input(
    workspace: &CodingWorkspace,
    cli: &Cli,
    active: &mut ActiveSession,
    provider: &mut ProviderChoice,
    model: &mut String,
    line: &str,
) -> Result<bool> {
    match line {
        "/quit" | "/exit" => return Ok(true),
        "/help" => eprintln!(
            "/help  /tools  /cwd  /mcp  /clear  /model  /quit\nAll other input runs a turn."
        ),
        "/cwd" => {
            let head = active
                .session
                .workspace_head()
                .context("missing workspace head")?;
            eprintln!(
                "workspace={} head={} base={}",
                head.workspace_id(),
                head.id(),
                head.base().unwrap_or("HEAD")
            );
        }
        "/tools" => {
            for tool in active.session.inspect().await?.tools {
                eprintln!("{} — {}", tool.name, tool.description);
            }
        }
        "/mcp" => {
            for tool in active
                .session
                .inspect()
                .await?
                .tools
                .into_iter()
                .filter(|tool| tool.name.starts_with("mcp_"))
            {
                eprintln!("{}", tool.name);
            }
        }
        "/clear" => {
            let environment = everruns::Environment::builder()
                .workspace(
                    active
                        .session
                        .workspace_head()
                        .context("missing workspace head")?
                        .clone(),
                )
                .build()?;
            active.session = active
                .engine
                .create(active.agent.clone())
                .environment(environment)
                .start()
                .await?;
            eprintln!("new session={}", active.session.session_id());
        }
        "/model" => {
            eprintln!("model={provider:?}/{model}");
        }
        command if command.starts_with("/model ") => {
            let selection = command.trim_start_matches("/model ").trim();
            let (next_provider, next_model) = parse_model_selection(selection, *provider)?;
            let next_agent = build_agent(workspace, cli, next_provider, &next_model)?;
            let next_engine = InMemoryEngine::new();
            next_engine
                .attach(active.session.session_id(), next_agent.clone())
                .await
                .context("attach session to selected model")?;
            let next_session = next_engine
                .resume(active.session.session_id())
                .await
                .context("resume session with selected model")?;
            *provider = next_provider;
            *model = next_model;
            active.engine = next_engine;
            active.agent = next_agent;
            active.session = next_session;
            eprintln!("model={provider:?}/{model}");
        }
        command if command.starts_with('/') => eprintln!("unknown command: {command}"),
        prompt => run_prompt(&active.session, prompt, true, cli.reasoning_effort()).await?,
    }
    Ok(false)
}

async fn tui(
    workspace: &CodingWorkspace,
    cli: &Cli,
    engine: InMemoryEngine,
    agent: Agent,
    session: Session,
    mut provider: ProviderChoice,
    mut model: String,
) -> Result<()> {
    let mut active = ActiveSession {
        engine,
        agent,
        session,
    };
    enable_raw_mode().context("enable terminal raw mode")?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(7),
        },
    )?;
    let mut input = TextArea::default();
    input.set_block(Block::default().borders(Borders::ALL).title(" prompt "));
    let mut status = format!(
        "{provider:?}/{model} · session {}",
        active.session.session_id()
    );

    let result = async {
        loop {
            terminal.draw(|frame| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(2), Constraint::Min(3)])
                    .split(frame.area());
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(status.as_str()),
                        Line::from("Enter send · Alt/Shift-Enter newline · Esc quit · /help"),
                    ]),
                    chunks[0],
                );
                frame.render_widget(&input, chunks[1]);
            })?;

            let TerminalEvent::Key(key) = event::read().context("read terminal input")? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if key.code == KeyCode::Esc {
                break;
            }
            if key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            {
                let text = input.lines().join("\n");
                if text.trim().is_empty() {
                    continue;
                }
                input = TextArea::default();
                input.set_block(Block::default().borders(Borders::ALL).title(" prompt "));
                terminal.clear()?;
                disable_raw_mode()?;
                let quit = handle_input(
                    workspace,
                    cli,
                    &mut active,
                    &mut provider,
                    &mut model,
                    text.trim(),
                )
                .await;
                enable_raw_mode()?;
                if quit? {
                    break;
                }
                status = format!(
                    "{provider:?}/{model} · session {}",
                    active.session.session_id()
                );
                continue;
            }
            let _ = input.input(key);
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let cleanup = terminal.clear().and_then(|_| terminal.show_cursor());
    disable_raw_mode().ok();
    cleanup?;
    result
}

fn parse_model_selection(
    selection: &str,
    current_provider: ProviderChoice,
) -> Result<(ProviderChoice, String)> {
    if selection.is_empty() {
        return Err(anyhow!("model selection must not be empty"));
    }
    if let Some((provider, model)) = selection.split_once('/')
        && let Ok(provider) = ProviderChoice::from_str(provider, true)
    {
        if model.is_empty() {
            return Err(anyhow!("model id must not be empty"));
        }
        return Ok((provider, model.to_string()));
    }
    Ok((current_provider, selection.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_command_accepts_open_provider_ids_and_bare_model_names() {
        assert_eq!(
            parse_model_selection("anthropic/claude-sonnet-4-5", ProviderChoice::Openai).unwrap(),
            (ProviderChoice::Anthropic, "claude-sonnet-4-5".to_string())
        );
        assert_eq!(
            parse_model_selection("openrouter/openai/gpt-5.2", ProviderChoice::Openai).unwrap(),
            (ProviderChoice::Openrouter, "openai/gpt-5.2".to_string())
        );
    }
}
