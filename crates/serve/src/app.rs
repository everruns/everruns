//! `App::builder().discover().build()`: collect what the macros registered,
//! check it against the layout conventions, and freeze it.
//!
//! Decision: discovery never fails early. Every problem in the app (duplicate
//! names, bad cron, unknown tool filters) is collected and reported together
//! by `serve::start`, the way a compiler reports all errors in a crate.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;

use crate::agent::Agent;
use crate::channel::Channel;
use crate::config::AppConfig;
use crate::registry::{
    AgentRegistration, AppInfoRegistration, AssetRegistration, ChannelRegistration,
    ConnectionRegistration, ConnectionValue, EvalRegistration, ScheduleRegistration,
    ToolRegistration,
};

/// How the binary is running. Decides hot reload and model fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `cargo run -- dev`: SQLite under `.serve/`, simulator fallback,
    /// Markdown hot reload, a live console.
    Dev,
    /// `start`: production. Every model must route; missing secrets fail.
    Start,
    /// `eval`: in-process, simulator fallback, throwaway storage.
    Eval,
}

impl Mode {
    pub(crate) fn hot_reload(self) -> bool {
        self == Mode::Dev
    }

    pub(crate) fn allow_offline(self) -> bool {
        self != Mode::Start
    }
}

/// A discovered agent.
#[derive(Clone)]
pub(crate) struct AgentEntry {
    pub name: &'static str,
    pub doc: &'static str,
    pub sub: bool,
    pub default: bool,
    pub source: &'static str,
    pub spec: Agent,
}

/// A discovered channel.
#[derive(Clone)]
pub(crate) struct ChannelEntry {
    pub name: &'static str,
    pub source: &'static str,
    pub channel: Arc<dyn Channel>,
}

/// A discovered schedule with its parsed cron.
#[derive(Clone)]
pub(crate) struct ScheduleEntry {
    pub registration: &'static ScheduleRegistration,
    pub schedule: cron::Schedule,
}

/// A discovered connection.
#[derive(Clone)]
pub(crate) struct ConnectionEntry {
    pub source: &'static str,
    pub value: ConnectionValue,
}

/// A skill from `agent/skills/<name>/SKILL.md`.
#[derive(Clone, Debug)]
pub(crate) struct Skill {
    pub name: String,
    pub description: String,
    pub asset: &'static AssetRegistration,
}

pub(crate) struct AppInner {
    pub name: String,
    pub version: String,
    pub config: AppConfig,
    pub agents: Vec<AgentEntry>,
    pub tools: Vec<&'static ToolRegistration>,
    pub channels: Vec<ChannelEntry>,
    pub schedules: Vec<ScheduleEntry>,
    pub connections: Vec<ConnectionEntry>,
    pub evals: Vec<&'static EvalRegistration>,
    pub assets: Vec<&'static AssetRegistration>,
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// A discovered, validated app. Cheap to clone.
#[derive(Clone)]
pub struct App {
    pub(crate) inner: Arc<AppInner>,
}

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Problems that stop the app from starting.
    pub fn errors(&self) -> &[String] {
        &self.inner.errors
    }

    /// Convention drift worth fixing but not fatal.
    pub fn warnings(&self) -> &[String] {
        &self.inner.warnings
    }

    /// The build-time contract with the host.
    pub fn manifest(&self) -> crate::Manifest {
        crate::Manifest::of(self)
    }

    pub(crate) fn agent(&self, name: &str) -> Option<&AgentEntry> {
        self.inner.agents.iter().find(|agent| agent.name == name)
    }

    /// The agent `POST /v1/sessions` uses when the body names none.
    pub(crate) fn default_agent(&self) -> Option<&AgentEntry> {
        let top: Vec<_> = self.inner.agents.iter().filter(|a| !a.sub).collect();
        top.iter()
            .find(|agent| agent.default)
            .or_else(|| (top.len() == 1).then(|| &top[0]))
            .copied()
    }

    pub(crate) fn channel(&self, name: &str) -> Option<&ChannelEntry> {
        self.inner.channels.iter().find(|entry| entry.name == name)
    }

    pub(crate) fn asset(&self, path: &str) -> Option<&'static AssetRegistration> {
        self.inner
            .assets
            .iter()
            .copied()
            .find(|asset| asset.path == path)
    }

    /// Tools an agent gets: its filter, or every discovered tool.
    pub(crate) fn tools_for(&self, agent: &AgentEntry) -> Vec<&'static ToolRegistration> {
        match &agent.spec.tools {
            Some(names) => self
                .inner
                .tools
                .iter()
                .copied()
                .filter(|tool| names.iter().any(|name| name == tool.name))
                .collect(),
            None => self.inner.tools.clone(),
        }
    }
}

/// Builder for [`App`].
#[derive(Default)]
#[must_use]
pub struct AppBuilder {
    discover: bool,
    config: Option<AppConfig>,
}

impl AppBuilder {
    /// Collect everything the attribute macros registered in this binary.
    pub fn discover(mut self) -> Self {
        self.discover = true;
        self
    }

    /// Use this configuration instead of the embedded `serve.toml`.
    pub fn config(mut self, config: AppConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Freeze the app. Problems are recorded, not returned: `serve::start`
    /// prints them all and exits. Use [`try_build`](Self::try_build) to get
    /// them as an error instead.
    pub fn build(self) -> App {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        if !self.discover {
            warnings.push("App built without .discover(); it has no agents".to_string());
        }
        let assets: Vec<&'static AssetRegistration> = if self.discover {
            inventory::iter::<AssetRegistration>.into_iter().collect()
        } else {
            Vec::new()
        };
        let config = match self.config {
            Some(config) => config,
            None => match assets.iter().find(|asset| asset.path == "@serve.toml") {
                Some(asset) => AppConfig::parse(asset.contents).unwrap_or_else(|err| {
                    errors.push(format!("serve.toml: {err}"));
                    AppConfig::default()
                }),
                None => AppConfig::default(),
            },
        };
        let info = inventory::iter::<AppInfoRegistration>.into_iter().next();
        let name = config
            .name
            .clone()
            .or_else(|| info.map(|info| info.name.to_string()))
            .unwrap_or_else(|| "serve-app".to_string());
        let version = info
            .map(|info| info.version.to_string())
            .unwrap_or_else(|| "0.0.0".to_string());
        if self.discover && info.is_none() {
            warnings.push(
                "serve::assets!() was not called; agent/** and serve.toml are not embedded"
                    .to_string(),
            );
        }

        let mut app = AppInner {
            name,
            version,
            config,
            agents: Vec::new(),
            tools: Vec::new(),
            channels: Vec::new(),
            schedules: Vec::new(),
            connections: Vec::new(),
            evals: Vec::new(),
            skills: skills(&assets, &mut warnings),
            assets,
            warnings,
            errors,
        };
        if self.discover {
            discover(&mut app);
        }
        App {
            inner: Arc::new(app),
        }
    }

    /// Like [`build`](Self::build), but fail with every problem found.
    pub fn try_build(self) -> crate::Result<App> {
        let app = self.build();
        if app.errors().is_empty() {
            Ok(app)
        } else {
            anyhow::bail!("{}", app.errors().join("\n"))
        }
    }
}

fn discover(app: &mut AppInner) {
    let mut tools: Vec<&'static ToolRegistration> =
        inventory::iter::<ToolRegistration>.into_iter().collect();
    tools.sort_by_key(|tool| tool.name);
    check_unique(&mut app.errors, "tool", tools.iter().map(|t| t.name));
    for tool in &tools {
        convention(&mut app.warnings, "tool", tool.name, tool.source, "tools");
    }

    let mut agents: Vec<&'static AgentRegistration> =
        inventory::iter::<AgentRegistration>.into_iter().collect();
    agents.sort_by_key(|agent| agent.name);
    check_unique(&mut app.errors, "agent", agents.iter().map(|a| a.name));
    for registration in agents {
        let spec = (registration.build)();
        let dir = if registration.sub { "subagents" } else { "" };
        if !dir.is_empty() {
            convention(
                &mut app.warnings,
                "subagent",
                registration.name,
                registration.source,
                dir,
            );
        }
        if spec.model.trim().is_empty() {
            app.errors.push(format!(
                "agent `{}` ({}) has no model; call .model(\"provider/model\")",
                registration.name, registration.source
            ));
        }
        if let Some(names) = &spec.tools {
            for name in names {
                if !tools.iter().any(|tool| tool.name == name) {
                    app.errors.push(format!(
                        "agent `{}` lists unknown tool `{name}`",
                        registration.name
                    ));
                }
            }
        }
        if registration.sub
            && tools
                .iter()
                .any(|tool| tool.name == format!("ask_{}", registration.name))
        {
            app.errors.push(format!(
                "tool `ask_{0}` collides with the tool generated for subagent `{0}`",
                registration.name
            ));
        }
        app.agents.push(AgentEntry {
            name: registration.name,
            doc: registration.doc,
            sub: registration.sub,
            default: registration.default,
            source: registration.source,
            spec,
        });
    }
    let top: Vec<_> = app.agents.iter().filter(|a| !a.sub).collect();
    let defaults = top.iter().filter(|a| a.default).count();
    if top.is_empty() {
        app.errors
            .push("no #[agent] found; add one (subagents alone cannot serve sessions)".into());
    } else if defaults > 1 {
        app.errors
            .push("more than one #[agent(default)]; mark exactly one".into());
    } else if top.len() > 1 && defaults == 0 {
        app.warnings.push(
            "several agents and none is #[agent(default)]; sessions must name one with agent_name"
                .into(),
        );
    }
    app.tools = tools;

    let mut channels: Vec<&'static ChannelRegistration> =
        inventory::iter::<ChannelRegistration>.into_iter().collect();
    channels.sort_by_key(|channel| channel.name);
    check_unique(&mut app.errors, "channel", channels.iter().map(|c| c.name));
    for registration in channels {
        convention(
            &mut app.warnings,
            "channel",
            registration.name,
            registration.source,
            "channels",
        );
        app.channels.push(ChannelEntry {
            name: registration.name,
            source: registration.source,
            channel: Arc::from((registration.build)()),
        });
    }

    let mut schedules: Vec<&'static ScheduleRegistration> = inventory::iter::<ScheduleRegistration>
        .into_iter()
        .collect();
    schedules.sort_by_key(|schedule| schedule.name);
    check_unique(
        &mut app.errors,
        "schedule",
        schedules.iter().map(|s| s.name),
    );
    for registration in schedules {
        convention(
            &mut app.warnings,
            "schedule",
            registration.name,
            registration.source,
            "schedules",
        );
        match parse_cron(registration.cron) {
            Ok(schedule) => app.schedules.push(ScheduleEntry {
                registration,
                schedule,
            }),
            Err(err) => app.errors.push(format!(
                "schedule `{}` has an invalid cron `{}`: {err}",
                registration.name, registration.cron
            )),
        }
    }

    let mut connections: Vec<&'static ConnectionRegistration> =
        inventory::iter::<ConnectionRegistration>
            .into_iter()
            .collect();
    connections.sort_by_key(|connection| connection.name);
    check_unique(
        &mut app.errors,
        "connection",
        connections.iter().map(|c| c.name),
    );
    let mut types = BTreeMap::new();
    for registration in connections {
        convention(
            &mut app.warnings,
            "connection",
            registration.name,
            registration.source,
            "connections",
        );
        let value = match (registration.build)() {
            Ok(value) => value,
            Err(err) => {
                app.errors.push(format!(
                    "connection `{}` ({}) failed to build: {err:#}",
                    registration.name, registration.source
                ));
                continue;
            }
        };
        if value.mcp.is_none()
            && let Some(other) = types.insert(value.type_name, registration.name)
        {
            app.errors.push(format!(
                "connections `{other}` and `{}` have the same type `{}`; cx.connection::<T>() could not tell them apart",
                registration.name, value.type_name
            ));
        }
        app.connections.push(ConnectionEntry {
            source: registration.source,
            value,
        });
    }

    let mut evals: Vec<&'static EvalRegistration> =
        inventory::iter::<EvalRegistration>.into_iter().collect();
    evals.sort_by_key(|eval| eval.name);
    check_unique(&mut app.errors, "eval", evals.iter().map(|e| e.name));
    app.evals = evals;
}

/// Five-field cron gains a seconds field; six or seven fields pass through.
pub(crate) fn parse_cron(expr: &str) -> Result<cron::Schedule, cron::error::Error> {
    let fields = expr.split_whitespace().count();
    let expr = if fields == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    };
    cron::Schedule::from_str(&expr)
}

fn check_unique<'a>(errors: &mut Vec<String>, kind: &str, names: impl Iterator<Item = &'a str>) {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            errors.push(format!("two {kind}s are named `{name}`"));
        }
    }
}

/// Warn when an item lives outside its conventional directory. The layout is
/// what makes an app readable at a glance, as in eve; it is not enforced.
fn convention(warnings: &mut Vec<String>, kind: &str, name: &str, source: &str, dir: &str) {
    let source = source.replace('\\', "/");
    if !source.contains(&format!("/{dir}/")) && !source.contains(&format!("/{dir}.rs")) {
        warnings.push(format!(
            "{kind} `{name}` is defined in {source}; by convention it lives in src/{dir}/{name}.rs"
        ));
    }
}

fn skills(assets: &[&'static AssetRegistration], warnings: &mut Vec<String>) -> Vec<Skill> {
    let mut skills = Vec::new();
    for asset in assets {
        let Some(rest) = asset.path.strip_prefix("skills/") else {
            continue;
        };
        let Some(dir) = rest.strip_suffix("/SKILL.md") else {
            continue;
        };
        let (name, description) = frontmatter(asset.contents);
        let name = name.unwrap_or_else(|| dir.to_string());
        let description = description.unwrap_or_else(|| {
            warnings.push(format!(
                "skill `{name}` has no `description:` in its frontmatter; the model sees only its name"
            ));
            String::new()
        });
        skills.push(Skill {
            name,
            description,
            asset,
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Read `name:` and `description:` from a `---` frontmatter block.
pub(crate) fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().trim_matches('"').to_string());
        }
    }
    (name, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_field_cron_gets_seconds() {
        let schedule = parse_cron("0 9 * * MON").unwrap();
        assert!(schedule.upcoming(chrono::Utc).next().is_some());
        assert!(parse_cron("0 0 9 * * MON").is_ok());
        assert!(parse_cron("every monday").is_err());
    }

    #[test]
    fn frontmatter_reads_name_and_description() {
        let text = "---\nname: sql-style\ndescription: \"How we write SQL\"\n---\n# Body\n";
        assert_eq!(
            frontmatter(text),
            (Some("sql-style".into()), Some("How we write SQL".into()))
        );
        assert_eq!(frontmatter("# no frontmatter"), (None, None));
    }

    #[test]
    fn convention_warns_outside_the_directory() {
        let mut warnings = Vec::new();
        convention(
            &mut warnings,
            "tool",
            "run_sql",
            "src/tools/run_sql.rs",
            "tools",
        );
        convention(&mut warnings, "tool", "run_sql", "src/tools.rs", "tools");
        assert!(warnings.is_empty(), "{warnings:?}");
        convention(&mut warnings, "tool", "run_sql", "src/main.rs", "tools");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("src/tools/run_sql.rs"));
    }

    #[test]
    fn duplicates_are_reported() {
        let mut errors = Vec::new();
        check_unique(&mut errors, "tool", ["a", "b", "a"].into_iter());
        assert_eq!(errors, vec!["two tools are named `a`"]);
    }
}
