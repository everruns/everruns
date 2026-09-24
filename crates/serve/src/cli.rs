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
use serde_json::Value;

use crate::app::{App, Mode};
use crate::host::Host;
use crate::store::WireEvent;

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
        host.build_agent(agent, &crate::Cx::app(&host), false)?;
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
        "  try:  curl -s localhost:{port}/v1/sessions -H 'content-type: application/json' -d '{{\"input\":\"hello\"}}'"
    );
    println!("        curl -N localhost:{port}/v1/sessions/<id>/events");
    println!();
}

/// The dev "terminal UI": one line per meaningful event, as it happens.
async fn console(host: Arc<Host>, port: u16) {
    let mut events = host.events.subscribe();
    let mut streaming = std::collections::HashSet::new();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        };
        if let Some(line) = console_line(&event, port, &mut streaming) {
            // Session ids share a long time-ordered prefix; the tail tells them apart.
            let tail = event.session_id.len().saturating_sub(6);
            let short = event.session_id.get(tail..).unwrap_or(&event.session_id);
            println!("{} {short} {line}", event.at.get(11..19).unwrap_or(""));
        }
    }
}

fn console_line(
    event: &WireEvent,
    port: u16,
    streaming: &mut std::collections::HashSet<String>,
) -> Option<String> {
    let data = &event.data;
    let text = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let clip = |value: String| {
        let flat = value.replace('\n', " ");
        if flat.chars().count() > 160 {
            format!("{}…", flat.chars().take(160).collect::<String>())
        } else {
            flat
        }
    };
    Some(match event.kind.as_str() {
        "session.created" => format!("● new session on {}", text("agent")),
        "session.resumed" => "● resumed after restart".to_string(),
        "message.accepted" => format!("▸ {} ({})", clip(text("input")), text("disposition")),
        "tool.started" => format!(
            "  ⚙ {} {}",
            text("tool_name"),
            clip(
                data.get("arguments")
                    .map(Value::to_string)
                    .unwrap_or_default()
            )
        ),
        "tool.completed" => {
            let ok = data.get("success").and_then(Value::as_bool) == Some(true);
            format!("  {} {}", if ok { "✓" } else { "✗" }, text("tool_name"))
        }
        "tool.progress" => format!("  … {}", text("message")),
        "approval.requested" => format!(
            "  ⏸ {} needs approval: {}\n      curl -X POST localhost:{port}/v1/sessions/{}/approvals/{} -H 'content-type: application/json' -d '{{\"decision\":\"approve\"}}'",
            text("tool"),
            clip(
                data.get("arguments")
                    .map(Value::to_string)
                    .unwrap_or_default()
            ),
            event.session_id,
            text("approval_id"),
        ),
        "approval.resolved" => format!("  ▶ {} {}", text("approval_id"), text("decision")),
        "subagent.started" => format!("  ↘ {}: {}", text("agent"), clip(text("task"))),
        "subagent.completed" => format!("  ↗ {}", text("agent")),
        "output.message.delta" => {
            streaming.insert(event.session_id.clone());
            return None;
        }
        "turn.result" => {
            streaming.remove(&event.session_id);
            if data.get("success").and_then(Value::as_bool) == Some(true) {
                format!("◂ {}", clip(text("response")))
            } else {
                format!("✗ turn failed: {}", text("error"))
            }
        }
        "delivery.completed" => format!("  ↳ delivered to {}", text("to")),
        "delivery.failed" => format!("  ✗ delivery to {} failed: {}", text("to"), text("error")),
        "session.build_changed" => format!(
            "  ⚠ session started on build {}; resuming on {}",
            text("from"),
            text("to")
        ),
        _ => return None,
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
    use serde_json::json;

    #[test]
    fn console_explains_approvals_with_a_ready_curl() {
        let event = WireEvent {
            seq: 4,
            session_id: "session_x".into(),
            kind: "approval.requested".into(),
            at: "2026-09-24T18:00:00.000Z".into(),
            data: json!({ "approval_id": "apr_1", "tool": "run_sql", "arguments": { "sql": "select 1" } }),
        };
        let line = console_line(&event, 3000, &mut Default::default()).unwrap();
        assert!(line.contains("run_sql needs approval"), "{line}");
        assert!(
            line.contains("/v1/sessions/session_x/approvals/apr_1"),
            "{line}"
        );
    }

    #[test]
    fn console_skips_deltas() {
        let event = WireEvent {
            seq: 1,
            session_id: "s".into(),
            kind: "output.message.delta".into(),
            at: String::new(),
            data: json!({ "delta": "x" }),
        };
        assert!(console_line(&event, 3000, &mut Default::default()).is_none());
    }
}
