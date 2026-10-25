//! `everruns docker`: run Everruns on this machine with Docker Compose.
//!
//! Decision: `up` describes an end state (a healthy instance and a CLI signed
//! in to it), not a first-time script. Every step checks before it acts, so
//! running it again is safe and fast, and it repairs what is missing instead of
//! starting over. Secrets are never regenerated and upgrades never happen as a
//! side effect: moving to a new image tag runs migrations, so it is asked for
//! explicitly with `--image-tag`.
//!
//! Design: knowledge/operations/local-setup.md.

mod checks;
mod settings;
mod signin;

use std::io::IsTerminal;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};

use settings::{Instance, Settings};

const DEFAULT_PORT: u16 = 9300;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(240);
const SERVER_IMAGE: &str = "ghcr.io/everruns/everruns-server";

#[derive(Subcommand)]
pub enum DockerCommand {
    /// Start Everruns on this machine, setting it up on first run. Safe to run again
    Up(UpArgs),

    /// Show whether the local instance runs, where, and which CLI profile points at it
    Status(InstanceArgs),

    /// Stop the containers. Data and settings are kept
    Stop(InstanceArgs),

    /// Show the containers' logs
    Logs {
        #[command(flatten)]
        instance: InstanceArgs,

        /// Only this service (server, worker-1, ui, caddy, postgres, ...)
        service: Option<String>,

        /// Print what is there and exit instead of following
        #[arg(long)]
        no_follow: bool,
    },

    /// Open the local UI in the browser
    Open(InstanceArgs),

    /// Check that Docker is installed, running and new enough
    Doctor(InstanceArgs),
}

#[derive(Args)]
pub struct InstanceArgs {
    /// Instance name, for more than one local Everruns
    #[arg(long, default_value = "default")]
    pub name: String,
}

#[derive(Args)]
pub struct UpArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Who signs in
    #[arg(long, value_enum)]
    pub auth: Option<AuthChoice>,

    /// Admin email, used to sign in to the UI
    #[arg(long)]
    pub admin_email: Option<String>,

    /// LLM provider for the server's default key. The key comes from
    /// OPENAI_API_KEY, ANTHROPIC_API_KEY or GEMINI_API_KEY, or a prompt
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    /// Port for the UI and API on this machine (default 9300)
    #[arg(long)]
    pub port: Option<u16>,

    /// Image tag to run, such as v0.48.0 or latest (default: this CLI's version)
    #[arg(long)]
    pub image_tag: Option<String>,

    /// Accept the default for every question
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Do not open the browser
    #[arg(long)]
    pub no_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum AuthChoice {
    /// Just me, with a password (one admin account)
    Single,
    /// No sign-in: anyone who reaches the port is admin; listens on 127.0.0.1 only
    None,
    /// Accounts: people sign up and sign in
    Accounts,
}

impl AuthChoice {
    fn server_mode(self) -> &'static str {
        match self {
            Self::Single => "admin",
            Self::None => "none",
            Self::Accounts => "full",
        }
    }

    fn from_server_mode(mode: &str) -> Option<Self> {
        match mode {
            "admin" => Some(Self::Single),
            "none" => Some(Self::None),
            "full" => Some(Self::Accounts),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Single => "Just me, with a password",
            Self::None => "No sign-in, for trying things out (only this machine can reach it)",
            Self::Accounts => "Accounts: people sign up and sign in",
        }
    }

    /// Whether the instance's default provider key may be shared with every
    /// user: only when no one can sign themselves up.
    fn shares_default_keys(self) -> bool {
        !matches!(self, Self::Accounts)
    }

    /// The server refuses no-sign-in outside the dev grade, which also turns
    /// on experimental features. That fits a throwaway local playground.
    fn deployment_grade(self) -> &'static str {
        match self {
            Self::None => "dev",
            Self::Single | Self::Accounts => "prod",
        }
    }

    /// Without sign-in, nothing outside this machine may reach the port.
    fn bind_address(self) -> &'static str {
        match self {
            Self::None => "127.0.0.1",
            Self::Single | Self::Accounts => "0.0.0.0",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum Provider {
    Openai,
    Anthropic,
    Gemini,
    /// Add a provider later in the UI
    Skip,
}

impl Provider {
    const ALL: [Provider; 4] = [Self::Openai, Self::Anthropic, Self::Gemini, Self::Skip];

    fn key_env(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::Skip => None,
        }
    }

    /// The server variable that seeds the provider's default key.
    fn server_env(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("DEFAULT_OPENAI_API_KEY"),
            Self::Anthropic => Some("DEFAULT_ANTHROPIC_API_KEY"),
            Self::Gemini => Some("DEFAULT_GEMINI_API_KEY"),
            Self::Skip => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Skip => "skip",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Openai => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
            Self::Skip => "Skip, I'll add one in the UI",
        }
    }

    fn env_key(self) -> Option<String> {
        self.key_env()
            .and_then(|k| std::env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
    }
}

pub async fn run(command: DockerCommand) -> Result<()> {
    match command {
        DockerCommand::Up(args) => Box::pin(up(args)).await,
        DockerCommand::Status(args) => Box::pin(status(&args)).await,
        DockerCommand::Stop(args) => stop(&args),
        DockerCommand::Logs {
            instance,
            service,
            no_follow,
        } => logs(&instance, service.as_deref(), no_follow),
        DockerCommand::Open(args) => open(&args),
        DockerCommand::Doctor(args) => doctor(&args),
    }
}

/// How `up` may talk to the person running it.
#[derive(Clone, Copy)]
struct Asker {
    interactive: bool,
    yes: bool,
}

impl Asker {
    fn can_ask(self) -> bool {
        self.interactive && !self.yes
    }
}

async fn up(args: UpArgs) -> Result<()> {
    let docker = checks::program();
    require_docker(&docker, true)?;

    let inst = Instance::new(&args.instance.name)?;
    inst.ensure_dir()?;
    let _lock = inst.lock()?;

    let asker = Asker {
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        yes: args.yes,
    };
    let mut s = Settings::load(&inst.env_path())?;
    let first_run = s.is_empty();

    // Who signs in.
    let auth = match args.auth {
        Some(auth) => auth,
        None => match s
            .value(settings::AUTH_MODE)
            .and_then(AuthChoice::from_server_mode)
        {
            Some(saved) => saved,
            None => ask_auth(asker)?,
        },
    };
    s.set(settings::AUTH_MODE, auth.server_mode());
    s.set(settings::BIND, auth.bind_address());
    s.set(settings::DEPLOYMENT_GRADE, auth.deployment_grade());

    // The admin account. Saved even without sign-in, so switching modes later
    // has an account ready.
    let email = match &args.admin_email {
        Some(email) => email.clone(),
        None => match s.value(settings::ADMIN_EMAIL) {
            Some(saved) => saved.to_string(),
            None if auth == AuthChoice::None => default_email(),
            None => ask_email(asker)?,
        },
    };
    s.set(settings::ADMIN_EMAIL, &email);
    s.set_missing(settings::ADMIN_PASSWORD, settings::password);
    s.fill_secrets();

    // LLM provider. The server only turns a default key into the default
    // organization's provider when nobody can sign themselves up; with
    // accounts, each organization adds its own key in the UI.
    s.set(
        settings::SEED_PROVIDER_KEYS,
        if auth.shares_default_keys() {
            "true"
        } else {
            "false"
        },
    );
    if !auth.shares_default_keys() {
        if args.provider.is_some_and(|p| p != Provider::Skip) {
            bail!("with --auth accounts, add provider keys in the UI; drop --provider");
        }
    } else if let Some(provider) = args.provider {
        set_provider(&mut s, provider, asker)?;
    } else if s.value(settings::LLM_PROVIDER).is_none() {
        let provider = ask_provider(asker)?;
        set_provider(&mut s, provider, asker)?;
    }

    // Port: an explicit one wins; otherwise keep the saved one, or find a
    // free one on first run.
    let running = running_services(&docker, &inst);
    let caddy_running = running.iter().any(|s| s == "caddy");
    let port = match args.port {
        Some(port) => port,
        None => s
            .value(settings::PORT)
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT),
    };
    let port = if caddy_running && s.value(settings::PORT) == Some(&port.to_string()) {
        port
    } else {
        choose_port(port, auth.bind_address(), args.port.is_some(), asker)?
    };
    s.set(settings::PORT, &port.to_string());

    // Image tag.
    let cli_tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let tag = match &args.image_tag {
        Some(tag) => tag.clone(),
        None => match s.value(settings::IMAGE_TAG) {
            Some(saved) => saved.to_string(),
            None => resolve_tag(&docker, &cli_tag),
        },
    };
    s.set(settings::IMAGE_TAG, &tag);

    s.save(&inst.env_path())?;
    inst.write_compose()?;

    if first_run {
        eprintln!(
            "Settings: {} (keep .env: it holds the encryption key)",
            inst.dir.display()
        );
    }

    // Start or update the containers. Compose leaves running containers whose
    // configuration did not change alone, so this is the idempotent step.
    let status = compose(&docker, &inst)
        .args(["up", "--detach", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;
    if !status.success() {
        bail!("`docker compose up` failed; see the output above");
    }

    let health = wait_healthy(&docker, &inst, port).await?;

    let api_url = format!("http://localhost:{port}/api");
    let creds = signin::Credentials {
        auth,
        email: &email,
        password: s.value(settings::ADMIN_PASSWORD).unwrap_or_default(),
    };
    let switch = if asker.yes {
        signin::SwitchPolicy::Always
    } else if asker.interactive {
        signin::SwitchPolicy::Ask
    } else {
        signin::SwitchPolicy::Never
    };
    let signed_in = Box::pin(signin::ensure_profile(
        &api_url,
        &inst.profile(),
        creds,
        switch,
    ))
    .await
    .context("The server is up, but signing the CLI in failed")?;

    let url = format!("http://localhost:{port}");
    eprintln!();
    eprintln!("Everruns {} is running at {url}", health.version);
    match auth {
        AuthChoice::None => eprintln!("  Sign-in: none (only this machine can reach it)"),
        AuthChoice::Single | AuthChoice::Accounts if first_run => eprintln!(
            "  Sign in: {email} / {}  (shown once; also in {})",
            s.value(settings::ADMIN_PASSWORD).unwrap_or_default(),
            inst.env_path().display()
        ),
        AuthChoice::Single | AuthChoice::Accounts => eprintln!(
            "  Sign in: {email} (password in {})",
            inst.env_path().display()
        ),
    }
    if signed_in.current {
        eprintln!(
            "  CLI: signed in as {} (profile '{}'), try `everruns agents list`",
            signed_in.email, signed_in.profile
        );
    } else {
        eprintln!(
            "  CLI: profile '{}'; use it with `everruns --profile {} agents list`",
            signed_in.profile, signed_in.profile
        );
    }
    if tag != cli_tag && tag != "latest" {
        eprintln!(
            "  This instance runs {tag}; this CLI is {cli_tag}. Move to it with \
             `everruns docker up --image-tag {cli_tag}` (runs database migrations)."
        );
    }

    if first_run && asker.interactive && !args.no_open && crate::browser::open(&url).is_err() {
        eprintln!("  Open {url} in your browser.");
    }
    Ok(())
}

fn require_docker(docker: &str, verbose: bool) -> Result<()> {
    let results = checks::run(docker);
    let failed = results.iter().any(checks::Check::failed);
    for check in &results {
        if verbose || failed || !matches!(check.outcome, checks::Outcome::Ok(_)) {
            check.print();
        }
    }
    if failed {
        bail!("Docker is not ready; fix the problem above and run this again");
    }
    Ok(())
}

/// `docker compose` for one instance. Variables `up` manages are removed from
/// the environment: Compose prefers the shell over `.env`, so a stray
/// `AUTH_MODE` in someone's shell would otherwise override the saved setup.
fn compose(docker: &str, inst: &Instance) -> Command {
    let mut cmd = Command::new(docker);
    cmd.arg("compose")
        .arg("--project-name")
        .arg(inst.project())
        .arg("--file")
        .arg(inst.compose_path())
        .arg("--env-file")
        .arg(inst.env_path());
    for key in settings::MANAGED_KEYS {
        cmd.env_remove(key);
    }
    cmd
}

fn running_services(docker: &str, inst: &Instance) -> Vec<String> {
    if !inst.compose_path().exists() {
        return Vec::new();
    }
    compose(docker, inst)
        .args(["ps", "--status", "running", "--services"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn port_free(bind: &str, port: u16) -> bool {
    TcpListener::bind((bind, port)).is_ok()
}

fn choose_port(wanted: u16, bind: &str, explicit: bool, asker: Asker) -> Result<u16> {
    if port_free(bind, wanted) {
        return Ok(wanted);
    }
    if explicit {
        bail!("port {wanted} is in use; pick another with --port");
    }
    let next = (wanted.saturating_add(1)..=wanted.saturating_add(100))
        .find(|p| port_free(bind, *p))
        .with_context(|| {
            format!("port {wanted} is in use and no nearby port is free; use --port")
        })?;
    if asker.can_ask() {
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!("Port {wanted} is in use. Use {next} instead?"))
            .default(true)
            .interact()
            .context("Failed to read the answer")?;
        if !ok {
            bail!("port {wanted} is in use; pick another with --port");
        }
    } else if !asker.yes {
        bail!("port {wanted} is in use; pick another with --port (or pass --yes to take {next})");
    } else {
        eprintln!("Port {wanted} is in use; using {next}.");
    }
    Ok(next)
}

/// This CLI's own release when its images exist, else `latest` (a CLI built
/// from an unreleased commit).
fn resolve_tag(docker: &str, cli_tag: &str) -> String {
    let exists = Command::new(docker)
        .args(["manifest", "inspect", &format!("{SERVER_IMAGE}:{cli_tag}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if exists {
        cli_tag.to_string()
    } else {
        eprintln!("No published images for {cli_tag} yet; using `latest`.");
        "latest".to_string()
    }
}

struct Health {
    version: String,
}

async fn wait_healthy(docker: &str, inst: &Instance, port: u16) -> Result<Health> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let url = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    let mut told = false;
    loop {
        if let Ok(resp) = http.get(&url).send().await
            && resp.status().is_success()
        {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Ok(Health {
                version: body["version"].as_str().unwrap_or("").to_string(),
            });
        }
        if started.elapsed() > HEALTH_TIMEOUT {
            eprintln!("The server did not become healthy. Its last log lines:");
            let _ = compose(docker, inst)
                .args(["logs", "--tail", "40", "server"])
                .status();
            bail!(
                "{url} did not answer within {}s; see `everruns docker logs server`",
                HEALTH_TIMEOUT.as_secs()
            );
        }
        if !told && started.elapsed() > Duration::from_secs(3) {
            eprintln!("Waiting for the server (the first start runs database migrations)...");
            told = true;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn ask_auth(asker: Asker) -> Result<AuthChoice> {
    if !asker.can_ask() {
        return no_prompt(asker, "--auth", AuthChoice::Single);
    }
    let options = [AuthChoice::Single, AuthChoice::None, AuthChoice::Accounts];
    let labels: Vec<&str> = options.iter().map(|o| o.label()).collect();
    let pick = dialoguer::Select::new()
        .with_prompt("Who signs in?")
        .items(&labels)
        .default(0)
        .interact()
        .context("Failed to read the answer")?;
    Ok(options[pick])
}

fn default_email() -> String {
    Command::new("git")
        .args(["config", "--get", "user.email"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|e| e.contains('@'))
        .unwrap_or_else(|| "admin@example.com".to_string())
}

fn ask_email(asker: Asker) -> Result<String> {
    let default = default_email();
    if !asker.can_ask() {
        return no_prompt(asker, "--admin-email", default);
    }
    dialoguer::Input::new()
        .with_prompt("Admin email")
        .default(default)
        .validate_with(|e: &String| {
            if e.contains('@') {
                Ok(())
            } else {
                Err("enter an email address")
            }
        })
        .interact_text()
        .context("Failed to read the answer")
}

fn ask_provider(asker: Asker) -> Result<Provider> {
    // A key already in the shell is the obvious default.
    let detected = Provider::ALL.into_iter().find(|p| p.env_key().is_some());
    if !asker.can_ask() {
        return no_prompt(asker, "--provider", detected.unwrap_or(Provider::Skip));
    }
    let labels: Vec<String> = Provider::ALL
        .iter()
        .map(|p| match (p.key_env(), p.env_key()) {
            (Some(var), Some(_)) => format!("{} (use {var} from your shell)", p.label()),
            _ => p.label().to_string(),
        })
        .collect();
    let default = detected
        .and_then(|d| Provider::ALL.iter().position(|p| *p == d))
        .unwrap_or(0);
    let pick = dialoguer::Select::new()
        .with_prompt("LLM provider")
        .items(&labels)
        .default(default)
        .interact()
        .context("Failed to read the answer")?;
    Ok(Provider::ALL[pick])
}

fn set_provider(s: &mut Settings, provider: Provider, asker: Asker) -> Result<()> {
    if let Some(var) = provider.server_env() {
        let key = match provider.env_key() {
            Some(key) => key,
            None if asker.can_ask() => dialoguer::Password::new()
                .with_prompt(format!("{} API key", provider.label()))
                .interact()
                .context("Failed to read the key")?,
            None => bail!(
                "--provider {} needs its key in {}",
                provider.name(),
                provider.key_env().unwrap_or_default()
            ),
        };
        s.set(var, key.trim());
    }
    s.set(settings::LLM_PROVIDER, provider.name());
    Ok(())
}

/// No one to ask: `--yes` takes the default, otherwise name the flag.
fn no_prompt<T>(asker: Asker, flag: &str, default: T) -> Result<T> {
    if asker.yes {
        Ok(default)
    } else {
        bail!("no terminal to ask on; pass {flag} (or --yes to accept the defaults)")
    }
}

/// An instance that has been set up, or a friendly "nothing here yet".
fn existing(args: &InstanceArgs) -> Result<Option<(Instance, Settings)>> {
    let inst = Instance::new(&args.name)?;
    if !inst.exists() {
        return Ok(None);
    }
    let s = Settings::load(&inst.env_path())?;
    Ok(Some((inst, s)))
}

fn not_set_up(args: &InstanceArgs) {
    let flag = if args.name == "default" {
        String::new()
    } else {
        format!(" --name {}", args.name)
    };
    eprintln!("No local instance yet. Run `everruns docker up{flag}`.");
}

fn port_of(s: &Settings) -> u16 {
    s.value(settings::PORT)
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

async fn status(args: &InstanceArgs) -> Result<()> {
    let Some((inst, s)) = existing(args)? else {
        not_set_up(args);
        return Ok(());
    };
    let docker = checks::program();
    let port = port_of(&s);
    let auth = s
        .value(settings::AUTH_MODE)
        .and_then(AuthChoice::from_server_mode)
        .map_or("unknown", AuthChoice::label);

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let health = match http
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => "healthy".to_string(),
        Ok(resp) => format!("unhealthy ({})", resp.status()),
        Err(_) => "not reachable".to_string(),
    };

    let store = crate::auth::CredentialStore::load().unwrap_or_default();
    let profile = inst.profile();
    let profile_line = match store.profiles.get(&profile) {
        Some(_) if store.current_profile == profile => format!("{profile} (current)"),
        Some(_) => profile.clone(),
        None => "none yet (run `everruns docker up`)".to_string(),
    };

    println!("Instance: {}", inst.name);
    println!("URL:      http://localhost:{port}");
    println!("Health:   {health}");
    println!(
        "Version:  {}",
        s.value(settings::IMAGE_TAG).unwrap_or("unknown")
    );
    println!("Sign-in:  {auth}");
    println!("Profile:  {profile_line}");
    println!("Settings: {}", inst.dir.display());
    println!();
    let _ = compose(&docker, &inst)
        .args(["ps", "--all", "--format", "table {{.Service}}\t{{.Status}}"])
        .status();
    Ok(())
}

fn stop(args: &InstanceArgs) -> Result<()> {
    let Some((inst, _)) = existing(args)? else {
        not_set_up(args);
        return Ok(());
    };
    let docker = checks::program();
    require_docker(&docker, false)?;
    let _lock = inst.lock()?;
    let status = compose(&docker, &inst).arg("stop").status()?;
    if !status.success() {
        bail!("`docker compose stop` failed");
    }
    eprintln!("Stopped. Data is kept; `everruns docker up` starts it again.");
    Ok(())
}

fn logs(args: &InstanceArgs, service: Option<&str>, no_follow: bool) -> Result<()> {
    let Some((inst, _)) = existing(args)? else {
        not_set_up(args);
        return Ok(());
    };
    let mut cmd = compose(&checks::program(), &inst);
    cmd.args(["logs", "--tail", "200"]);
    if !no_follow {
        cmd.arg("--follow");
    }
    if let Some(service) = service {
        cmd.arg(service);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("`docker compose logs` failed");
    }
    Ok(())
}

fn open(args: &InstanceArgs) -> Result<()> {
    let Some((_, s)) = existing(args)? else {
        not_set_up(args);
        return Ok(());
    };
    let url = format!("http://localhost:{}", port_of(&s));
    if crate::browser::open(&url).is_err() {
        println!("{url}");
    }
    Ok(())
}

fn doctor(args: &InstanceArgs) -> Result<()> {
    let docker = checks::program();
    let results = checks::run(&docker);
    for check in &results {
        check.print();
    }
    if results.iter().any(checks::Check::failed) {
        bail!("Docker is not ready for a local instance");
    }
    if let Some((inst, s)) = existing(args)? {
        let port = port_of(&s);
        let bind = s.value(settings::BIND).unwrap_or("0.0.0.0");
        if running_services(&docker, &inst)
            .iter()
            .any(|s| s == "caddy")
        {
            eprintln!("✓ Port: {port} is this instance's");
        } else if port_free(bind, port) {
            eprintln!("✓ Port: {port} is free");
        } else {
            eprintln!(
                "✗ Port: {port} is used by something else\n  Pick another with `everruns docker up --port <port>`"
            );
            bail!("port {port} is taken");
        }
    } else if port_free("0.0.0.0", DEFAULT_PORT) {
        eprintln!("✓ Port: {DEFAULT_PORT} is free");
    } else {
        eprintln!("! Port: {DEFAULT_PORT} is in use; `everruns docker up` will offer another");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_modes_round_trip_through_the_server_names() {
        for choice in [AuthChoice::Single, AuthChoice::None, AuthChoice::Accounts] {
            assert_eq!(
                AuthChoice::from_server_mode(choice.server_mode()),
                Some(choice)
            );
        }
        assert_eq!(AuthChoice::from_server_mode("external"), None);
    }

    #[test]
    fn only_no_sign_in_is_kept_off_the_network() {
        assert_eq!(AuthChoice::None.bind_address(), "127.0.0.1");
        assert_eq!(AuthChoice::Single.bind_address(), "0.0.0.0");
    }

    #[test]
    fn without_a_terminal_the_flag_is_named() {
        let quiet = Asker {
            interactive: false,
            yes: false,
        };
        let err = no_prompt(quiet, "--auth", 1).unwrap_err().to_string();
        assert!(err.contains("--auth") && err.contains("--yes"), "{err}");
        let yes = Asker {
            interactive: false,
            yes: true,
        };
        assert_eq!(no_prompt(yes, "--auth", 1).unwrap(), 1);
    }

    #[test]
    fn provider_flag_without_a_key_fails_without_a_terminal() {
        let quiet = Asker {
            interactive: false,
            yes: true,
        };
        let mut s = Settings::default();
        // Skip needs no key and records the answer.
        set_provider(&mut s, Provider::Skip, quiet).unwrap();
        assert_eq!(s.value(settings::LLM_PROVIDER), Some("skip"));
    }

    #[test]
    fn a_taken_explicit_port_is_an_error() {
        let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let asker = Asker {
            interactive: false,
            yes: true,
        };
        assert!(choose_port(port, "127.0.0.1", true, asker).is_err());
        let other = choose_port(port, "127.0.0.1", false, asker).unwrap();
        assert_ne!(other, port);
        let quiet = Asker {
            interactive: false,
            yes: false,
        };
        assert!(choose_port(port, "127.0.0.1", false, quiet).is_err());
    }
}
