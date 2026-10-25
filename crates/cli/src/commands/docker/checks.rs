//! Is Docker usable for a local instance? Each check names what is wrong and
//! the one thing to do about it.
//!
//! Decision: shell out to the `docker` CLI rather than talk to the daemon API.
//! The CLI already knows the user's context, socket, Docker Desktop or Colima
//! setup, and `docker compose` is what runs the stack anyway, so there is no
//! client library to add and no second way to reach the daemon to get wrong.

use std::process::{Command, Output, Stdio};

/// Minimum Compose: profiles with `depends_on` across them need 2.20.
const MIN_COMPOSE: (u32, u32) = (2, 20);
const MIN_ENGINE: (u32, u32) = (20, 10);
const MIN_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The container CLI to run: `docker`, or what `EVERRUNS_DOCKER` names
/// (for example `podman` with its Docker-compatible CLI).
pub fn program() -> String {
    std::env::var("EVERRUNS_DOCKER")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "docker".to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ok(String),
    Warn(String),
    Fail { problem: String, fix: String },
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub outcome: Outcome,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Ok(detail.into()),
        }
    }
    fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Warn(detail.into()),
        }
    }
    fn fail(name: &'static str, problem: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Fail {
                problem: problem.into(),
                fix: fix.into(),
            },
        }
    }

    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Fail { .. })
    }

    pub fn print(&self) {
        match &self.outcome {
            Outcome::Ok(detail) => eprintln!("✓ {}: {detail}", self.name),
            Outcome::Warn(detail) => eprintln!("! {}: {detail}", self.name),
            Outcome::Fail { problem, fix } => eprintln!("✗ {}: {problem}\n  {fix}", self.name),
        }
    }
}

/// Run every Docker check. Later checks are skipped once one fails, since
/// they would only repeat the same cause.
pub fn run(program: &str) -> Vec<Check> {
    let mut checks = Vec::new();

    let server = run_quiet(
        program,
        &[
            "version",
            "--format",
            "{{.Server.Version}}|{{.Server.Arch}}",
        ],
    );
    let output = match server {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            checks.push(Check::fail(
                "Docker",
                format!("`{program}` is not installed"),
                install_hint(),
            ));
            return checks;
        }
        Err(e) => {
            checks.push(Check::fail(
                "Docker",
                format!("could not run `{program}`: {e}"),
                install_hint(),
            ));
            return checks;
        }
        Ok(output) => output,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        checks.push(daemon_failure(&stderr));
        return checks;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (version, arch) = stdout.trim().split_once('|').unwrap_or((stdout.trim(), ""));
    checks.push(engine_version(version));
    checks.push(architecture(arch));

    let compose = run_quiet(program, &["compose", "version", "--short"]);
    checks.push(match compose {
        Ok(out) if out.status.success() => {
            compose_version(String::from_utf8_lossy(&out.stdout).trim())
        }
        _ => Check::fail(
            "Docker Compose",
            "the Compose plugin (`docker compose`) is missing",
            "Install Docker Compose v2: https://docs.docker.com/compose/install/",
        ),
    });

    if let Ok(out) = run_quiet(program, &["info", "--format", "{{.MemTotal}}"])
        && out.status.success()
    {
        checks.push(memory(String::from_utf8_lossy(&out.stdout).trim()));
    }

    checks
}

fn run_quiet(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
}

fn install_hint() -> String {
    if cfg!(target_os = "macos") {
        "Install Docker Desktop (https://docs.docker.com/desktop/setup/install/mac-install/), \
         OrbStack or Colima, then run this again."
            .to_string()
    } else if cfg!(windows) {
        "Install Docker Desktop: https://docs.docker.com/desktop/setup/install/windows-install/"
            .to_string()
    } else {
        "Install Docker Engine: https://docs.docker.com/engine/install/".to_string()
    }
}

pub(crate) fn daemon_failure(stderr: &str) -> Check {
    let lower = stderr.to_lowercase();
    if lower.contains("permission denied") {
        Check::fail(
            "Docker",
            "your user may not use the Docker daemon",
            "Add yourself to the docker group, then log out and back in: \
             sudo usermod -aG docker $USER",
        )
    } else {
        let fix = if cfg!(target_os = "macos") || cfg!(windows) {
            "Start Docker Desktop (or OrbStack / `colima start`) and run this again."
        } else {
            "Start it: sudo systemctl start docker"
        };
        Check::fail("Docker", "Docker is installed but not running", fix)
    }
}

pub(crate) fn engine_version(version: &str) -> Check {
    match parse_version(version) {
        Some(v) if v >= MIN_ENGINE => Check::ok("Docker", format!("engine {version}")),
        Some(_) => Check::fail(
            "Docker",
            format!("engine {version} is too old"),
            format!(
                "Update Docker to {}.{} or newer.",
                MIN_ENGINE.0, MIN_ENGINE.1
            ),
        ),
        None => Check::warn(
            "Docker",
            format!("could not read the engine version ('{version}')"),
        ),
    }
}

pub(crate) fn compose_version(version: &str) -> Check {
    match parse_version(version) {
        Some(v) if v >= MIN_COMPOSE => Check::ok("Docker Compose", version.to_string()),
        Some(_) => Check::fail(
            "Docker Compose",
            format!("{version} is too old"),
            format!(
                "Update Docker Compose to {}.{} or newer: https://docs.docker.com/compose/install/",
                MIN_COMPOSE.0, MIN_COMPOSE.1
            ),
        ),
        None => Check::warn(
            "Docker Compose",
            format!("could not read the version ('{version}')"),
        ),
    }
}

pub(crate) fn architecture(arch: &str) -> Check {
    match arch {
        "amd64" | "arm64" => Check::ok("CPU", arch.to_string()),
        "" => Check::warn("CPU", "could not read the architecture"),
        other => Check::fail(
            "CPU",
            format!("{other} is not supported"),
            "Everruns images are built for amd64 and arm64 only.",
        ),
    }
}

pub(crate) fn memory(total: &str) -> Check {
    match total.parse::<u64>() {
        Ok(bytes) if bytes < MIN_MEMORY_BYTES => Check::warn(
            "Memory",
            format!(
                "Docker has {:.1} GB; give it at least 2 GB (Docker Desktop: Settings > Resources)",
                bytes as f64 / 1e9
            ),
        ),
        Ok(bytes) => Check::ok(
            "Memory",
            format!("{:.1} GB available to Docker", bytes as f64 / 1e9),
        ),
        Err(_) => Check::warn("Memory", "could not read Docker's memory limit"),
    }
}

/// `major.minor` from versions such as `27.3.1`, `v2.29.7` or `2.20.0-desktop.1`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let text = text.trim().trim_start_matches('v');
    let mut parts = text.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_the_shapes_docker_prints() {
        assert_eq!(parse_version("27.3.1"), Some((27, 3)));
        assert_eq!(parse_version("v2.29.7"), Some((2, 29)));
        assert_eq!(parse_version("2.20.0-desktop.1"), Some((2, 20)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("dev"), None);
    }

    #[test]
    fn old_compose_fails_with_the_minimum_named() {
        assert!(!compose_version("2.29.7").failed());
        assert!(!compose_version("2.20.0").failed());
        let old = compose_version("2.17.3");
        assert!(old.failed());
        let Outcome::Fail { fix, .. } = old.outcome else {
            unreachable!()
        };
        assert!(fix.contains("2.20"), "{fix}");
    }

    #[test]
    fn old_engine_fails() {
        assert!(!engine_version("27.3.1").failed());
        assert!(engine_version("19.03.5").failed());
        assert!(
            !engine_version("weird").failed(),
            "unknown versions only warn"
        );
    }

    #[test]
    fn daemon_errors_point_at_the_right_fix() {
        let denied =
            daemon_failure("permission denied while trying to connect to the Docker daemon socket");
        let Outcome::Fail { fix, .. } = denied.outcome else {
            unreachable!()
        };
        assert!(fix.contains("docker group"), "{fix}");

        let down =
            daemon_failure("Cannot connect to the Docker daemon. Is the docker daemon running?");
        let Outcome::Fail { problem, .. } = down.outcome else {
            unreachable!()
        };
        assert!(problem.contains("not running"), "{problem}");
    }

    #[test]
    fn unsupported_architectures_fail() {
        assert!(!architecture("arm64").failed());
        assert!(architecture("s390x").failed());
        assert!(!architecture("").failed());
    }

    #[test]
    fn low_memory_only_warns() {
        assert!(matches!(memory("1073741824").outcome, Outcome::Warn(_)));
        assert!(matches!(memory("8589934592").outcome, Outcome::Ok(_)));
    }

    #[test]
    fn missing_binary_is_a_failure_with_an_install_link() {
        let checks = run("everruns-no-such-docker-binary");
        assert_eq!(checks.len(), 1);
        let Outcome::Fail { fix, .. } = &checks[0].outcome else {
            panic!("{checks:?}")
        };
        assert!(fix.contains("https://"), "{fix}");
    }
}
