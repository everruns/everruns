//! `everruns k8s`: run Everruns in a Kubernetes cluster with the Helm chart.
//!
//! Decisions:
//! - Same contract as `everruns docker up`: `up` describes an end state (a
//!   healthy install and a CLI signed in to it), asks only what is not known
//!   yet, and is the repair path when run again. It never moves the chart
//!   version; `upgrade` does.
//! - The release is the record. Helm keeps the values the chart was installed
//!   with and the chart keeps its generated secrets in the cluster, so nothing
//!   secret is written on this machine. The local record
//!   (`everruns/k8s/<name>/instance.json`) only says where the install is.
//! - A cluster is often shared or remote, unlike a laptop's Docker, so the
//!   first `up` and any change of context ask before installing.
//! - Without a public URL, the CLI and browser reach the install through
//!   `kubectl port-forward` on a fixed local port, which `port-forward` keeps
//!   open.

mod cluster;
mod destroy;
mod rollout;
mod values;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::docker::{self, Asker, AuthChoice, LlmProvider, signin};
use crate::ui;
use cluster::{Target, cluster_checks, tool_checks};
use rollout::helm_apply;
use values::{Answers, Database, Known};

const CHART_OCI: &str = "oci://ghcr.io/everruns/charts/everruns";
const DEFAULT_PORT: u16 = 9300;
const DEFAULT_NAMESPACE: &str = "everruns";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Subcommand)]
pub enum K8sCommand {
    /// Install Everruns into the current Kubernetes cluster, or repair it. Safe to run again
    Up(Box<UpArgs>),

    /// Show the install: where it is, its pods, and which CLI profile points at it
    Status(InstanceArgs),

    /// Show the pods' logs
    Logs {
        #[command(flatten)]
        instance: InstanceArgs,

        /// Only this part (server, worker, ui, proxy, nats, postgresql, ...)
        component: Option<String>,

        /// Print what is there and exit instead of following
        #[arg(long)]
        no_follow: bool,
    },

    /// Open the UI in the browser (through a port-forward when there is no public URL)
    Open(InstanceArgs),

    /// Keep a port-forward open so the CLI and browser reach an install without a public URL
    PortForward(InstanceArgs),

    /// Check the tools and the cluster, and run the system doctor on the server
    Doctor(InstanceArgs),

    /// Print the admin password the chart generated
    Password(InstanceArgs),

    /// Move the install to this CLI's chart version (or --chart-version), backing up a bundled database first
    Upgrade(UpgradeArgs),

    /// Remove the workloads. The database, its volumes and the generated secrets are kept
    Uninstall(InstanceArgs),

    /// Delete the install and everything it made: workloads, secrets, volumes, the namespace
    /// when nothing else is in it, and the CLI profile. Asks first
    Destroy(DestroyArgs),
}

#[derive(Args)]
pub struct InstanceArgs {
    /// Install name, also the Helm release name
    #[arg(long, default_value = "everruns")]
    pub name: String,
}

#[derive(Args)]
pub struct UpArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Kubernetes context (default: kubectl's current context)
    #[arg(long)]
    pub context: Option<String>,

    /// Namespace, created when missing (default: everruns)
    #[arg(long)]
    pub namespace: Option<String>,

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

    /// Your PostgreSQL 15+ URL (postgres://user:password@host:5432/db)
    #[arg(long, conflicts_with = "bundled_postgres")]
    pub database_url: Option<String>,

    /// Run a single PostgreSQL pod in the cluster, for trying Everruns out (no backups or failover)
    #[arg(long)]
    pub bundled_postgres: bool,

    /// Public address people use, such as https://everruns.example.com; adds an Ingress
    #[arg(long)]
    pub public_url: Option<String>,

    /// Ingress class for the public address, such as nginx
    #[arg(long)]
    pub ingress_class: Option<String>,

    /// Chart to install: a local path such as charts/everruns, or an OCI reference
    #[arg(long)]
    pub chart: Option<String>,

    /// Chart version for a new install (default: this CLI's version)
    #[arg(long)]
    pub chart_version: Option<String>,

    /// Image tag to run, such as v0.51.0 or latest (default: the chart's release)
    #[arg(long)]
    pub image_tag: Option<String>,

    /// Extra chart value (key=value), as with helm --set; kept on later runs
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,

    /// Extra values file, as with helm --values
    #[arg(long = "values", short = 'f', value_name = "FILE")]
    pub values: Vec<PathBuf>,

    /// Local port for the port-forward when there is no public URL (default 9300)
    #[arg(long)]
    pub port: Option<u16>,

    /// How long to wait for the pods, such as 15m
    #[arg(long, default_value = "15m")]
    pub timeout: String,

    /// Accept the default for every question, the cluster context included
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Do not open the browser
    #[arg(long)]
    pub no_open: bool,
}

#[derive(Args)]
pub struct UpgradeArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Chart version to move to, such as 0.52.0 (default: this CLI's version)
    #[arg(long)]
    pub chart_version: Option<String>,

    /// Skip the backup of a bundled database taken before the upgrade
    #[arg(long)]
    pub no_backup: bool,

    /// How long to wait for the pods, such as 15m
    #[arg(long, default_value = "15m")]
    pub timeout: String,

    /// Do not ask, also when the move is to an older version
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Args)]
pub struct DestroyArgs {
    #[command(flatten)]
    pub instance: InstanceArgs,

    /// Keep the namespace even when nothing else is left in it
    #[arg(long)]
    pub keep_namespace: bool,

    /// Do not ask for the install name (for scripts)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub async fn run(command: K8sCommand) -> Result<()> {
    match command {
        K8sCommand::Up(args) => Box::pin(up(*args)).await,
        K8sCommand::Status(args) => Box::pin(status(&args)).await,
        K8sCommand::Logs {
            instance,
            component,
            no_follow,
        } => logs(&instance, component.as_deref(), no_follow),
        K8sCommand::Open(args) => open(&args).await,
        K8sCommand::PortForward(args) => port_forward(&args).await,
        K8sCommand::Doctor(args) => doctor(&args),
        K8sCommand::Password(args) => password(&args),
        K8sCommand::Upgrade(args) => Box::pin(upgrade(args)).await,
        K8sCommand::Uninstall(args) => uninstall(&args),
        K8sCommand::Destroy(args) => destroy::destroy(&args),
    }
}

/// Where an install is, kept on this machine. Nothing secret.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Record {
    context: String,
    namespace: String,
    release: String,
    /// What `up` installs from: the OCI chart or a local path.
    chart: String,
    port: u16,
    /// The answers that are not secret, so a reinstall after `uninstall`
    /// asks nothing again. An external database URL is secret and is asked.
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    admin_email: Option<String>,
    #[serde(default)]
    bundled_postgres: bool,
}

struct Instance {
    name: String,
    dir: PathBuf,
}

impl Instance {
    fn new(name: &str) -> Result<Self> {
        let valid = !name.is_empty()
            && name.len() <= 40
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-');
        if !valid {
            bail!("install name must be up to 40 lowercase letters, digits and '-', got '{name}'");
        }
        let base = crate::user_dirs::data_dir()
            .ok_or_else(|| anyhow!("Could not determine the data directory"))?;
        Ok(Self {
            name: name.to_string(),
            dir: base.join("everruns").join("k8s").join(name),
        })
    }

    fn path(&self) -> PathBuf {
        self.dir.join("instance.json")
    }

    fn load(&self) -> Result<Option<Record>> {
        match std::fs::read_to_string(self.path()) {
            Ok(text) => Ok(Some(serde_json::from_str(&text).with_context(|| {
                format!("Failed to read {}", self.path().display())
            })?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("Failed to read {}", self.path().display())),
        }
    }

    fn save(&self, record: &Record) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("Failed to create {}", self.dir.display()))?;
        std::fs::write(self.path(), serde_json::to_string_pretty(record)?)
            .with_context(|| format!("Failed to write {}", self.path().display()))
    }

    fn profile(&self) -> String {
        if self.name == "everruns" {
            "k8s".to_string()
        } else {
            format!("k8s-{}", self.name)
        }
    }

    fn name_flag(&self) -> String {
        if self.name == "everruns" {
            String::new()
        } else {
            format!(" --name {}", self.name)
        }
    }
}

impl Record {
    fn target(&self) -> Target {
        Target {
            context: self.context.clone(),
            namespace: self.namespace.clone(),
            release: self.release.clone(),
        }
    }
}

fn asker(yes: bool) -> Asker {
    Asker {
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        yes,
    }
}

fn require(checks: &[cluster::Check], verbose: bool, what: &str) -> Result<()> {
    let failed = checks.iter().any(cluster::Check::failed);
    let shown =
        |c: &cluster::Check| verbose || failed || !matches!(c.outcome, cluster::Outcome::Ok(_));
    if checks.iter().any(shown) {
        ui::section(&format!("{what} checks"));
    }
    for check in checks {
        if verbose || failed || !matches!(check.outcome, cluster::Outcome::Ok(_)) {
            check.print();
        }
    }
    if failed {
        bail!("{what} is not ready; fix the problem above and run this again");
    }
    Ok(())
}

async fn up(args: UpArgs) -> Result<()> {
    require(&tool_checks(), false, "This machine")?;
    let inst = Instance::new(&args.instance.name)?;
    let asker = asker(args.yes);
    let saved = inst.load()?;

    // Where to install. Asking is the guard against the wrong cluster: the
    // first run, and any run that would install somewhere new, confirms.
    let context = match (&args.context, &saved) {
        (Some(c), _) => c.clone(),
        (None, Some(r)) => r.context.clone(),
        (None, None) => cluster::current_context()?,
    };
    let namespace = args
        .namespace
        .clone()
        .or_else(|| saved.as_ref().map(|r| r.namespace.clone()))
        .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());
    let moved = saved
        .as_ref()
        .is_none_or(|r| r.context != context || r.namespace != namespace);
    ui::banner(
        "k8s up",
        &[
            ("cluster", &context),
            ("namespace", &namespace),
            ("release", &inst.name),
        ],
    );
    if moved {
        let reason = format!(
            "Install Everruns '{}' into cluster context '{context}', namespace '{namespace}'?",
            inst.name
        );
        if !ask_yes(asker, &reason, "--yes")? {
            bail!("nothing installed; pick another cluster with --context and --namespace");
        }
    }
    let target = Target {
        context,
        namespace,
        release: inst.name.clone(),
    };

    let current = target.release_values()?;
    let known_value = current.clone().unwrap_or(Value::Null);
    let known = Known(&known_value);
    let first_install = current.is_none();
    if first_install {
        ui::section("Setup");
    }

    // Questions, each only when the release does not answer it already.
    let mut answers = Answers::default();
    let remembered = saved.as_ref();
    let auth = match args.auth {
        Some(a) => a,
        None => match known.auth().or_else(|| {
            remembered
                .and_then(|r| r.auth.as_deref())
                .and_then(AuthChoice::from_server_mode)
        }) {
            Some(a) => a,
            None => docker::ask_auth(asker)?,
        },
    };
    if known.auth() != Some(auth) {
        answers.auth = Some(auth);
    }
    let email = match &args.admin_email {
        Some(e) => e.clone(),
        None => match known
            .admin_email()
            .or_else(|| remembered.and_then(|r| r.admin_email.as_deref()))
        {
            Some(e) => e.to_string(),
            None if auth == AuthChoice::None => docker::default_email(),
            None => docker::ask_email(asker)?,
        },
    };
    if known.admin_email() != Some(email.as_str()) {
        answers.admin_email = Some(email.clone());
    }
    answers.database = if args.bundled_postgres {
        Some(Database::Bundled)
    } else if let Some(url) = &args.database_url {
        Some(Database::Url(url.clone()))
    } else if known.has_database() {
        None
    } else if remembered.is_some_and(|r| r.bundled_postgres) {
        Some(Database::Bundled)
    } else {
        Some(ask_database(asker)?)
    };
    if let Some(url) = &args.public_url {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            bail!("--public-url must start with http:// or https://, got '{url}'");
        }
        answers.public_url = Some(url.clone());
    } else if first_install && args.set.is_empty() && args.values.is_empty() {
        answers.public_url = ask_public_url(asker)?;
    }
    answers.ingress_class = args.ingress_class.clone();
    if auth.shares_default_keys() {
        let provider = match args.provider {
            Some(p) => Some(p),
            None if first_install => Some(docker::ask_provider(asker)?),
            None => None,
        };
        if let Some(provider) = provider {
            answers.provider_key = provider_key(provider, asker)?;
        }
    } else if args.provider.is_some_and(|p| p != LlmProvider::Skip) {
        bail!("with --auth accounts, add provider keys in the UI; drop --provider");
    }
    answers.image_tag = args.image_tag.clone();

    let values = values::build(&known_value, &answers);
    let bundled = Known(&values).bundled_postgres();
    let autoscaling = Known(&values).autoscaling()
        || args
            .set
            .iter()
            .any(|s| s.replace(' ', "") == "worker.autoscaling.enabled=true");
    require(
        &cluster_checks(&target, bundled, autoscaling),
        first_install,
        "Cluster",
    )?;

    // The chart. A rerun keeps the deployed version; only `upgrade` moves it.
    let chart = match &args.chart {
        // A local chart is kept as an absolute path, so later runs from
        // another directory find it.
        Some(c) if !is_oci(c) => std::fs::canonicalize(c)
            .with_context(|| format!("chart {c} not found"))?
            .to_string_lossy()
            .to_string(),
        Some(c) => c.clone(),
        None => saved
            .as_ref()
            .map_or_else(|| CHART_OCI.to_string(), |r| r.chart.clone()),
    };
    let version = if is_oci(&chart) {
        let version = match (&args.chart_version, target.deployed_chart_version()) {
            (Some(v), _) => v.clone(),
            (None, Some(deployed)) if !first_install => deployed,
            _ => cli_version(),
        };
        require_chart(&chart, &version)?;
        Some(version)
    } else {
        None
    };

    let port = match (args.port, &saved) {
        (Some(p), _) => p,
        (None, Some(r)) => r.port,
        (None, None) => DEFAULT_PORT,
    };
    let record = Record {
        context: target.context.clone(),
        namespace: target.namespace.clone(),
        release: target.release.clone(),
        chart: chart.clone(),
        port,
        auth: Some(auth.server_mode().to_string()),
        admin_email: Some(email.clone()),
        bundled_postgres: bundled,
    };
    inst.save(&record)?;

    ui::section(if first_install {
        "Installing"
    } else {
        "Updating"
    });
    helm_apply(
        &target,
        &chart,
        version.as_deref(),
        &values,
        &args.set,
        &args.values,
        &args.timeout,
    )?;

    // Sign the CLI in, through the public address when it answers, else a
    // port-forward that lives for the sign-in.
    let public = Known(&values).public_url().map(str::to_string);
    let (url, _forward) = reach(&target, public.as_deref(), port).await?;
    let api_url = format!("{url}/api");
    let health = health(&url).await?;
    let password = if auth == AuthChoice::None {
        String::new()
    } else {
        target.secret_value(&secret_name(&target, &values), "AUTH_ADMIN_PASSWORD")?
    };
    let hint = format!(
        "store the new one as AUTH_ADMIN_PASSWORD in Secret {} (or set auth.adminPassword)",
        secret_name(&target, &values)
    );
    let creds = signin::Credentials {
        auth,
        email: &email,
        password: &password,
        password_hint: &hint,
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
            return Err(e.context("Everruns is up, but signing the CLI in failed"));
        }
    };

    let ui_url = public.clone().unwrap_or_else(|| local_url(port));
    let mut rows = vec![("UI", ui::cmd(&ui_url).to_string())];
    let again = ui::cmd(format!("everruns k8s password{}", inst.name_flag()));
    match auth {
        AuthChoice::None => {}
        // A single admin login is the only way in, so show the password itself.
        AuthChoice::Single => {
            rows.push(("Sign in", email.clone()));
            rows.push((
                "Password",
                format!(
                    "{}  {}",
                    ui::bold(&password),
                    ui::dim(format!("(again: {again})"))
                ),
            ));
        }
        AuthChoice::Accounts => {
            rows.push(("Sign in", email.clone()));
            rows.push(("Password", again.to_string()));
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
    ui::summary(
        &format!("Everruns {health} is running"),
        &format!("in {}/{}", target.context, target.namespace),
        &rows,
    );
    if public.is_none() {
        ui::warn(format!(
            "No public URL: open it with {}, or rerun with {} for an Ingress.",
            ui::cmd(format!("everruns k8s open{}", inst.name_flag())),
            ui::cmd("--public-url")
        ));
    }
    if bundled {
        ui::warn(format!(
            "The bundled PostgreSQL has no backups or failover; use {} for anything you keep.",
            ui::cmd("--database-url")
        ));
    }
    if first_install && values["secrets"]["existingSecret"].as_str().is_none() {
        ui::warn(format!(
            "Back up Secret {} with the database: it holds the key that encrypts stored provider keys and secrets.",
            ui::bold(secret_name(&target, &values))
        ));
    }
    if let Some(v) = &version
        && *v != cli_version()
    {
        ui::warn(format!(
            "This install runs chart {v}; this CLI is {}. Move to it with {}.",
            cli_version(),
            ui::cmd(format!("everruns k8s upgrade{}", inst.name_flag()))
        ));
    }

    if first_install
        && asker.interactive
        && !args.no_open
        && let Some(url) = &public
    {
        let _ = crate::browser::open(url);
    }
    Ok(())
}

/// `everruns k8s password`: the generated admin password, for the UI.
fn password(args: &InstanceArgs) -> Result<()> {
    let Some((_inst, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let values = target
        .release_values()?
        .context("not installed; `everruns k8s up` installs it")?;
    if Known(&values).auth() == Some(AuthChoice::None) {
        ui::note("This install has no sign-in; nothing to show.");
        return Ok(());
    }
    let password = target.secret_value(&secret_name(&target, &values), "AUTH_ADMIN_PASSWORD")?;
    println!("{password}");
    Ok(())
}

fn cli_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn is_oci(chart: &str) -> bool {
    chart.starts_with("oci://")
}

fn local_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

fn secret_name(target: &Target, values: &Value) -> String {
    values["secrets"]["existingSecret"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map_or_else(
            || format!("{}-generated", target.fullname()),
            str::to_string,
        )
}

/// The OCI chart must exist at `version`; a CLI built from an unreleased
/// commit has none, and a checkout's chart is the way in then.
fn require_chart(chart: &str, version: &str) -> Result<()> {
    let ok = std::process::Command::new(cluster::helm_program())
        .args(["show", "chart", chart, "--version", version])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        bail!(
            "chart {chart} {version} is not published (or not reachable). From a checkout of \
             everruns/everruns, pass --chart charts/everruns; or pick a published version with --chart-version"
        );
    }
    Ok(())
}

/// A URL that reaches the install: the public one when it answers, else a
/// port-forward kept alive by the returned guard.
async fn reach(
    target: &Target,
    public: Option<&str>,
    port: u16,
) -> Result<(String, Option<cluster::PortForward>)> {
    if let Some(url) = public {
        if health(url).await.is_ok() {
            return Ok((url.trim_end_matches('/').to_string(), None));
        }
        ui::warn(format!(
            "{url} does not answer yet (DNS or Ingress); signing in through a port-forward."
        ));
    }
    if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
        // Most likely an `everruns k8s port-forward` already running.
        if health(&local_url(port)).await.is_ok() {
            return Ok((local_url(port), None));
        }
        bail!("local port {port} is in use; pick another with --port");
    }
    let forward = target.port_forward(port)?;
    let started = Instant::now();
    while started.elapsed() < HEALTH_TIMEOUT {
        if health(&local_url(port)).await.is_ok() {
            return Ok((local_url(port), Some(forward)));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!(
        "the install did not answer through a port-forward on {port} within {}s; see `everruns k8s status`",
        HEALTH_TIMEOUT.as_secs()
    )
}

/// The server's version from `/health` through the proxy.
async fn health(url: &str) -> Result<String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let resp = http
        .get(format!("{}/health", url.trim_end_matches('/')))
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("health answered {}", resp.status());
    }
    let body: Value = resp.json().await.unwrap_or_default();
    Ok(body["version"].as_str().unwrap_or("").to_string())
}

fn ask_yes(asker: Asker, question: &str, flag: &str) -> Result<bool> {
    if asker.yes {
        ui::ok(format!("{question} {}", ui::dim(format!("yes ({flag})"))));
        return Ok(true);
    }
    if !asker.interactive {
        bail!("{question} No terminal to ask on; pass {flag} to go ahead");
    }
    dialoguer::Confirm::with_theme(crate::ui::theme())
        .with_prompt(question)
        .default(true)
        .interact()
        .context("Failed to read the answer")
}

fn ask_database(asker: Asker) -> Result<Database> {
    if !asker.can_ask() {
        if asker.yes {
            bail!(
                "no database chosen; pass --database-url <postgres://...>, or --bundled-postgres to try it out"
            );
        }
        bail!("no terminal to ask on; pass --database-url or --bundled-postgres");
    }
    let options = [
        "My own PostgreSQL 15+ (a managed database or an operator such as CloudNativePG)",
        "A single PostgreSQL pod in the cluster, for trying it out (no backups or failover)",
    ];
    let pick = dialoguer::Select::with_theme(crate::ui::theme())
        .with_prompt("Database")
        .items(options)
        .default(0)
        .interact()
        .context("Failed to read the answer")?;
    if pick == 1 {
        return Ok(Database::Bundled);
    }
    let url: String = dialoguer::Password::with_theme(crate::ui::theme())
        .with_prompt("PostgreSQL URL (postgres://user:password@host:5432/db)")
        .interact()
        .context("Failed to read the URL")?;
    let url = url.trim().to_string();
    if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
        bail!("that is not a postgres:// URL");
    }
    Ok(Database::Url(url))
}

fn ask_public_url(asker: Asker) -> Result<Option<String>> {
    if !asker.can_ask() {
        return Ok(None);
    }
    let url: String = dialoguer::Input::with_theme(crate::ui::theme())
        .with_prompt("Public URL with an Ingress, such as https://everruns.example.com (empty: reach it with a port-forward)")
        .allow_empty(true)
        .interact_text()
        .context("Failed to read the answer")?;
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(None);
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("the public URL must start with http:// or https://");
    }
    Ok(Some(url))
}

/// The provider's key under the chart's `providerKeys`.
fn chart_provider_key(provider: LlmProvider) -> Option<&'static str> {
    match provider {
        LlmProvider::Openai => Some("openai"),
        LlmProvider::Anthropic => Some("anthropic"),
        LlmProvider::Gemini => Some("gemini"),
        LlmProvider::Openrouter => Some("openrouter"),
        LlmProvider::Skip => None,
    }
}

/// The chart's `providerKeys` entry for the provider, from the shell or a prompt.
fn provider_key(provider: LlmProvider, asker: Asker) -> Result<Option<(&'static str, String)>> {
    let Some(chart_key) = chart_provider_key(provider) else {
        return Ok(None);
    };
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
    Ok(Some((chart_key, key.trim().to_string())))
}

/// An install this machine set up, or a friendly "nothing here yet".
fn existing(args: &InstanceArgs) -> Result<Option<(Instance, Record)>> {
    let inst = Instance::new(&args.name)?;
    match inst.load()? {
        Some(record) => Ok(Some((inst, record))),
        None => {
            let flag = if args.name == "everruns" {
                String::new()
            } else {
                format!(" --name {}", args.name)
            };
            ui::note(format!(
                "No Kubernetes install set up from this machine. Install one with {}.",
                ui::cmd(format!("everruns k8s up{flag}"))
            ));
            Ok(None)
        }
    }
}

async fn status(args: &InstanceArgs) -> Result<()> {
    let Some((inst, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let values = target.release_values()?;
    ui::banner(
        "k8s status",
        &[
            ("cluster", &record.context),
            ("namespace", &record.namespace),
            ("release", &inst.name),
        ],
    );
    let Some(values) = values else {
        ui::warn(format!(
            "Not installed (uninstalled, or the namespace is gone). {} installs it again.",
            ui::cmd(format!("everruns k8s up{}", inst.name_flag()))
        ));
        return Ok(());
    };
    if let Some(v) = target.deployed_chart_version() {
        ui::stderr_field("Chart", v);
    }
    let public = Known(&values).public_url().map(str::to_string);
    match &public {
        Some(url) => match health(url).await {
            Ok(v) => ui::stderr_field(
                "URL",
                format!("{} {}", ui::cmd(url), ui::good(format!("healthy ({v})"))),
            ),
            Err(e) => ui::stderr_field(
                "URL",
                format!(
                    "{} {}",
                    ui::cmd(url),
                    ui::bad(format!("not answering ({e})"))
                ),
            ),
        },
        None => ui::stderr_field(
            "URL",
            format!(
                "{} {}",
                ui::cmd(local_url(record.port)),
                ui::dim(format!(
                    "through `everruns k8s port-forward{}`",
                    inst.name_flag()
                ))
            ),
        ),
    }
    let store = crate::auth::CredentialStore::load().unwrap_or_default();
    if store.profiles.contains_key(&inst.profile()) {
        let current = if store.current_profile == inst.profile() {
            " (current)"
        } else {
            ""
        };
        ui::stderr_field("Profile", format!("{}{}", inst.profile(), ui::dim(current)));
    }
    ui::section("Pods");
    let status = target
        .kubectl()
        .args([
            "get",
            "pods",
            "--selector",
            &target.selector(None),
            "--output",
            "wide",
        ])
        .status()
        .context("Failed to run kubectl")?;
    if !status.success() {
        bail!("`kubectl get pods` failed");
    }
    Ok(())
}

fn logs(args: &InstanceArgs, component: Option<&str>, no_follow: bool) -> Result<()> {
    let Some((_, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let mut cmd = target.kubectl();
    cmd.args([
        "logs",
        "--selector",
        &target.selector(component),
        "--prefix",
        "--all-containers",
        "--max-log-requests",
        "20",
        "--tail",
        "200",
    ]);
    if !no_follow {
        cmd.arg("--follow");
    }
    let status = cmd.status().context("Failed to run kubectl")?;
    if !status.success() {
        bail!("`kubectl logs` failed");
    }
    Ok(())
}

/// Keep `kubectl port-forward` open in the foreground until Ctrl+C.
///
/// Decision: the CLI waits on kubectl and stops it on SIGINT or SIGTERM. A
/// terminal's Ctrl+C reaches both anyway, but a script or service manager that
/// stops only this process would otherwise leave kubectl holding the port,
/// forwarding to a pod that may be gone.
async fn forward_foreground(target: &Target, port: u16) -> Result<()> {
    ui::ok(format!(
        "Everruns at {} {}",
        ui::cmd(local_url(port)),
        ui::dim("(Ctrl+C to stop the port-forward)")
    ));
    let mut cmd = tokio::process::Command::from(target.kubectl());
    cmd.args([
        "port-forward",
        &format!("svc/{}", target.fullname()),
        &format!("{port}:9300"),
    ])
    .stdout(Stdio::null())
    .kill_on_drop(true);
    let mut child = cmd.spawn().context("Failed to run kubectl port-forward")?;
    let status = tokio::select! {
        status = child.wait() => status.context("Failed to run kubectl port-forward")?,
        () = stop_signal() => {
            let _ = child.kill().await;
            return Ok(());
        }
    };
    if !status.success() {
        bail!("`kubectl port-forward` stopped; is local port {port} free and the install running?");
    }
    Ok(())
}

/// Resolves on Ctrl+C, or on SIGTERM where there is one.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn port_forward(args: &InstanceArgs) -> Result<()> {
    let Some((_, record)) = existing(args)? else {
        return Ok(());
    };
    forward_foreground(&record.target(), record.port).await
}

async fn open(args: &InstanceArgs) -> Result<()> {
    let Some((_, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let values = target.release_values()?.unwrap_or(Value::Null);
    if let Some(url) = Known(&values).public_url() {
        if crate::browser::open(url).is_err() {
            ui::note(format!("Open {} in your browser.", ui::cmd(url)));
        }
        return Ok(());
    }
    let url = local_url(record.port);
    // The browser waits for the port-forward to accept; kubectl starts fast.
    let opener = url.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        if crate::browser::open(&opener).is_err() {
            ui::note(format!("Open {} in your browser.", ui::cmd(&opener)));
        }
    });
    forward_foreground(&target, record.port).await
}

fn doctor(args: &InstanceArgs) -> Result<()> {
    ui::section("Tools");
    let tools = tool_checks();
    for c in &tools {
        c.print();
    }
    if tools.iter().any(cluster::Check::failed) {
        bail!("install the missing tools and run this again");
    }
    let Some((_, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let values = target.release_values()?.unwrap_or(Value::Null);
    let known = Known(&values);
    let checks = cluster_checks(&target, known.bundled_postgres(), known.autoscaling());
    ui::section("Cluster");
    for c in &checks {
        c.print();
    }
    if checks.iter().any(cluster::Check::failed) {
        bail!("the cluster is not ready; fix the problems above");
    }
    system_doctor(&target)
}

/// The server's own doctor, run inside its pod so it sees what the server sees.
fn system_doctor(target: &Target) -> Result<()> {
    ui::section("Server");
    let output = target
        .kubectl()
        .args([
            "exec",
            &format!("deploy/{}-server", target.fullname()),
            "--container",
            "server",
            "--",
            "/app/everruns-server",
            "--doctor",
        ])
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run kubectl exec")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => bail!("the system doctor found problems; see the fixes above"),
        _ => {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            bail!("the system doctor could not run in the server pod")
        }
    }
}

/// Decision: as with Docker, an upgrade is when migrations run and they only
/// go forward. A bundled database is dumped to this machine first; an
/// external one is the person's to back up (managed databases snapshot).
async fn upgrade(args: UpgradeArgs) -> Result<()> {
    let Some((inst, record)) = existing(&args.instance)? else {
        return Ok(());
    };
    require(&tool_checks(), false, "This machine")?;
    let target = record.target();
    let Some(values) = target.release_values()? else {
        bail!(
            "'{}' is not installed; `everruns k8s up{}` installs it",
            inst.name,
            inst.name_flag()
        );
    };
    if !is_oci(&record.chart) {
        bail!(
            "this install runs the chart from {}; run `everruns k8s up{}` from the newer checkout instead",
            record.chart,
            inst.name_flag()
        );
    }
    let target_version = args.chart_version.clone().unwrap_or_else(cli_version);
    let current = target.deployed_chart_version().unwrap_or_default();
    if current == target_version {
        ui::ok(format!("Already on chart {current}."));
        return Ok(());
    }
    let asker = asker(args.yes);
    if docker::is_downgrade(&current, &target_version) {
        let reason = format!(
            "{target_version} is older than {current}. The database was migrated for {current}, \
             so {target_version} may fail unless you restore a backup taken before."
        );
        if !docker::confirm(asker, &reason, "--yes")? {
            bail!("not moved; still on {current}");
        }
    }
    require_chart(&record.chart, &target_version)?;
    let known = Known(&values);
    if known.bundled_postgres() && !args.no_backup {
        let file = backup(&inst, &target, &current)?;
        ui::ok(format!("Backed up the database to {}", file.display()));
    } else if !known.bundled_postgres() {
        ui::warn("Back up your database before this if you have not: migrations only go forward.");
    }
    ui::section(&format!(
        "Moving {} from chart {current} to {target_version}",
        inst.name
    ));
    helm_apply(
        &target,
        &record.chart,
        Some(&target_version),
        &values,
        &[],
        &[],
        &args.timeout,
    )?;
    ui::ok(ui::bold(format!(
        "Everruns is now on chart {target_version}."
    )));
    Ok(())
}

/// Dump the bundled database into the install's `backups/` folder here.
fn backup(inst: &Instance, target: &Target, version: &str) -> Result<PathBuf> {
    let dir = inst.dir.join("backups");
    std::fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("everruns-{version}-{stamp}.dump"));
    let file = std::fs::File::create(&path)
        .with_context(|| format!("Failed to create {}", path.display()))?;
    // Private before anything is written: the dump holds every record.
    docker::settings::set_private(&path)?;
    let dumped = target
        .kubectl()
        .args([
            "exec",
            &format!("statefulset/{}-postgresql", target.fullname()),
            "--",
            "pg_dump",
            "--username",
            "everruns",
            "--format",
            "custom",
            "everruns",
        ])
        .stdout(file)
        .status()
        .context("Failed to run pg_dump in the PostgreSQL pod")?;
    if !dumped.success() {
        let _ = std::fs::remove_file(&path);
        bail!("the database backup failed; pass --no-backup to upgrade without one");
    }
    Ok(path)
}

fn uninstall(args: &InstanceArgs) -> Result<()> {
    let Some((inst, record)) = existing(args)? else {
        return Ok(());
    };
    let target = record.target();
    let status = target
        .helm()
        .args(["uninstall", &target.release, "--wait"])
        .status()
        .context("Failed to run helm")?;
    if !status.success() {
        bail!("`helm uninstall` failed");
    }
    ui::ok("Removed the workloads.");
    ui::detail(format!(
        "Kept: the generated secrets ({0}-generated, {0}-internal) and the database volume, so {1} \
         brings it back as it was. {2} deletes them.",
        target.fullname(),
        ui::cmd(format!("everruns k8s up{}", inst.name_flag())),
        ui::cmd(format!("everruns k8s destroy{}", inst.name_flag()))
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: K8sCommand,
    }

    #[test]
    fn up_parses_every_flag() {
        let cli = Cli::try_parse_from([
            "k8s",
            "up",
            "--name",
            "demo",
            "--context",
            "kind-kind",
            "--namespace",
            "ns",
            "--auth",
            "single",
            "--admin-email",
            "a@b.c",
            "--bundled-postgres",
            "--public-url",
            "https://e.example.com",
            "--chart",
            "charts/everruns",
            "--set",
            "server.replicas=2",
            "--set",
            "valkey.enabled=true",
            "--yes",
        ])
        .unwrap();
        let K8sCommand::Up(args) = cli.command else {
            panic!("not up")
        };
        assert_eq!(args.instance.name, "demo");
        assert!(args.bundled_postgres);
        assert_eq!(args.set.len(), 2);
        assert_eq!(args.timeout, "15m");
    }

    #[test]
    fn a_database_url_and_the_bundled_one_conflict() {
        assert!(
            Cli::try_parse_from([
                "k8s",
                "up",
                "--bundled-postgres",
                "--database-url",
                "postgres://x"
            ])
            .is_err()
        );
    }

    #[test]
    fn install_names_are_release_names() {
        assert!(Instance::new("everruns").is_ok());
        assert!(Instance::new("team-a").is_ok());
        assert!(Instance::new("Team").is_err());
        assert!(Instance::new("-a").is_err());
        assert!(Instance::new("").is_err());
    }

    #[test]
    fn profiles_are_named_after_the_install() {
        assert_eq!(Instance::new("everruns").unwrap().profile(), "k8s");
        assert_eq!(Instance::new("team-a").unwrap().profile(), "k8s-team-a");
    }

    #[test]
    fn the_generated_secret_unless_one_is_managed() {
        let t = Target {
            context: "c".into(),
            namespace: "n".into(),
            release: "everruns".into(),
        };
        assert_eq!(secret_name(&t, &Value::Null), "everruns-generated");
        let v = serde_json::json!({ "secrets": { "existingSecret": "mine" } });
        assert_eq!(secret_name(&t, &v), "mine");
    }

    /// Every provider `up` offers lands in a chart value the chart turns into
    /// the server's `DEFAULT_*_API_KEY`, and the server pod reads that key.
    #[test]
    fn every_offered_provider_reaches_the_server() {
        let chart = concat!(env!("CARGO_MANIFEST_DIR"), "/../../charts/everruns");
        let read = |f: &str| std::fs::read_to_string(format!("{chart}/{f}")).unwrap();
        let values: serde_yaml::Value = serde_yaml::from_str(&read("values.yaml")).unwrap();
        let secrets = read("templates/secrets.yaml");
        let server = read("templates/server.yaml");
        for provider in LlmProvider::ALL {
            let Some(key) = chart_provider_key(provider) else {
                assert_eq!(provider, LlmProvider::Skip);
                continue;
            };
            let env = provider.server_env().unwrap();
            assert!(
                values["providerKeys"].get(key).is_some(),
                "values.yaml has no providerKeys.{key}"
            );
            assert!(
                secrets.contains(&format!("{env}: {{{{ .{key} | quote }}}}")),
                "secrets.yaml does not map providerKeys.{key} to {env}"
            );
            assert!(
                server.contains(&format!("\"{env}\"")),
                "server.yaml does not read {env}"
            );
        }
    }
}
