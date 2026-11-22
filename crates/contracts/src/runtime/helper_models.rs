//! Helper models: the models an agent's spawned subagents may run on.
//!
//! Decision: the spawning model names a helper *key* (`fast`, `second_opinion`,
//! ...), never a raw model name, so the agent owner controls cost and which
//! providers see the data. Each key points at a *target* the host resolves to
//! one of the organization's models when it creates the child session: a job
//! of the agent's own preset or defaults, a job of a named preset, a concrete
//! model, or a job of another provider's preset. The keys live in the
//! `subagents` capability config, so they travel with the agent, its package
//! and every plane that builds the `spawn_agent` tool, and the tool schema is
//! built once when the session starts (cache-stable tool list).

use serde::{Deserialize, Serialize};

/// Config key in the `subagents` capability holding the helper list.
pub const HELPER_MODELS_CONFIG_KEY: &str = "helper_models";
/// Config key in the `subagents` capability holding who picks a key.
pub const HELPER_MODEL_PICK_CONFIG_KEY: &str = "helper_model_pick";
/// The key that keeps the parent's model.
pub const SAME_HELPER_KEY: &str = "same";
/// Most keys an agent may declare (the tool schema lists every one).
pub const MAX_HELPER_MODELS: usize = 16;

/// A job a helper target names. Mirrors the server's model preset jobs.
pub const HELPER_JOBS: [&str; 6] = ["default", "chat", "coding", "research", "small", "decision"];

/// Where a helper key's model comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperModelTarget {
    /// The parent session's model.
    Same,
    /// A job of the agent's own preset, else the organization default for it.
    Job { job: String },
    /// A job of a named model preset (`provider:gemini`, `automatic`, a custom id).
    Preset { preset: String, job: String },
    /// A concrete organization model.
    Model { model_id: String },
    /// A job of the first connected provider preset whose provider differs
    /// from the parent's model's provider.
    OtherProvider { job: String },
}

/// One helper key the spawning model may name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperModel {
    /// What the model writes in `spawn_agent.model`.
    pub key: String,
    /// When to use it, shown to the model next to the key.
    pub when: String,
    /// Where the model comes from.
    pub target: HelperModelTarget,
}

/// Who picks the key when the spawning model names none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperModelPick {
    /// No key means `same`.
    #[default]
    Agent,
    /// The organization's decision model reads the brief and picks a key.
    Auto,
}

impl HelperModelPick {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Auto => "auto",
        }
    }
}

/// How the key of one spawn was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperModelPickedBy {
    /// The spawning model named the key.
    Agent,
    /// The decision model picked it.
    Auto,
    /// Nobody named one: `same`.
    Default,
}

impl HelperModelPickedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Auto => "auto",
            Self::Default => "default",
        }
    }
}

/// The helper model a spawn asks the host for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnHelperModel {
    pub key: String,
    pub target: HelperModelTarget,
    pub picked_by: HelperModelPickedBy,
    /// Why Auto picked the key: its "when" line and how sure the pick was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The keys every agent gets unless its `subagents` config lists its own.
pub fn builtin_helper_models() -> Vec<HelperModel> {
    let job = |job: &str| HelperModelTarget::Job { job: job.into() };
    vec![
        HelperModel {
            key: SAME_HELPER_KEY.into(),
            when: "Most work. Runs on your own model.".into(),
            target: HelperModelTarget::Same,
        },
        HelperModel {
            key: "fast".into(),
            when: "Search, lookups, summaries: cheap and quick.".into(),
            target: job("small"),
        },
        HelperModel {
            key: "coding".into(),
            when: "Implementation work.".into(),
            target: job("coding"),
        },
        HelperModel {
            key: "deep".into(),
            when: "Hard reasoning and planning.".into(),
            target: job("research"),
        },
        HelperModel {
            key: "second_opinion".into(),
            when:
                "Verify or review work done by another agent, on a different provider than yours."
                    .into(),
            target: HelperModelTarget::OtherProvider {
                job: "default".into(),
            },
        },
    ]
}

/// The helper settings in a `subagents` capability config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperModelSettings {
    pub models: Vec<HelperModel>,
    pub pick: HelperModelPick,
}

impl Default for HelperModelSettings {
    fn default() -> Self {
        Self {
            models: builtin_helper_models(),
            pick: HelperModelPick::Agent,
        }
    }
}

impl HelperModelSettings {
    /// Read the settings from a `subagents` capability config. A missing list
    /// means the built-ins; a malformed one is reported by
    /// [`validate_helper_model_config`] and read here as the built-ins.
    pub fn from_config(config: &serde_json::Value) -> Self {
        let models = config
            .get(HELPER_MODELS_CONFIG_KEY)
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::from_value::<Vec<HelperModel>>(value.clone()).ok())
            .unwrap_or_else(builtin_helper_models);
        let pick = config
            .get(HELPER_MODEL_PICK_CONFIG_KEY)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default();
        Self { models, pick }
    }

    pub fn get(&self, key: &str) -> Option<&HelperModel> {
        self.models.iter().find(|model| model.key == key)
    }

    /// Whether a spawn can change models at all.
    pub fn can_switch(&self) -> bool {
        self.models
            .iter()
            .any(|model| model.target != HelperModelTarget::Same)
    }

    pub fn keys(&self) -> Vec<&str> {
        self.models.iter().map(|model| model.key.as_str()).collect()
    }
}

/// Validate the helper settings of a `subagents` capability config.
pub fn validate_helper_model_config(config: &serde_json::Value) -> Result<(), String> {
    if let Some(pick) = config.get(HELPER_MODEL_PICK_CONFIG_KEY)
        && serde_json::from_value::<HelperModelPick>(pick.clone()).is_err()
    {
        return Err(format!(
            "{HELPER_MODEL_PICK_CONFIG_KEY} must be \"agent\" or \"auto\""
        ));
    }
    let Some(value) = config
        .get(HELPER_MODELS_CONFIG_KEY)
        .filter(|value| !value.is_null())
    else {
        return Ok(());
    };
    let models: Vec<HelperModel> = serde_json::from_value(value.clone())
        .map_err(|error| format!("{HELPER_MODELS_CONFIG_KEY} is invalid: {error}"))?;
    if models.is_empty() {
        return Err(format!(
            "{HELPER_MODELS_CONFIG_KEY} needs at least one key; keep only `same` to stop subagents from switching models"
        ));
    }
    if models.len() > MAX_HELPER_MODELS {
        return Err(format!(
            "{HELPER_MODELS_CONFIG_KEY} allows at most {MAX_HELPER_MODELS} keys"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for model in &models {
        let key = model.key.as_str();
        let valid_key = !key.is_empty()
            && key.len() <= 40
            && key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !valid_key {
            return Err(format!(
                "helper key \"{key}\" must be 1 to 40 lowercase letters, digits, `_` or `-`"
            ));
        }
        if !seen.insert(key) {
            return Err(format!("helper key \"{key}\" is listed twice"));
        }
        if model.when.trim().is_empty() || model.when.len() > 300 {
            return Err(format!(
                "helper key \"{key}\" needs a \"when\" line of 1 to 300 characters"
            ));
        }
        match &model.target {
            HelperModelTarget::Same => {}
            HelperModelTarget::Job { job }
            | HelperModelTarget::Preset { job, .. }
            | HelperModelTarget::OtherProvider { job } => {
                if !HELPER_JOBS.contains(&job.as_str()) {
                    return Err(format!(
                        "helper key \"{key}\" names unknown job \"{job}\"; use one of {}",
                        HELPER_JOBS.join(", ")
                    ));
                }
                if let HelperModelTarget::Preset { preset, .. } = &model.target
                    && preset.trim().is_empty()
                {
                    return Err(format!("helper key \"{key}\" needs a preset id"));
                }
            }
            HelperModelTarget::Model { model_id } => {
                if model_id.trim().is_empty() {
                    return Err(format!("helper key \"{key}\" needs a model_id"));
                }
            }
        }
    }
    Ok(())
}

/// JSON Schema for the helper settings, merged into the `subagents` config schema.
pub fn helper_model_config_schema() -> (serde_json::Value, serde_json::Value) {
    let models = serde_json::json!({
        "type": "array",
        "maxItems": MAX_HELPER_MODELS,
        "description": "Models this agent's subagents may run on. The spawning model names a key in spawn_agent.model, never a raw model name. Omit to use the built-in keys (same, fast, coding, deep, second_opinion); keep only `same` to stop subagents from switching models.",
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["key", "when", "target"],
            "properties": {
                "key": {"type": "string", "pattern": "^[a-z0-9_-]{1,40}$", "description": "What the spawning model writes in spawn_agent.model."},
                "when": {"type": "string", "minLength": 1, "maxLength": 300, "description": "When to use this key, shown to the model."},
                "target": {
                    "type": "object",
                    "description": "Where the model comes from: {\"type\":\"same\"}, {\"type\":\"job\",\"job\":\"small\"} (the agent's preset or the organization default), {\"type\":\"preset\",\"preset\":\"provider:gemini\",\"job\":\"default\"}, {\"type\":\"model\",\"model_id\":\"model_...\"} or {\"type\":\"other_provider\",\"job\":\"default\"}.",
                    "required": ["type"],
                    "properties": {
                        "type": {"type": "string", "enum": ["same", "job", "preset", "model", "other_provider"]},
                        "job": {"type": "string", "enum": HELPER_JOBS},
                        "preset": {"type": "string"},
                        "model_id": {"type": "string"}
                    }
                }
            }
        }
    });
    let pick = serde_json::json!({
        "type": "string",
        "enum": ["agent", "auto"],
        "default": "agent",
        "description": "Who picks a helper key when the spawning model names none: agent (no key means `same`) or auto (the organization's decision model reads the brief and picks one)."
    });
    (models, pick)
}

/// What an organization's helper models resolve against: its models, its
/// presets and its job defaults. The control plane builds it from its
/// tables for its own workers and pushes the same catalog to a data plane
/// (`dataplane::HELPER_MODEL_SETTING`), so both planes resolve a helper key
/// with [`resolve_helper_model`] and land on the same model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperCatalog {
    /// Every model of the organization.
    #[serde(default)]
    pub models: Vec<CatalogModel>,
    /// Every preset (built-in first, in the order the console lists them,
    /// then custom), with the model it assigns each job.
    #[serde(default)]
    pub presets: Vec<CatalogPreset>,
    /// The organization default for each job (the Defaults tab), by job.
    #[serde(default)]
    pub defaults: std::collections::BTreeMap<String, String>,
}

/// One organization model in a [`HelperCatalog`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// The organization model id.
    pub id: String,
    /// Its provider's key (the provider's normalized name), as an agent
    /// package's `model.provider` names it.
    pub provider: String,
    /// The provider's driver (`openai`, `gemini`, ...).
    pub provider_type: String,
    /// The provider's model name.
    pub model: String,
    /// Whether a turn can run on it (enabled, on a connected provider).
    #[serde(default)]
    pub usable: bool,
}

/// One preset in a [`HelperCatalog`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPreset {
    /// `automatic`, `provider:<driver>` or a custom preset's key.
    pub id: String,
    /// The driver of a provider preset; `None` for any other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// The model id it assigns each job it sets, by job.
    #[serde(default)]
    pub jobs: std::collections::BTreeMap<String, String>,
}

impl HelperCatalog {
    /// The model `id`, whether usable or not.
    pub fn model(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|model| model.id == id)
    }

    fn preset_job(&self, preset: &str, job: &str) -> Option<&str> {
        self.presets
            .iter()
            .find(|candidate| candidate.id == preset)
            .and_then(|preset| preset.jobs.get(job))
            .map(String::as_str)
    }
}

/// The model `target` resolves to in `catalog`: a job of the agent's own
/// preset (`agent_preset`) else the organization default, a job of a named
/// preset, a usable concrete model, or a job of the first provider preset
/// whose driver is not `parent_provider_type`. `None` keeps the child on its
/// ordinary model resolution. [`HelperModelTarget::Same`] is the caller's:
/// each plane knows what its parent runs on.
pub fn resolve_helper_model<'a>(
    catalog: &'a HelperCatalog,
    target: &HelperModelTarget,
    agent_preset: Option<&str>,
    parent_provider_type: Option<&str>,
) -> Option<&'a CatalogModel> {
    let job_ok = |job: &str| HELPER_JOBS.contains(&job);
    let id = match target {
        HelperModelTarget::Same => return None,
        HelperModelTarget::Job { job } if job_ok(job) => agent_preset
            .and_then(|preset| catalog.preset_job(preset, job))
            .or_else(|| catalog.defaults.get(job).map(String::as_str))?,
        HelperModelTarget::Preset { preset, job } if job_ok(job) => {
            catalog.preset_job(preset, job)?
        }
        HelperModelTarget::Model { model_id } => {
            return catalog.model(model_id).filter(|model| model.usable);
        }
        HelperModelTarget::OtherProvider { job } if job_ok(job) => catalog
            .presets
            .iter()
            .filter(|preset| preset.provider_type.is_some())
            .filter(|preset| preset.provider_type.as_deref() != parent_provider_type)
            .find_map(|preset| preset.jobs.get(job.as_str()).or(preset.jobs.get("default")))?,
        _ => return None,
    };
    catalog.model(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> HelperCatalog {
        let model = |id: &str, provider_type: &str, usable: bool| CatalogModel {
            id: id.into(),
            provider: format!("{provider_type}-key"),
            provider_type: provider_type.into(),
            model: format!("{id}-name"),
            usable,
        };
        let jobs = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(job, id)| (job.to_string(), id.to_string()))
                .collect()
        };
        HelperCatalog {
            models: vec![
                model("m-openai", "openai", true),
                model("m-gemini", "gemini", true),
                model("m-off", "openai", false),
            ],
            presets: vec![
                CatalogPreset {
                    id: "provider:openai".into(),
                    provider_type: Some("openai".into()),
                    jobs: jobs(&[("default", "m-openai")]),
                },
                CatalogPreset {
                    id: "provider:gemini".into(),
                    provider_type: Some("gemini".into()),
                    jobs: jobs(&[("default", "m-gemini"), ("small", "m-gemini")]),
                },
                CatalogPreset {
                    id: "mine".into(),
                    provider_type: None,
                    jobs: jobs(&[("coding", "m-gemini")]),
                },
            ],
            defaults: jobs(&[("coding", "m-openai"), ("small", "m-openai")]),
        }
    }

    #[test]
    fn a_helper_target_resolves_against_the_catalog() {
        let catalog = catalog();
        let id = |target: HelperModelTarget, preset: Option<&str>, parent: Option<&str>| {
            resolve_helper_model(&catalog, &target, preset, parent).map(|m| m.id.clone())
        };
        let job = |job: &str| HelperModelTarget::Job { job: job.into() };
        assert_eq!(
            id(job("coding"), Some("mine"), None).as_deref(),
            Some("m-gemini")
        );
        assert_eq!(
            id(job("coding"), None, None).as_deref(),
            Some("m-openai"),
            "no agent preset: the org default"
        );
        assert_eq!(id(job("nope"), None, None), None);
        let preset = HelperModelTarget::Preset {
            preset: "provider:gemini".into(),
            job: "small".into(),
        };
        assert_eq!(id(preset, None, None).as_deref(), Some("m-gemini"));
        let model = |m: &str| HelperModelTarget::Model { model_id: m.into() };
        assert_eq!(
            id(model("m-gemini"), None, None).as_deref(),
            Some("m-gemini")
        );
        assert_eq!(id(model("m-off"), None, None), None, "unusable");
        let other = HelperModelTarget::OtherProvider {
            job: "coding".into(),
        };
        assert_eq!(
            id(other.clone(), None, Some("openai")).as_deref(),
            Some("m-gemini"),
            "another provider's preset, its default job"
        );
        assert_eq!(id(other, None, Some("gemini")).as_deref(), Some("m-openai"));
        assert_eq!(id(HelperModelTarget::Same, None, None), None);
    }

    #[test]
    fn missing_config_means_builtins_picked_by_the_agent() {
        let settings = HelperModelSettings::from_config(&json!({}));
        assert_eq!(
            settings.keys(),
            ["same", "fast", "coding", "deep", "second_opinion"]
        );
        assert_eq!(settings.pick, HelperModelPick::Agent);
        assert!(settings.can_switch());
        assert!(validate_helper_model_config(&json!({})).is_ok());
    }

    #[test]
    fn only_same_cannot_switch_models() {
        let config = json!({"helper_models": [
            {"key": "same", "when": "Everything.", "target": {"type": "same"}}
        ], "helper_model_pick": "auto"});
        assert!(validate_helper_model_config(&config).is_ok());
        let settings = HelperModelSettings::from_config(&config);
        assert!(!settings.can_switch());
        assert_eq!(settings.pick, HelperModelPick::Auto);
    }

    #[test]
    fn custom_keys_round_trip_and_bad_ones_are_named() {
        let config = json!({"helper_models": [
            {"key": "gemini", "when": "Council member.", "target": {"type": "preset", "preset": "provider:gemini", "job": "default"}},
            {"key": "pinned", "when": "Exact model.", "target": {"type": "model", "model_id": "model_01"}}
        ]});
        assert!(validate_helper_model_config(&config).is_ok());
        assert_eq!(
            HelperModelSettings::from_config(&config)
                .get("gemini")
                .unwrap()
                .target,
            HelperModelTarget::Preset {
                preset: "provider:gemini".into(),
                job: "default".into()
            }
        );
        for (bad, needle) in [
            (json!({"helper_models": []}), "at least one"),
            (json!({"helper_model_pick": "model"}), "agent"),
            (
                json!({"helper_models": [{"key": "Fast", "when": "x", "target": {"type": "same"}}]}),
                "lowercase",
            ),
            (
                json!({"helper_models": [
                    {"key": "a", "when": "x", "target": {"type": "same"}},
                    {"key": "a", "when": "y", "target": {"type": "same"}}
                ]}),
                "twice",
            ),
            (
                json!({"helper_models": [{"key": "a", "when": "x", "target": {"type": "job", "job": "vision"}}]}),
                "unknown job",
            ),
        ] {
            let error = validate_helper_model_config(&bad).unwrap_err();
            assert!(error.contains(needle), "{error}");
        }
    }
}
