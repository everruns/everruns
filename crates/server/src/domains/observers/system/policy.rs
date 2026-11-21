use everruns_contracts::typed_id::ModelId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Consent and spending admission for system-managed compaction observers.
/// No content is shared outside the organization's configured model provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct QualityMonitoringPolicy {
    pub enabled: bool,
    /// Fraction of eligible checkpoints receiving a content-based assessment.
    pub sampling_rate: f64,
    /// Explicit org model. None enables mechanical checks only.
    pub model_id: Option<ModelId>,
    /// UTC-day admission cap, including reserved uncertain/retry costs.
    /// Zero disables model calls while retaining mechanical checks.
    pub daily_budget_usd: f64,
    /// Sessions with any of these tags are excluded before reading content.
    pub excluded_tags: Vec<String>,
}

impl Default for QualityMonitoringPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            sampling_rate: 0.02,
            model_id: None,
            daily_budget_usd: 0.0,
            excluded_tags: Vec::new(),
        }
    }
}

impl QualityMonitoringPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !self.sampling_rate.is_finite() || !(0.0..=1.0).contains(&self.sampling_rate) {
            return Err("Monitoring sampling_rate must be between 0 and 1".into());
        }
        if !self.daily_budget_usd.is_finite() || !(0.0..=100.0).contains(&self.daily_budget_usd) {
            return Err("Monitoring daily_budget_usd must be between 0 and 100".into());
        }
        if self.enabled && self.daily_budget_usd > 0.0 && self.model_id.is_none() {
            return Err("Select an organization model before enabling paid monitoring".into());
        }
        if self.excluded_tags.len() > 32
            || self
                .excluded_tags
                .iter()
                .any(|tag| tag.trim().is_empty() || tag.len() > 128)
        {
            return Err("Monitoring excluded_tags must contain at most 32 non-empty tags of at most 128 bytes".into());
        }
        Ok(())
    }

    pub fn allows(&self, tags: &[String]) -> bool {
        self.enabled
            && !tags
                .iter()
                .any(|tag| tag == "eval" || self.excluded_tags.contains(tag))
    }

    pub fn daily_budget_micros(&self) -> i64 {
        (self.daily_budget_usd * 1_000_000.0).floor() as i64
    }
}
