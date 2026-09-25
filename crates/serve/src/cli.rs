//! `serve::start`: one binary, several commands.
//!
//! ```text
//! cargo run -- dev                 # local server + live console (default)
//! cargo run -- start               # production mode, same wire API
//! cargo run -- manifest [--out f]  # the host contract, as JSON
//! cargo run -- eval [--against URL] [FILTER]
//! cargo run -- deploy              # what a host would provision (stub)
//! ```
//!
//! Decision: the commands live in the app binary rather than a separate
//! `serve` CLI, because only the linked binary knows its registrations. A
//! future `cargo serve` would be a thin wrapper that builds and runs these.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail};
use clap::{Parser, Subcommand};
use everruns::{EventStreamError, SessionEvent};
use serde_json::Value;

use crate::app::{App, Mode};
use crate::host::Host;
use crate::host::Notice;

#[derive(Parser)]
#[command(about = "A serve app (experimental). Runs `dev` when no command is given.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Local server with SQLite, simulator fallback, Markdown hot reload and a live console.
    Dev {
        #[arg(long, env = "PORT", default_value_t = 3000)]
        port: u16,
    },
    /// Production mode: every model must route through a gateway, secrets must be set.
    Start {
        #[arg(long, env = "PORT", default_value_t = 3000)]
        port: u16,
    },
    /// Print the manifest (the contract with the host).
    Manifest {
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the #[eval]s in-process, or against a running deployment.
    Eval {
        /// Base URL of a running app, e.g. a preview deploy.
        #[arg(long)]
        against: Option<String>,
        /// Only evals whose name contains this.
        filter: Option<String>,
    },
    /// Show what a host would provision for this build. No cloud target yet.
    Deploy,
}

/// Run the app according to the command line. See the module docs.
pub async fn start(app: App) -> crate::Result {
    let cli = Cli::parse();
    if !app.errors().is_empty() {
        eprintln!("serve: this app cannot start:");
        for error in app.errors() {
            eprintln!("  ✗ {error}");
        }
        bail!("{} problem(s) found during discovery", app.errors().len());
    }
    match cli.command.unwrap_or(Command::Dev { port: 3000 }) {
        Command::Dev { port } => serve(app, Mode::Dev, port).await,
        Command::Start { port } => serve(app, Mode::Start, port).await,
        Command::Manifest { out } => {
            let json = serde_json::to_string_pretty(&app.manifest())?;
            match out {
                Some(path) => std::fs::write(&path, json + "\n")?,
                None => println!("{json}"),
            }
            Ok(())
        }
        Command::Eval { against, filter } => {
            println!(
                "serve eval · {} · {}",
                app.name(),
                against.as_deref().unwrap_or("in-process")
            );
            let report = crate::eval::run(&app, against.as_deref(), filter.as_deref()).await?;
            let failed = report.results.iter().filter(|r| !r.passed).count();
            println!("{} passed, {failed} failed", report.results.len() - failed);
            if failed > 0 {
                bail!("{failed} eval(s) failed");
            }
            Ok(())
        }
        Command::Deploy => {
            deploy_plan(&app);
            Ok(())
        }
    }
}

fn data_dir() -> crate::Result<PathBuf> {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return match url
            .strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"))
        {
            Some(path) => Ok(PathBuf::from(path)),
            None => Err(anyhow!(
                "DATABASE_URL `{url}` is not SQLite; this proof of concept stores sessions in SQLite only"
            )),
        };
    }
    Ok(std::env::var_os("SERVE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".serve")))
}

async fn serve(app: App, mode: Mode, port: u16) -> crate::Result {
    let manifest = app.manifest();
    let missing: Vec<&str> = manifest
        .secrets
        .iter()
        .map(|secret| secret.name.as_str())
        .filter(|name| std::env::var_os(name).is_none_or(|value| value.is_empty()))
        .collect();
    if mode == Mode::Start && !missing.is_empty() {
        bail!("missing secrets: {}", missing.join(", "));
    }
    let data_dir = data_dir()?;
    let host = Host::new(app.clone(), mode, Some(data_dir.clone()))?;
    // Resolve every agent once so a bad model or tool schema fails at boot.
    for agent in &app.inner.agents {
        host.build_agent(agent, None, false)?;
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    banner(&host, mode, port, &data_dir, &missing);
    if mode == Mode::Dev {
        tokio::spawn(console(host.clone(), port));
    }
    crate::scheduler::spawn(&host);
    axum::serve(listener, crate::server::router(host))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn banner(host: &Arc<Host>, mode: Mode, port: u16, data_dir: &std::path::Path, missing: &[&str]) {
    let app = &host.app;
    let manifest = app.manifest();
    let env = crate::gateway::Env::from_process();
    println!();
    println!(
        "  serve · {} {} · {:?} · build {}",
        manifest.app.name, manifest.app.version, mode, host.build_id
    );
    println!("  ⚠ experimental proof of concept; APIs will change");
    println!();
    for agent in &manifest.agents {
        let route = crate::gateway::route(&agent.model, &env)
            .map_or("simulator (no gateway configured)", |route| route.label());
        let role = if agent.sub {
            " (sub)"
        } else if agent.default {
            " (default)"
        } else {
            ""
        };
        println!(
            "  agent    {}{role} · {} → {route}",
            agent.name, agent.model
        );
    }
    for tool in &manifest.tools {
        let gate = if tool.needs_approval == "never" {
            String::new()
        } else {
            format!(" · approval: {}", tool.needs_approval)
        };
        println!("  tool     {}{gate}", tool.name);
    }
    for skill in &manifest.skills {
        println!("  skill    {}", skill.name);
    }
    for channel in &manifest.channels {
        println!(
            "  channel  {} ({}) · POST {}",
            channel.name, channel.kind, channel.route
        );
    }
    for schedule in &manifest.schedules {
        println!("  schedule {} · {}", schedule.name, schedule.cron);
    }
    for connection in &manifest.connections {
        println!("  connect  {} ({})", connection.name, connection.kind);
    }
    println!("  sandbox  {}", manifest.sandbox.kind);
    if !missing.is_empty() {
        println!(
            "  secrets  missing: {} (features using them are off in dev)",
            missing.join(", ")
        );
    }
    for warning in app.warnings() {
        println!("  note     {warning}");
    }
    println!();
    println!("  data     {}", data_dir.display());
    println!("  listen   http://localhost:{port}");
    println!();
    println!(
        "  try:  curl -s localhost:{port}/v1/sessions -H 'content-type: application/json' -d '{{}}'"
    );
    println!(
        "        curl -s localhost:{port}/v1/sessions/<id>/messages -H 'content-type: application/json' \\\n          -d '{{\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}}}}'"
    );
    println!("        curl -N 'localhost:{port}/v1/sessions/<id>/sse?after_sequence=0'");
    println!();
}

/// The dev "terminal UI": one line per meaningful event, as it happens. It
/// follows each live session's canonical events, plus the host's notices for
/// what has no event (pending approvals and questions, deliveries).
async fn console(host: Arc<Host>, port: u16) {
    let mut notices = host.notices.subscribe();
    loop {
        let notice = match notices.recv().await {
            Ok(notice) => notice,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        };
        if let Notice::Live {
            session_id, after, ..
        } = &notice
        {
            tokio::spawn(follow(host.clone(), session_id.clone(), *after));
        }
        if let Some((session, line)) = notice_line(&notice, port) {
            print_line(&stamp(), &session, &line);
        }
    }
}

fn stamp() -> String {
    chrono::Utc::now().format("%H:%M:%S").to_string()
}

fn print_line(at: &str, session: &str, line: &str) {
    // Session ids share a long time-ordered prefix; the tail tells them apart.
    let tail = session.len().saturating_sub(6);
    let short = session.get(tail..).unwrap_or(session);
    println!("{at} {short} {line}");
}

/// Print one session's canonical events after `after`, replay then live.
async fn follow(host: Arc<Host>, id: String, mut after: i32) {
    let Ok(session) = host.session(&id).await else {
        return;
    };
    let Ok(mut events) = session.events_from(after).await else {
        return;
    };
    loop {
        match events.recv().await {
            Ok(Some(event)) => {
                after = event.sequence().unwrap_or(after).max(after);
                if let Some(line) = event_line(&event) {
                    print_line(event.timestamp().get(11..19).unwrap_or(""), &id, &line);
                }
            }
            Err(EventStreamError::Lagged { .. }) => match session.events_from(after).await {
                Ok(fresh) => events = fresh,
                Err(_) => return,
            },
            _ => return,
        }
    }
}

fn clip(value: &str) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() > 160 {
        format!("{}…", flat.chars().take(160).collect::<String>())
    } else {
        flat
    }
}

/// Joined text parts of a canonical message.
fn message_text(message: &Value) -> String {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn event_line(event: &SessionEvent) -> Option<String> {
    let data = &event.canonical_json()["data"];
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    Some(match event.event_type() {
        "input.message" => format!("▸ {}", clip(&message_text(&data["message"]))),
        "tool.started" => format!(
            "  ⚙ {} {}",
            text(&data["tool_call"]["name"]),
            clip(&data["tool_call"]["arguments"].to_string())
        ),
        "tool.progress" => format!("  … {}", text(&data["message"])),
        "tool.completed" => {
            let ok = data["success"].as_bool() == Some(true);
            format!(
                "  {} {}",
                if ok { "✓" } else { "✗" },
                text(&data["tool_name"])
            )
        }
        "output.message.completed" => {
            let reply = message_text(&data["message"]);
            if reply.is_empty() {
                return None;
            }
            format!("◂ {}", clip(&reply))
        }
        "turn.failed" => format!("✗ turn failed: {}", text(&data["error"])),
        "turn.cancelled" => "✗ turn cancelled".to_string(),
        _ => return None,
    })
}

fn notice_line(notice: &Notice, port: u16) -> Option<(String, String)> {
    Some(match notice {
        Notice::Live {
            session_id,
            agent,
            resumed,
            ..
        } => (
            session_id.clone(),
            if *resumed {
                format!("● resumed {agent} after restart")
            } else {
                format!("● new session on {agent}")
            },
        ),
        Notice::BuildChanged { session_id, from } => (
            session_id.clone(),
            format!("  ⚠ session started on build {from}; resuming on this build"),
        ),
        Notice::ApprovalRequested(view) => (
            view.session_id.clone(),
            format!(
                "  ⏸ {} needs approval: {}\n      curl -X POST localhost:{port}/v1/sessions/{}/approvals/{} -H 'content-type: application/json' -d '{{\"decision\":\"approve\"}}'",
                view.tool_name,
                clip(&view.arguments.to_string()),
                view.session_id,
                view.tool_call_id,
            ),
        ),
        Notice::QuestionAsked {
            session_id,
            tool_call_id,
            questions,
        } => {
            let asked: Vec<String> = questions
                .iter()
                .map(|question| {
                    let options: Vec<&str> = question
                        .options
                        .iter()
                        .map(|option| option.label.as_str())
                        .collect();
                    format!(
                        "{} [{}] ({})",
                        question.question,
                        question.id.as_deref().unwrap_or("?"),
                        options.join(" / ")
                    )
                })
                .collect();
            (
                session_id.clone(),
                format!(
                    "  ? {}\n      curl -X POST localhost:{port}/v1/sessions/{session_id}/question-answers -H 'content-type: application/json' -d '{{\"tool_call_id\":\"{tool_call_id}\",\"answers\":[{{\"id\":\"…\",\"selected\":[\"…\"]}}]}}'",
                    asked.join("; ")
                ),
            )
        }
        Notice::Delivered {
            session_id,
            to,
            error,
        } => (
            session_id.clone(),
            match error {
                None => format!("  ↳ delivered to {to}"),
                Some(error) => format!("  ✗ delivery to {to} failed: {error}"),
            },
        ),
    })
}

fn deploy_plan(app: &App) {
    let manifest = app.manifest();
    let target = manifest
        .app
        .deploy_target
        .as_deref()
        .unwrap_or("everruns-cloud");
    println!(
        "serve deploy · {} · build {} → {target}",
        manifest.app.name, manifest.build_id
    );
    println!();
    println!("This proof of concept has no cloud target. A host reading this manifest would:");
    println!("  1. build the binary and an OCI image; publish manifest.json beside it");
    for schedule in &manifest.schedules {
        println!(
            "  2. create cron `{}` → run schedule `{}`",
            schedule.cron, schedule.name
        );
    }
    for channel in &manifest.channels {
        println!(
            "  3. route https://<preview>/v1/channels/{} → this build ({})",
            channel.name, channel.kind
        );
    }
    for secret in &manifest.secrets {
        let state = if std::env::var_os(&secret.name).is_some() {
            "set locally"
        } else {
            "ask the owner"
        };
        println!("  4. secret {} ({state})", secret.name);
    }
    println!(
        "  5. provision DATABASE_URL, NATS_URL and the `{}` sandbox adapter",
        manifest.sandbox.kind
    );
    for model in &manifest.models {
        println!("  6. route model `{model}` through the gateway (SERVE_GATEWAY_URL)");
    }
    println!("  7. give the branch a preview URL; run `eval --against <preview>` before promotion");
    println!(
        "  8. pin new sessions to {}; keep the previous build until its sessions finish",
        manifest.build_id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::PendingApprovalView;
    use serde_json::json;

    #[test]
    fn console_explains_approvals_with_a_ready_curl() {
        let notice = Notice::ApprovalRequested(PendingApprovalView {
            session_id: "session_x".into(),
            tool_call_id: "call_1".into(),
            tool_name: "run_sql".into(),
            arguments: json!({ "sql": "select 1" }),
        });
        let (session, line) = notice_line(&notice, 3000).unwrap();
        assert_eq!(session, "session_x");
        assert!(line.contains("run_sql needs approval"), "{line}");
        assert!(
            line.contains("/v1/sessions/session_x/approvals/call_1"),
            "{line}"
        );
    }

    #[test]
    fn message_text_joins_text_parts() {
        let message = json!({ "content": [
            { "type": "text", "text": "a" },
            { "type": "image", "url": "x" },
            { "type": "text", "text": "b" },
        ] });
        assert_eq!(message_text(&message), "ab");
    }
}
