//! Linux defense-in-depth launcher for the pinned, adjacent `monty` binary.
//! No shell, PATH lookup, user configuration or fallback on sandbox failure.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[cfg(target_os = "linux")]
#[path = "../monty/isolation.rs"]
mod isolation;

fn main() {
    #[cfg(target_os = "linux")]
    if let Err(error) = run() {
        eprintln!("Monty isolation unavailable: {error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("The hardened Monty worker requires Linux with Landlock ABI 3 or newer");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn run() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    if std::env::args().skip(1).collect::<Vec<_>>() != ["subprocess"] {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "expected subprocess",
        ));
    }
    let runtime = std::env::current_exe()?
        .with_file_name("monty")
        .canonicalize()?;
    isolation::restrict(&runtime)?;
    Err(std::process::Command::new(runtime)
        .arg("subprocess")
        .env_clear()
        .exec())
}
