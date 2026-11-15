//! `everruns docker doctor` and `health`: the machine's checks, and the
//! server's own system doctor run inside its container.

use std::process::Stdio;

use anyhow::{Context, Result, bail};

use super::*;
use crate::ui;

pub(super) fn doctor(args: &InstanceArgs) -> Result<()> {
    let docker = checks::program();
    let results = checks::run(&docker);
    ui::section("Docker");
    for check in &results {
        check.print();
    }
    if results.iter().any(checks::Check::failed) {
        bail!("Docker is not ready for a local instance");
    }
    if let Some((inst, s)) = existing(args)? {
        let port = port_of(&s);
        let bind = s.value(settings::BIND).unwrap_or("0.0.0.0");
        let running = running_services(&docker, &inst);
        if running.iter().any(|s| s == "caddy") {
            ui::ok(ui::check_line(
                "Port",
                &format!("{port} is this instance's"),
            ));
        } else if port_free(bind, port) {
            ui::ok(ui::check_line("Port", &format!("{port} is free")));
        } else {
            ui::fail(format!("Port: {port} is used by something else"));
            ui::detail(format!(
                "Pick another with {}",
                ui::cmd("everruns docker up --port <port>")
            ));
            bail!("port {port} is taken");
        }
        if running.iter().any(|s| s == "server") {
            system_doctor(&docker, &inst)?;
        }
    } else if port_free("0.0.0.0", DEFAULT_PORT) {
        ui::ok(ui::check_line("Port", &format!("{DEFAULT_PORT} is free")));
    } else {
        ui::warn(format!(
            "Port: {DEFAULT_PORT} is in use; {} will offer another",
            ui::cmd("everruns docker up")
        ));
    }
    Ok(())
}

/// Only the server's own checks: `doctor` without the Docker and port checks.
pub(super) fn health_checks(args: &InstanceArgs) -> Result<()> {
    let Some((inst, _)) = existing(args)? else {
        not_set_up(args);
        return Ok(());
    };
    let docker = checks::program();
    if !running_services(&docker, &inst)
        .iter()
        .any(|s| s == "server")
    {
        bail!(
            "the server is not running; {} starts it",
            ui::cmd(format!("everruns docker up{}", name_flag(&args.name)))
        );
    }
    system_doctor(&docker, &inst)
}

/// With the instance running, Docker being ready says little about Everruns
/// itself, so run the system doctor inside the server container. It needs no
/// sign-in there and sees the same settings the server does.
fn system_doctor(docker: &str, inst: &Instance) -> Result<()> {
    ui::section("Server");
    let output = compose(docker, inst)
        .args(["exec", "-T", "server", "/app/everruns-server", "--doctor"])
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run docker compose")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Images from before the doctor reject the flag.
    if stderr.contains("unexpected argument") {
        ui::warn(format!(
            "System doctor: this server version has none; {} adds it",
            ui::cmd("everruns docker upgrade")
        ));
        return Ok(());
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => bail!("the system doctor found problems; see the fixes above"),
        _ => {
            eprint!("{stderr}");
            bail!("the system doctor could not run in the server container")
        }
    }
}
