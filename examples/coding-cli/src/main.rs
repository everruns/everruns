//! `ercode` — a terminal coding agent using public Framework APIs.

use std::io::{IsTerminal, Write};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use crossterm::event::{self, Event as TerminalEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use everruns::{
    Agent, AgentBuilder, Controls, InMemoryEngine, InputMessage, Model, OpenAI, ReasoningConfig,
    Session, SessionEventKind, ToolStartContext, WorkspaceHeadAccess,
};
use everruns_coding_cli::{Cli, CodingWorkspace, SessionMode, coding_agent, shared_head};
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

    let model = if cli.offline() {
        "llmsim".to_string()
    } else {
        cli.model()
    };
    let agent = build_agent(&workspace, &cli)?;
    let engine = InMemoryEngine::new();

    let session = match session_mode {
        SessionMode::NewHead { name, base, shared } => {
            let head = workspace.create_head(name, base.as_deref(), shared).await?;
            engine.create(agent.clone()).workspace(head).start().await?
        }
        SessionMode::SharedHead { recorded_session } => {
            let recorded = resume_session(&engine, &agent, recorded_session).await?;
            engine
                .create(agent.clone())
                .workspace(shared_head(&recorded)?)
                .start()
                .await?
        }
        SessionMode::Resume { session_id } => resume_session(&engine, &agent, session_id).await?,
    };

    let head = session
        .workspace_head()
        .context("session did not bind a workspace head")?;
    let access = match head.access() {
        WorkspaceHeadAccess::Isolated => "isolated",
        WorkspaceHeadAccess::Shared => "shared",
    };
    eprintln!(
        "session={} workspace={} head={} name={:?} access={} base={} model={}",
        session.session_id(),
        head.workspace_id(),
        head.id(),
        head.name(),
        access,
        head.base().unwrap_or("HEAD"),
        model,
    );
    eprintln!("state={}", state_dir.display());

    if let Some(prompt) = cli.prompt_text() {
        run_prompt(&session, &prompt, false, cli.reasoning_effort()).await
    } else if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        tui(&cli, engine, agent, session, model).await
    } else {
        Err(anyhow!(
            "interactive mode requires a terminal; use --print <PROMPT>"
        ))
    }
}

async fn resume_session(
    engine: &InMemoryEngine,
    agent: &Agent,
    session_id: everruns::SessionId,
) -> Result<Session> {
    engine
        .attach(session_id, agent.clone())
        .await
        .with_context(|| format!("attach session {session_id} to the local engine"))?;
    engine
        .resume(session_id)
        .await
        .with_context(|| format!("resume session {session_id} on its recorded head"))
}

fn build_agent(workspace: &CodingWorkspace, cli: &Cli) -> Result<Agent> {
    let model = if cli.offline() {
        Model::simulated("Offline simulator: configure a real model to execute coding tool calls.")
    } else {
        Model::new(cli.model(), OpenAI::from_env()?)
    };
    let mut builder = workspace.configure_agent(coding_agent(model), cli.workspace_policy());
    if cli.ask() {
        builder = with_approval(builder);
    }
    builder.build().context("build the coding agent")
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
    cli: &Cli,
    active: &mut ActiveSession,
    model: &str,
    line: &str,
) -> Result<bool> {
    match line {
        "/quit" | "/exit" => return Ok(true),
        "/help" => {
            eprintln!("/help  /tools  /cwd  /clear  /model  /quit\nAll other input runs a turn.")
        }
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
        "/clear" => {
            let head = active
                .session
                .workspace_head()
                .context("missing workspace head")?
                .clone();
            active.session = active
                .engine
                .create(active.agent.clone())
                .workspace(head)
                .start()
                .await?;
            eprintln!("new session={}", active.session.session_id());
        }
        "/model" => eprintln!("model={model}"),
        command if command.starts_with('/') => eprintln!("unknown command: {command}"),
        prompt => run_prompt(&active.session, prompt, true, cli.reasoning_effort()).await?,
    }
    Ok(false)
}

async fn tui(
    cli: &Cli,
    engine: InMemoryEngine,
    agent: Agent,
    session: Session,
    model: String,
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
    let mut status = format!("{model} · session {}", active.session.session_id());

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
                let quit = handle_input(cli, &mut active, &model, text.trim()).await;
                enable_raw_mode()?;
                if quit? {
                    break;
                }
                status = format!("{model} · session {}", active.session.session_id());
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
