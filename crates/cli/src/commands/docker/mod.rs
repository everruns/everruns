//! `everruns docker`: run Everruns on this machine with Docker Compose.
//!
//! Decision: `up` describes an end state (a healthy instance and a CLI signed
//! in to it), not a first-time script. Every step checks before it acts, so
//! running it again is safe and fast, and it repairs what is missing instead of
//! starting over. Secrets are never regenerated and upgrades never happen as a
//! side effect: moving to a new image tag runs migrations, so it is asked for
//! explicitly with `upgrade`. `destroy` is the one command that deletes data,
//! and it asks for the instance name first.

mod checks;
mod public;
pub(crate) mod settings;
pub(crate) mod signin;

use std::io::IsTerminal;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ui;
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

    /// Check that Docker is ready, and run the system doctor on a running instance
    Doctor(InstanceArgs),

    /// Move the instance to this CLI's version (or --image-tag), backing up the database first
    Upgrade(UpgradeArgs),

    /// Delete the instance and everything it made: containers, database, settings. Asks first
    Destroy(DestroyArgs),
}

#[derive(Args)]
pub struct UpgradeArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Image tag to move to, such as v0.49.0 or latest (default: this CLI's version)
    #[arg(long)]
    pub image_tag: Option<String>,

    /// Skip the database backup taken before the upgrade
    #[arg(long)]
    pub no_backup: bool,

    /// Do not ask, also when the move is to an older version
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Args)]
pub struct DestroyArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Do not ask for the instance name (for scripts)
    #[arg(long, short = 'y')]
    pub yes: bool,
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
    /// OPENAI_API_KEY, ANTHROPIC_API_KEY, GEMINI_API_KEY or OPENROUTER_API_KEY, or a prompt
    #[arg(long, value_enum)]
    pub provider: Option<LlmProvider>,

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

    /// Public HTTPS address people and webhooks use, such as
    /// https://agents.example.com. Passed to the server as PUBLIC_APP_URL;
    /// an empty value removes it
    #[arg(long, value_name = "URL")]
    pub public_url: Option<String>,

    /// Give the instance a public address through a tunnel that runs with the
    /// stack: tailscale (free stable *.ts.net), cloudflare (named with
    /// --public-url, else a quick one whose address changes), ngrok, or none
    #[arg(long, value_enum)]
    pub tunnel: Option<public::TunnelChoice>,
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
    pub(crate) fn server_mode(self) -> &'static str {
        match self {
            Self::Single => "admin",
            Self::None => "none",
            Self::Accounts => "full",
        }
    }

    pub(crate) fn from_server_mode(mode: &str) -> Option<Self> {
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
    pub(crate) fn shares_default_keys(self) -> bool {
        !matches!(self, Self::Accounts)
    }

    /// The server refuses no-sign-in outside the dev grade, which also turns
    /// on experimental features. That fits a throwaway local playground.
    pub(crate) fn deployment_grade(self) -> &'static str {
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
pub enum LlmProvider {
    Openai,
    Anthropic,
    Gemini,
    Openrouter,
    /// Add a provider later in the UI
    Skip,
}

impl LlmProvider {
    pub(crate) const ALL: [LlmProvider; 5] = [
        Self::Openai,
        Self::Anthropic,
        Self::Gemini,
        Self::Openrouter,
        Self::Skip,
    ];

    pub(crate) fn key_env(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::Openrouter => Some("OPENROUTER_API_KEY"),
            Self::Skip => None,
        }
    }

    /// The server variable that seeds the provider's default key.
    pub(crate) fn server_env(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("DEFAULT_OPENAI_API_KEY"),
            Self::Anthropic => Some("DEFAULT_ANTHROPIC_API_KEY"),
            Self::Gemini => Some("DEFAULT_GEMINI_API_KEY"),
            Self::Openrouter => Some("DEFAULT_OPENROUTER_API_KEY"),
            Self::Skip => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Openrouter => "openrouter",
            Self::Skip => "skip",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Openai => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
            Self::Openrouter => "OpenRouter",
            Self::Skip => "Skip, I'll add one in the UI",
        }
    }

    pub(crate) fn env_key(self) -> Option<String> {
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
        DockerCommand::Upgrade(args) => Box::pin(upgrade(args)).await,
        DockerCommand::Destroy(args) => destroy(&args),
    }
}

/// How `up` may talk to the person running it.
#[derive(Clone, Copy)]
pub(crate) struct Asker {
    pub(crate) interactive: bool,
    pub(crate) yes: bool,
}

impl Asker {
    pub(crate) fn can_ask(self) -> bool {
        self.interactive && !self.yes
    }
}

async fn up(args: UpArgs) -> Result<()> {
    let inst = Instance::new(&args.instance.name)?;
    ui::banner("docker up", &[("instance", &inst.name)]);
    let docker = checks::program();
    require_docker(&docker, true)?;

    inst.ensure_dir()?;
    let _lock = inst.lock()?;

    let asker = Asker {
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        yes: args.yes,
    };
    let mut s = Settings::load(&inst.env_path())?;
    let first_run = s.is_empty();
    if first_run {
        ui::section("Setup");
    }

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
        if args.provider.is_some_and(|p| p != LlmProvider::Skip) {
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

    // Public address. Decided after sign-in, which it depends on.
    let public_plan = public::plan(
        &s,
        args.tunnel,
        args.public_url.as_deref(),
        auth == AuthChoice::None,
    )?;
    public::ensure_token(&mut s, &public_plan, asker.can_ask())?;
    public::apply(&mut s, &public_plan);
    if public_plan
        .url
        .as_deref()
        .is_some_and(|u| u.starts_with("http://"))
    {
        ui::warn(format!(
            "{} is plain http. Slack and OAuth callbacks need https.",
            public_plan.url.as_deref().unwrap_or_default()
        ));
    }

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
        ui::ok(format!("Settings saved to {}", inst.dir.display()));
        ui::detail("Keep its .env: it holds the encryption key.");
    }

    ui::section("Starting");

    // Start or update the containers. Compose leaves running containers whose
    // configuration did not change alone, so this is the idempotent step.
    let status = compose(&docker, &inst)
        .args(["up", "--detach", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;
    if !status.success() {
        bail!("`docker compose up` failed; see the output above");
    }
    stop_other_tunnels(&docker, &inst, &running, public_plan.service);

    // A tunnel that picks its own address reports it once up; the server is
    // then recreated with it, which Compose does for a changed variable.
    let mut public_url = public_plan.url.clone();
    if let Some(service) = public_plan.service.filter(|t| t.discovers_url) {
        let found = public::discover_url(|| compose(&docker, &inst), service).await?;
        if public_url.as_deref() != Some(found.as_str()) {
            s.set(settings::PUBLIC_URL, &found);
            s.save(&inst.env_path())?;
            let status = compose(&docker, &inst)
                .args(["up", "--detach", "server"])
                .status()
                .context("Failed to run docker compose")?;
            if !status.success() {
                bail!("restarting the server with the public address failed");
            }
        }
        public_url = Some(found);
    }

    let health = wait_healthy(&docker, &inst, port).await?;

    let api_url = format!("http://localhost:{port}/api");
    let creds = signin::Credentials {
        auth,
        email: &email,
        password: s.value(settings::ADMIN_PASSWORD).unwrap_or_default(),
        password_hint: "put the new one in AUTH_ADMIN_PASSWORD in the instance's .env",
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
    .await;
    let signed_in = match signed_in {
        Ok(s) => {
            ui::ok(format!(
                "CLI signed in as {} {}",
                s.email,
                ui::dim(format!("(profile {})", s.profile))
            ));
            s
        }
        Err(e) => {
            return Err(e.context("The server is up, but signing the CLI in failed"));
        }
    };

    let url = format!("http://localhost:{port}");
    let mut rows = vec![("UI", ui::cmd(&url).to_string())];
    let mut notes: Vec<String> = Vec::new();
    match auth {
        AuthChoice::None => rows.push((
            "Sign in",
            ui::dim("none (only this machine can reach it)").to_string(),
        )),
        // A single admin login has nothing else to sign in with, so its
        // password is shown on every run; with accounts it is the bootstrap
        // admin's and is shown only when it was generated.
        AuthChoice::Single => {
            rows.push(("Sign in", email.clone()));
            rows.push((
                "Password",
                format!(
                    "{} {}",
                    ui::bold(s.value(settings::ADMIN_PASSWORD).unwrap_or_default()),
                    ui::dim("(also in .env)")
                ),
            ));
        }
        AuthChoice::Accounts if first_run => {
            rows.push(("Sign in", email.clone()));
            rows.push((
                "Password",
                format!(
                    "{} {}",
                    ui::bold(s.value(settings::ADMIN_PASSWORD).unwrap_or_default()),
                    ui::dim("(shown once; also in .env)")
                ),
            ));
        }
        AuthChoice::Accounts => {
            rows.push(("Sign in", email.clone()));
            rows.push((
                "Password",
                ui::dim(format!(
                    "AUTH_ADMIN_PASSWORD in {}",
                    inst.env_path().display()
                ))
                .to_string(),
            ));
        }
    }
    if let Some(public_url) = &public_url {
        if public::reachable(public_url).await {
            rows.push(("Public", ui::cmd(public_url).to_string()));
        } else {
            rows.push((
                "Public",
                format!(
                    "{} {}",
                    ui::cmd(public_url),
                    ui::warning("(not answering yet)")
                ),
            ));
            notes.push(public::unreachable_hint(public_plan.tunnel).to_string());
        }
        if public_plan
            .service
            .is_some_and(|t| t.profile == "cloudflare-quick")
        {
            notes.push(format!(
                "This quick tunnel's address changes whenever it restarts; run {} again to pick up the new one.",
                ui::cmd(format!("everruns docker up{}", name_flag(&inst.name)))
            ));
        }
    }
    rows.push(if signed_in.current {
        ("CLI", ui::cmd("everruns agents list").to_string())
    } else {
        (
            "CLI",
            ui::cmd(format!(
                "everruns --profile {} agents list",
                signed_in.profile
            ))
            .to_string(),
        )
    });
    if tag != cli_tag && tag != "latest" {
        notes.push(format!(
            "This instance runs {tag}; this CLI is {cli_tag}. Move to it with {} \
             (backs up the database, then runs migrations).",
            ui::cmd(format!("everruns docker upgrade{}", name_flag(&inst.name)))
        ));
    }
    ui::summary(
        &format!("Everruns {} is running", health.version),
        "on Docker",
        &rows,
    );
    for note in notes {
        ui::warn(note);
    }

    if first_run && asker.interactive && !args.no_open && crate::browser::open(&url).is_err() {
        ui::note(format!("Open {} in your browser.", ui::cmd(&url)));
    }
    Ok(())
}

fn require_docker(docker: &str, verbose: bool) -> Result<()> {
    let results = checks::run(docker);
    let failed = results.iter().any(checks::Check::failed);
    if results
        .iter()
        .any(|c| verbose || failed || !matches!(c.outcome, checks::Outcome::Ok(_)))
    {
        ui::section("Docker checks");
    }
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

/// Stop and remove tunnels that are no longer chosen. Compose leaves a
/// service whose profile was turned off running, since it is not an orphan.
fn stop_other_tunnels(
    docker: &str,
    inst: &Instance,
    running: &[String],
    keep: Option<public::TunnelService>,
) {
    for tunnel in public::TUNNEL_SERVICES {
        if Some(tunnel) != keep && running.iter().any(|s| s == tunnel.service) {
            let _ = compose(docker, inst)
                .args(["rm", "--stop", "--force", tunnel.service])
                .stdout(Stdio::null())
                .status();
        }
    }
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
        let ok = dialoguer::Confirm::with_theme(crate::ui::theme())
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
        ui::warn(format!("Port {wanted} is in use; using {next}."));
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
        ui::warn(format!(
            "No published images for {cli_tag} yet; using `latest`."
        ));
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
    let spinner =
        ui::Spinner::start("Waiting for the server (the first start runs database migrations)");
    loop {
        if let Ok(resp) = http.get(&url).send().await
            && resp.status().is_success()
        {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            spinner.done("Server is healthy");
            return Ok(Health {
                version: body["version"].as_str().unwrap_or("").to_string(),
            });
        }
        if started.elapsed() > HEALTH_TIMEOUT {
            spinner.failed("The server did not become healthy. Its last log lines:");
            let _ = compose(docker, inst)
                .args(["logs", "--tail", "40", "server"])
                .status();
            bail!(
                "{url} did not answer within {}s; see `everruns docker logs server`",
                HEALTH_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub(crate) fn ask_auth(asker: Asker) -> Result<AuthChoice> {
    if !asker.can_ask() {
        return no_prompt(asker, "--auth", AuthChoice::Single);
    }
    let options = [AuthChoice::Single, AuthChoice::None, AuthChoice::Accounts];
    let labels: Vec<&str> = options.iter().map(|o| o.label()).collect();
    let pick = dialoguer::Select::with_theme(crate::ui::theme())
        .with_prompt("Who signs in?")
        .items(&labels)
        .default(0)
        .interact()
        .context("Failed to read the answer")?;
    Ok(options[pick])
}

pub(crate) fn default_email() -> String {
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

pub(crate) fn ask_email(asker: Asker) -> Result<String> {
    let default = default_email();
    if !asker.can_ask() {
        return no_prompt(asker, "--admin-email", default);
    }
    dialoguer::Input::with_theme(crate::ui::theme())
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

pub(crate) fn ask_provider(asker: Asker) -> Result<LlmProvider> {
    // A key already in the shell is the obvious default.
    let detected = LlmProvider::ALL.into_iter().find(|p| p.env_key().is_some());
    if !asker.can_ask() {
        return no_prompt(asker, "--provider", detected.unwrap_or(LlmProvider::Skip));
    }
    let labels: Vec<String> = LlmProvider::ALL
        .iter()
        .map(|p| match (p.key_env(), p.env_key()) {
            (Some(var), Some(_)) => format!("{} (use {var} from your shell)", p.label()),
            _ => p.label().to_string(),
        })
        .collect();
    let default = detected
        .and_then(|d| LlmProvider::ALL.iter().position(|p| *p == d))
        .unwrap_or(0);
    let pick = dialoguer::Select::with_theme(crate::ui::theme())
        .with_prompt("LLM provider")
        .items(&labels)
        .default(default)
        .interact()
        .context("Failed to read the answer")?;
    Ok(LlmProvider::ALL[pick])
}

fn set_provider(s: &mut Settings, provider: LlmProvider, asker: Asker) -> Result<()> {
    if let Some(var) = provider.server_env() {
        let key = match provider.env_key() {
            Some(key) => key,
            None if asker.can_ask() => dialoguer::Password::with_theme(crate::ui::theme())
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
pub(crate) fn no_prompt<T>(asker: Asker, flag: &str, default: T) -> Result<T> {
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

/// ` --name <n>` for messages that suggest a command, empty for the default.
fn name_flag(name: &str) -> String {
    if name == "default" {
        String::new()
    } else {
        format!(" --name {name}")
    }
}

fn not_set_up(args: &InstanceArgs) {
    ui::note(format!(
        "No local instance yet. Run {}.",
        ui::cmd(format!("everruns docker up{}", name_flag(&args.name)))
    ));
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

    ui::field("Instance", &inst.name);
    ui::field("URL", format!("http://localhost:{port}"));
    if let Some(public_url) = s.value(settings::PUBLIC_URL) {
        let via = s
            .value(settings::TUNNEL)
            .filter(|t| *t != "none")
            .map(|t| format!(" (through {t})"))
            .unwrap_or_default();
        ui::field("Public", format!("{public_url}{via}"));
    }
    ui::field("Health", health);
    ui::field("Version", s.value(settings::IMAGE_TAG).unwrap_or("unknown"));
    ui::field("Sign-in", auth);
    ui::field("Profile", profile_line);
    ui::field("Settings", inst.dir.display());
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
    ui::ok(format!(
        "Stopped. Data is kept; {} starts it again.",
        ui::cmd("everruns docker up")
    ));
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

/// Decision: an upgrade is the one moment a database migration runs, and
/// migrations only go forward, so the database is dumped first. A dump is
/// small next to the cost of a migration that goes wrong on someone's only
/// copy. An older tag is allowed (after a question) for undoing a bad move,
/// but only together with restoring the backup taken before it.
async fn upgrade(args: UpgradeArgs) -> Result<()> {
    let Some((inst, mut s)) = existing(&args.instance)? else {
        not_set_up(&args.instance);
        return Ok(());
    };
    let docker = checks::program();
    require_docker(&docker, false)?;
    let _lock = inst.lock()?;

    let cli_tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let target = match &args.image_tag {
        Some(tag) => tag.clone(),
        None => resolve_tag(&docker, &cli_tag),
    };
    let current = s.value(settings::IMAGE_TAG).unwrap_or("latest").to_string();
    // `latest` moves under the same name, so it always pulls.
    if target == current && target != "latest" {
        ui::ok(format!("Already on {current}."));
        return Ok(());
    }
    if is_downgrade(&current, &target) {
        let asker = Asker {
            interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
            yes: args.yes,
        };
        let reason = format!(
            "{target} is older than {current}. The database was migrated for {current}, \
             so {target} may fail unless you restore a backup taken before."
        );
        if !confirm(asker, &reason, "--yes")? {
            bail!("not moved; still on {current}");
        }
    }

    if !args.no_backup {
        let file = backup(&docker, &inst, &current)?;
        ui::ok(format!("Backed up the database to {}", file.display()));
    }

    s.set(settings::IMAGE_TAG, &target);
    s.save(&inst.env_path())?;
    // A newer CLI may carry a newer compose file too.
    inst.write_compose()?;

    let pulled = compose(&docker, &inst)
        .arg("pull")
        .status()
        .context("Failed to run docker compose")?;
    if !pulled.success() {
        // Leave the instance on the tag it can run.
        s.set(settings::IMAGE_TAG, &current);
        s.save(&inst.env_path())?;
        bail!("pulling the {target} images failed; still on {current}");
    }
    let status = compose(&docker, &inst)
        .args(["up", "--detach", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;
    if !status.success() {
        bail!("`docker compose up` failed; see the output above");
    }
    let health = wait_healthy(&docker, &inst, port_of(&s)).await?;
    ui::ok(ui::bold(format!(
        "Everruns is now on {}.",
        display_version(&health, &target)
    )));
    Ok(())
}

fn display_version(health: &Health, tag: &str) -> String {
    if health.version.is_empty() {
        tag.to_string()
    } else {
        format!("{} ({tag})", health.version)
    }
}

/// `vX.Y.Z` as numbers; anything else (such as `latest`) is not comparable.
pub(crate) fn release(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub(crate) fn is_downgrade(current: &str, target: &str) -> bool {
    matches!((release(current), release(target)), (Some(c), Some(t)) if t < c)
}

/// Dump the database into the instance's `backups/` folder. Starts PostgreSQL
/// alone if the instance is stopped, so a backup never depends on the server.
fn backup(docker: &str, inst: &Instance, tag: &str) -> Result<std::path::PathBuf> {
    let started = compose(docker, inst)
        .args(["up", "--detach", "--wait", "postgres"])
        .stdout(Stdio::null())
        .status()
        .context("Failed to run docker compose")?;
    if !started.success() {
        bail!("could not start PostgreSQL for the backup; pass --no-backup to skip it");
    }
    let dir = inst.dir.join("backups");
    std::fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("everruns-{tag}-{stamp}.dump"));
    let file = std::fs::File::create(&path)
        .with_context(|| format!("Failed to create {}", path.display()))?;
    // Private before anything is written: the dump holds every record.
    settings::set_private(&path)?;
    let dumped = compose(docker, inst)
        .args([
            "exec",
            "-T",
            "postgres",
            "pg_dump",
            "--username",
            "everruns",
            "--format",
            "custom",
            "everruns",
        ])
        .stdout(file)
        .status()
        .context("Failed to run pg_dump")?;
    if !dumped.success() {
        let _ = std::fs::remove_file(&path);
        bail!("the database backup failed; pass --no-backup to upgrade without one");
    }
    Ok(path)
}

/// Delete everything the instance has: containers, the database volume, the
/// settings folder (with the encryption key and backups) and its CLI profile.
fn destroy(args: &DestroyArgs) -> Result<()> {
    let inst = Instance::new(&args.instance.name)?;
    let docker = checks::program();
    // Without a settings folder there can still be containers and a volume
    // from a folder deleted by hand, so check Docker for the project too.
    let has_project = project_exists(&docker, &inst);
    if !inst.dir.exists() && !has_project {
        ui::note(format!(
            "Nothing to destroy: no local instance '{}'.",
            inst.name
        ));
        return Ok(());
    }
    require_docker(&docker, false)?;

    let asker = Asker {
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        yes: args.yes,
    };
    if !asker.yes {
        if !asker.interactive {
            bail!(
                "no terminal to ask on; pass --yes to delete instance '{}'",
                inst.name
            );
        }
        ui::warn(format!(
            "This deletes the local instance '{}': its containers, database and settings \
             in {}, including the backups.",
            inst.name,
            inst.dir.display()
        ));
        let typed: String = dialoguer::Input::with_theme(crate::ui::theme())
            .with_prompt(format!("Type '{}' to delete it", inst.name))
            .allow_empty(true)
            .interact_text()
            .context("Failed to read the answer")?;
        if typed.trim() != inst.name {
            bail!("not deleted");
        }
    }

    {
        let _lock = if inst.dir.exists() {
            Some(inst.lock()?)
        } else {
            None
        };
        if inst.dir.exists() {
            // `down` needs the compose file; an old or missing one is replaced.
            inst.write_compose()?;
            if !inst.env_path().exists() {
                std::fs::write(inst.env_path(), "")?;
            }
            let down = compose(&docker, &inst)
                .args(["down", "--volumes", "--remove-orphans"])
                .status()
                .context("Failed to run docker compose")?;
            if !down.success() {
                bail!("`docker compose down` failed; nothing else was deleted");
            }
        } else {
            remove_project(&docker, &inst)?;
        }
    }
    if inst.dir.exists() {
        std::fs::remove_dir_all(&inst.dir)
            .with_context(|| format!("Failed to delete {}", inst.dir.display()))?;
    }
    let removed_profile = forget_profile(&inst.profile())?;

    ui::ok(format!("Deleted the local instance '{}'.", inst.name));
    if removed_profile {
        ui::ok(format!("Removed the CLI profile '{}'.", inst.profile()));
    }
    Ok(())
}

/// Whether Docker has containers or volumes of the instance's project.
fn project_exists(docker: &str, inst: &Instance) -> bool {
    let label = format!("label=com.docker.compose.project={}", inst.project());
    [
        vec!["ps", "--all", "--quiet", "--filter", &label],
        vec!["volume", "ls", "--quiet", "--filter", &label],
    ]
    .iter()
    .any(|args| {
        Command::new(docker)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
    })
}

/// Remove a project's containers and volumes when its compose file is gone.
fn remove_project(docker: &str, inst: &Instance) -> Result<()> {
    let label = format!("label=com.docker.compose.project={}", inst.project());
    for (list, remove) in [
        (vec!["ps", "--all", "--quiet"], vec!["rm", "--force"]),
        (
            vec!["volume", "ls", "--quiet"],
            vec!["volume", "rm", "--force"],
        ),
        (vec!["network", "ls", "--quiet"], vec!["network", "rm"]),
    ] {
        let out = Command::new(docker)
            .args(&list)
            .args(["--filter", &label])
            .output()
            .context("Failed to run docker")?;
        let ids: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let status = Command::new(docker)
            .args(&remove)
            .args(&ids)
            .stdout(Stdio::null())
            .status()
            .context("Failed to run docker")?;
        if !status.success() {
            bail!("`docker {}` failed", remove.join(" "));
        }
    }
    Ok(())
}

/// Drop the instance's CLI profile, so nothing points at a server that is gone.
pub(crate) fn forget_profile(profile: &str) -> Result<bool> {
    let mut store = crate::auth::CredentialStore::load().unwrap_or_default();
    if !store.remove_profile(profile) {
        return Ok(false);
    }
    if store.current_profile == profile {
        store.current_profile = String::new();
    }
    store.save()?;
    Ok(true)
}

/// Ask whether to go ahead despite `reason`; without a terminal only `flag`
/// says yes.
pub(crate) fn confirm(asker: Asker, reason: &str, flag: &str) -> Result<bool> {
    if asker.yes {
        return Ok(true);
    }
    if !asker.interactive {
        bail!("{reason} No terminal to ask on; pass {flag} to go ahead");
    }
    dialoguer::Confirm::with_theme(crate::ui::theme())
        .with_prompt(format!("{reason} Continue?"))
        .default(false)
        .interact()
        .context("Failed to read the answer")
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
        set_provider(&mut s, LlmProvider::Skip, quiet).unwrap();
        assert_eq!(s.value(settings::LLM_PROVIDER), Some("skip"));
    }

    #[test]
    fn only_an_older_release_is_a_downgrade() {
        assert!(is_downgrade("v0.49.0", "v0.48.3"));
        assert!(is_downgrade("v1.0.0", "v0.99.99"));
        assert!(!is_downgrade("v0.48.0", "v0.49.0"));
        assert!(!is_downgrade("v0.48.0", "v0.48.0"));
        // `latest` and other names cannot be compared, so they never ask.
        assert!(!is_downgrade("latest", "v0.48.0"));
        assert!(!is_downgrade("v0.48.0", "latest"));
        assert_eq!(release("v0.48.10"), Some((0, 48, 10)));
        assert_eq!(release("main"), None);
    }

    #[test]
    fn suggested_commands_name_only_other_instances() {
        assert_eq!(name_flag("default"), "");
        assert_eq!(name_flag("demo"), " --name demo");
    }

    #[test]
    fn confirm_without_a_terminal_needs_the_flag() {
        let quiet = Asker {
            interactive: false,
            yes: false,
        };
        let err = confirm(quiet, "It is older.", "--yes")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("It is older.") && err.contains("--yes"),
            "{err}"
        );
        let yes = Asker {
            interactive: false,
            yes: true,
        };
        assert!(confirm(yes, "It is older.", "--yes").unwrap());
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
