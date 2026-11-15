//! `everruns-sandbox-guest`: the agent inside a sandbox image. Serves the
//! guest protocol (`everruns_contracts::sandbox_guest`) on port 8080.
//!
//! Environment: `EVERRUNS_GUEST_TOKEN_FILE` (or `EVERRUNS_GUEST_TOKEN`),
//! `EVERRUNS_GUEST_WORKSPACE`, `EVERRUNS_GUEST_EXEC_ENV_FILE`,
//! `EVERRUNS_GUEST_LISTEN` (default `0.0.0.0:8080`).
//!
//! With arguments after `--`, the guest also starts that command (the image's
//! own entrypoint) and keeps serving while it runs.

use everruns_contracts::sandbox_guest::GUEST_PORT;
use everruns_integrations::sandbox_guest::server::{config_from_env, router};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("everruns-sandbox-guest {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let config = match config_from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("everruns-sandbox-guest: {message}");
            std::process::exit(2);
        }
    };
    if let Err(err) = std::fs::create_dir_all(&config.workspace) {
        eprintln!(
            "everruns-sandbox-guest: cannot create workspace {}: {err}",
            config.workspace.display()
        );
    }
    let listen =
        std::env::var("EVERRUNS_GUEST_LISTEN").unwrap_or_else(|_| format!("0.0.0.0:{GUEST_PORT}"));
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("everruns-sandbox-guest: cannot listen on {listen}: {err}");
            std::process::exit(1);
        }
    };
    if let Some(position) = args.iter().position(|arg| arg == "--")
        && let Some((program, rest)) = args[position + 1..].split_first()
    {
        match tokio::process::Command::new(program).args(rest).spawn() {
            Ok(mut child) => {
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
            }
            Err(err) => eprintln!("everruns-sandbox-guest: cannot start {program}: {err}"),
        }
    }
    eprintln!("everruns-sandbox-guest listening on {listen}");
    let shutdown = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(err) => {
                eprintln!(
                    "everruns-sandbox-guest: no SIGTERM handler ({err}); stopping on Ctrl-C only"
                );
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    if let Err(err) = axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("everruns-sandbox-guest: {err}");
        std::process::exit(1);
    }
}
