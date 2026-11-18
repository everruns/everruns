// Sandbox diagnostics: why a Sandbox is in the state it is, for people and
// for agents (Platform Chat reaches these through the command catalog).
//
// Decisions:
// - Three read-only commands. `sandboxes events list` is the decision
//   history (`sandbox_events`) beside the state history; `sandboxes provider
//   get` asks the provider about the sandbox now; `sandboxes diagnose` puts
//   both together with the pinned lifecycle policy and the session's work
//   state, as findings in the System Doctor's shape (what is true, why it
//   matters, how to fix it). Deterministic: no LLM, no tokens.
// - Asking the provider is for organization admins only (`sandbox.inspect`):
//   it spends a provider API call on the organization's credential and shows
//   the provider's view of the sandbox. `diagnose` leaves the provider part
//   out for other members and says so, instead of failing.
// - Provider answers are built field by field (`ProviderSandboxView`) and
//   their free text is scrubbed here again, so a provider error that echoes
//   a key is never returned.
// - A v2 (data plane) session's sandbox lives in its cell: its state and the
//   session's work are read from there (`session_read("sandbox")`), and the
//   control plane's idle timer and sweep do not manage it, which diagnose
//   reports as a fact.

use chrono::{DateTime, Duration, Utc};
use everruns_contracts::session_sandbox::{ProviderSandboxView, SessionSandboxCredentialSource};
use everruns_contracts::typed_id::{SandboxId, SessionId};
use everruns_core::Policy;
use everruns_core::organization::OrgRole;
use everruns_core::permissions::Rule;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::types::{SandboxFleetItem, SandboxStateSpan};
use crate::background::sandbox_idle_sweep::{STALE_TURN, SWEEP_PERIOD};
use crate::domains::common::*;
use crate::domains::sandbox_templates::record::{
    ResolvedSandboxSpec, SandboxIdleAction, SandboxTargetKind,
};
use crate::domains::sessions::SESSION_VIEW;
use crate::domains::system::doctor::CheckStatus;
use crate::storage::backend::sandbox_events::{
    MAX_SANDBOX_EVENTS, SandboxEventRow, SandboxLifecycleInputs,
};
use crate::storage::backend::sandbox_fleet::SandboxFleetRow;

/// Asking a provider about one of the organization's sandboxes.
pub const SANDBOX_INSPECT: Policy = Policy {
    id: "sandbox.inspect",
    rules: &[Rule::UserHasRole(OrgRole::Admin)],
};

/// A provider that does not answer within this is reported as unreachable.
const PROVIDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Events `diagnose` includes.
const RECENT_EVENTS: i64 = 20;
/// A turn running this long without progress is worth a look.
const LONG_TURN: Duration = Duration::hours(1);

// ============================================================================
// Shapes
// ============================================================================

/// One recorded cause: who acted on the Sandbox (or decided not to), and why.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxEvent {
    pub at: DateTime<Utc>,
    /// Incarnation the event applied to.
    pub generation: i64,
    /// `idle_timer`, `idle_sweep`, `retention_sweep` or `person`.
    #[schema(example = "idle_sweep")]
    pub actor: String,
    /// `pause`, `resume` or `delete`.
    #[schema(example = "pause")]
    pub action: String,
    /// `done`, `skipped` or `failed`.
    #[schema(example = "skipped")]
    pub outcome: String,
    /// Stable reason code, such as `idle_limit`, `stale_turn`, `session_active`,
    /// `idle_pause_off`, `retention`, `idle_action_delete` or `provider_error`.
    #[schema(example = "session_active")]
    pub reason: String,
    /// What happened, in one sentence.
    pub message: String,
    #[schema(value_type = Object)]
    pub detail: serde_json::Value,
}

impl From<SandboxEventRow> for SandboxEvent {
    fn from(row: SandboxEventRow) -> Self {
        Self {
            at: row.at,
            generation: row.generation,
            actor: row.actor,
            action: row.action,
            outcome: row.outcome,
            reason: row.reason,
            message: row.message,
            detail: row.detail,
        }
    }
}

/// A Sandbox's decisions and states over a window.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxEventList {
    pub sandbox_id: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Recorded causes, oldest first.
    pub events: Vec<SandboxEvent>,
    /// States over the same window, oldest first. A state change with no
    /// event beside it was made by an agent's sandbox call or found on the
    /// provider (a lost sandbox), not by a lifecycle decision.
    pub states: Vec<SandboxStateSpan>,
}

/// What the provider reports about the sandbox now. Built from an allowlist of
/// fields; never the provider's raw object.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProviderSandboxState {
    #[schema(example = "daytona")]
    pub provider: String,
    pub external_id: String,
    pub checked_at: DateTime<Utc>,
    /// Whether the provider still knows the sandbox.
    pub exists: bool,
    /// Provider-native state, such as `started`, `stopped` or `archived`.
    #[schema(example = "stopped")]
    pub state: String,
    /// Whether it holds compute (and so bills for it), when known.
    pub active: Option<bool>,
    pub desired_state: Option<String>,
    /// The provider's own error or exit reason.
    pub error: Option<String>,
    pub recoverable: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    /// The provider's own idle stop in minutes (0 is off). It acts on its own,
    /// independently of Everruns' idle pause.
    pub auto_stop_minutes: Option<i64>,
    pub auto_archive_minutes: Option<i64>,
    pub auto_delete_minutes: Option<i64>,
    /// When the provider ends the sandbox on its own.
    pub ends_at: Option<DateTime<Utc>>,
    /// Labels Everruns set on it.
    pub labels: std::collections::BTreeMap<String, String>,
    /// Size, region, image and similar non-secret details.
    pub details: std::collections::BTreeMap<String, String>,
}

impl ProviderSandboxState {
    fn new(provider: &str, view: ProviderSandboxView) -> Self {
        let scrub = |text: Option<String>| {
            text.map(|text| everruns_contracts::secret_scrub::scrub_secrets(&text))
        };
        Self {
            provider: provider.to_string(),
            external_id: view.external_id,
            checked_at: Utc::now(),
            exists: view.exists,
            state: view.state,
            active: view.active,
            desired_state: view.desired_state,
            error: scrub(view.error),
            recoverable: view.recoverable,
            created_at: view.created_at,
            updated_at: view.updated_at,
            last_activity_at: view.last_activity_at,
            auto_stop_minutes: view.auto_stop_minutes,
            auto_archive_minutes: view.auto_archive_minutes,
            auto_delete_minutes: view.auto_delete_minutes,
            ends_at: view.ends_at,
            labels: view.labels,
            details: view.details,
        }
    }
}

/// The lifecycle policy pinned when the Session was created, and what it
/// means for this Sandbox now.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxLifecycleStatus {
    /// Inactivity before the idle action.
    pub idle_after_seconds: u64,
    /// `checkpoint_and_stop`, `stop`, `keep_running` or `delete`.
    #[schema(example = "checkpoint_and_stop")]
    pub idle_action: String,
    /// How long a paused Sandbox is kept before deletion; `null` keeps it.
    pub delete_after_seconds: Option<u64>,
    /// Last activity, from which every deadline counts.
    pub idle_since: DateTime<Utc>,
    /// When the in-process idle timer pauses it (running only).
    pub pause_due_at: Option<DateTime<Utc>>,
    /// When the sweep pauses it if the timer did not (running only).
    pub sweep_pause_at: Option<DateTime<Utc>>,
    /// When the sweep pauses it although its session claims a running turn.
    pub stale_turn_pause_at: Option<DateTime<Utc>>,
    /// When the retention sweep deletes the provider sandbox (paused only).
    pub delete_due_at: Option<DateTime<Utc>>,
}

/// The session's work, as the runtime sees it.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct SandboxSessionWork {
    /// `active` means a turn is running.
    pub session_status: Option<String>,
    /// Where the turn is: `idle`, `queued`, `running`, ...
    pub phase: Option<String>,
    pub phase_since: Option<DateTime<Utc>>,
    /// Waiting for a worker since.
    pub queued_since: Option<DateTime<Utc>>,
    /// Inputs admitted but not yet taken by a turn.
    pub pending_inputs: Option<i64>,
    /// Whether a worker holds the turn right now.
    pub claimed: Option<bool>,
    pub claim_expires_at: Option<DateTime<Utc>>,
    pub attempts: Option<i32>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// The session runs on a v2 data plane (a cell).
    pub data_plane: bool,
}

/// One finding: what is true, why it matters, how to fix it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxFinding {
    /// Stable identifier, such as `provider.state` or `lifecycle.overdue`.
    #[schema(example = "provider.auto_stop")]
    pub id: String,
    pub status: CheckStatus,
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// Everything known about why a Sandbox is in its state.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxDiagnosis {
    pub generated_at: DateTime<Utc>,
    /// The worst status of any finding.
    pub status: CheckStatus,
    pub sandbox: SandboxFleetItem,
    pub lifecycle: Option<SandboxLifecycleStatus>,
    pub work: SandboxSessionWork,
    /// What the provider says now; absent when it was not asked.
    pub provider: Option<ProviderSandboxState>,
    /// Why the provider was not asked, or what went wrong asking it.
    pub provider_note: Option<String>,
    /// The sandbox as its cell records it (v2 sessions).
    #[schema(value_type = Option<Object>)]
    pub cell_sandbox: Option<serde_json::Value>,
    /// Problems and warnings first.
    pub findings: Vec<SandboxFinding>,
    /// The latest recorded causes, oldest first.
    pub recent_events: Vec<SandboxEvent>,
}

// ============================================================================
// Lifecycle
// ============================================================================

fn idle_action_name(action: &SandboxIdleAction) -> &'static str {
    match action {
        SandboxIdleAction::CheckpointAndStop => "checkpoint_and_stop",
        SandboxIdleAction::Stop => "stop",
        SandboxIdleAction::KeepRunning => "keep_running",
        SandboxIdleAction::Delete => "delete",
    }
}

/// The pinned spec of a Sandbox row, when it has one.
pub fn pinned_spec(inputs: &SandboxLifecycleInputs) -> Option<ResolvedSandboxSpec> {
    serde_json::from_value(inputs.profile.clone()?).ok()
}

/// Deadlines the idle timer, the idle sweep and the retention sweep apply.
/// Pure, so the rules are tested here (`background::sandbox_idle_sweep` is
/// the query side of the same rules).
pub fn lifecycle_status(inputs: &SandboxLifecycleInputs) -> Option<SandboxLifecycleStatus> {
    let lifecycle = pinned_spec(inputs)?.lifecycle;
    let idle_since = inputs.last_activity_at.unwrap_or(inputs.updated_at);
    let idle_after = Duration::seconds(lifecycle.idle_after_seconds as i64);
    let keep_running = lifecycle.idle_action == SandboxIdleAction::KeepRunning;
    let running = inputs.observed_state == "ready" && inputs.desired_state == "ready";
    let active = inputs.session_status.as_deref() == Some("active");
    let sweep = Duration::seconds(SWEEP_PERIOD.as_secs() as i64);
    let (pause_due_at, sweep_pause_at, stale_turn_pause_at) = if running && !keep_running {
        if active {
            (
                None,
                None,
                Some(idle_since + Duration::seconds(STALE_TURN.as_secs() as i64)),
            )
        } else {
            (
                Some(idle_since + idle_after),
                Some(idle_since + idle_after + sweep),
                None,
            )
        }
    } else {
        (None, None, None)
    };
    let delete_due_at = (inputs.observed_state == "paused" && !active)
        .then(|| match lifecycle.idle_action {
            SandboxIdleAction::Delete => Some(idle_since),
            _ => lifecycle
                .delete_after_seconds
                .map(|after| idle_since + Duration::seconds(after as i64)),
        })
        .flatten();
    Some(SandboxLifecycleStatus {
        idle_after_seconds: lifecycle.idle_after_seconds,
        idle_action: idle_action_name(&lifecycle.idle_action).to_string(),
        delete_after_seconds: lifecycle.delete_after_seconds,
        idle_since,
        pause_due_at,
        sweep_pause_at,
        stale_turn_pause_at,
        delete_due_at,
    })
}

// ============================================================================
// Findings
// ============================================================================

fn finding(
    id: &str,
    status: CheckStatus,
    title: &str,
    summary: impl Into<String>,
) -> SandboxFinding {
    SandboxFinding {
        id: id.to_string(),
        status,
        title: title.to_string(),
        summary: summary.into(),
        why: None,
        fix: None,
    }
}

impl SandboxFinding {
    fn why(mut self, why: impl Into<String>) -> Self {
        self.why = Some(why.into());
        self
    }
    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

/// Everything diagnosis reasons over. Pure inputs, so findings are tested
/// without a database or a provider.
pub struct DiagnosisInputs<'a> {
    pub now: DateTime<Utc>,
    pub sandbox: &'a SandboxFleetItem,
    pub lifecycle: Option<&'a SandboxLifecycleStatus>,
    pub work: &'a SandboxSessionWork,
    pub provider: Option<&'a ProviderSandboxState>,
    pub events: &'a [SandboxEvent],
    pub orphan_sighted_at: Option<DateTime<Utc>>,
}

/// The findings, problems first.
pub fn findings(input: &DiagnosisInputs<'_>) -> Vec<SandboxFinding> {
    let mut out = Vec::new();
    let sandbox = input.sandbox;
    let running = sandbox.state == "running";
    let paused = sandbox.state == "paused";

    out.push(finding(
        "record.state",
        CheckStatus::Info,
        "Everruns record",
        format!(
            "Everruns records the sandbox as {} (moving toward {}), incarnation {}.",
            sandbox.state, sandbox.desired_state, sandbox.generation
        ),
    ));

    if input.work.data_plane {
        out.push(
            finding(
                "data_plane.lifecycle",
                CheckStatus::Info,
                "Runs on a v2 data plane",
                "This session runs in a cell. The control plane's idle timer, idle sweep and retention sweep do not manage its sandbox; between turns it stops only by the provider's own idle stop.",
            )
            .why("If the provider's own idle stop is off, the sandbox keeps running (and billing) after the session goes idle.")
            .fix("Check `auto_stop_minutes` in the provider section below; pause it by hand with `everruns sessions sandbox manage --action pause` if it is running unused."),
        );
    }

    if let Some(provider) = input.provider {
        provider_findings(
            &mut out,
            sandbox,
            input.lifecycle,
            provider,
            running,
            paused,
        );
    }

    if let Some(lifecycle) = input.lifecycle {
        lifecycle_findings(&mut out, input, lifecycle, running);
    }

    if input.work.session_status.as_deref() == Some("active") {
        let since = input.work.phase_since;
        let long = since.is_some_and(|since| input.now - since > LONG_TURN);
        let unclaimed = input.work.claimed == Some(false)
            || input.work.claim_expires_at.is_some_and(|at| at < input.now);
        if long && unclaimed {
            out.push(
                finding(
                    "session.stuck_turn",
                    CheckStatus::Warning,
                    "Turn may be stuck",
                    format!(
                        "The session has claimed a running turn since {} and no worker holds it now.",
                        since.map_or("an unknown time".to_string(), |s| s.to_rfc3339())
                    ),
                )
                .why("A turn whose worker stopped keeps its session active; its sandbox is then paused only after 6 hours unused.")
                .fix("Cancel the turn (`everruns sessions cancel`), or send a message to start a fresh one."),
            );
        }
    }

    if let Some(failed) = input.events.iter().rev().find(|e| e.outcome == "failed") {
        let later_done = input
            .events
            .iter()
            .any(|e| e.at > failed.at && e.action == failed.action && e.outcome == "done");
        if !later_done {
            out.push(
                finding(
                    "events.failure",
                    CheckStatus::Problem,
                    "Last lifecycle action failed",
                    format!("{} at {}: {}", failed.actor, failed.at.to_rfc3339(), failed.message),
                )
                .why("A failed pause or delete is retried by the next sweep, every minute; repeated failures mean the provider is refusing the call.")
                .fix("Compare with the provider state below. A key that lost access shows here as a provider error."),
            );
        }
    }

    if let Some(error) = sandbox.last_init_error.as_deref() {
        out.push(
            finding("record.init_failed", CheckStatus::Warning, "Init commands failed", error.to_string())
                .why("The sandbox runs, but without what the template's bootstrap commands set up.")
                .fix("Fix the bootstrap commands in the Sandbox Template; the next new sandbox runs them again."),
        );
    }

    if sandbox.generation > 1 {
        out.push(finding(
            "record.rebuilt",
            CheckStatus::Info,
            "Rebuilt",
            format!(
                "The provider resource was replaced {} time(s): each replacement follows a lost or deleted sandbox.",
                sandbox.generation - 1
            ),
        ));
    }

    if let Some(at) = input.orphan_sighted_at {
        out.push(
            finding(
                "orphan.sighting",
                CheckStatus::Warning,
                "Seen as untracked",
                format!("The hourly orphan scan found this provider sandbox untracked by any live record at {}.", at.to_rfc3339()),
            )
            .why("The orphan scan stops an untracked sandbox and deletes it a day later."),
        );
    }

    out.sort_by_key(|f| match f.status {
        CheckStatus::Problem => 0,
        CheckStatus::Warning => 1,
        CheckStatus::Ok => 2,
        CheckStatus::Info => 3,
    });
    out
}

fn provider_findings(
    out: &mut Vec<SandboxFinding>,
    sandbox: &SandboxFleetItem,
    lifecycle: Option<&SandboxLifecycleStatus>,
    provider: &ProviderSandboxState,
    running: bool,
    paused: bool,
) {
    let name = &provider.provider;
    if !provider.exists {
        if running || paused {
            out.push(
                finding(
                    "provider.missing",
                    CheckStatus::Warning,
                    "Gone at the provider",
                    format!("{name} no longer knows sandbox {}, but Everruns records it as {}.", provider.external_id, sandbox.state),
                )
                .why("The next sandbox call finds it lost and builds a new one, restoring the workspace from its last checkpoint.")
                .fix("Nothing to do unless the work in it mattered; check the provider's own auto-delete setting."),
            );
        }
        return;
    }
    match provider.active {
        Some(false) if running => out.push(
            finding(
                "provider.stopped_early",
                CheckStatus::Info,
                "Stopped by the provider",
                format!("Everruns records the sandbox as running, but {name} reports it {}.", provider.state),
            )
            .why(match provider.auto_stop_minutes {
                Some(minutes) if minutes > 0 => format!("{name} stops a sandbox after {minutes} minutes without activity on its own; Everruns notices on its next call to it and resumes or pauses it from there."),
                _ => format!("{name} stopped it on its own; Everruns notices on its next call to it."),
            }),
        ),
        Some(true) if paused => out.push(
            finding(
                "provider.still_running",
                CheckStatus::Problem,
                "Still running at the provider",
                format!("Everruns records the sandbox as paused, but {name} reports it {}.", provider.state),
            )
            .why("It holds compute and bills until something stops it.")
            .fix("Pause it again: `everruns sessions sandbox manage --action pause`. If that fails, the provider error is in the events."),
        ),
        _ => {}
    }
    if let Some(error) = provider.error.as_deref() {
        out.push(
            finding(
                "provider.error",
                if provider.recoverable == Some(true) { CheckStatus::Warning } else { CheckStatus::Problem },
                "Provider reports an error",
                format!("{name}: {error}"),
            )
            .why("The provider reports this sandbox in error; Everruns' next call to it may fail or rebuild it."),
        );
    }
    if let (Some(minutes), Some(lifecycle)) = (provider.auto_stop_minutes, lifecycle) {
        let idle_minutes = lifecycle.idle_after_seconds.div_ceil(60) as i64;
        if minutes == 0 {
            out.push(finding(
                "provider.auto_stop",
                CheckStatus::Info,
                "Provider idle stop is off",
                format!(
                    "{name}'s own idle stop is off, so only Everruns' idle pause ({} s) stops it.",
                    lifecycle.idle_after_seconds
                ),
            ));
        } else if minutes < idle_minutes {
            out.push(finding(
                "provider.auto_stop",
                CheckStatus::Info,
                "Provider stops it first",
                format!(
                    "{name} stops it after {minutes} min without activity, before Everruns' idle pause ({} s). A stop Everruns did not make then shows as a state change with no event beside it.",
                    lifecycle.idle_after_seconds
                ),
            ));
        }
    }
}

fn lifecycle_findings(
    out: &mut Vec<SandboxFinding>,
    input: &DiagnosisInputs<'_>,
    lifecycle: &SandboxLifecycleStatus,
    running: bool,
) {
    if running && lifecycle.idle_action == "keep_running" {
        out.push(finding(
            "lifecycle.keep_running",
            CheckStatus::Info,
            "Kept running on purpose",
            "The pinned idle action is keep_running: Everruns never pauses it for idleness.",
        ));
    }
    if let Some(at) = lifecycle.sweep_pause_at
        && input.now > at + Duration::minutes(2)
    {
        out.push(
            finding(
                "lifecycle.overdue",
                CheckStatus::Warning,
                "Overdue for idle pause",
                format!(
                    "Idle since {}; the idle sweep should have paused it by {}.",
                    lifecycle.idle_since.to_rfc3339(),
                    at.to_rfc3339()
                ),
            )
            .why("It keeps holding compute. The sweep runs every minute, so a sandbox overdue for longer is being skipped or failing.")
            .fix("Look at the recent events for a failed or skipped pause; a platform operator can check the sweep's runs with `everruns durable schedules list` (schedule `sandbox-idle-sweep`) and `everruns durable schedules executions list`."),
        );
    }
    if let Some(at) = lifecycle.stale_turn_pause_at {
        out.push(finding(
            "lifecycle.turn_running",
            CheckStatus::Info,
            "Kept while a turn runs",
            format!("Its session is running a turn, so idle pause waits; if the sandbox stays unused, the sweep pauses it at {}.", at.to_rfc3339()),
        ));
    }
    if let Some(at) = lifecycle.delete_due_at {
        out.push(finding(
            "lifecycle.delete_due",
            CheckStatus::Info,
            "Deletion scheduled",
            format!("The retention sweep deletes the paused provider sandbox at or after {} (the record stays).", at.to_rfc3339()),
        ));
    }
}

/// The worst status, `Info` when there are no findings.
pub fn worst(findings: &[SandboxFinding]) -> CheckStatus {
    findings
        .iter()
        .map(|f| f.status)
        .filter(|s| *s != CheckStatus::Info)
        .max()
        .unwrap_or(CheckStatus::Ok)
}

// ============================================================================
// Reads
// ============================================================================

fn parse_sandbox_id(id: &str) -> Result<SandboxId, CommandError> {
    id.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid Sandbox ID: {e}")))
}

async fn load_row(
    ctx: &Ctx,
    id: SandboxId,
) -> Result<(SandboxFleetRow, SandboxLifecycleInputs), CommandError> {
    let row = ctx
        .db
        .get_sandbox_fleet_row(ctx.org_id(), id.uuid())
        .await?
        .ok_or_else(|| CommandError::not_found("Sandbox"))?;
    let inputs = ctx
        .db
        .get_sandbox_lifecycle_inputs(ctx.org_id(), id.uuid())
        .await?
        .ok_or_else(|| CommandError::not_found("Sandbox"))?;
    Ok((row, inputs))
}

/// Ask the provider about the Sandbox's latest provider resource.
pub(crate) async fn ask_provider(
    ctx: &Ctx,
    row: &SandboxFleetRow,
    inputs: &SandboxLifecycleInputs,
    external_id: Option<&str>,
) -> Result<ProviderSandboxState, String> {
    let external_id = external_id
        .or(row.external_id.as_deref())
        .ok_or("The sandbox was never created at its provider.")?;
    let spec = pinned_spec(inputs).ok_or("The sandbox has no pinned specification.")?;
    if spec.target.kind != SandboxTargetKind::Managed {
        return Err("Only managed provider sandboxes can be asked; this one runs in process or on a registered machine.".into());
    }
    if spec.target.options.get("api_base").is_some() {
        return Err("The sandbox's template sends its provider calls to a custom endpoint, which diagnostics does not call.".into());
    }
    let provider_id = spec
        .target
        .provider
        .clone()
        .unwrap_or_else(|| row.provider.clone());
    let provider =
        everruns_contracts::session_sandbox::create_session_sandbox_provider(&provider_id)
            .ok_or_else(|| {
                format!("The {provider_id} provider is not available on this install.")
            })?;
    let token = provider_token(ctx, row, &spec, &provider_id).await?;
    let answer = tokio::time::timeout(
        PROVIDER_TIMEOUT,
        provider.describe_by_id(&token, external_id),
    )
    .await
    .map_err(|_| {
        format!(
            "{provider_id} did not answer within {} s.",
            PROVIDER_TIMEOUT.as_secs()
        )
    })?;
    match answer {
        Ok(view) => Ok(ProviderSandboxState::new(&provider_id, view)),
        Err(everruns_contracts::session_sandbox::SandboxAdminError::Unsupported) => Err(format!(
            "The {provider_id} provider cannot describe a sandbox."
        )),
        Err(everruns_contracts::session_sandbox::SandboxAdminError::Provider(error)) => {
            Err(format!(
                "{provider_id} refused: {}",
                everruns_contracts::secret_scrub::scrub_secrets(&error)
            ))
        }
    }
}

/// The provider key the Sandbox's session was pinned with, resolved as the
/// session's own sandbox calls resolve it.
async fn provider_token(
    ctx: &Ctx,
    row: &SandboxFleetRow,
    spec: &ResolvedSandboxSpec,
    provider: &str,
) -> Result<String, String> {
    if matches!(
        spec.target.credential.source,
        SessionSandboxCredentialSource::None
    ) {
        return Ok(String::new());
    }
    let session = row
        .session_id
        .map(SessionId::from_uuid)
        .ok_or("The sandbox's session is deleted, so its provider key cannot be resolved.")?;
    let encryption = ctx
        .encryption
        .as_ref()
        .ok_or("Provider keys cannot be decrypted on this install (no encryption key).")?;
    let resolver = crate::storage::DbConnectionResolver::new(
        ctx.db.as_ref().clone(),
        encryption.as_ref().clone(),
        None,
        std::sync::Arc::new(
            everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
        ),
    );
    use everruns_core::UserConnectionResolver as _;
    resolver
        .get_sandbox_connection_token(session, provider, &spec.target.credential)
        .await
        .map_err(|e| {
            everruns_contracts::secret_scrub::scrub_secrets(&format!(
                "Could not resolve the provider key: {e}"
            ))
        })?
        .ok_or_else(|| format!("No {provider} account is connected for this sandbox's credential."))
}

/// The session's work state, from the control plane (v1) or its cell (v2).
async fn session_work(
    ctx: &Ctx,
    session: Option<uuid::Uuid>,
) -> (SandboxSessionWork, Option<serde_json::Value>) {
    let Some(session) = session else {
        return (SandboxSessionWork::default(), None);
    };
    let id = SessionId::from_uuid(session);
    if let Ok(Some(plane)) =
        crate::domains::data_plane::session_plane::SessionPlane::of(&ctx.db, ctx.org_id(), id).await
    {
        let read = plane
            .plane
            .session_read(&plane.org, id, "sandbox", "")
            .await;
        let (work, sandbox) = match read {
            Ok(value) => cell_work(&value),
            Err(error) => {
                let mut work = SandboxSessionWork {
                    data_plane: true,
                    ..Default::default()
                };
                work.phase = Some(format!("unknown: the cell did not answer ({error})"));
                (work, None)
            }
        };
        return (work, sandbox);
    }
    let actor = ctx
        .db
        .monitoring_actor(Some(ctx.org_id()), Utc::now(), session)
        .await
        .ok()
        .flatten();
    let work = match actor {
        Some(a) => SandboxSessionWork {
            session_status: Some(a.session_status),
            phase: Some(a.phase),
            phase_since: Some(a.phase_since),
            queued_since: a.runnable_since,
            pending_inputs: match (a.inbox_seq, a.consumed_seq) {
                (Some(inbox), Some(consumed)) => Some((inbox - consumed).max(0)),
                _ => None,
            },
            claimed: a.leased.map(|leased| leased && a.expired != Some(true)),
            claim_expires_at: a.lease_expires_at,
            attempts: a.attempts,
            cancel_requested_at: a.cancel_requested_at,
            data_plane: false,
        },
        None => SandboxSessionWork {
            session_status: ctx
                .db
                .get_session_unscoped(id)
                .await
                .ok()
                .flatten()
                .map(|s| s.status),
            ..Default::default()
        },
    };
    (work, None)
}

/// A cell's `sandbox` read: `{ "work": {...}, "sandbox": {...} | null }`.
fn cell_work(value: &serde_json::Value) -> (SandboxSessionWork, Option<serde_json::Value>) {
    let work = &value["work"];
    let ms = |v: &serde_json::Value| v.as_i64().and_then(DateTime::from_timestamp_millis);
    let running = work["turn_running"].as_bool();
    (
        SandboxSessionWork {
            session_status: running.map(|r| if r { "active" } else { "idle" }.to_string()),
            phase: work["phase"].as_str().map(str::to_string),
            phase_since: None,
            queued_since: ms(&work["runnable_since_ms"]),
            pending_inputs: work["inbox"].as_i64(),
            claimed: work["claimed"].as_bool(),
            claim_expires_at: None,
            attempts: work["attempts"].as_i64().map(|a| a as i32),
            cancel_requested_at: None,
            data_plane: true,
        },
        value.get("sandbox").filter(|s| !s.is_null()).cloned(),
    )
}

// ============================================================================
// Commands
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListSandboxEvents {
    /// Logical Sandbox id.
    #[param(ignore)]
    pub id: String,
    /// Window start (RFC 3339). Defaults to the Sandbox's creation.
    pub from: Option<DateTime<Utc>>,
    /// Window end (RFC 3339). Defaults to now.
    pub to: Option<DateTime<Utc>>,
    /// Most events, 1 to 500 (the newest when there are more). Defaults to 200.
    pub limit: Option<i64>,
}

#[command(
    name = "list_sandbox_events",
    category = "sandboxes",
    description = "List why a Sandbox changed state, or did not: each pause, resume and delete by the idle timer, idle sweep, retention sweep or a person, each decision to skip, and each failure, beside the state history.",
    summary = "List the decisions behind a Sandbox's states",
    method = "GET",
    path = "/v1/sandboxes/{id}/events",
    policy = SESSION_VIEW,
    http = plain,
    params(ListSandboxEvents),
    cli = CliRoute::new(&["sandboxes", "events"], "list")
        .with_args(&[CliArg::new("id").at(1)])
        .with_examples(&[CliExample::new(
            "See who paused a Sandbox and why, since a given time",
            "everruns sandboxes events list sandbox_01h9 --from 2026-11-09T02:25:00Z",
        )]),
    responses((status = 404, description = "Sandbox not found")),
)]
impl Command for ListSandboxEvents {
    type Output = SandboxEventList;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxEventList, CommandError> {
        let id = parse_sandbox_id(&self.id)?;
        let (row, _) = load_row(ctx, id).await?;
        let to = self.to.unwrap_or_else(Utc::now);
        let from = self.from.unwrap_or(row.created_at);
        if from >= to {
            return Err(CommandError::bad_request("`from` must be before `to`"));
        }
        let limit = self.limit.unwrap_or(200).clamp(1, MAX_SANDBOX_EVENTS);
        let events = ctx
            .db
            .list_sandbox_events(ctx.org_id(), id.uuid(), from, to, limit)
            .await?;
        let states = ctx
            .db
            .list_sandbox_transitions(ctx.org_id(), Some(id.uuid()), from, to)
            .await?;
        Ok(SandboxEventList {
            sandbox_id: id.to_string(),
            from,
            to,
            events: events.into_iter().map(Into::into).collect(),
            states: states.into_iter().map(Into::into).collect(),
        })
    }
}

/// Ask a Sandbox's provider what it reports now.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetSandboxProviderState {
    /// Logical Sandbox id.
    pub id: String,
}

#[command(
    name = "get_sandbox_provider_state",
    category = "sandboxes",
    description = "Ask the Sandbox's provider (Daytona, E2B, Modal, boxd, Kubernetes) what it reports now: its state, errors, last activity and its own idle stop and delete settings. Read-only; organization admins only. Secrets in the provider's record are never returned.",
    summary = "Ask the provider about a Sandbox now",
    method = "GET",
    path = "/v1/sandboxes/{id}/provider",
    policy = SANDBOX_INSPECT,
    positional = "id",
    http = plain,
    cli = CliRoute::new(&["sandboxes", "provider"], "get")
        .with_args(&[CliArg::new("id").at(1)])
        .with_examples(&[CliExample::new(
            "Check whether Daytona stopped a Sandbox on its own",
            "everruns sandboxes provider get sandbox_01h9",
        )]),
    responses(
        (status = 404, description = "Sandbox not found"),
        (status = 503, description = "The provider could not be asked")
    ),
)]
impl Command for GetSandboxProviderState {
    type Output = ProviderSandboxState;

    async fn execute(self, ctx: &Ctx) -> Result<ProviderSandboxState, CommandError> {
        let id = parse_sandbox_id(&self.id)?;
        let (row, inputs) = load_row(ctx, id).await?;
        let external_id = match row.external_id.clone() {
            Some(id) => Some(id),
            None => cell_external_id(ctx, row.session_id).await,
        };
        ask_provider(ctx, &row, &inputs, external_id.as_deref())
            .await
            .map_err(CommandError::unavailable)
    }
}

/// The provider id a v2 session's cell recorded for its sandbox.
async fn cell_external_id(ctx: &Ctx, session: Option<uuid::Uuid>) -> Option<String> {
    let (_, sandbox) = session_work(ctx, session).await;
    sandbox?["external_id"].as_str().map(str::to_string)
}

/// Explain a Sandbox's state.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnoseSandbox {
    /// Logical Sandbox id.
    pub id: String,
}

#[command(
    name = "diagnose_sandbox",
    category = "sandboxes",
    description = "Explain why a Sandbox is in its state: its record against what the provider reports, the pinned lifecycle policy and its deadlines, whether its session is running or stuck in a turn, the latest lifecycle decisions and failures, as findings with what to do. Deterministic. The provider is asked only for organization admins.",
    summary = "Explain why a Sandbox is in its state",
    method = "GET",
    path = "/v1/sandboxes/{id}/diagnosis",
    policy = SESSION_VIEW,
    positional = "id",
    http = plain,
    cli = CliRoute::new(&["sandboxes"], "diagnose")
        .with_args(&[CliArg::new("id").at(1)])
        .with_examples(&[CliExample::new(
            "Find out why a Sandbox did not pause",
            "everruns sandboxes diagnose sandbox_01h9",
        )]),
    responses((status = 404, description = "Sandbox not found")),
)]
impl Command for DiagnoseSandbox {
    type Output = SandboxDiagnosis;

    async fn execute(self, ctx: &Ctx) -> Result<SandboxDiagnosis, CommandError> {
        let id = parse_sandbox_id(&self.id)?;
        let (row, inputs) = load_row(ctx, id).await?;
        let now = Utc::now();
        let (work, cell_sandbox) = session_work(ctx, row.session_id).await;
        let lifecycle = if work.data_plane {
            None
        } else {
            lifecycle_status(&inputs)
        };
        let external_id = row.external_id.clone().or_else(|| {
            cell_sandbox.as_ref()?["external_id"]
                .as_str()
                .map(str::to_string)
        });
        let may_inspect = SANDBOX_INSPECT
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .is_ok();
        let (provider, provider_note) = if !may_inspect {
            (
                None,
                Some(
                    "Only organization admins can ask the provider; the provider part is left out."
                        .to_string(),
                ),
            )
        } else {
            match ask_provider(ctx, &row, &inputs, external_id.as_deref()).await {
                Ok(state) => (Some(state), None),
                Err(note) => (None, Some(note)),
            }
        };
        let events: Vec<SandboxEvent> = ctx
            .db
            .list_sandbox_events(
                ctx.org_id(),
                id.uuid(),
                row.created_at,
                now + Duration::seconds(1),
                RECENT_EVENTS,
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        let orphan_sighted_at = match &external_id {
            Some(external) => ctx
                .db
                .get_orphan_sighting(&row.provider, external)
                .await?
                .map(|s| s.first_seen_at),
            None => None,
        };
        let sandbox = SandboxFleetItem::from(row);
        let found = findings(&DiagnosisInputs {
            now,
            sandbox: &sandbox,
            lifecycle: lifecycle.as_ref(),
            work: &work,
            provider: provider.as_ref(),
            events: &events,
            orphan_sighted_at,
        });
        Ok(SandboxDiagnosis {
            generated_at: now,
            status: worst(&found),
            sandbox,
            lifecycle,
            work,
            provider,
            provider_note,
            cell_sandbox,
            findings: found,
            recent_events: events,
        })
    }
}

#[cfg(test)]
mod tests;
