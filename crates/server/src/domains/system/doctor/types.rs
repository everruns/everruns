use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// How a check came out. Ordered from best to worst so the report's verdict
/// is the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Working as intended.
    Ok,
    /// Not used by this install, or a fact worth knowing. Never a fault.
    Info,
    /// Works, but degraded or risky.
    Warning,
    /// Broken or unsafe now.
    Problem,
}

impl CheckStatus {
    /// Position in a report: what needs attention first, notes last.
    pub(super) fn display_rank(self) -> u8 {
        match self {
            Self::Problem => 0,
            Self::Warning => 1,
            Self::Ok => 2,
            Self::Info => 3,
        }
    }
}

/// The part of the install a check covers. The order is the display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckArea {
    ControlPlane,
    Database,
    Workers,
    Security,
    Messaging,
    Storage,
    Observability,
    Models,
}

/// One check's outcome, written for an operator: what is true, why it
/// matters, and what to do.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DoctorCheck {
    /// Stable identifier, `area.name`.
    #[schema(example = "database.migrations")]
    pub id: String,
    pub area: CheckArea,
    /// Short name of what is checked.
    #[schema(example = "Migrations applied")]
    pub title: String,
    pub status: CheckStatus,
    /// What the check found, in one sentence.
    #[schema(example = "All 14 migrations are applied.")]
    pub summary: String,
    /// What goes wrong if this is not fixed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// How to fix it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// When the check observed this.
    pub observed_at: DateTime<Utc>,
}

/// The whole install's health.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DoctorReport {
    /// The worst status of any check.
    pub status: CheckStatus,
    /// When the checks ran.
    pub checked_at: DateTime<Utc>,
    /// Server build version.
    #[schema(example = "0.50.0")]
    pub version: String,
    /// Deployment grade (`dev`, `poc`, `preview`, `prod`). Decides how strict
    /// the security checks are.
    #[schema(example = "prod")]
    pub grade: String,
    /// Authentication mode.
    #[schema(example = "full")]
    pub auth_mode: String,
    /// What the doctor found this install is made of, one fact per entry.
    #[schema(example = json!(["Postgres 16.4", "3 workers", "NATS", "S3 blob storage"]))]
    pub setup: Vec<String>,
    /// Every check, problems first.
    pub checks: Vec<DoctorCheck>,
}

impl DoctorCheck {
    pub(super) fn new(area: CheckArea, id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            area,
            title: title.to_string(),
            status: CheckStatus::Ok,
            summary: String::new(),
            why: None,
            fix: None,
            observed_at: Utc::now(),
        }
    }

    fn with(mut self, status: CheckStatus, summary: impl Into<String>) -> Self {
        self.status = status;
        self.summary = summary.into();
        self
    }

    pub(super) fn ok(self, summary: impl Into<String>) -> Self {
        self.with(CheckStatus::Ok, summary)
    }

    pub(super) fn info(self, summary: impl Into<String>) -> Self {
        self.with(CheckStatus::Info, summary)
    }

    pub(super) fn warning(self, summary: impl Into<String>) -> Self {
        self.with(CheckStatus::Warning, summary)
    }

    pub(super) fn problem(self, summary: impl Into<String>) -> Self {
        self.with(CheckStatus::Problem, summary)
    }

    /// A problem in production, a warning on a lower grade. Used for settings
    /// that are fine to skip on a laptop and unsafe on a real install.
    pub(super) fn problem_if(self, strict: bool, summary: impl Into<String>) -> Self {
        let status = if strict {
            CheckStatus::Problem
        } else {
            CheckStatus::Warning
        };
        self.with(status, summary)
    }

    pub(super) fn why(mut self, why: impl Into<String>) -> Self {
        self.why = Some(why.into());
        self
    }

    pub(super) fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}
