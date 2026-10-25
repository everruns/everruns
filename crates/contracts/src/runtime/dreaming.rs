//! Dreaming configuration and durable maintenance boundary. No scheduler or host dependencies.
use crate::runtime::error::{AgentLoopError, Result};
use crate::typed_id::SessionId;
use crate::{CapabilityRef, CapabilitySpec, IntoCapability};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub const DREAMING_CAPABILITY_ID: &str = "dreaming";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DreamingConfig {
    pub destinations: Vec<DreamDestination>,
    #[serde(default)]
    pub trigger: DreamTrigger,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub budget: DreamBudget,
    #[serde(default)]
    pub commit_mode: DreamCommitMode,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DreamDestination {
    pub mount: String,
    pub path: String,
    #[serde(default)]
    pub instructions: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DreamTrigger {
    pub mode: DreamTriggerMode,
    pub idle_after_seconds: u32,
    pub min_new_turns: u32,
}
impl Default for DreamTrigger {
    fn default() -> Self {
        Self {
            mode: DreamTriggerMode::Idle,
            idle_after_seconds: 60,
            min_new_turns: 3,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DreamTriggerMode {
    #[default]
    Idle,
    AfterTurn,
    Manual,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DreamCommitMode {
    #[default]
    Automatic,
    Propose,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DreamBudget {
    pub max_output_tokens_per_run: u32,
    pub max_runs_per_day: u32,
    pub max_cost_usd_per_day: Option<f64>,
}
impl Default for DreamBudget {
    fn default() -> Self {
        Self {
            max_output_tokens_per_run: 4096,
            max_runs_per_day: 12,
            max_cost_usd_per_day: Some(1.0),
        }
    }
}
fn valid_path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() <= 512
        && !p.contains(['\0', '\\'])
        && !p.contains("//")
        && p.split('/').skip(1).all(|s| !matches!(s, "." | ".."))
        && (p == "/" || !p.ends_with('/'))
}
impl DreamingConfig {
    pub fn new(destination: DreamDestination) -> Self {
        Self {
            destinations: vec![destination],
            trigger: DreamTrigger::default(),
            model: None,
            budget: DreamBudget::default(),
            commit_mode: DreamCommitMode::default(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        let invalid = |s: &str| AgentLoopError::config(format!("dreaming: {s}"));
        if self.destinations.is_empty() || self.destinations.len() > 4 {
            return Err(invalid("configure 1–4 destinations"));
        }
        for (i, d) in self.destinations.iter().enumerate() {
            if !valid_path(&d.mount)
                || !valid_path(&d.path)
                || d.mount == "/"
                || d.instructions.len() > 4096
            {
                return Err(invalid("invalid destination path or instructions"));
            }
            if self.destinations[..i].iter().any(|x| {
                x.mount == d.mount
                    && (x.path == d.path
                        || x.path
                            .starts_with(&format!("{}/", d.path.trim_end_matches('/')))
                        || d.path
                            .starts_with(&format!("{}/", x.path.trim_end_matches('/'))))
            }) {
                return Err(invalid("destinations overlap"));
            }
        }
        if self.trigger.min_new_turns == 0
            || self.trigger.min_new_turns > 1000
            || self.trigger.idle_after_seconds > 86400
        {
            return Err(invalid("invalid trigger limits"));
        }
        if !(128..=16384).contains(&self.budget.max_output_tokens_per_run)
            || !(1..=1000).contains(&self.budget.max_runs_per_day)
        {
            return Err(invalid("invalid budget limits"));
        }
        if self
            .budget
            .max_cost_usd_per_day
            .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 1000.0)
        {
            return Err(invalid("invalid daily spending limit"));
        }
        if self
            .model
            .as_ref()
            .is_some_and(|m| m.is_empty() || m.len() > 256)
        {
            return Err(invalid("invalid model"));
        }
        Ok(())
    }
}
impl IntoCapability for DreamingConfig {
    fn into_capability(self) -> CapabilitySpec {
        let value = match self.validate() {
            Ok(()) => serde_json::json!(self),
            Err(error) => serde_json::json!({"invalid_configuration":error.to_string()}),
        };
        CapabilityRef::new(DREAMING_CAPABILITY_ID)
            .config(value)
            .into()
    }
}

/// Host-authorized identity; a mount alias is never itself authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamMemoryScope {
    pub key: String,
    pub private: bool,
}
/// Opaque, versioned maintenance state. Values must never appear in public traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamStateRow {
    pub revision: i64,
    pub value: String,
    pub session_id: Option<SessionId>,
    pub due_at: Option<i64>,
}
#[async_trait]
pub trait DreamingStore: Send + Sync {
    async fn get(&self, org_id: i64, key: &str) -> Result<Option<DreamStateRow>>;
    /// Atomic CAS, including creation when expected revision is absent.
    async fn compare_exchange(
        &self,
        org_id: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool>;
    /// Bounded runnable session identities; the claim itself is a CAS.
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> DreamingConfig {
        DreamingConfig::new(DreamDestination {
            mount: "/memory/user".into(),
            path: "/learned".into(),
            instructions: String::new(),
        })
    }
    #[test]
    fn rejects_path_aliases_and_unbounded_work() {
        for path in ["../bad", "/a/../b", "/a//b", "/a/", "/a\\b", "/a\0b"] {
            let mut c = config();
            c.destinations[0].path = path.into();
            assert!(c.validate().is_err(), "{path:?}");
        }
        let mut c = config();
        c.budget.max_cost_usd_per_day = Some(f64::NAN);
        assert!(c.validate().is_err());
    }
    #[test]
    fn rejects_overlapping_destinations_and_unknown_options() {
        let mut c = config();
        let mut d = c.destinations[0].clone();
        d.path = "/learned/other".into();
        c.destinations.push(d);
        assert!(c.validate().is_err());
        assert!(
            serde_json::from_value::<DreamingConfig>(
                serde_json::json!({"destinations":[],"typo":1})
            )
            .is_err()
        );
    }
}
