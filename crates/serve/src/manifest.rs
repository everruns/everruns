//! `manifest.json`: what the app declares, and the host provides.
//!
//! Decision: the manifest is produced by the built binary itself
//! (`cargo run -- manifest`), not by `build.rs`, because only the linked
//! binary knows what the macros registered. A host runs it once per build and
//! from it creates cron entries and webhook routes, asks for missing secrets,
//! provisions storage and the sandbox, and wires the model gateway. The app
//! never writes infrastructure config.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::App;

/// Schema identifier of this manifest format.
pub const SCHEMA: &str = "serve.manifest/v0";

/// The host contract. Serialize with `serde_json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema: String,
    /// Always true for now: serve is a proof of concept.
    pub experimental: bool,
    pub app: AppInfo,
    /// Stable hash of everything below; sessions are pinned to it.
    pub build_id: String,
    pub agents: Vec<AgentInfo>,
    pub tools: Vec<ToolInfo>,
    pub skills: Vec<SkillInfo>,
    pub channels: Vec<ChannelInfo>,
    pub schedules: Vec<ScheduleInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub secrets: Vec<SecretInfo>,
    pub sandbox: SandboxInfo,
    /// Every model string, for the gateway.
    pub models: Vec<String>,
    pub evals: Vec<EvalInfo>,
    /// The wire API this build serves.
    pub routes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub deploy_target: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentInfo {
    pub name: String,
    pub model: String,
    pub description: Option<String>,
    pub default: bool,
    pub sub: bool,
    pub tools: Vec<String>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// `never`, `always`, or `conditional`.
    pub needs_approval: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChannelInfo {
    pub name: String,
    pub kind: String,
    pub route: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScheduleInfo {
    pub name: String,
    pub cron: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConnectionInfo {
    pub name: String,
    /// `mcp` or `typed`.
    pub kind: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub url: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SecretInfo {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SandboxInfo {
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvalInfo {
    pub name: String,
    pub description: String,
}

impl Manifest {
    pub(crate) fn of(app: &App) -> Self {
        let mut manifest = Self::without_build_id(app);
        manifest.build_id = hash(&manifest, app);
        manifest
    }

    fn without_build_id(app: &App) -> Self {
        let inner = &app.inner;
        let agents = inner
            .agents
            .iter()
            .map(|agent| {
                let mut tools: Vec<String> = app
                    .tools_for(agent)
                    .iter()
                    .map(|t| t.name.to_string())
                    .collect();
                if !inner.skills.is_empty() {
                    tools.push("load_skill".into());
                }
                if !agent.sub {
                    tools.extend(
                        inner
                            .agents
                            .iter()
                            .filter(|a| a.sub)
                            .map(|a| format!("ask_{}", a.name)),
                    );
                }
                AgentInfo {
                    name: agent.name.into(),
                    model: agent.spec.model.clone(),
                    description: agent
                        .spec
                        .description
                        .clone()
                        .or_else(|| (!agent.doc.is_empty()).then(|| agent.doc.to_string())),
                    default: app.default_agent().is_some_and(|d| d.name == agent.name),
                    sub: agent.sub,
                    tools,
                    source: agent.source.into(),
                }
            })
            .collect();

        let tools = inner
            .tools
            .iter()
            .map(|tool| ToolInfo {
                name: tool.name.into(),
                description: tool.description.into(),
                parameters: (tool.schema)(),
                needs_approval: tool.approval.label().into(),
                source: tool.source.into(),
            })
            .collect();

        let mut secrets: BTreeSet<String> = inner.config.secrets.required.iter().cloned().collect();
        for entry in &inner.channels {
            secrets.extend(entry.channel.secrets().iter().map(|s| s.name().to_string()));
        }
        // Connections and channels name their secrets while being built.
        secrets.extend(
            crate::connection::declared_secrets()
                .into_iter()
                .map(str::to_string),
        );

        let mut routes = vec![
            "POST /v1/sessions".to_string(),
            "POST /v1/sessions/{id}/messages".into(),
            "GET /v1/sessions/{id}/events".into(),
            "POST /v1/sessions/{id}/cancel".into(),
            "POST /v1/sessions/{id}/approvals/{approval_id}".into(),
            "GET /v1/agent".into(),
            "POST /v1/agents/{name}/sessions".into(),
            "GET /health".into(),
        ];
        routes.extend(
            inner
                .channels
                .iter()
                .map(|c| format!("POST /v1/channels/{}", c.name)),
        );

        let models: BTreeSet<String> = inner.agents.iter().map(|a| a.spec.model.clone()).collect();
        Manifest {
            schema: SCHEMA.into(),
            experimental: true,
            app: AppInfo {
                name: inner.name.clone(),
                version: inner.version.clone(),
                deploy_target: inner.config.deploy.target.clone(),
            },
            build_id: String::new(),
            agents,
            tools,
            skills: inner
                .skills
                .iter()
                .map(|skill| SkillInfo {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    path: format!("agent/{}", skill.asset.path),
                })
                .collect(),
            channels: inner
                .channels
                .iter()
                .map(|entry| ChannelInfo {
                    name: entry.name.into(),
                    kind: entry.channel.kind().into(),
                    route: format!("/v1/channels/{}", entry.name),
                    source: entry.source.into(),
                })
                .collect(),
            schedules: inner
                .schedules
                .iter()
                .map(|entry| ScheduleInfo {
                    name: entry.registration.name.into(),
                    cron: entry.registration.cron.into(),
                    source: entry.registration.source.into(),
                })
                .collect(),
            connections: inner
                .connections
                .iter()
                .map(|entry| ConnectionInfo {
                    name: entry.value.name.into(),
                    kind: if entry.value.mcp.is_some() {
                        "mcp"
                    } else {
                        "typed"
                    }
                    .into(),
                    type_name: entry.value.type_name.into(),
                    url: entry.value.mcp.as_ref().map(|mcp| mcp.url().to_string()),
                    source: entry.source.into(),
                })
                .collect(),
            secrets: secrets
                .into_iter()
                .map(|name| SecretInfo { name })
                .collect(),
            sandbox: SandboxInfo {
                kind: inner.config.sandbox.kind.as_str().into(),
            },
            models: models.into_iter().collect(),
            evals: inner
                .evals
                .iter()
                .map(|eval| EvalInfo {
                    name: eval.name.into(),
                    description: eval.doc.into(),
                })
                .collect(),
            routes,
        }
    }
}

/// Content hash of the manifest plus the embedded assets, so a prompt edit is
/// a new build even when no code changed.
pub(crate) fn build_id(app: &App) -> String {
    Manifest::of(app).build_id
}

fn hash(manifest: &Manifest, app: &App) -> String {
    // DefaultHasher is fixed-key SipHash: stable for the same toolchain, which
    // is all a build id needs.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(manifest)
        .unwrap_or_default()
        .hash(&mut hasher);
    let assets: BTreeMap<&str, &str> = app
        .inner
        .assets
        .iter()
        .map(|asset| (asset.path, asset.contents))
        .collect();
    assets.hash(&mut hasher);
    format!("b{:016x}", hasher.finish())
}
