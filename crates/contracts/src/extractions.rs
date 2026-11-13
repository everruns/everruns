//! Typed document extraction, separate from chat and decision classification.
//!
//! Evidence offsets count Unicode code points, not bytes or UTF-16 units.
//! Missing uncertainty is unknown, never synthesized as zero or confidence one.

use crate::error::{AgentLoopError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The output shape requested for one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtractionType {
    /// A string extracted from the source.
    Text,
    /// A numeric value.
    Number,
    /// An ISO calendar date (YYYY-MM-DD).
    Date,
    /// A time of day, with hours and minutes and optional seconds or fractional seconds.
    Time,
    /// An RFC 3339 timestamp or a local ISO date and time with seconds.
    Datetime,
    /// A numeric duration in the requested unit.
    Duration,
    /// A homogeneous array with the configured element type.
    List,
    /// A numeric or date estimate with optional distribution statistics.
    Estimate,
    /// A horizon-length array of numeric or date predictions; individual values may be null.
    Forecast,
}

/// A list element type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtractionItemType {
    /// String elements.
    Text,
    /// Numeric elements.
    Number,
    /// ISO calendar date elements (YYYY-MM-DD).
    Date,
}

/// A numeric or calendar bound for an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum ExtractionBound {
    /// A finite numeric endpoint.
    Number(f64),
    /// An ISO calendar date endpoint (YYYY-MM-DD).
    Date(String),
}

/// The resolution of an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum ExtractionStep {
    /// A finite, positive numeric resolution.
    Number(f64),
    /// A month or year resolution, valid only with date bounds.
    Calendar(CalendarStep),
}

/// Calendar resolution for an estimate or forecast with date bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CalendarStep {
    /// Resolve dates by month.
    Month,
    /// Resolve dates by year.
    Year,
}

/// Scale requested for an estimate or forecast distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtractionScale {
    /// Request a linear distribution scale.
    Linear,
    /// Request a logarithmic distribution scale; numeric lower bounds must be positive.
    Log,
}

/// Spacing between successive forecast values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ForecastInterval {
    /// Abstract successive steps without a calendar unit.
    Step,
    /// One-minute intervals.
    Minute,
    /// One-hour intervals.
    Hour,
    /// One-day intervals.
    Day,
    /// One-week intervals.
    Week,
    /// One-month intervals.
    Month,
    /// One-quarter intervals.
    Quarter,
    /// One-year intervals.
    Year,
}

/// A typed field specification. Constructors select a shape; validation rejects
/// options which do not apply to it before any provider call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtractionField {
    /// Requested output shape; determines which field options are valid.
    #[serde(rename = "type")]
    pub kind: ExtractionType,
    /// Field-specific extraction guidance, which may reference named assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Requested value unit. Duration units are seconds, minutes, hours, days, weeks, months or
    /// years.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Requested null-suppression threshold, finite and within 0..=1. Providers may impose stricter
    /// limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_threshold: Option<f64>,
    /// Request integer precision for a number, duration or list of numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integer: Option<bool>,
    /// Requested decimal precision, at most six, for a number, duration or list of numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Required element type for lists; invalid for other field types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ExtractionItemType>,
    /// Required lower bound for estimates and forecasts; must be below max and use the same numeric
    /// or date type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<ExtractionBound>,
    /// Required upper bound for estimates and forecasts; must exceed min and use the same numeric
    /// or date type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<ExtractionBound>,
    /// Optional estimate or forecast resolution. Numeric steps must be finite and positive;
    /// calendar steps require date bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<ExtractionStep>,
    /// Optional estimate or forecast scale. Logarithmic numeric bounds must be positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ExtractionScale>,
    /// Required number of forecast values, from 1 through 64; invalid for other field types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<u8>,
    /// Required spacing between forecast values; invalid for other field types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<ForecastInterval>,
}
impl ExtractionField {
    pub fn new(kind: ExtractionType) -> Self {
        Self {
            kind,
            instructions: None,
            unit: None,
            null_threshold: None,
            integer: None,
            decimals: None,
            items: None,
            min: None,
            max: None,
            step: None,
            scale: None,
            horizon: None,
            interval: None,
        }
    }
    pub fn text() -> Self {
        Self::new(ExtractionType::Text)
    }
    pub fn number() -> Self {
        Self::new(ExtractionType::Number)
    }
    pub fn date() -> Self {
        Self::new(ExtractionType::Date)
    }
    pub fn list(items: ExtractionItemType) -> Self {
        Self {
            items: Some(items),
            ..Self::new(ExtractionType::List)
        }
    }
    pub fn instructions(mut self, value: impl Into<String>) -> Self {
        self.instructions = Some(value.into());
        self
    }
    pub fn validate(&self) -> Result<()> {
        use ExtractionType::*;
        if self.null_threshold.is_some_and(|n| !probability(n)) {
            return invalid("null threshold must be finite and between zero and one");
        }
        if self.decimals.is_some_and(|n| n > 6) {
            return invalid("decimals must be at most six");
        }
        if (self.integer.is_some() || self.decimals.is_some())
            && !matches!(self.kind, Number | Duration)
            && !(self.kind == List && self.items == Some(ExtractionItemType::Number))
        {
            return invalid("numeric precision requires a numeric field");
        }
        if (self.kind == List) != self.items.is_some() {
            return invalid("list fields require items; other fields cannot specify items");
        }
        if self.kind == Duration
            && self.unit.as_deref().is_some_and(|u| {
                !matches!(
                    u,
                    "seconds" | "minutes" | "hours" | "days" | "weeks" | "months" | "years"
                )
            })
        {
            return invalid("unsupported duration unit");
        }
        let distribution = matches!(self.kind, Estimate | Forecast);
        if distribution {
            match (&self.min, &self.max) {
                (Some(ExtractionBound::Number(a)), Some(ExtractionBound::Number(b)))
                    if a.is_finite() && b.is_finite() && a < b =>
                {
                    if self.scale == Some(ExtractionScale::Log) && *a <= 0.0 {
                        return invalid("logarithmic bounds must be positive");
                    }
                }
                (Some(ExtractionBound::Date(a)), Some(ExtractionBound::Date(b)))
                    if valid_date(a) && valid_date(b) && a < b => {}
                _ => {
                    return invalid(
                        "estimate bounds must be ordered finite numbers or ISO dates of the same type",
                    );
                }
            }
            if self.step.as_ref().is_some_and(
                |s| matches!(s, ExtractionStep::Number(n) if !n.is_finite() || *n <= 0.0),
            ) {
                return invalid("estimate step must be finite and positive");
            }
            if matches!(self.step, Some(ExtractionStep::Calendar(_)))
                && !matches!(self.min, Some(ExtractionBound::Date(_)))
            {
                return invalid("calendar steps require date bounds");
            }
        } else if self.min.is_some()
            || self.max.is_some()
            || self.step.is_some()
            || self.scale.is_some()
        {
            return invalid("bounds and scale require an estimate or forecast");
        }
        if self.kind == Forecast {
            if !self.horizon.is_some_and(|h| (1..=64).contains(&h)) || self.interval.is_none() {
                return invalid("forecast requires horizon 1..64 and an interval");
            }
        } else if self.horizon.is_some() || self.interval.is_some() {
            return invalid("horizon and interval require a forecast");
        }
        Ok(())
    }
}

/// Media category of an inline extraction asset; must agree with its format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtractionAssetType {
    /// An image encoded as PNG, JPEG or WebP.
    Image,
    /// A document encoded as PDF.
    Document,
}
/// Supported inline asset encoding; provider support may be narrower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ExtractionAssetFormat {
    /// PNG image data.
    Png,
    /// JPEG image data, using the jpg format label.
    Jpg,
    /// JPEG image data, using the jpeg format label.
    Jpeg,
    /// WebP image data.
    Webp,
    /// PDF document data; unsupported by image-only providers.
    Pdf,
}
/// Inline asset; URLs are deliberately not fetched by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtractionAsset {
    /// Optional media category. PDF requires document; other formats require image when specified.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ExtractionAssetType>,
    /// Encoding of the supplied bytes; support depends on the selected provider.
    pub format: ExtractionAssetFormat,
    /// Nonempty standard base64 content, without a data URL prefix.
    pub data_base64: String,
}

/// Named typed fields to extract from text or inline assets, with optional provider routing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtractionRequest {
    /// Optional explicit provider account for Framework routing; the Server rejects this override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub provider: Option<crate::ProviderKey>,
    /// Optional nonempty provider model identifier; the Server selects its model outside this
    /// input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Source text and extraction context. May be empty when inline assets are supplied.
    #[serde(default)]
    pub state: String,
    /// Nonempty map of field names to typed extraction specifications; names must not be blank.
    pub fields: BTreeMap<String, ExtractionField>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    /// Defensive runtime ceiling: 32 assets and 32 MiB total encoded data.
    /// Individual providers and the HTTP API may impose lower limits.
    pub assets: BTreeMap<String, ExtractionAsset>,
    /// Optional ISO calendar date (YYYY-MM-DD) for interpreting relative dates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_date: Option<String>,
    /// Optional caller-supplied user identifier forwarded by supporting providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Optional caller-supplied session identifier forwarded by supporting providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional trace metadata forwarded by supporting providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Value>,
    /// Provider-specific options, without routing credentials or headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_options: BTreeMap<String, Value>,
}
impl ExtractionRequest {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            ..Self::default()
        }
    }
    pub fn field(mut self, name: impl Into<String>, field: ExtractionField) -> Self {
        self.fields.insert(name.into(), field);
        self
    }
    /// Select an explicit provider account and model without interpreting model IDs.
    pub fn on(mut self, model: crate::ModelSpec) -> Self {
        self.provider = Some(model.provider);
        self.model = Some(model.model);
        self
    }
    pub fn asset(mut self, name: impl Into<String>, asset: ExtractionAsset) -> Self {
        self.assets.insert(name.into(), asset);
        self
    }
    /// Bound inline assets before any base64 decoding, including custom services.
    pub fn validate(&self) -> Result<()> {
        if self.assets.len() > 32
            || self.assets.values().fold(0usize, |total, asset| {
                total.saturating_add(asset.data_base64.len())
            }) > 32 * 1024 * 1024
        {
            return invalid("extraction assets exceed runtime limits (32 assets, 32 MiB encoded)");
        }
        if self.fields.is_empty() {
            return invalid("extraction needs at least one field");
        }
        if self.state.trim().is_empty() && self.assets.is_empty() {
            return invalid("extraction needs state or assets");
        }
        if self.model.as_ref().is_some_and(|m| m.trim().is_empty()) {
            return invalid("extraction model cannot be empty");
        }
        if self
            .reference_date
            .as_deref()
            .is_some_and(|s| !valid_date(s))
        {
            return invalid("reference_date must be an ISO date");
        }
        for (name, field) in &self.fields {
            if name.trim().is_empty() {
                return invalid("field name cannot be empty");
            }
            field.validate()?;
        }
        use base64::Engine as _;
        for (name, asset) in &self.assets {
            if name.trim().is_empty() || asset.data_base64.is_empty() {
                return invalid("asset name and data cannot be empty");
            }
            if asset.kind.is_some_and(|k| {
                (k == ExtractionAssetType::Document) != (asset.format == ExtractionAssetFormat::Pdf)
            }) {
                return invalid("asset type does not match format");
            }
            if base64::engine::general_purpose::STANDARD
                .decode(&asset.data_base64)
                .is_err()
            {
                return invalid("asset must contain standard base64 without a data URL prefix");
            }
            if !self.state.is_empty()
                && !self.state.contains(name)
                && !self
                    .fields
                    .values()
                    .any(|f| f.instructions.as_ref().is_some_and(|s| s.contains(name)))
            {
                return invalid("asset must be referenced by state or field instructions");
            }
        }
        Ok(())
    }
}

/// Unicode code-point offsets into the returned state text, or original state.
pub type ExtractionSpan = [usize; 2];
/// Which input supplied an answer. Text evidence indexes the state string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtractionSource {
    /// Evidence from state text.
    Text,
    /// Evidence from an image asset.
    Image,
    /// Evidence from a document asset.
    Document,
}
/// One typed answer with optional provider-reported evidence and uncertainty.
/// Missing probabilities remain unknown; a null value is not replaced by its guess.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExtractionAnswer {
    /// Output shape matching the requested field type.
    #[serde(rename = "type")]
    pub kind: ExtractionType,
    /// Typed extracted value, or null when absent or suppressed. Forecast arrays match the
    /// requested horizon.
    pub value: Value,
    /// Optional typed candidate retained even when value is null; not an automatic replacement for
    /// value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guess: Option<Value>,
    /// Optional half-open [start, end] evidence range in Unicode code points of returned state_text
    /// or the original state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<usize>>))]
    pub span: Option<ExtractionSpan>,
    /// Optional per-list-item evidence ranges, aligned with value items or a retained guess; null
    /// entries indicate missing evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<Option<Vec<usize>>>>))]
    pub spans: Option<Vec<Option<ExtractionSpan>>>,
    /// Optional provider-reported probability of no answer, within 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_null: Option<f64>,
    /// Optional provider-reported answer confidence, within 0..=1; absence means unknown, not
    /// certainty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Optional per-list-item confidence values within 0..=1, aligned with value items or a
    /// retained guess; null entries are unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidences: Option<Vec<Option<f64>>>,
    /// Optional provider-reported spelling confidence, within 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell_confidence: Option<f64>,
    /// Optional source media category; image and document answers may have no textual span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ExtractionSource>,
    /// Provider-reported unit of the extracted value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Optional distribution mean for an estimate, or horizon-aligned means for a forecast, using
    /// the field value type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<Value>,
    /// Optional distribution median for an estimate, or horizon-aligned medians for a forecast,
    /// using the field value type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median: Option<Value>,
    /// Optional tenth percentile for an estimate, or horizon-aligned percentiles for a forecast,
    /// using the field value type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p10: Option<Value>,
    /// Optional ninetieth percentile for an estimate, or horizon-aligned percentiles for a
    /// forecast, using the field value type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90: Option<Value>,
    /// Optional provider-reported probability below the requested lower bound, within 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_below: Option<f64>,
    /// Optional provider-reported probability above the requested upper bound, within 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_above: Option<f64>,
    /// Optional provider-specific quantile data for an estimate or forecast, preserved as JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantiles: Option<Value>,
    /// Optional provider-specific distribution step data for an estimate or forecast, preserved as
    /// JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Value>,
}

/// Provider-reported token usage, cost and extraction work.
/// Optional counters remain absent when the provider does not report them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExtractionUsage {
    /// Provider-reported input token count; defaults to zero if omitted.
    #[serde(default)]
    pub input_tokens: u64,
    /// Provider-reported output token count; defaults to zero if omitted.
    #[serde(default)]
    pub output_tokens: u64,
    /// Optional finite, nonnegative provider-reported cost; no cost is inferred when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// Optional token count attributed to input state text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_tokens: Option<u64>,
    /// Optional token count attributed to image inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<u64>,
    /// Optional token count attributed to field specifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_tokens: Option<u64>,
    /// Optional number of extraction passes reported by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passes: Option<u64>,
}
/// Validated extraction results, keyed by the requested field names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExtractionOutcome {
    /// Optional provider-assigned extraction result identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Model identifier reported for the extraction.
    pub model: String,
    /// Optional provider identifier associated with the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Exactly one typed answer per requested field, keyed by the original field names.
    pub answers: BTreeMap<String, ExtractionAnswer>,
    /// Whether the provider reports that it did not read the whole input; defaults to false if
    /// omitted.
    #[serde(default)]
    pub truncated: bool,
    /// Optional returned source text used for evidence offsets; when absent, offsets refer to the
    /// original state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<String>,
    /// Provider usage details; defaults to zero token counts and absent optional metrics when
    /// omitted.
    #[serde(default)]
    pub usage: ExtractionUsage,
}
impl ExtractionOutcome {
    /// Reject malformed provider output; absence and uncertainty stay explicit.
    pub fn validate(&self, request: &ExtractionRequest) -> Result<()> {
        if self.answers.len() != request.fields.len() {
            return invalid("extraction answer set differs from requested fields");
        }
        let length = self
            .state_text
            .as_deref()
            .unwrap_or(&request.state)
            .chars()
            .count();
        for (name, field) in &request.fields {
            let answer = self
                .answers
                .get(name)
                .ok_or_else(|| AgentLoopError::llm("missing extraction answer"))?;
            if answer.kind != field.kind || !valid_value(&answer.value, field) {
                return invalid("extraction answer has the wrong type");
            }
            if answer
                .guess
                .as_ref()
                .is_some_and(|v| !valid_value(v, field))
            {
                return invalid("extraction guess has the wrong type");
            }
            if matches!(
                field.kind,
                ExtractionType::Estimate | ExtractionType::Forecast
            ) {
                for statistic in [&answer.mean, &answer.median, &answer.p10, &answer.p90]
                    .into_iter()
                    .flatten()
                {
                    if !valid_value(statistic, field) {
                        return invalid("extraction distribution has the wrong type or horizon");
                    }
                }
            } else if answer.mean.is_some()
                || answer.median.is_some()
                || answer.p10.is_some()
                || answer.p90.is_some()
                || answer.quantiles.is_some()
                || answer.steps.is_some()
            {
                return invalid("extraction distribution requires an estimate or forecast");
            }
            for p in [
                answer.p_null,
                answer.confidence,
                answer.spell_confidence,
                answer.p_below,
                answer.p_above,
            ]
            .into_iter()
            .flatten()
            {
                if !probability(p) {
                    return invalid("extraction probability is outside zero to one");
                }
            }
            if answer
                .confidences
                .as_ref()
                .is_some_and(|c| c.iter().flatten().any(|p| !probability(*p)))
            {
                return invalid("invalid per-item extraction confidence");
            }
            if answer.span.is_some_and(|s| s[0] > s[1] || s[1] > length)
                || answer
                    .spans
                    .as_ref()
                    .is_some_and(|ss| ss.iter().flatten().any(|s| s[0] > s[1] || s[1] > length))
            {
                return invalid("extraction evidence offset is outside state text");
            }
            // Null suppression may retain a typed guess and its evidence. Never
            // manufacture an empty list when neither value nor guess has items.
            let items = answer.value.as_array().or_else(|| {
                answer
                    .value
                    .is_null()
                    .then(|| answer.guess.as_ref()?.as_array())
                    .flatten()
            });
            if let Some(spans) = &answer.spans
                && (field.kind != ExtractionType::List
                    || items.is_none_or(|v| v.len() != spans.len()))
            {
                return invalid("extraction spans must align with list items");
            }
            if let Some(confidences) = &answer.confidences
                && (field.kind != ExtractionType::List
                    || items.is_none_or(|v| v.len() != confidences.len()))
            {
                return invalid("extraction confidences must align with list items");
            }
        }
        if self.usage.cost.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return invalid("invalid extraction cost");
        }
        Ok(())
    }
}

fn probability(n: f64) -> bool {
    n.is_finite() && (0.0..=1.0).contains(&n)
}
fn valid_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
fn invalid<T>(message: &str) -> Result<T> {
    Err(AgentLoopError::llm(message))
}
fn valid_value(value: &Value, field: &ExtractionField) -> bool {
    if value.is_null() {
        return true;
    }
    use ExtractionType::*;
    match field.kind {
        Text => value.is_string(),
        Date => value.as_str().is_some_and(valid_date),
        Time => value.as_str().is_some_and(|s| {
            chrono::NaiveTime::parse_from_str(s, "%H:%M:%S%.f").is_ok()
                || chrono::NaiveTime::parse_from_str(s, "%H:%M").is_ok()
        }),
        Datetime => value.as_str().is_some_and(|s| {
            chrono::DateTime::parse_from_rfc3339(s).is_ok()
                || chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        }),
        Number | Duration => value.is_number(),
        Estimate => match field.min {
            Some(ExtractionBound::Date(_)) => value.as_str().is_some_and(valid_date),
            _ => value.is_number(),
        },
        Forecast => value.as_array().is_some_and(|values| {
            values.len() == usize::from(field.horizon.unwrap_or(0))
                && values.iter().all(|v| {
                    v.is_null()
                        || match field.min {
                            Some(ExtractionBound::Date(_)) => v.as_str().is_some_and(valid_date),
                            _ => v.is_number(),
                        }
                })
        }),
        List => value.as_array().is_some_and(|values| {
            values.iter().all(|v| match field.items {
                Some(ExtractionItemType::Text) => v.is_string(),
                Some(ExtractionItemType::Number) => v.is_number(),
                Some(ExtractionItemType::Date) => v.as_str().is_some_and(valid_date),
                None => false,
            })
        }),
    }
}

#[async_trait]
pub trait ExtractionsService: Send + Sync {
    fn is_configured(&self) -> bool;
    async fn extract(&self, request: ExtractionRequest) -> Result<ExtractionOutcome>;
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests;
